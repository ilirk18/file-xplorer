// Destructive filesystem work, and clipboard interop.
//
// Everything routes through the shell's IFileOperation rather than std::fs.
// That is not ceremony: it is what gives us the Recycle Bin, the progress
// dialog, per-file conflict prompts ("replace / skip / keep both"), the
// elevation prompt when a path needs admin rights, and correct handling of
// junctions and symlinks. The previous hand-rolled recursive copy silently
// truncated existing destinations and would follow a junction loop until the
// stack ran out.
//
// Operations run on a worker thread so a slow or network volume can never
// freeze the window.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_DIRECTORY;
use windows::Win32::System::Com::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_HDROP;
use windows::Win32::UI::Shell::*;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A window handle handed to a worker thread purely so the shell can parent its
/// progress and conflict dialogs. HWNDs are not `Send` because most Win32 calls
/// on them must happen on the owning thread; `SetOwnerWindow` is documented to
/// accept a handle from another thread, so the wrapper is sound for this use.
#[derive(Clone, Copy)]
pub struct OwnerWindow(pub HWND);
unsafe impl Send for OwnerWindow {}

impl OwnerWindow {
    /// Read the handle through a method, not a field. Edition-2021 closures
    /// capture disjoint fields, so `owner.0` inside a `move` closure would
    /// capture the bare non-Send HWND and defeat the wrapper.
    pub fn hwnd(self) -> HWND {
        self.0
    }
}

/// What the user asked for. Paths are absolute.
#[derive(Clone, Debug)]
pub enum Op {
    Copy {
        sources: Vec<String>,
        dest_dir: String,
    },
    Move {
        sources: Vec<String>,
        dest_dir: String,
    },
    Delete {
        sources: Vec<String>,
        /// Skip the Recycle Bin. The shell still shows its own "are you sure"
        /// warning because we set FOF_WANTNUKEWARNING.
        permanent: bool,
    },
    Rename {
        source: String,
        new_name: String,
    },
    /// Several renames in one operation, so a batch rename is a single undo
    /// step rather than N of them.
    RenameMany {
        /// (absolute source path, new bare name)
        items: Vec<(String, String)>,
    },
    NewFolder {
        parent: String,
        name: String,
    },
}

impl Op {
    /// Directories whose listings this operation could change, so the UI knows
    /// which panes to refresh when it finishes.
    pub fn affected_dirs(&self) -> Vec<String> {
        match self {
            Op::Copy { sources, dest_dir } | Op::Move { sources, dest_dir } => {
                let mut v = vec![dest_dir.clone()];
                v.extend(sources.iter().filter_map(|s| crate::fs::path_parent(s)));
                v
            }
            Op::Delete { sources, .. } => {
                sources.iter().filter_map(|s| crate::fs::path_parent(s)).collect()
            }
            Op::Rename { source, .. } => {
                crate::fs::path_parent(source).into_iter().collect()
            }
            Op::RenameMany { items } => items
                .iter()
                .filter_map(|(src, _)| crate::fs::path_parent(src))
                .collect(),
            Op::NewFolder { parent, .. } => vec![parent.clone()],
        }
    }
}

/// Result of one completed operation, delivered back to the UI thread.
pub struct OpResult {
    pub op: Op,
    /// None on success. Some(message) when the shell reported a failure; the
    /// user having cancelled is reported as success with `aborted` set.
    pub error: Option<String>,
    pub aborted: bool,
}

fn shell_item(path: &str) -> windows::core::Result<IShellItem> {
    let wide = to_wide(path);
    unsafe { SHCreateItemFromParsingName(PCWSTR::from_raw(wide.as_ptr()), None) }
}

/// Build and run one IFileOperation. Blocking; call on a worker thread that has
/// already entered a single-threaded apartment.
fn perform(op: &Op, owner: HWND) -> windows::core::Result<bool> {
    let file_op: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL)? };

    // FOF_NOCONFIRMMKDIR: creating intermediate folders is implied by the copy,
    // not something to ask about. Everything else stays interactive on purpose.
    let mut flags = FOF_NOCONFIRMMKDIR.0 | FOFX_ADDUNDORECORD.0 | FOFX_SHOWELEVATIONPROMPT.0;
    match op {
        Op::Delete { permanent: true, .. } => {
            flags |= FOF_WANTNUKEWARNING.0;
        }
        Op::Delete { permanent: false, .. } => {
            flags |= FOF_ALLOWUNDO.0 | FOFX_RECYCLEONDELETE.0;
        }
        _ => {
            flags |= FOF_ALLOWUNDO.0;
        }
    }
    unsafe {
        file_op.SetOperationFlags(FILEOPERATION_FLAGS(flags))?;
        file_op.SetOwnerWindow(owner)?;
    }

    match op {
        Op::Copy { sources, dest_dir } => {
            let dest = shell_item(dest_dir)?;
            for src in sources {
                let item = shell_item(src)?;
                unsafe { file_op.CopyItem(&item, &dest, PCWSTR::null(), None)? };
            }
        }
        Op::Move { sources, dest_dir } => {
            let dest = shell_item(dest_dir)?;
            for src in sources {
                let item = shell_item(src)?;
                unsafe { file_op.MoveItem(&item, &dest, PCWSTR::null(), None)? };
            }
        }
        Op::Delete { sources, .. } => {
            for src in sources {
                let item = shell_item(src)?;
                unsafe { file_op.DeleteItem(&item, None)? };
            }
        }
        Op::Rename { source, new_name } => {
            let item = shell_item(source)?;
            let name = to_wide(new_name);
            unsafe { file_op.RenameItem(&item, PCWSTR::from_raw(name.as_ptr()), None)? };
        }
        Op::RenameMany { items } => {
            for (source, new_name) in items {
                let item = shell_item(source)?;
                let name = to_wide(new_name);
                unsafe { file_op.RenameItem(&item, PCWSTR::from_raw(name.as_ptr()), None)? };
            }
        }
        Op::NewFolder { parent, name } => {
            let dest = shell_item(parent)?;
            let name = to_wide(name);
            unsafe {
                file_op.NewItem(
                    &dest,
                    FILE_ATTRIBUTE_DIRECTORY.0,
                    PCWSTR::from_raw(name.as_ptr()),
                    PCWSTR::null(),
                    None,
                )?
            };
        }
    }

    unsafe {
        file_op.PerformOperations()?;
        Ok(file_op.GetAnyOperationsAborted()?.as_bool())
    }
}

/// Run `op` on a worker thread, then hand the boxed result to `deliver` (which
/// is expected to PostMessage it to the UI thread and return immediately).
pub fn spawn<F>(op: Op, owner: OwnerWindow, deliver: F)
where
    F: FnOnce(Box<OpResult>) + Send + 'static,
{
    std::thread::spawn(move || {
        // IFileOperation requires an STA: its progress UI is a window.
        let com = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let result = match perform(&op, owner.hwnd()) {
            Ok(aborted) => OpResult {
                op,
                error: None,
                aborted,
            },
            Err(e) => OpResult {
                op,
                error: Some(format_hresult(&e)),
                aborted: false,
            },
        };
        if com.is_ok() {
            unsafe { CoUninitialize() };
        }
        deliver(Box::new(result));
    });
}

/// Turn an HRESULT into something worth showing a person.
pub fn format_hresult(e: &windows::core::Error) -> String {
    let msg = e.message();
    if msg.trim().is_empty() {
        format!("Operation failed (0x{:08X}).", e.code().0)
    } else {
        msg
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

/// Whether a clipboard file set was cut or copied. Matches the shell's
/// DROPEFFECT values so Explorer understands what we put there, and vice versa.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DropEffect {
    Copy = 1,
    Move = 2,
}

/// Explorer marks a cut-vs-copy with this registered format. Registering the
/// same name gets us the same id, which is how interop works.
fn preferred_drop_effect_format() -> u32 {
    let name = to_wide("Preferred DropEffect");
    unsafe { RegisterClipboardFormatW(PCWSTR::from_raw(name.as_ptr())) }
}

/// Copy or cut a set of paths to the system clipboard as CF_HDROP, so the files
/// can be pasted into Explorer (and Explorer's own cut/copy can be pasted here).
pub fn clipboard_write(owner: HWND, paths: &[String], effect: DropEffect) -> windows::core::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    // CF_HDROP payload: a DROPFILES header followed by the wide paths, each
    // NUL-terminated, with a second NUL closing the list.
    let mut chars: Vec<u16> = Vec::new();
    for p in paths {
        chars.extend(p.encode_utf16());
        chars.push(0);
    }
    chars.push(0);

    let header = std::mem::size_of::<DROPFILES>();
    let bytes = header + chars.len() * 2;

    unsafe {
        OpenClipboard(Some(owner))?;
        // Everything below must run before CloseClipboard, and the clipboard
        // must be closed on every path out.
        let result = (|| -> windows::core::Result<()> {
            EmptyClipboard()?;

            let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let ptr = GlobalLock(hglobal);
            if ptr.is_null() {
                return Err(windows::core::Error::from_thread());
            }
            let df = ptr as *mut DROPFILES;
            (*df).pFiles = header as u32;
            (*df).fWide = true.into();
            std::ptr::copy_nonoverlapping(
                chars.as_ptr(),
                (ptr as *mut u8).add(header) as *mut u16,
                chars.len(),
            );
            let _ = GlobalUnlock(hglobal);
            SetClipboardData(CF_HDROP.0 as u32, Some(HANDLE(hglobal.0)))?;

            // Tell the receiver whether this was a cut or a copy.
            let fmt = preferred_drop_effect_format();
            if fmt != 0 {
                let eff = GlobalAlloc(GMEM_MOVEABLE, 4)?;
                let eptr = GlobalLock(eff);
                if !eptr.is_null() {
                    *(eptr as *mut u32) = effect as u32;
                    let _ = GlobalUnlock(eff);
                    SetClipboardData(fmt, Some(HANDLE(eff.0)))?;
                }
            }
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Pull the path list out of an HDROP. Shared by the clipboard and by
/// drag-and-drop, which receive the same structure from different places.
pub fn paths_from_hdrop(hdrop: HDROP) -> Vec<String> {
    unsafe {
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        let mut paths = Vec::with_capacity(count as usize);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None);
            if len == 0 {
                continue;
            }
            let mut buf = vec![0u16; len as usize + 1];
            let written = DragQueryFileW(hdrop, i, Some(&mut buf));
            if written > 0 {
                paths.push(String::from_utf16_lossy(&buf[..written as usize]));
            }
        }
        paths
    }
}

/// Read a CF_HDROP file list off the clipboard, with the cut/copy hint.
pub fn clipboard_read(owner: HWND) -> Option<(Vec<String>, DropEffect)> {
    unsafe {
        if !IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() {
            return None;
        }
        if OpenClipboard(Some(owner)).is_err() {
            return None;
        }
        let out = (|| {
            let handle = GetClipboardData(CF_HDROP.0 as u32).ok()?;
            let paths = paths_from_hdrop(HDROP(handle.0));
            if paths.is_empty() {
                return None;
            }

            // Absent the hint, treat it as a copy: the safe reading.
            let mut effect = DropEffect::Copy;
            let fmt = preferred_drop_effect_format();
            if fmt != 0 {
                if let Ok(eh) = GetClipboardData(fmt) {
                    let hg = HGLOBAL(eh.0);
                    if GlobalSize(hg) >= 4 {
                        let p = GlobalLock(hg);
                        if !p.is_null() {
                            if *(p as *const u32) & DropEffect::Move as u32 != 0 {
                                effect = DropEffect::Move;
                            }
                            let _ = GlobalUnlock(hg);
                        }
                    }
                }
            }
            Some((paths, effect))
        })();
        let _ = CloseClipboard();
        out
    }
}

/// Open a file with its default handler, or reveal its shell property sheet.
pub fn shell_open(owner: HWND, path: &str) -> windows::core::Result<()> {
    let wide = to_wide(path);
    let verb = to_wide("open");
    let result = unsafe {
        ShellExecuteW(
            Some(owner),
            PCWSTR::from_raw(verb.as_ptr()),
            PCWSTR::from_raw(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns >32 on success; the value is a legacy pseudo-handle.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread())
    }
}

/// Show the shell's Properties dialog for a path.
pub fn shell_properties(owner: HWND, path: &str) -> windows::core::Result<()> {
    let wide = to_wide(path);
    let verb = to_wide("properties");
    let result = unsafe {
        ShellExecuteW(
            Some(owner),
            PCWSTR::from_raw(verb.as_ptr()),
            PCWSTR::from_raw(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(windows::core::Error::from_thread())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affected_dirs_covers_both_ends_of_a_move() {
        let op = Op::Move {
            sources: vec!["C:\\a\\one.txt".into(), "C:\\a\\two.txt".into()],
            dest_dir: "C:\\b".into(),
        };
        let dirs = op.affected_dirs();
        assert!(dirs.contains(&"C:\\b".to_string()));
        assert!(dirs.contains(&"C:\\a".to_string()));
    }

    #[test]
    fn affected_dirs_for_delete_is_the_parent() {
        let op = Op::Delete {
            sources: vec!["C:\\a\\one.txt".into()],
            permanent: false,
        };
        assert_eq!(op.affected_dirs(), vec!["C:\\a".to_string()]);
    }

    #[test]
    fn batch_rename_touches_every_source_directory() {
        let op = Op::RenameMany {
            items: vec![
                ("C:\\a\\one.txt".into(), "1.txt".into()),
                ("C:\\a\\two.txt".into(), "2.txt".into()),
            ],
        };
        let dirs = op.affected_dirs();
        assert!(dirs.iter().all(|d| d == "C:\\a"));
        assert_eq!(dirs.len(), 2);
    }

    #[test]
    fn drop_effect_matches_shell_values() {
        // Explorer interop depends on these exact numbers.
        assert_eq!(DropEffect::Copy as u32, 1);
        assert_eq!(DropEffect::Move as u32, 2);
    }
}

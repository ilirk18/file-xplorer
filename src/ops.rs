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
use windows::Win32::System::Ole::{CF_HDROP, CF_UNICODETEXT};
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::Win32::UI::Shell::*;

use crate::fs::wide;

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

    /// The operation that puts things back, or None when there is no honest
    /// way to do so.
    ///
    /// Deleting is the interesting omission. `FOF_ALLOWUNDO` puts deleted
    /// items in the Recycle Bin, but nothing in the shell API asks for them
    /// back by name, and inventing a "restore" that guessed would be worse
    /// than not offering one. The Recycle Bin already does this job.
    ///
    /// An undone copy or new folder goes to the Recycle Bin rather than being
    /// erased, because undo should not be the one operation in the app that
    /// destroys something permanently.
    ///
    /// Undo cannot bring back a file that was overwritten during the original
    /// operation: the shell asked before replacing it, and the answer is gone.
    pub fn inverse(&self) -> Option<Op> {
        use crate::fs::{path_join, path_leaf, path_parent};
        match self {
            Op::Copy { sources, dest_dir } => Some(Op::Delete {
                sources: sources
                    .iter()
                    .map(|s| path_join(dest_dir, &path_leaf(s)))
                    .collect(),
                permanent: false,
            }),
            Op::Move { sources, dest_dir } => {
                // Everything has to go back to one folder, so everything must
                // have come from one. A selection always does; a drop of a
                // mixed set from elsewhere might not, and is left un-undoable
                // rather than half-restored.
                let home = path_parent(sources.first()?)?;
                if sources
                    .iter()
                    .any(|s| path_parent(s).as_deref() != Some(home.as_str()))
                {
                    return None;
                }
                Some(Op::Move {
                    sources: sources
                        .iter()
                        .map(|s| path_join(dest_dir, &path_leaf(s)))
                        .collect(),
                    dest_dir: home,
                })
            }
            Op::Rename { source, new_name } => {
                let parent = path_parent(source)?;
                Some(Op::Rename {
                    source: path_join(&parent, new_name),
                    new_name: path_leaf(source),
                })
            }
            Op::RenameMany { items } => Some(Op::RenameMany {
                items: items
                    .iter()
                    .filter_map(|(src, new)| {
                        let parent = path_parent(src)?;
                        Some((path_join(&parent, new), path_leaf(src)))
                    })
                    .collect(),
            }),
            Op::NewFolder { parent, name } => Some(Op::Delete {
                sources: vec![path_join(parent, name)],
                permanent: false,
            }),
            Op::Delete { .. } => None,
        }
    }

    /// Every path this operation would read or write.
    fn paths(&self) -> Vec<&str> {
        match self {
            Op::Copy { sources, dest_dir } | Op::Move { sources, dest_dir } => {
                let mut v: Vec<&str> = sources.iter().map(|s| s.as_str()).collect();
                v.push(dest_dir);
                v
            }
            Op::Delete { sources, .. } => sources.iter().map(|s| s.as_str()).collect(),
            Op::Rename { source, .. } => vec![source],
            Op::RenameMany { items } => items.iter().map(|(s, _)| s.as_str()).collect(),
            Op::NewFolder { parent, .. } => vec![parent],
        }
    }

    /// True when any path leads inside an archive. Nothing here writes to one:
    /// 7-Zip's CLI can, but a file operation that half succeeds inside a
    /// container is not something to bolt on, so these are refused outright.
    /// True when any path this would write to is somewhere this app only
    /// reads: inside an archive, or in the shell namespace.
    ///
    /// `IFileOperation` can in fact restore from the Recycle Bin and write to
    /// a network place, but "delete" and "rename" mean something different in
    /// each namespace and getting one subtly wrong costs somebody their files.
    /// Refusing here rather than in each command is what makes it true of the
    /// paths nobody thought about — a drop, an undo, a paste.
    pub fn is_read_only(&self) -> bool {
        self.paths().iter().any(|p| {
            crate::archive::split(p).is_some() || crate::shellns::is_shell_path(p)
        })
    }

    /// Does this move bytes about? Two of these at once on one disk take
    /// longer together than one after the other; a rename or a delete is over
    /// before the head has moved and must not wait behind a ten-minute copy.
    pub fn is_transfer(&self) -> bool {
        matches!(self, Op::Copy { .. } | Op::Move { .. })
    }

    /// What the footer says while this is running.
    pub fn progress_text(&self) -> &'static str {
        match self {
            Op::Copy { .. } => "Copying\u{2026}",
            Op::Move { .. } => "Moving\u{2026}",
            Op::Delete { .. } => "Deleting\u{2026}",
            Op::Rename { .. } | Op::RenameMany { .. } => "Renaming\u{2026}",
            Op::NewFolder { .. } => "Creating folder\u{2026}",
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
    let wide = wide(path);
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
            let name = wide(new_name);
            unsafe { file_op.RenameItem(&item, PCWSTR::from_raw(name.as_ptr()), None)? };
        }
        Op::RenameMany { items } => {
            for (source, new_name) in items {
                let item = shell_item(source)?;
                let name = wide(new_name);
                unsafe { file_op.RenameItem(&item, PCWSTR::from_raw(name.as_ptr()), None)? };
            }
        }
        Op::NewFolder { parent, name } => {
            let dest = shell_item(parent)?;
            let name = wide(name);
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
/// One transfer at a time.
///
/// Queueing copies rather than running them all at once is the whole feature:
/// four copies to one disk finish later than four copies in a row, and four
/// progress dialogs each claiming a share of the throughput is a worse lie
/// than one honest one with the rest waiting.
///
/// ponytail: a lock is the queue. It gives the ordering a queue would and
/// nothing else — no list to look at, no reordering, no cancelling one that
/// has not started. Build those the day there is somewhere to show them.
static TRANSFERS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Is a transfer running, or waiting to? What the footer uses to say so.
pub fn transfer_running() -> bool {
    TRANSFERS.try_lock().is_err()
}

pub fn spawn<F>(op: Op, owner: OwnerWindow, deliver: F)
where
    F: FnOnce(Box<OpResult>) + Send + 'static,
{
    std::thread::spawn(move || {
        // Held for the whole operation, so the next one starts when this one
        // is finished rather than beside it.
        let _turn = op
            .is_transfer()
            .then(|| TRANSFERS.lock().unwrap_or_else(|e| e.into_inner()));
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
    let name = wide("Preferred DropEffect");
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
/// Whether the clipboard holds files right now.
///
/// Asked once per repaint, to decide whether the command bar's Paste is live.
/// Deliberately not `clipboard_read`: that opens the clipboard, which is a
/// lock every other app on the machine contends for, and doing it sixty times
/// a second to grey out a button would be a good way to break somebody's copy.
pub fn clipboard_has_files() -> bool {
    unsafe { IsClipboardFormatAvailable(CF_HDROP.0 as u32).is_ok() }
}

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
/// Put plain text on the clipboard. Used by "Copy path"; the file list itself
/// goes on as CF_HDROP, which is a different thing entirely.
pub fn clipboard_write_text(owner: HWND, text: &str) -> windows::core::Result<()> {
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        OpenClipboard(Some(owner))?;
        let _ = EmptyClipboard();
        let bytes = wide.len() * 2;
        let result = (|| -> windows::core::Result<()> {
            let mem = GlobalAlloc(GMEM_MOVEABLE, bytes)?;
            let dst = GlobalLock(mem);
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, dst as *mut u8, bytes);
            let _ = GlobalUnlock(mem);
            // The clipboard owns the block from here; do not free it.
            SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(mem.0)))?;
            Ok(())
        })();
        let _ = CloseClipboard();
        result
    }
}

/// Take plain text off the clipboard, if there is any.
///
/// The mirror of `clipboard_write_text`, for the editors that are ours rather
/// than Windows': a real EDIT control does its own pasting.
pub fn clipboard_read_text(owner: HWND) -> Option<String> {
    unsafe {
        if !IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_ok() {
            return None;
        }
        OpenClipboard(Some(owner)).ok()?;
        let out = (|| -> Option<String> {
            let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
            let hg = HGLOBAL(handle.0);
            let p = GlobalLock(hg) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut len = 0usize;
            while *p.add(len) != 0 && len < 4096 {
                len += 1;
            }
            let text = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(hg);
            Some(text)
        })();
        let _ = CloseClipboard();
        out
    }
}

/// Open a shell at `dir`. Windows Terminal if it is installed, PowerShell if
/// not — both are launched through the shell, so neither needs a full path.
pub fn open_terminal(owner: HWND, dir: &str) -> windows::core::Result<()> {
    let verb = wide("open");
    let cwd = wide(dir);
    let mut last = windows::core::Error::empty();
    for exe in ["wt.exe", "powershell.exe", "cmd.exe"] {
        let file = wide(exe);
        let h = unsafe {
            ShellExecuteW(
                Some(owner),
                PCWSTR::from_raw(verb.as_ptr()),
                PCWSTR::from_raw(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::from_raw(cwd.as_ptr()),
                SW_SHOWNORMAL,
            )
        };
        // ShellExecuteW returns a fake HINSTANCE; anything over 32 is success.
        if h.0 as isize > 32 {
            return Ok(());
        }
        last = windows::core::Error::from_thread();
    }
    Err(last)
}

pub fn shell_open(owner: HWND, path: &str) -> windows::core::Result<()> {
    let file = wide(path);
    let verb = wide("open");
    let result = unsafe {
        ShellExecuteW(
            Some(owner),
            PCWSTR::from_raw(verb.as_ptr()),
            PCWSTR::from_raw(file.as_ptr()),
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
    let file = wide(path);
    let verb = wide("properties");
    let result = unsafe {
        ShellExecuteW(
            Some(owner),
            PCWSTR::from_raw(verb.as_ptr()),
            PCWSTR::from_raw(file.as_ptr()),
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
    fn only_the_operations_that_move_bytes_wait_their_turn() {
        // The point of the queue: a rename or a delete is instant and must
        // not sit behind a copy, which is the one thing that is not.
        let paths = || vec!["C:\\a\\one.txt".to_string()];
        assert!(Op::Copy { sources: paths(), dest_dir: "D:\\b".into() }.is_transfer());
        assert!(Op::Move { sources: paths(), dest_dir: "D:\\b".into() }.is_transfer());
        assert!(!Op::Delete { sources: paths(), permanent: false }.is_transfer());
        assert!(!Op::Rename { source: paths()[0].clone(), new_name: "two.txt".into() }.is_transfer());
        assert!(!Op::NewFolder { parent: "C:\\a".into(), name: "new".into() }.is_transfer());
    }

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

#[cfg(test)]
mod undo_tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn undoing_a_copy_recycles_what_landed_at_the_destination() {
        let op = Op::Copy {
            sources: v(&["C:\\a\\one.txt", "C:\\a\\two.txt"]),
            dest_dir: "D:\\b".into(),
        };
        match op.inverse().unwrap() {
            Op::Delete { sources, permanent } => {
                assert_eq!(sources, v(&["D:\\b\\one.txt", "D:\\b\\two.txt"]));
                assert!(!permanent, "undo must never be the one path that erases");
            }
            other => panic!("expected a delete, got {:?}", other),
        }
    }

    #[test]
    fn undoing_a_move_sends_everything_home() {
        let op = Op::Move {
            sources: v(&["C:\\a\\one.txt", "C:\\a\\two.txt"]),
            dest_dir: "D:\\b".into(),
        };
        match op.inverse().unwrap() {
            Op::Move { sources, dest_dir } => {
                assert_eq!(sources, v(&["D:\\b\\one.txt", "D:\\b\\two.txt"]));
                assert_eq!(dest_dir, "C:\\a");
            }
            other => panic!("expected a move, got {:?}", other),
        }
    }

    #[test]
    fn a_move_from_two_folders_cannot_be_undone() {
        // There is no single folder to put them back into, and a half-restore
        // would be worse than saying no.
        let op = Op::Move {
            sources: v(&["C:\\a\\one.txt", "C:\\other\\two.txt"]),
            dest_dir: "D:\\b".into(),
        };
        assert!(op.inverse().is_none());
    }

    #[test]
    fn undoing_a_rename_puts_the_old_name_back() {
        let op = Op::Rename {
            source: "C:\\a\\old.txt".into(),
            new_name: "new.txt".into(),
        };
        match op.inverse().unwrap() {
            Op::Rename { source, new_name } => {
                assert_eq!(source, "C:\\a\\new.txt");
                assert_eq!(new_name, "old.txt");
            }
            other => panic!("expected a rename, got {:?}", other),
        }
    }

    #[test]
    fn undoing_a_batch_rename_is_one_step_not_n() {
        let op = Op::RenameMany {
            items: vec![
                ("C:\\a\\x.txt".into(), "1.txt".into()),
                ("C:\\a\\y.txt".into(), "2.txt".into()),
            ],
        };
        match op.inverse().unwrap() {
            Op::RenameMany { items } => {
                assert_eq!(items.len(), 2);
                assert_eq!(items[0], ("C:\\a\\1.txt".to_string(), "x.txt".to_string()));
            }
            other => panic!("expected a batch rename, got {:?}", other),
        }
    }

    #[test]
    fn undoing_a_new_folder_removes_it() {
        let op = Op::NewFolder {
            parent: "C:\\a".into(),
            name: "Notes".into(),
        };
        match op.inverse().unwrap() {
            Op::Delete { sources, .. } => assert_eq!(sources, v(&["C:\\a\\Notes"])),
            other => panic!("expected a delete, got {:?}", other),
        }
    }

    #[test]
    fn a_delete_is_the_recycle_bins_to_undo_not_ours() {
        let op = Op::Delete {
            sources: v(&["C:\\a\\gone.txt"]),
            permanent: false,
        };
        assert!(op.inverse().is_none());
    }

    #[test]
    fn undo_round_trips_through_its_own_inverse() {
        // Undoing an undo has to land back where it started, or the stack
        // would drift after two presses.
        let op = Op::Move {
            sources: v(&["C:\\a\\one.txt"]),
            dest_dir: "D:\\b".into(),
        };
        let back = op.inverse().unwrap().inverse().unwrap();
        match back {
            Op::Move { sources, dest_dir } => {
                assert_eq!(sources, v(&["C:\\a\\one.txt"]));
                assert_eq!(dest_dir, "D:\\b");
            }
            other => panic!("expected a move, got {:?}", other),
        }
    }
}

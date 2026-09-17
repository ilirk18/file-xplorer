// Batch rename: a pattern, a live preview, and one undoable operation.
//
// The pattern language is deliberately four tokens. Anything more and it wants
// a parser, a manual, and a test suite of its own; these cover the renames
// people actually do.
//
//   {n}   original name without extension
//   {e}   extension without the dot
//   {#}   1, 2, 3 ...   {##} 01, 02 ...   {###} 001, 002 ...
//   {d}   date modified, YYYY-MM-DD
//   {id}  8 hex characters derived from the original name
//
// Everything else is literal. A pattern with no {e} loses the extension, which
// is occasionally what you want and always what you asked for.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "FileXplorerBatchRename";
const IDC_PATTERN: i32 = 3001;
const IDC_PREVIEW: i32 = 3002;
const IDC_OK: i32 = 3003;
const IDC_CANCEL: i32 = 3004;

use crate::fs::wide;

/// One file the rename applies to.
///
/// `date` arrives preformatted because only the caller holds the FILETIME, and
/// pulling `fs`'s formatter in here would make this module need a clock to test.
#[derive(Clone, Debug)]
pub struct Item {
    pub name: String,
    /// Modified date as `YYYY-MM-DD`. Empty when it could not be read, which
    /// `{d}` then expands to nothing rather than to a wrong date.
    pub date: String,
}

/// 8 hex characters from the original name: stable, so running the same rename
/// twice produces the same ids, and distinct for distinct names.
///
/// ponytail: FNV-1a, not a cryptographic hash. A collision renames two files to
/// the same name, which `is_valid` catches before anything runs.
fn short_id(name: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in name.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{:08x}", (h >> 32) as u32)
}

/// Split a file name into (stem, extension-without-dot).
/// A leading dot is part of the stem: `.gitignore` has no extension.
pub fn split_name(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    }
}

/// Expand `pattern` for one file. `index` is 1-based.
pub fn expand(pattern: &str, item: &Item, index: usize) -> String {
    let name = item.name.as_str();
    let (stem, ext) = split_name(name);
    let mut out = String::with_capacity(pattern.len() + stem.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] != '{' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        let Some(close) = chars[i..].iter().position(|c| *c == '}') else {
            out.push(chars[i]);
            i += 1;
            continue;
        };
        let token: String = chars[i + 1..i + close].iter().collect();
        match token.as_str() {
            "n" => out.push_str(stem),
            "e" => out.push_str(ext),
            "d" => out.push_str(&item.date),
            "id" => out.push_str(&short_id(name)),
            t if !t.is_empty() && t.chars().all(|c| c == '#') => {
                out.push_str(&format!("{:0width$}", index, width = t.len()));
            }
            // Unknown token: leave it alone rather than silently eating it.
            _ => out.push_str(&chars[i..=i + close].iter().collect::<String>()),
        }
        i += close + 1;
    }
    out
}

/// Build the (old, new) list for a selection. Entries whose name would not
/// change are dropped: renaming a file to itself is a no-op the shell would
/// otherwise report as a conflict.
pub fn plan(items: &[Item], pattern: &str) -> Vec<(String, String)> {
    items
        .iter()
        .enumerate()
        .map(|(i, it)| (it.name.clone(), expand(pattern, it, i + 1)))
        .filter(|(old, new)| old != new && !new.is_empty())
        .collect()
}

/// Human-readable preview, one line per rename.
pub fn preview_text(items: &[Item], pattern: &str) -> String {
    if pattern.is_empty() {
        return "Type a pattern above.\r\n\r\n{n} name   {e} extension   {#} counter".into();
    }
    let plan = plan(items, pattern);
    if plan.is_empty() {
        return "Nothing would change.".into();
    }
    // Flag collisions: two files renamed to the same thing is the classic way
    // a batch rename eats data.
    let mut seen: Vec<&str> = Vec::new();
    let mut lines = Vec::with_capacity(plan.len());
    for (old, new) in &plan {
        let clash = seen.contains(&new.as_str());
        seen.push(new);
        let bad = clash || crate::fs::validate_file_name(new).is_err();
        lines.push(format!(
            "{}{}  ->  {}",
            if bad { "! " } else { "" },
            old,
            new
        ));
    }
    lines.join("\r\n")
}

/// True when every proposed name is valid and unique.
pub fn is_valid(items: &[Item], pattern: &str) -> bool {
    let plan = plan(items, pattern);
    if plan.is_empty() {
        return false;
    }
    let mut seen: Vec<&str> = Vec::new();
    for (_, new) in &plan {
        if seen.contains(&new.as_str()) || crate::fs::validate_file_name(new).is_err() {
            return false;
        }
        seen.push(new);
    }
    true
}

struct State {
    items: Vec<Item>,
    pattern: String,
    accepted: bool,
    dpi: u32,
}

/// Show the dialog. Returns the pattern the user accepted.
pub fn prompt(parent: HWND, dpi: u32, items: Vec<Item>) -> Option<String> {
    unsafe { prompt_impl(parent, dpi, items) }
}

unsafe fn prompt_impl(parent: HWND, dpi: u32, items: Vec<Item>) -> Option<String> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = wide(CLASS);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(proc_),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
        hbrBackground: HBRUSH(((COLOR_WINDOW.0 + 1) as isize) as *mut _),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);

    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale).round() as i32;
    let (w, h) = (s(560.0), s(440.0));

    let mut pr = RECT::default();
    let _ = GetWindowRect(parent, &mut pr);
    let x = pr.left + ((pr.right - pr.left) - w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - h) / 4;

    let st = Box::into_raw(Box::new(State {
        items,
        pattern: "{n}".to_string(),
        accepted: false,
        dpi,
    }));

    let title = wide("Batch rename");
    let dlg = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title.as_ptr()),
        WS_POPUPWINDOW | WS_CAPTION,
        x,
        y,
        w,
        h,
        Some(parent),
        None,
        Some(instance.into()),
        Some(st as *const _),
    );
    let Ok(dlg) = dlg else {
        drop(Box::from_raw(st));
        return None;
    };

    let font = crate::dialog::UiFont::new(dpi);
    if let Some(f) = &font {
        f.apply_to_children(dlg);
    }

    let _ = EnableWindow(parent, false);
    let _ = ShowWindow(dlg, SW_SHOW);

    let mut quit_code: Option<i32> = None;
    let mut msg = MSG::default();
    while IsWindow(Some(dlg)).as_bool() {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if r.0 == -1 {
            break;
        }
        if r.0 == 0 {
            quit_code = Some(msg.wParam.0 as i32);
            break;
        }
        if !IsDialogMessageW(dlg, &mut msg).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }

    let _ = EnableWindow(parent, true);
    let _ = SetActiveWindow(parent);
    if IsWindow(Some(dlg)).as_bool() {
        let _ = DestroyWindow(dlg);
    }

    drop(font);
    let st = Box::from_raw(st);
    if let Some(code) = quit_code {
        PostQuitMessage(code);
    }
    if st.accepted {
        Some(st.pattern.clone())
    } else {
        None
    }
}

unsafe fn refresh_preview(hwnd: HWND) {
    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return;
    }
    let Ok(pattern_box) = GetDlgItem(Some(hwnd), IDC_PATTERN) else {
        return;
    };
    let len = GetWindowTextLengthW(pattern_box).max(0) as usize;
    let mut buf = vec![0u16; len + 1];
    let n = GetWindowTextW(pattern_box, &mut buf);
    let pattern = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    (*ptr).pattern = pattern.clone();

    if let Ok(preview) = GetDlgItem(Some(hwnd), IDC_PREVIEW) {
        let text = wide(&preview_text(&(*ptr).items, &pattern));
        let _ = SetWindowTextW(preview, PCWSTR::from_raw(text.as_ptr()));
    }
    if let Ok(ok) = GetDlgItem(Some(hwnd), IDC_OK) {
        let _ = EnableWindow(ok, is_valid(&(*ptr).items, &pattern));
    }
}

extern "system" fn proc_(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let st = cs.lpCreateParams as *mut State;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, st as isize);
                let scale = (*st).dpi as f32 / 96.0;
                let s = |v: f32| (v * scale).round() as i32;
                let instance = cs.hInstance;

                let static_class = wide("Static");
                let label = wide("Pattern:   {n} name   {e} extension   {#} counter");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(static_class.as_ptr()),
                    PCWSTR::from_raw(label.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    s(14.0),
                    s(12.0),
                    s(520.0),
                    s(18.0),
                    Some(hwnd),
                    None,
                    Some(instance),
                    None,
                );

                let edit = wide("Edit");
                let pattern = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(edit.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    s(14.0),
                    s(34.0),
                    s(526.0),
                    s(26.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PATTERN as *mut _)),
                    Some(instance),
                    None,
                );

                let _ = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(edit.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WINDOW_STYLE(
                            (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
                        ),
                    s(14.0),
                    s(70.0),
                    s(526.0),
                    s(280.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PREVIEW as *mut _)),
                    Some(instance),
                    None,
                );

                let button = wide("Button");
                let ok_text = wide("Rename");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button.as_ptr()),
                    PCWSTR::from_raw(ok_text.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                    s(346.0),
                    s(362.0),
                    s(92.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_OK as *mut _)),
                    Some(instance),
                    None,
                );
                let cancel_text = wide("Cancel");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button.as_ptr()),
                    PCWSTR::from_raw(cancel_text.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    s(448.0),
                    s(362.0),
                    s(92.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_CANCEL as *mut _)),
                    Some(instance),
                    None,
                );

                if let Ok(pattern) = pattern {
                    let initial = wide(&(*st).pattern);
                    let _ = SetWindowTextW(pattern, PCWSTR::from_raw(initial.as_ptr()));
                    let _ = SetFocus(Some(pattern));
                }
                refresh_preview(hwnd);
                LRESULT(0)
            }

            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDC_PATTERN && code == EN_CHANGE {
                    refresh_preview(hwnd);
                } else if id == IDC_OK {
                    let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
                    if !ptr.is_null() && is_valid(&(*ptr).items, &(*ptr).pattern) {
                        (*ptr).accepted = true;
                    }
                    let _ = DestroyWindow(hwnd);
                } else if id == IDC_CANCEL {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(name: &str) -> Item {
        Item {
            name: name.to_string(),
            date: "2026-09-14".into(),
        }
    }

    fn names(v: &[&str]) -> Vec<Item> {
        v.iter()
            .map(|s| Item {
                name: s.to_string(),
                date: "2026-09-14".into(),
            })
            .collect()
    }

    #[test]
    fn splits_stem_and_extension() {
        assert_eq!(split_name("photo.jpg"), ("photo", "jpg"));
        assert_eq!(split_name("archive.tar.gz"), ("archive.tar", "gz"));
        assert_eq!(split_name("README"), ("README", ""));
        // A leading dot is part of the name, not an extension.
        assert_eq!(split_name(".gitignore"), (".gitignore", ""));
    }

    #[test]
    fn expands_name_and_extension() {
        assert_eq!(expand("{n}.{e}", &one("photo.jpg"), 1), "photo.jpg");
        assert_eq!(expand("holiday-{n}.{e}", &one("photo.jpg"), 1), "holiday-photo.jpg");
    }

    #[test]
    fn counter_pads_to_the_hash_count() {
        assert_eq!(expand("{#}.{e}", &one("a.txt"), 7), "7.txt");
        assert_eq!(expand("{##}.{e}", &one("a.txt"), 7), "07.txt");
        assert_eq!(expand("{###}.{e}", &one("a.txt"), 7), "007.txt");
        assert_eq!(expand("{##}.{e}", &one("a.txt"), 123), "123.txt", "never truncates");
    }

    #[test]
    fn literal_text_passes_through() {
        assert_eq!(expand("IMG_{###}", &one("x.jpg"), 4), "IMG_004");
    }

    #[test]
    fn unknown_tokens_are_left_alone() {
        // Better to show the user their typo than to silently swallow it.
        assert_eq!(expand("{bogus}-{n}", &one("a.txt"), 1), "{bogus}-a");
    }

    #[test]
    fn unclosed_brace_is_literal() {
        assert_eq!(expand("{n", &one("a.txt"), 1), "{n");
    }

    #[test]
    fn plan_drops_unchanged_names() {
        let p = plan(&names(&["a.txt", "b.txt"]), "{n}.{e}");
        assert!(p.is_empty(), "renaming to itself is not a rename");
    }

    #[test]
    fn plan_numbers_from_one_in_order() {
        let p = plan(&names(&["a.txt", "b.txt"]), "{#}.{e}");
        assert_eq!(
            p,
            vec![
                ("a.txt".to_string(), "1.txt".to_string()),
                ("b.txt".to_string(), "2.txt".to_string()),
            ]
        );
    }

    #[test]
    fn collisions_are_rejected() {
        // Both files would become "same.txt" - the classic way a batch rename
        // destroys data.
        assert!(!is_valid(&names(&["a.txt", "b.txt"]), "same.{e}"));
    }

    #[test]
    fn illegal_names_are_rejected() {
        assert!(!is_valid(&names(&["a.txt"]), "sub\\dir\\{n}"));
        assert!(!is_valid(&names(&["a.txt"]), "CON"));
        assert!(!is_valid(&names(&["a.txt"]), "{n}."));
    }

    #[test]
    fn a_valid_plan_passes() {
        assert!(is_valid(&names(&["a.txt", "b.txt"]), "photo-{##}.{e}"));
    }

    #[test]
    fn date_and_id_expand() {
        assert_eq!(expand("{d}-{n}.{e}", &one("a.txt"), 1), "2026-09-14-a.txt");
        let id = expand("{id}.{e}", &one("a.txt"), 1);
        assert_eq!(id.len(), "00000000.txt".len());
        assert_eq!(id, expand("{id}.{e}", &one("a.txt"), 9), "stable per name");
        assert_ne!(id, expand("{id}.{e}", &one("b.txt"), 1), "distinct per name");
    }

    #[test]
    fn a_missing_date_expands_to_nothing() {
        let undated = Item {
            name: "a.txt".into(),
            date: String::new(),
        };
        assert_eq!(expand("{d}{n}.{e}", &undated, 1), "a.txt");
    }

    #[test]
    fn preview_marks_the_bad_rows() {
        let text = preview_text(&names(&["a.txt", "b.txt"]), "same.{e}");
        assert!(text.contains("!"), "collision should be flagged: {}", text);
    }

    #[test]
    fn empty_pattern_shows_help_not_an_error() {
        let text = preview_text(&names(&["a.txt"]), "");
        assert!(text.contains("{n}"));
    }
}

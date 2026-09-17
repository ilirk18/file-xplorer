// Command palette: a search box over a list, used to reach every command
// without memorising a chord.
//
// The list is a stock Win32 LISTBOX. Keyboard navigation, scrolling, selection
// highlighting and mouse wheel all come free with it — owner-drawing this to
// match the app's theme would be a lot of code for a transient popup.
//
// Arrow keys and Enter are intercepted in the modal loop rather than by
// subclassing the edit control: we own that loop already, so it is the cheaper
// hook.

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "FileXplorerPalette";
const IDC_QUERY: i32 = 2001;
const IDC_LIST: i32 = 2002;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub struct Item {
    pub label: String,
    /// Shortcut or hint, shown dimmed after the label.
    pub detail: String,
}

/// Score `label` against `query`, higher is better. None means no match.
///
/// Subsequence matching, so "nf" finds "New folder". Consecutive characters and
/// word starts score higher, which is what makes short queries land on the
/// command people meant rather than the first alphabetical hit.
pub fn score(label: &str, query: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let l: Vec<char> = label.to_lowercase().chars().collect();
    let q: Vec<char> = query.to_lowercase().chars().collect();

    // A whole-query prefix match is the strongest signal there is, and must
    // outrank a scattered match that happens to land on more word starts:
    // typing "ref" means Refresh, not "Rename file".
    let prefix_bonus = if l.len() >= q.len() && l[..q.len()] == q[..] {
        20
    } else {
        0
    };

    let mut total = 0i32;
    let mut li = 0usize;
    let mut last_hit: Option<usize> = None;

    for &qc in &q {
        let mut found = None;
        while li < l.len() {
            if l[li] == qc {
                found = Some(li);
                break;
            }
            li += 1;
        }
        let hit = found?;
        total += 1;
        // Start of a word, or the very beginning.
        if hit == 0 || l[hit - 1] == ' ' || l[hit - 1] == '-' {
            total += 8;
        }
        if last_hit == Some(hit.wrapping_sub(1)) {
            total += 5; // consecutive
        }
        last_hit = Some(hit);
        li += 1;
    }
    // Prefer shorter labels when scores tie: less to read, likelier the target.
    Some(total + prefix_bonus - (label.chars().count() as i32) / 8)
}

/// Indices of `items` matching `query`, best first.
pub fn filter(items: &[Item], query: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| score(&it.label, query).map(|s| (s, i)))
        .collect();
    // Stable by score then original order, so an empty query keeps the
    // author's grouping instead of scrambling it.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, i)| i).collect()
}

struct State {
    items: Vec<Item>,
    /// Indices into `items`, in display order.
    shown: Vec<usize>,
    result: Option<usize>,
    accepted: bool,
    dpi: u32,
}

/// Show the palette. Returns the chosen index into `items`.
pub fn pick(parent: HWND, dpi: u32, title: &str, items: Vec<Item>) -> Option<usize> {
    unsafe { pick_impl(parent, dpi, title, items) }
}

unsafe fn pick_impl(parent: HWND, dpi: u32, title: &str, items: Vec<Item>) -> Option<usize> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = wide(CLASS);

    // Registering twice is harmless: the second call fails and we carry on.
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
    let (w, h) = (s(520.0), s(420.0));

    let mut pr = RECT::default();
    let _ = GetWindowRect(parent, &mut pr);
    let x = pr.left + ((pr.right - pr.left) - w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - h) / 4;

    let shown = filter(&items, "");
    let st = Box::into_raw(Box::new(State {
        items,
        shown,
        result: None,
        accepted: false,
        dpi,
    }));

    let title_w = wide(title);
    let dlg = CreateWindowExW(
        WS_EX_DLGMODALFRAME | WS_EX_TOPMOST,
        PCWSTR::from_raw(class.as_ptr()),
        PCWSTR::from_raw(title_w.as_ptr()),
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

    let list = GetDlgItem(Some(dlg), IDC_LIST).ok();
    let mut quit_code: Option<i32> = None;
    let mut msg = MSG::default();

    while IsWindow(Some(dlg)).as_bool() {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if r.0 == -1 {
            break;
        }
        if r.0 == 0 {
            // WM_QUIT arrived while modal: remember it and re-post on the way
            // out, or the app can never be closed from here.
            quit_code = Some(msg.wParam.0 as i32);
            break;
        }

        // Arrows drive the list even though the edit box has focus.
        if msg.message == WM_KEYDOWN {
            let vk = msg.wParam.0 as u16;
            if let Some(list) = list {
                if vk == 0x26 || vk == 0x28 || vk == 0x21 || vk == 0x22 {
                    let count = SendMessageW(list, LB_GETCOUNT, None, None).0;
                    let cur = SendMessageW(list, LB_GETCURSEL, None, None).0;
                    let step: isize = match vk {
                        0x26 => -1,
                        0x28 => 1,
                        0x21 => -10,
                        _ => 10,
                    };
                    let next = (cur + step).clamp(0, (count - 1).max(0));
                    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(next as usize)), None);
                    continue;
                }
            }
            if vk == 0x0D {
                accept(dlg);
                continue;
            }
            if vk == 0x1B {
                let _ = DestroyWindow(dlg);
                continue;
            }
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
        st.result
    } else {
        None
    }
}

unsafe fn accept(dlg: HWND) {
    let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return;
    }
    // Take one real reference rather than dereferencing the raw pointer
    // repeatedly: fewer aliasing assumptions, and the borrow checker helps.
    let st = &mut *ptr;
    if let Ok(list) = GetDlgItem(Some(dlg), IDC_LIST) {
        let sel = SendMessageW(list, LB_GETCURSEL, None, None).0;
        if sel >= 0 {
            if let Some(index) = st.shown.get(sel as usize).copied() {
                st.result = Some(index);
                st.accepted = true;
            }
        }
    }
    let _ = DestroyWindow(dlg);
}

unsafe fn refill(dlg: HWND, query: &str) {
    let ptr = GetWindowLongPtrW(dlg, GWLP_USERDATA) as *mut State;
    if ptr.is_null() {
        return;
    }
    let Ok(list) = GetDlgItem(Some(dlg), IDC_LIST) else {
        return;
    };
    let st = &mut *ptr;
    let shown = filter(&st.items, query);
    st.shown = shown;

    SendMessageW(list, LB_RESETCONTENT, None, None);
    for &i in &st.shown {
        let it = &st.items[i];
        let text = if it.detail.is_empty() {
            it.label.clone()
        } else {
            format!("{}      {}", it.label, it.detail)
        };
        let w = wide(&text);
        SendMessageW(
            list,
            LB_ADDSTRING,
            None,
            Some(LPARAM(w.as_ptr() as isize)),
        );
    }
    SendMessageW(list, LB_SETCURSEL, Some(WPARAM(0)), None);
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

                let edit = wide("Edit");
                let _ = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(edit.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    s(12.0),
                    s(12.0),
                    s(486.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_QUERY as *mut _)),
                    Some(instance),
                    None,
                );

                let listbox = wide("ListBox");
                let _ = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(listbox.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD
                        | WS_VISIBLE
                        | WS_VSCROLL
                        | WINDOW_STYLE(LBS_NOTIFY as u32),
                    s(12.0),
                    s(48.0),
                    s(486.0),
                    s(320.0),
                    Some(hwnd),
                    Some(HMENU(IDC_LIST as *mut _)),
                    Some(instance),
                    None,
                );

                refill(hwnd, "");
                if let Ok(q) = GetDlgItem(Some(hwnd), IDC_QUERY) {
                    let _ = SetFocus(Some(q));
                }
                LRESULT(0)
            }

            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
                if id == IDC_QUERY && code == EN_CHANGE {
                    if let Ok(q) = GetDlgItem(Some(hwnd), IDC_QUERY) {
                        let len = GetWindowTextLengthW(q).max(0) as usize;
                        let mut buf = vec![0u16; len + 1];
                        let n = GetWindowTextW(q, &mut buf);
                        let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
                        refill(hwnd, &text);
                    }
                } else if id == IDC_LIST && code == LBN_DBLCLK {
                    accept(hwnd);
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            // The caller owns the box; the window only clears its pointer.
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

    fn items(labels: &[&str]) -> Vec<Item> {
        labels
            .iter()
            .map(|l| Item {
                label: l.to_string(),
                detail: String::new(),
            })
            .collect()
    }

    #[test]
    fn empty_query_keeps_every_item_in_order() {
        let it = items(&["Zebra", "Apple", "Mango"]);
        assert_eq!(filter(&it, ""), vec![0, 1, 2]);
    }

    #[test]
    fn initials_find_a_multi_word_command() {
        let it = items(&["New folder", "Refresh", "Rename"]);
        let got = filter(&it, "nf");
        assert_eq!(it[got[0]].label, "New folder");
    }

    #[test]
    fn non_subsequence_does_not_match() {
        assert_eq!(score("New folder", "xyz"), None);
        assert_eq!(score("New folder", "fn"), None, "order matters");
    }

    #[test]
    fn word_starts_beat_mid_word_hits() {
        let start = score("Copy path", "cp").unwrap();
        let middle = score("Recompute", "cp").unwrap();
        assert!(start > middle, "{} should beat {}", start, middle);
    }

    #[test]
    fn a_prefix_match_beats_a_scattered_one() {
        // "Rename file" matches r-e-f across two word starts, which without a
        // prefix bonus outscored the obvious answer.
        let run = score("Refresh", "ref").unwrap();
        let scattered = score("Rename file", "ref").unwrap();
        assert!(run > scattered, "{} should beat {}", run, scattered);
    }

    #[test]
    fn exact_prefix_ranks_first() {
        let it = items(&["Paste", "Properties", "Open in new tab"]);
        let got = filter(&it, "pa");
        assert_eq!(it[got[0]].label, "Paste");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("New Folder", "NEWF").is_some());
        assert!(score("new folder", "NF").is_some());
    }
}

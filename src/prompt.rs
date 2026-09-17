// A modal single-line text prompt, used by rename, new folder and search.

use windows::core::PCWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{COLOR_WINDOW, HBRUSH};
use windows::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetActiveWindow, SetFocus};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{wide, AppState};
use crate::dialog;
use windows::Win32::UI::Shell::{
    SHAutoComplete, SHACF_AUTOSUGGEST_FORCE_ON, SHACF_FILESYS_ONLY, SHACF_USETAB,
};

// ---------------------------------------------------------------------------
// Modal text prompt
//
// The previous rename dialog kept its result in a `static mut`, never disabled
// its parent (so it was not actually modal and could be opened recursively),
// and ran a nested message loop that swallowed WM_QUIT â€” meaning the app could
// not be closed while it was open. This version keeps per-dialog state in the
// window, disables the owner for real, and re-posts WM_QUIT on the way out.
// ---------------------------------------------------------------------------

pub const IDC_PROMPT_EDIT: i32 = 1001;
pub const IDC_PROMPT_OK: i32 = 1002;
pub const IDC_PROMPT_CANCEL: i32 = 1003;
pub const PROMPT_CLASS: &str = "FileXplorerPromptDlg";
/// EM_SETSEL. Declared here rather than pulling in all of Win32_UI_Controls
/// for one constant.
pub const EM_SETSEL: u32 = 0x00B1;

pub struct PromptState {
    label: String,
    initial: String,
    /// Hand the edit box to the shell's own path completion.
    complete_paths: bool,
    result: Option<String>,
    accepted: bool,
    dpi: u32,
}

pub fn prompt_text(
    hwnd: HWND,
    state: &mut AppState,
    title: &str,
    label: &str,
    initial: &str,
) -> Option<String> {
    prompt_impl(hwnd, state, title, label, initial, false)
}

/// Same box, with the shell's filesystem auto-complete attached. That is a
/// folder picker's worth of behaviour for one API call.
pub fn prompt_path(
    hwnd: HWND,
    state: &mut AppState,
    title: &str,
    label: &str,
    initial: &str,
) -> Option<String> {
    prompt_impl(hwnd, state, title, label, initial, true)
}

fn prompt_impl(
    hwnd: HWND,
    state: &mut AppState,
    title: &str,
    label: &str,
    initial: &str,
    complete_paths: bool,
) -> Option<String> {
    if state.modal {
        return None; // Refuse to stack dialogs.
    }
    state.modal = true;
    let out = unsafe { prompt_text_impl(hwnd, title, label, initial, state.dpi, complete_paths) };
    state.modal = false;
    out
}

pub unsafe fn prompt_text_impl(
    parent: HWND,
    title: &str,
    label: &str,
    initial: &str,
    dpi: u32,
    complete_paths: bool,
) -> Option<String> {
    let instance = GetModuleHandleW(None).ok()?;
    let class = wide(PROMPT_CLASS);

    // Registering twice is harmless: the second call fails and we carry on.
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(prompt_proc),
        hInstance: instance.into(),
        hCursor: LoadCursorW(None, IDC_ARROW).ok()?,
        hbrBackground: HBRUSH(((COLOR_WINDOW.0 + 1) as isize) as *mut _),
        lpszClassName: PCWSTR::from_raw(class.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);

    let scale = dpi as f32 / 96.0;
    let s = |v: f32| (v * scale).round() as i32;
    let (w, h) = (s(380.0), s(160.0));

    // Centre on the parent.
    let mut pr = RECT::default();
    let _ = GetWindowRect(parent, &mut pr);
    let x = pr.left + ((pr.right - pr.left) - w) / 2;
    let y = pr.top + ((pr.bottom - pr.top) - h) / 3;

    let st = Box::into_raw(Box::new(PromptState {
        label: label.to_string(),
        initial: initial.to_string(),
        complete_paths,
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

    let font = dialog::UiFont::new(dpi);
    if let Some(f) = &font {
        f.apply_to_children(dlg);
    }

    // Real modality: the owner stops accepting input until we are done.
    let _ = EnableWindow(parent, false);
    let _ = ShowWindow(dlg, SW_SHOW);
    let _ = SetFocus(Some(dlg));

    let mut quit_code: Option<i32> = None;
    let mut msg = MSG::default();
    while IsWindow(Some(dlg)).as_bool() {
        let r = GetMessageW(&mut msg, None, 0, 0);
        if r.0 == -1 {
            break;
        }
        if r.0 == 0 {
            // WM_QUIT arrived while we were modal. Remember it and re-post
            // after unwinding so the app still exits.
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
        st.result.clone()
    } else {
        None
    }
}

extern "system" fn prompt_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let cs = &*(lparam.0 as *const CREATESTRUCTW);
                let st = cs.lpCreateParams as *mut PromptState;
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, st as isize);
                let state = &*st;
                let scale = state.dpi as f32 / 96.0;
                let s = |v: f32| (v * scale).round() as i32;
                let instance = cs.hInstance;

                let static_class = wide("Static");
                let label = wide(&state.label);
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(static_class.as_ptr()),
                    PCWSTR::from_raw(label.as_ptr()),
                    WS_CHILD | WS_VISIBLE,
                    s(14.0),
                    s(12.0),
                    s(340.0),
                    s(18.0),
                    Some(hwnd),
                    None,
                    Some(instance),
                    None,
                );

                let edit_class = wide("Edit");
                let edit = CreateWindowExW(
                    WS_EX_CLIENTEDGE,
                    PCWSTR::from_raw(edit_class.as_ptr()),
                    PCWSTR::null(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
                    s(14.0),
                    s(34.0),
                    s(346.0),
                    s(26.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_EDIT as *mut _)),
                    Some(instance),
                    None,
                );

                let button_class = wide("Button");
                let ok = wide("OK");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button_class.as_ptr()),
                    PCWSTR::from_raw(ok.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(BS_DEFPUSHBUTTON as u32),
                    s(174.0),
                    s(74.0),
                    s(90.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_OK as *mut _)),
                    Some(instance),
                    None,
                );

                let cancel = wide("Cancel");
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE::default(),
                    PCWSTR::from_raw(button_class.as_ptr()),
                    PCWSTR::from_raw(cancel.as_ptr()),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                    s(270.0),
                    s(74.0),
                    s(90.0),
                    s(28.0),
                    Some(hwnd),
                    Some(HMENU(IDC_PROMPT_CANCEL as *mut _)),
                    Some(instance),
                    None,
                );

                if let Ok(edit) = edit {
                    let initial = wide(&state.initial);
                    let _ = SetWindowTextW(edit, PCWSTR::from_raw(initial.as_ptr()));
                    // Preselect the stem so typing replaces the name but keeps
                    // the extension a keystroke away.
                    let stem_len = state
                        .initial
                        .rfind('.')
                        .filter(|i| *i > 0)
                        .unwrap_or(state.initial.chars().count());
                    SendMessageW(
                        edit,
                        EM_SETSEL,
                        Some(WPARAM(0)),
                        Some(LPARAM(stem_len as isize)),
                    );
                    if state.complete_paths {
                        // SHACF_FILESYS_ONLY plus a forced dropdown: typing a
                        // parent folder then a backslash lists what is in it.
                        let _ = SHAutoComplete(
                            edit,
                            SHACF_FILESYS_ONLY | SHACF_AUTOSUGGEST_FORCE_ON | SHACF_USETAB,
                        );
                    }
                    let _ = SetFocus(Some(edit));
                }
                LRESULT(0)
            }

            WM_COMMAND => {
                let id = (wparam.0 & 0xFFFF) as i32;
                let st = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState;
                if id == IDC_PROMPT_OK && !st.is_null() {
                    if let Ok(edit) = GetDlgItem(Some(hwnd), IDC_PROMPT_EDIT) {
                        // Ask how long the text is instead of assuming MAX_PATH,
                        // which silently truncated long names before.
                        let len = GetWindowTextLengthW(edit).max(0) as usize;
                        let mut buf = vec![0u16; len + 1];
                        let n = GetWindowTextW(edit, &mut buf);
                        let text = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
                        (*st).result = Some(text.trim().to_string());
                        (*st).accepted = true;
                    }
                    let _ = DestroyWindow(hwnd);
                } else if id == IDC_PROMPT_CANCEL {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }

            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }

            // The owner frees the state; the dialog only clears its pointer.
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }

            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}



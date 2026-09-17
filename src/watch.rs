// Directory change notification.
//
// One blocking `ReadDirectoryChangesW` loop per watched pane, on its own thread.
// Blocking rather than overlapped because there is nothing else for that thread
// to do, and `CancelIoEx` from the UI thread is the documented way to unblock
// it — which is what `stop()` does.
//
// The thread posts a bare message with no payload: by the time the UI reacts,
// the only listing worth re-reading is whatever the pane is showing *now*, so
// there is nothing useful to carry across.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LPARAM, WPARAM};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;


/// A handle passed between threads. Windows handles are process-wide values;
/// the only rule we need to honour is that nobody uses one after it is closed,
/// which the stop flag and the single owning thread guarantee.
#[derive(Clone, Copy)]
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

#[derive(Clone, Copy)]
struct SendHwnd(windows::Win32::Foundation::HWND);
unsafe impl Send for SendHwnd {}

// Read these through a method, never the field. Edition-2021 closures capture
// disjoint fields, so `wrapper.0` inside a `move` closure would capture the
// bare non-Send handle and defeat the wrapper entirely.
impl SendHandle {
    fn get(self) -> HANDLE {
        self.0
    }
}
impl SendHwnd {
    fn get(self) -> windows::Win32::Foundation::HWND {
        self.0
    }
}

pub struct Watcher {
    handle: SendHandle,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Begin watching `path`. Returns None if the directory cannot be opened,
    /// which is not an error worth reporting: it just means no live updates.
    pub fn start(
        hwnd: windows::Win32::Foundation::HWND,
        pid: usize,
        message: u32,
        path: &str,
    ) -> Option<Watcher> {
        if path.is_empty() {
            return None;
        }
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();

        // FILE_FLAG_BACKUP_SEMANTICS is what makes CreateFileW open a directory.
        // Sharing everything so our watch never blocks anyone else's rename or
        // delete of this folder.
        let handle = unsafe {
            CreateFileW(
                PCWSTR::from_raw(wide.as_ptr()),
                FILE_LIST_DIRECTORY.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .ok()?;

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let thread_handle = SendHandle(handle);
        let thread_hwnd = SendHwnd(hwnd);


        std::thread::spawn(move || {
            let handle = thread_handle.get();
            let hwnd = thread_hwnd.get();
            let mut buf = vec![0u8; 8192];
            loop {
                if thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                let mut returned: u32 = 0;
                let ok = unsafe {
                    ReadDirectoryChangesW(
                        handle,
                        buf.as_mut_ptr() as *mut _,
                        buf.len() as u32,
                        false, // this directory only; a recursive watch on a
                        // large tree is a lot of kernel work for a list we
                        // are not showing.
                        FILE_NOTIFY_CHANGE_FILE_NAME
                            | FILE_NOTIFY_CHANGE_DIR_NAME
                            | FILE_NOTIFY_CHANGE_ATTRIBUTES
                            | FILE_NOTIFY_CHANGE_SIZE
                            | FILE_NOTIFY_CHANGE_LAST_WRITE,
                        Some(&mut returned),
                        None,
                        None,
                    )
                };
                if ok.is_err() || thread_stop.load(Ordering::Relaxed) {
                    break;
                }
                // The contents of the buffer are ignored on purpose: the UI is
                // going to re-read the whole directory anyway, and parsing the
                // record list would only tell us something we discard.
                let posted = unsafe {
                    PostMessageW(Some(hwnd), message, WPARAM(pid), LPARAM(0))
                };
                if posted.is_err() {
                    break;
                }
            }
            unsafe {
                let _ = CloseHandle(handle);
            }
        });

        Some(Watcher {
            handle: SendHandle(handle),
            stop,
        })
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblocks the thread's ReadDirectoryChangesW; the thread then closes
        // the handle itself, so there is exactly one owner of the close.
        unsafe {
            let _ = CancelIoEx(self.handle.0, None);
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}

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
use std::sync::{Arc, Mutex};

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
    /// Filled in by the watching thread once the directory is open, and
    /// closed by that same thread when it exits. The lock is what lets the UI
    /// cancel a read without racing that close.
    handle: Arc<Mutex<Option<SendHandle>>>,
    stop: Arc<AtomicBool>,
}

impl Watcher {
    /// Begin watching `path`.
    ///
    /// Returns immediately: opening the directory happens on the watching
    /// thread, because `CreateFileW` on an unreachable share blocks until SMB
    /// gives up, and this is called from the UI thread every time a pane
    /// navigates. A share that is merely slow used to freeze the window.
    ///
    /// Failing to open is not an error worth reporting — it only means no
    /// live updates for that folder.
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

        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let shared: Arc<Mutex<Option<SendHandle>>> = Arc::new(Mutex::new(None));
        let thread_shared = shared.clone();
        let thread_hwnd = SendHwnd(hwnd);

        std::thread::spawn(move || {
            // FILE_FLAG_BACKUP_SEMANTICS is what makes CreateFileW open a
            // directory. Sharing everything so our watch never blocks anyone
            // else's rename or delete of this folder.
            let Ok(handle) = (unsafe {
                CreateFileW(
                    PCWSTR::from_raw(wide.as_ptr()),
                    FILE_LIST_DIRECTORY.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAG_BACKUP_SEMANTICS,
                    None,
                )
            }) else {
                return;
            };
            // The pane may already have moved on while the share was timing
            // out. Publishing the handle and checking the flag under one lock
            // is what keeps `stop` from missing it.
            {
                let mut slot = thread_shared.lock().unwrap_or_else(|e| e.into_inner());
                if thread_stop.load(Ordering::Relaxed) {
                    unsafe {
                        let _ = CloseHandle(handle);
                    }
                    return;
                }
                *slot = Some(SendHandle(handle));
            }
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
            // This thread is the only closer. Taking it under the lock is
            // what stops `stop` from cancelling a handle that is already gone.
            let taken = thread_shared
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            if let Some(h) = taken {
                unsafe {
                    let _ = CloseHandle(h.get());
                }
            }
        });

        Some(Watcher { handle: shared, stop })
    }

    fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        // The thread is still the only one that closes the handle, as before.
        // Cancelling under the same lock it closes under is what stops us
        // cancelling a handle it has just closed and Windows has handed to
        // somebody else.
        //
        // If the directory is not open yet — a share that is still timing
        // out — there is nothing to cancel, and the flag set above is what
        // the thread checks before it publishes a handle at all.
        let slot = self.handle.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(h) = *slot {
            unsafe {
                let _ = CancelIoEx(h.get(), None);
            }
        }
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.stop();
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Foundation::HWND;

    #[test]
    fn starting_a_watch_never_waits_for_the_directory() {
        // The whole point of opening on the thread: an unreachable share makes
        // CreateFileW sit there until SMB gives up, and this is called from the
        // UI thread on every navigation. A .invalid host cannot resolve, by
        // RFC 2606, so this is the slow case on any machine.
        let path = concat!(r"\\", r"\\", "no-such-host.invalid", r"\\", "share");
        let began = std::time::Instant::now();
        let w = Watcher::start(HWND::default(), 0, 0, path);
        assert!(
            began.elapsed() < std::time::Duration::from_millis(250),
            "start blocked for {:?}",
            began.elapsed()
        );
        // And stopping one whose handle was never published is not a crash:
        // the thread is still inside CreateFileW at this point.
        drop(w);
    }

    #[test]
    fn a_watch_on_a_real_folder_starts_and_stops_cleanly() {
        let dir = std::env::temp_dir().to_string_lossy().into_owned();
        let w = Watcher::start(HWND::default(), 0, 0, &dir).expect("a watcher");
        // Long enough for the thread to have opened the directory and be
        // blocked in ReadDirectoryChangesW, which is the case `stop` cancels.
        std::thread::sleep(std::time::Duration::from_millis(120));
        drop(w);
    }

    #[test]
    fn an_empty_path_is_not_watched_at_all() {
        assert!(Watcher::start(HWND::default(), 0, 0, "").is_none());
    }
}

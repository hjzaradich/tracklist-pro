//! Hears Windows announce that a volume arrived or left (`WM_DEVICECHANGE`),
//! so the app can look at its volumes again (1aB-9).
//!
//! A drive letter says nothing about which drive is behind it: unplug one
//! USB drive and plug in another, and both are `E:`. Anything that learned
//! "volume X is at `E:\`" before the swap would open the new drive's files
//! as X's. Windows broadcasts `WM_DEVICECHANGE` to every top-level window
//! when a volume (a disk, a card, a mapped share) arrives or is removed, so
//! a hidden window on its own thread listens for it.
//!
//! Each volume message is passed on at once (`on_each`), so anything that
//! answers "where is volume X" stops trusting what it knew before the next
//! answer. The slower follow-up (`on_settled`: updating the database,
//! telling the frontend) runs once per burst of messages (one per
//! partition, arrival then letter assignment), when the burst has been
//! quiet for [`SETTLE`], or at the latest [`SETTLE_AT_MOST`] after it began.

use std::cell::RefCell;
use std::io;
use std::iter;
use std::ptr;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
    DBT_DEVICEARRIVAL, DBT_DEVICEREMOVECOMPLETE, DBT_DEVTYP_VOLUME, DEV_BROADCAST_HDR, MSG,
    WM_DEVICECHANGE, WNDCLASSW,
};

/// How long the messages must stop before `on_settled` runs.
pub const SETTLE: Duration = Duration::from_millis(500);
/// How long a burst that never stops can hold `on_settled` back.
pub const SETTLE_AT_MOST: Duration = Duration::from_secs(5);

/// A running watch. It lives as long as the process.
#[derive(Debug, Clone, Copy)]
pub struct DeviceWatch {
    /// The hidden window, as a number so the watch can cross threads.
    /// Tests send it messages.
    #[cfg_attr(not(test), allow(dead_code))]
    window: isize,
}

impl DeviceWatch {
    #[cfg(test)]
    pub(crate) fn window(&self) -> HWND {
        self.window as HWND
    }
}

/// What the window's thread does with each volume message: calls `on_each`
/// and wakes the settle thread.
struct Heard {
    on_each: Box<dyn Fn()>,
    settle: mpsc::Sender<()>,
}

thread_local! {
    static HEARD: RefCell<Option<Heard>> = const { RefCell::new(None) };
}

/// Starts watching. For every volume arrival or removal, `on_each` runs at
/// once, on the watch's own thread, so keep it quick (e.g. bump a
/// counter). `on_settled` runs on a thread of its own, once per burst.
pub fn watch(
    on_each: impl Fn() + Send + 'static,
    on_settled: impl Fn() + Send + 'static,
) -> io::Result<DeviceWatch> {
    watch_with(SETTLE, SETTLE_AT_MOST, on_each, on_settled)
}

/// [`watch`], with the settle times given.
fn watch_with(
    quiet: Duration,
    at_most: Duration,
    on_each: impl Fn() + Send + 'static,
    on_settled: impl Fn() + Send + 'static,
) -> io::Result<DeviceWatch> {
    let (heard, messages) = mpsc::channel::<()>();
    thread::Builder::new()
        .name("device-settle".into())
        .spawn(move || settle(&messages, quiet, at_most, on_settled))?;

    let (made, window) = mpsc::channel();
    thread::Builder::new()
        .name("device-watch".into())
        .spawn(move || {
            HEARD.with(|h| {
                *h.borrow_mut() = Some(Heard {
                    on_each: Box::new(on_each),
                    settle: heard,
                })
            });
            // As a number: a window handle can't cross threads as a pointer.
            let window = hidden_window().map(|w| w as isize);
            let ok = window.is_ok();
            let _ = made.send(window);
            if ok {
                pump();
            }
        })?;
    let window = window
        .recv()
        .map_err(|_| io::Error::other("the device watch thread stopped"))??;
    Ok(DeviceWatch { window })
}

/// Calls `on_settled` once for each burst of messages: after the burst has
/// been quiet for `quiet`, or `at_most` after it began if it goes on.
/// Returns when every sender is gone.
fn settle(
    messages: &mpsc::Receiver<()>,
    quiet: Duration,
    at_most: Duration,
    on_settled: impl Fn(),
) {
    while messages.recv().is_ok() {
        let latest = Instant::now() + at_most;
        loop {
            let left = latest.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match messages.recv_timeout(quiet.min(left)) {
                Ok(()) => continue,
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => {
                    on_settled();
                    return;
                }
            }
        }
        on_settled();
    }
}

/// Whether a `WM_DEVICECHANGE` is about a volume arriving or leaving.
/// `header` is the message's `DEV_BROADCAST_HDR`, if it has one.
fn is_volume_change(event: WPARAM, header: Option<&DEV_BROADCAST_HDR>) -> bool {
    let event = event as u32;
    (event == DBT_DEVICEARRIVAL || event == DBT_DEVICEREMOVECOMPLETE)
        && header.is_some_and(|h| h.dbch_devicetype == DBT_DEVTYP_VOLUME)
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DEVICECHANGE {
        // For arrivals and removals, `lparam` points at the header of the
        // device's broadcast structure; for other events it may be 0.
        let header = (lparam as *const DEV_BROADCAST_HDR).as_ref();
        if is_volume_change(wparam, header) {
            HEARD.with(|h| {
                if let Some(heard) = h.borrow().as_ref() {
                    (heard.on_each)();
                    let _ = heard.settle.send(());
                }
            });
        }
        // TRUE: nothing here refuses a device change.
        return 1;
    }
    DefWindowProcW(window, message, wparam, lparam)
}

/// A top-level window that's never shown: `WM_DEVICECHANGE` is broadcast
/// to top-level windows only, not to message-only ones.
fn hidden_window() -> io::Result<HWND> {
    let class: Vec<u16> = "tracklist-pro device watch"
        .encode_utf16()
        .chain(iter::once(0))
        .collect();
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class.as_ptr(),
        ..WNDCLASSW::default()
    };
    // Registered once per process; a second watch reuses the class.
    if unsafe { RegisterClassW(&wc) } == 0
        && unsafe { GetLastError() } != ERROR_CLASS_ALREADY_EXISTS
    {
        return Err(io::Error::last_os_error());
    }
    // No WS_VISIBLE, and it's never shown.
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            instance,
            ptr::null(),
        )
    };
    if window.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(window)
}

/// Delivers the window's messages until the thread's queue closes.
fn pump() {
    let mut message = MSG::default();
    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
        unsafe { DispatchMessageW(&message) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        SendMessageW, DBT_DEVICEQUERYREMOVE, DBT_DEVTYP_PORT, DEV_BROADCAST_VOLUME,
    };

    fn volume_message(kind: u32) -> DEV_BROADCAST_VOLUME {
        DEV_BROADCAST_VOLUME {
            dbcv_size: std::mem::size_of::<DEV_BROADCAST_VOLUME>() as u32,
            dbcv_devicetype: kind,
            dbcv_unitmask: 1 << 4, // E:
            ..Default::default()
        }
    }

    fn send(watch: &DeviceWatch, event: u32, body: Option<&DEV_BROADCAST_VOLUME>) {
        let lparam = body.map_or(0, |b| b as *const DEV_BROADCAST_VOLUME as LPARAM);
        unsafe { SendMessageW(watch.window(), WM_DEVICECHANGE, event as WPARAM, lparam) };
    }

    /// A watch that counts both callbacks. The quiet time is long, so a
    /// test's burst stays one burst even on a busy machine.
    fn counting() -> (Arc<AtomicUsize>, Arc<AtomicUsize>, DeviceWatch) {
        let (each, settled) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (e, d) = (each.clone(), settled.clone());
        let watch = watch_with(
            Duration::from_secs(2),
            Duration::from_secs(60),
            move || {
                e.fetch_add(1, Ordering::SeqCst);
            },
            move || {
                d.fetch_add(1, Ordering::SeqCst);
            },
        )
        .unwrap();
        (each, settled, watch)
    }

    fn wait_for(calls: &AtomicUsize, n: usize) {
        let start = Instant::now();
        while calls.load(Ordering::SeqCst) < n {
            assert!(start.elapsed() < Duration::from_secs(30), "never called");
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn every_volume_message_is_passed_on_at_once_and_a_burst_settles_into_one_call() {
        let (each, settled, watch) = counting();
        let volume = volume_message(DBT_DEVTYP_VOLUME);
        for event in [
            DBT_DEVICEREMOVECOMPLETE,
            DBT_DEVICEARRIVAL,
            DBT_DEVICEARRIVAL,
        ] {
            send(&watch, event, Some(&volume));
        }
        // SendMessageW returns after the window handled it: no waiting.
        assert_eq!(each.load(Ordering::SeqCst), 3);
        assert_eq!(settled.load(Ordering::SeqCst), 0);
        wait_for(&settled, 1);

        // A later change is another burst.
        send(&watch, DBT_DEVICEARRIVAL, Some(&volume));
        assert_eq!(each.load(Ordering::SeqCst), 4);
        wait_for(&settled, 2);
        assert_eq!(settled.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn device_messages_that_are_not_a_volume_arriving_or_leaving_are_ignored() {
        let (each, settled, watch) = counting();
        // A serial port (e.g. a MIDI controller's) arriving.
        send(
            &watch,
            DBT_DEVICEARRIVAL,
            Some(&volume_message(DBT_DEVTYP_PORT)),
        );
        // Windows asking whether a volume may be removed: not removed yet.
        send(
            &watch,
            DBT_DEVICEQUERYREMOVE,
            Some(&volume_message(DBT_DEVTYP_VOLUME)),
        );
        // An arrival with no details at all.
        send(&watch, DBT_DEVICEARRIVAL, None);
        assert_eq!(each.load(Ordering::SeqCst), 0);
        thread::sleep(Duration::from_millis(200));
        assert_eq!(settled.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_burst_that_never_goes_quiet_still_settles_after_the_longest_wait() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (heard, messages) = mpsc::channel();
        let counter = calls.clone();
        let settler = thread::spawn(move || {
            // Quiet never comes; only the cap can end the burst.
            settle(
                &messages,
                Duration::from_secs(3600),
                Duration::from_millis(100),
                move || {
                    counter.fetch_add(1, Ordering::SeqCst);
                },
            )
        });
        let start = Instant::now();
        while calls.load(Ordering::SeqCst) == 0 {
            assert!(start.elapsed() < Duration::from_secs(30), "never settled");
            heard.send(()).unwrap();
            thread::sleep(Duration::from_millis(10));
        }
        drop(heard);
        settler.join().unwrap();
    }

    #[test]
    fn a_change_still_in_progress_when_the_app_stops_listening_is_passed_on() {
        let calls = Arc::new(AtomicUsize::new(0));
        let (heard, messages) = mpsc::channel();
        heard.send(()).unwrap();
        drop(heard);
        let counter = calls.clone();
        settle(
            &messages,
            Duration::from_secs(60),
            Duration::from_secs(60),
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
            },
        );
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

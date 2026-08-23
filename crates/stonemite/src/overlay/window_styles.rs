use std::collections::{HashMap, HashSet};
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
};

use super::clients::ClientRegistry;

pub(super) struct WindowStyleState {
    hide_background: bool,
    originals: HashMap<isize, isize>,
}

impl WindowStyleState {
    pub(super) fn new(hide_background: bool) -> Self {
        Self {
            hide_background,
            originals: HashMap::new(),
        }
    }

    pub(super) fn hide_background(&self) -> bool {
        self.hide_background
    }

    pub(super) fn set_hide_background(&mut self, enabled: bool) {
        self.hide_background = enabled;
    }

    pub(super) unsafe fn apply(&mut self, clients: &ClientRegistry) {
        if !self.hide_background {
            return;
        }
        let active_pid = clients.active_pid();
        for window in &clients.windows {
            if active_pid == Some(window.pid) {
                self.restore(window.hwnd);
            } else {
                self.hide(window.hwnd);
            }
        }
        let live_hwnds: HashSet<isize> = clients
            .windows
            .iter()
            .map(|window| window.hwnd.0 as isize)
            .collect();
        self.originals.retain(|hwnd, _| live_hwnds.contains(hwnd));
    }

    pub(super) unsafe fn restore_all(&mut self, clients: &ClientRegistry) {
        for window in &clients.windows {
            self.restore(window.hwnd);
        }
    }

    unsafe fn hide(&mut self, hwnd: HWND) {
        let key = hwnd.0 as isize;
        self.originals
            .entry(key)
            .or_insert_with(|| GetWindowLongPtrW(hwnd, GWL_EXSTYLE));
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let hidden = (style | WS_EX_TOOLWINDOW.0 as isize) & !(WS_EX_APPWINDOW.0 as isize);
        set_extended_style(hwnd, hidden);
    }

    unsafe fn restore(&mut self, hwnd: HWND) {
        if let Some(original) = self.originals.get(&(hwnd.0 as isize)).copied() {
            set_extended_style(hwnd, original);
        }
    }
}

enum Request {
    Set { hwnd: usize, style: isize },
    Flush(mpsc::SyncSender<()>),
}

fn sender() -> &'static mpsc::Sender<Request> {
    static SENDER: OnceLock<mpsc::Sender<Request>> = OnceLock::new();
    SENDER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("stonemite-window-styles".to_owned())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    match request {
                        Request::Set { hwnd, style } => unsafe {
                            SetWindowLongPtrW(HWND(hwnd as *mut _), GWL_EXSTYLE, style);
                        },
                        Request::Flush(done) => {
                            let _ = done.send(());
                        }
                    }
                }
            })
            .expect("failed to start the window-style worker");
        sender
    })
}

/// Queue an extended-style update on one ordered worker so rapid active-client
/// changes cannot reorder hide and restore operations.
pub(super) fn set_extended_style(hwnd: HWND, style: isize) {
    let _ = sender().send(Request::Set {
        hwnd: hwnd.0 as usize,
        style,
    });
}

/// Wait briefly for all previously queued style writes to finish during shutdown.
pub(super) fn flush() -> bool {
    let (done, completed) = mpsc::sync_channel(0);
    sender().send(Request::Flush(done)).is_ok()
        && completed.recv_timeout(Duration::from_secs(2)).is_ok()
}

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetWindowThreadProcessId};

use super::state::OverlayState;

pub(super) fn publish(state: &OverlayState) {
    let sources = state
        .clients
        .windows
        .iter()
        .map(|window| trushar::control::SourceClient {
            private_key: u64::from(window.pid),
            character: window.character.clone(),
            server: window.server.clone(),
            class_code: window.class.clone(),
            window_number: window.number,
            active: state.clients.active_pid() == Some(window.pid),
            activatable: state.clients.active_pid() == Some(window.pid)
                || state.clients.pips().contains(&window.pid),
            input_ready: crate::broadcast::is_target_ready(window.pid),
        })
        .collect();
    let mouse_clutch = trushar::control::MouseClutchState {
        phase: match crate::broadcast::mouse_clutch_status() {
            crate::broadcast::MouseClutchStatus::Inactive => {
                trushar::control::MouseClutchPhase::Inactive
            }
            crate::broadcast::MouseClutchStatus::Active => {
                trushar::control::MouseClutchPhase::Active
            }
            crate::broadcast::MouseClutchStatus::Releasing => {
                trushar::control::MouseClutchPhase::Releasing
            }
        },
        availability: match crate::broadcast::mouse_clutch_availability() {
            crate::broadcast::MouseClutchAvailability::Ready => {
                trushar::control::MouseClutchAvailability::Ready
            }
            crate::broadcast::MouseClutchAvailability::NoActiveClient => {
                trushar::control::MouseClutchAvailability::NoActiveClient
            }
            crate::broadcast::MouseClutchAvailability::NoCompatibleTargets => {
                trushar::control::MouseClutchAvailability::NoCompatibleTargets
            }
            crate::broadcast::MouseClutchAvailability::InputUnavailable => {
                trushar::control::MouseClutchAvailability::InputUnavailable
            }
        },
    };
    crate::control::publish(
        sources,
        trushar::control::BroadcastState {
            available: crate::broadcast::is_available(),
            enabled: crate::broadcast::is_active(),
        },
        mouse_clutch,
    );
}

fn is_overlay_window(hwnd: HWND, state: &OverlayState) -> bool {
    if hwnd == state.presentation.menu_owner_hwnd
        || hwnd == state.presentation.active_label_hwnd
        || hwnd == state.presentation.broadcast_label_hwnd
        || hwnd == state.presentation.toast.hwnd
    {
        return true;
    }
    state
        .presentation
        .pip_windows
        .iter()
        .any(|pip| pip.hwnd == hwnd || pip.label_hwnd == hwnd)
        || state
            .presentation
            .pending_composition_destroys
            .contains(&hwnd)
}

#[derive(Clone, Copy, Eq, PartialEq)]
struct MouseGeometry {
    origin_x: i32,
    origin_y: i32,
    width: i32,
    height: i32,
    dpi: u32,
}

unsafe fn mouse_geometry(hwnd: HWND) -> Option<MouseGeometry> {
    let mut client = RECT::default();
    GetClientRect(hwnd, &mut client).ok()?;
    let mut origin = POINT::default();
    if !ClientToScreen(hwnd, &mut origin).as_bool() {
        return None;
    }
    Some(MouseGeometry {
        origin_x: origin.x,
        origin_y: origin.y,
        width: client.right - client.left,
        height: client.bottom - client.top,
        dpi: GetDpiForWindow(hwnd),
    })
}

pub(super) fn sync_mouse_eligibility(state: &OverlayState) {
    let eligible = unsafe {
        state
            .clients
            .active_pid()
            .and_then(|source_pid| {
                let source = state
                    .clients
                    .windows
                    .iter()
                    .find(|window| window.pid == source_pid)?;
                let source_geometry = mouse_geometry(source.hwnd)?;
                Some(
                    state
                        .clients
                        .windows
                        .iter()
                        .filter(|window| mouse_geometry(window.hwnd) == Some(source_geometry))
                        .map(|window| window.pid)
                        .collect::<Vec<_>>(),
                )
            })
            .unwrap_or_default()
    };
    crate::broadcast::update_mouse_eligible_pids(&eligible);
}

pub(super) fn owns_foreground(hwnd: HWND, state: &OverlayState) -> bool {
    if is_overlay_window(hwnd, state)
        || unsafe { crate::settings_dialog::foreground_window_is_settings(hwnd) }
    {
        return true;
    }
    if state
        .clients
        .windows
        .iter()
        .any(|window| window.hwnd == hwnd)
    {
        return true;
    }
    if !state.clients.windows.is_empty() {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
        }
        if pid != 0 {
            return state.clients.windows.iter().any(|window| window.pid == pid);
        }
    }
    false
}

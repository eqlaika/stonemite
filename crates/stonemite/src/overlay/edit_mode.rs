use windows::Win32::Foundation::RECT;
use windows::Win32::UI::WindowsAndMessaging::GetWindowRect;

use super::runtime::{try_with_state, try_with_state_mut};
use super::state::OverlayState;
use super::surfaces::request_redraw;
use crate::config;

pub(super) fn toggle() {
    let _ = try_with_state_mut(|state| unsafe { toggle_inner(state) });
}

pub(super) fn is_active() -> bool {
    try_with_state(|state| state.interaction.edit_mode).unwrap_or(false)
}

pub(super) unsafe fn toggle_inner(state: &mut OverlayState) {
    if state.interaction.edit_mode {
        let mut positions = Vec::new();
        for (index, pip) in state.presentation.pip_windows.iter().enumerate() {
            let mut rect = RECT::default();
            let _ = GetWindowRect(pip.hwnd, &mut rect);
            positions.push(config::PipPosition {
                slot: index,
                x: rect.left,
                y: rect.top,
                width: (rect.right - rect.left) as u32,
                height: (rect.bottom - rect.top) as u32,
            });
        }
        let mut config = config::Config::load();
        config.pip_positions = positions;
        let _ = config.save();
        state.layout.has_custom_positions = true;
        state.interaction.edit_mode = false;
    } else {
        state.interaction.edit_mode = true;
    }
    for pip in &state.presentation.pip_windows {
        request_redraw(pip.label_hwnd);
    }
}

use std::collections::HashSet;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;

use super::state::OverlayState;
use super::surfaces::request_redraw;
use super::timers;

pub(super) const TIMER_ID: usize = 44;
pub(super) const INTERVAL_MS: u32 = 100;

pub(super) fn active_timer(state: &OverlayState, now: Instant) -> Option<&timers::TimerOverlay> {
    let source_id = state.clients.active_pid().map(|pid| format!("pid:{pid}"));
    state.timers.visible_for(source_id.as_deref(), true, now)
}

pub(super) fn format_remaining(remaining: Duration) -> String {
    let tenths = remaining.as_millis().div_ceil(100);
    format!("{}.{:01}s", tenths / 10, tenths % 10)
}

pub(super) fn owner_hwnds(state: &OverlayState, now: Instant) -> Vec<HWND> {
    let mut owners = Vec::new();
    let active_source = state.clients.active_pid().map(|pid| format!("pid:{pid}"));
    if state
        .timers
        .visible_for(active_source.as_deref(), true, now)
        .is_some()
    {
        owners.push(state.presentation.active_label_hwnd);
    }
    for pip in &state.presentation.pip_windows {
        let source_id = format!("pid:{}", pip.pid);
        if state
            .timers
            .visible_for(Some(&source_id), false, now)
            .is_some()
        {
            owners.push(pip.label_hwnd);
        }
    }
    owners
}

pub(super) fn redraw_targets(
    expired: bool,
    active_label_hwnd: HWND,
    pip_label_hwnds: &[HWND],
    previous_owners: &[HWND],
    current_owners: &[HWND],
) -> Vec<HWND> {
    let mut targets = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |hwnd: HWND| {
        if !(hwnd.is_invalid() || expired && hwnd == active_label_hwnd)
            && seen.insert(hwnd.0 as isize)
        {
            targets.push(hwnd);
        }
    };
    if expired {
        // Expired timers are already excluded by visible_for. Redrawing the
        // bounded PiP set clears whichever client owned the expired scene.
        pip_label_hwnds.iter().copied().for_each(&mut add);
    } else {
        previous_owners
            .iter()
            .chain(current_owners)
            .copied()
            .for_each(add);
    }
    targets
}

pub(super) unsafe fn invalidate_labels(state: &OverlayState) {
    for hwnd in owner_hwnds(state, Instant::now()) {
        if hwnd != state.presentation.active_label_hwnd {
            request_redraw(hwnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_rounds_up_to_the_next_tenth() {
        assert_eq!(format_remaining(Duration::from_secs(10)), "10.0s");
        assert_eq!(format_remaining(Duration::from_millis(9901)), "10.0s");
        assert_eq!(format_remaining(Duration::from_millis(9900)), "9.9s");
        assert_eq!(format_remaining(Duration::from_millis(1)), "0.1s");
        assert_eq!(format_remaining(Duration::ZERO), "0.0s");
    }

    #[test]
    fn expiry_redraws_every_pip_after_visible_owners_are_empty() {
        let active = HWND(1usize as *mut _);
        let first_pip = HWND(2usize as *mut _);
        let expired_owner = HWND(3usize as *mut _);

        assert_eq!(
            redraw_targets(true, active, &[first_pip, expired_owner], &[], &[]),
            vec![first_pip, expired_owner]
        );
        assert_eq!(
            redraw_targets(
                false,
                active,
                &[first_pip, expired_owner],
                &[expired_owner],
                &[expired_owner],
            ),
            vec![expired_owner]
        );
    }
}

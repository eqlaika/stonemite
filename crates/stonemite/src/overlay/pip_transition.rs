use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmFlush, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES,
    DWM_TNP_OPACITY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, KillTimer, SetTimer, ShowWindow, SW_HIDE,
};

use super::presentation::PipWindowEntry;
use super::state::OverlayState;
use super::surfaces::{position_window_pair, unregister_composition_surface};
use crate::diagnostics::debug_log;

pub(super) const TIMER_ID: usize = 45;
const FRAME_INTERVAL_MS: u32 = 15;
const DURATION_MS: u64 = 180;

#[derive(Clone, Copy)]
pub(super) struct PipMotion {
    pub(super) hwnd: HWND,
    pub(super) label_hwnd: HWND,
    pub(super) from: RECT,
    pub(super) to: RECT,
}

#[derive(Clone, Copy)]
pub(super) struct ThumbnailHandoff {
    pub(super) outgoing: isize,
    pub(super) incoming: isize,
    pub(super) switched: bool,
}

pub(super) struct PipTransition {
    started_at: Instant,
    pub(super) motions: Vec<PipMotion>,
    pub(super) handoffs: Vec<ThumbnailHandoff>,
    normal_alpha: u8,
}

impl PipTransition {
    pub(super) fn new(normal_alpha: u8) -> Self {
        Self {
            started_at: Instant::now(),
            motions: Vec::new(),
            handoffs: Vec::new(),
            normal_alpha,
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.motions.is_empty() && self.handoffs.is_empty()
    }
}

fn ease_out_expo(progress: f64) -> f64 {
    let progress = progress.clamp(0.0, 1.0);
    if progress == 0.0 || progress == 1.0 {
        progress
    } else {
        1.0 - 2.0f64.powf(-10.0 * progress)
    }
}

fn interpolate(start: i32, end: i32, progress: f64) -> i32 {
    (f64::from(start) + f64::from(end - start) * progress).round() as i32
}

fn interpolate_rect(from: RECT, to: RECT, progress: f64) -> RECT {
    RECT {
        left: interpolate(from.left, to.left, progress),
        top: interpolate(from.top, to.top, progress),
        right: interpolate(from.right, to.right, progress),
        bottom: interpolate(from.bottom, to.bottom, progress),
    }
}

unsafe fn set_thumbnail_alpha(thumbnail: isize, alpha: u8) -> bool {
    if thumbnail == 0 {
        return false;
    }
    let properties = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_OPACITY,
        opacity: alpha,
        ..Default::default()
    };
    match DwmUpdateThumbnailProperties(thumbnail, &properties) {
        Ok(()) => true,
        Err(error) => {
            debug_log(&format!("DWM PiP transition update failed: {error}"));
            false
        }
    }
}

pub(super) unsafe fn destroy_pip_window(s: &mut OverlayState, pip: PipWindowEntry) {
    let composition_detached = unregister_composition_surface(s, pip.label_hwnd);
    if pip.thumb != 0 {
        let _ = DwmUnregisterThumbnail(pip.thumb);
    }
    if !pip.label_hwnd.is_invalid() {
        if composition_detached {
            let _ = DestroyWindow(pip.label_hwnd);
        } else {
            let _ = ShowWindow(pip.label_hwnd, SW_HIDE);
            if !s
                .presentation
                .pending_composition_destroys
                .contains(&pip.label_hwnd)
            {
                s.presentation
                    .pending_composition_destroys
                    .push(pip.label_hwnd);
            }
        }
    }
    if !pip.hwnd.is_invalid() {
        let _ = DestroyWindow(pip.hwnd);
    }
}

unsafe fn position_at_destination(transition: &PipTransition) {
    for motion in &transition.motions {
        let width = motion.to.right - motion.to.left;
        let height = motion.to.bottom - motion.to.top;
        position_window_pair(
            motion.hwnd,
            motion.label_hwnd,
            motion.to.left,
            motion.to.top,
            width,
            height,
        );
    }
}

unsafe fn restore_outgoing_frame(transition: &PipTransition) {
    for handoff in &transition.handoffs {
        if !handoff.switched {
            let _ = set_thumbnail_alpha(handoff.outgoing, transition.normal_alpha);
            let _ = set_thumbnail_alpha(handoff.incoming, 0);
        }
    }
    if DwmFlush().is_err() {
        debug_log("DWM PiP transition rollback flush failed");
    }
}

/// Publish every incoming relationship before retiring its outgoing one. The
/// visual transition is handled by HWND motion; avoiding per-frame DWM opacity
/// updates keeps the source handoff crisp on real desktops.
unsafe fn switch_thumbnail_handoffs(transition: &mut PipTransition) -> bool {
    if transition.handoffs.iter().all(|handoff| handoff.switched) {
        return true;
    }
    let mut incoming_ready = true;
    for handoff in &transition.handoffs {
        if !handoff.switched {
            incoming_ready &= set_thumbnail_alpha(handoff.incoming, transition.normal_alpha);
        }
    }
    if !incoming_ready || DwmFlush().is_err() {
        debug_log("DWM PiP handoff switch failed; preserving outgoing thumbnails");
        restore_outgoing_frame(transition);
        return false;
    }
    for handoff in &mut transition.handoffs {
        if !handoff.switched {
            if handoff.outgoing != 0 {
                let _ = DwmUnregisterThumbnail(handoff.outgoing);
                handoff.outgoing = 0;
            }
            handoff.switched = true;
        }
    }
    true
}

unsafe fn settle_transition(mut transition: PipTransition) -> Result<(), PipTransition> {
    position_at_destination(&transition);
    if switch_thumbnail_handoffs(&mut transition) {
        Ok(())
    } else {
        Err(transition)
    }
}

pub(super) unsafe fn start(s: &mut OverlayState, mut transition: PipTransition) {
    debug_assert!(s.presentation.pip_transition.is_none());
    if transition.is_empty() {
        return;
    }
    transition.started_at = Instant::now();
    let _ = switch_thumbnail_handoffs(&mut transition);
    s.presentation.pip_transition = Some(transition);
    if SetTimer(
        s.presentation.active_label_hwnd,
        TIMER_ID,
        FRAME_INTERVAL_MS,
        None,
    ) == 0
    {
        debug_log("PiP transition timer unavailable; settling synchronously");
        let transition = s
            .presentation
            .pip_transition
            .take()
            .expect("transition was installed before timer creation");
        if let Err(transition) = settle_transition(transition) {
            s.presentation.pip_transition = Some(transition);
        }
    }
}

pub(super) unsafe fn finish(s: &mut OverlayState) -> bool {
    let _ = KillTimer(s.presentation.active_label_hwnd, TIMER_ID);
    let Some(transition) = s.presentation.pip_transition.take() else {
        return true;
    };
    match settle_transition(transition) {
        Ok(()) => true,
        Err(transition) => {
            s.presentation.pip_transition = Some(transition);
            false
        }
    }
}

pub(super) unsafe fn force_finish(s: &mut OverlayState) {
    let _ = KillTimer(s.presentation.active_label_hwnd, TIMER_ID);
    let Some(transition) = s.presentation.pip_transition.take() else {
        return;
    };
    if let Err(transition) = settle_transition(transition) {
        // Shutdown cannot leave HWND-owned relationships behind. Visibility no
        // longer matters, so release them even when DWM cannot confirm a frame.
        for handoff in transition.handoffs {
            if handoff.outgoing != 0 {
                let _ = DwmUnregisterThumbnail(handoff.outgoing);
            }
        }
    }
}

pub(super) unsafe fn settle_now(s: &mut OverlayState, transition: PipTransition) {
    debug_assert!(s.presentation.pip_transition.is_none());
    if transition.is_empty() {
        return;
    }
    if let Err(transition) = settle_transition(transition) {
        s.presentation.pip_transition = Some(transition);
    }
}

pub(super) unsafe fn tick(s: &mut OverlayState, timer_hwnd: HWND) {
    let Some(mut transition) = s.presentation.pip_transition.take() else {
        let _ = KillTimer(timer_hwnd, TIMER_ID);
        return;
    };
    let elapsed = transition.started_at.elapsed();
    let linear = elapsed.as_secs_f64() / Duration::from_millis(DURATION_MS).as_secs_f64();
    let complete = linear >= 1.0;
    let progress = ease_out_expo(linear);

    for motion in &transition.motions {
        let rect = interpolate_rect(motion.from, motion.to, progress);
        position_window_pair(
            motion.hwnd,
            motion.label_hwnd,
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        );
    }
    if !transition.handoffs.iter().all(|handoff| handoff.switched) {
        let _ = switch_thumbnail_handoffs(&mut transition);
    }

    if complete {
        match settle_transition(transition) {
            Ok(()) => {
                let _ = KillTimer(timer_hwnd, TIMER_ID);
            }
            Err(transition) => {
                s.presentation.pip_transition = Some(transition);
            }
        }
    } else {
        s.presentation.pip_transition = Some(transition);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exponential_motion_has_exact_endpoints_and_fast_arrival() {
        assert_eq!(ease_out_expo(0.0), 0.0);
        assert_eq!(ease_out_expo(1.0), 1.0);
        assert!(ease_out_expo(0.5) > 0.95);
    }

    #[test]
    fn rectangle_interpolation_reaches_the_authored_destination() {
        let from = RECT {
            left: 0,
            top: 10,
            right: 100,
            bottom: 70,
        };
        let to = RECT {
            left: 40,
            top: 30,
            right: 140,
            bottom: 90,
        };
        assert_eq!(interpolate_rect(from, to, 1.0), to);
    }
}

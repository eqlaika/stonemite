use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::WM_USER;

pub(super) const DEFAULT_HEIGHT: i32 = 64;
pub(super) const DEFAULT_DURATION_MS: u32 = 2000;
pub(super) const FADE_STEP_MS: u32 = 30;
pub(super) const MAX_ALPHA: u8 = 220;
pub(super) const BACKGROUND_COLOR: u32 = 0x00403020;
pub(super) const TIMER_ID: usize = 42;
pub(super) const CLEAR_INVITE_CAPTURE_MESSAGE: u32 = WM_USER + 44;
pub(super) const SERVICE_COMPOSITOR_RECOVERY_MESSAGE: u32 = WM_USER + 45;

const ALPHA_STEP: u8 = 25;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum Phase {
    Hidden,
    FadingIn,
    Visible,
    FadingOut,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum FadeEffect {
    None,
    UpdateOpacity(u8),
    HideAndStop,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) struct FadeTransition {
    pub phase: Phase,
    pub alpha: u8,
    pub phase_start: u64,
    pub effect: FadeEffect,
}

pub(super) fn publication_allowed(
    phase: Phase,
    scene_ready: bool,
    overlay_visible: bool,
    surface_attached: bool,
) -> bool {
    phase != Phase::Hidden && scene_ready && overlay_visible && surface_attached
}

pub(super) fn advance_fade(
    phase: Phase,
    alpha: u8,
    phase_start: u64,
    duration_ms: u32,
    now: u64,
) -> FadeTransition {
    match phase {
        Phase::FadingIn => {
            let alpha = alpha.saturating_add(ALPHA_STEP).min(MAX_ALPHA);
            let (phase, phase_start) = if alpha == MAX_ALPHA {
                (Phase::Visible, now)
            } else {
                (Phase::FadingIn, phase_start)
            };
            FadeTransition {
                phase,
                alpha,
                phase_start,
                effect: FadeEffect::UpdateOpacity(alpha),
            }
        }
        Phase::Visible if now.saturating_sub(phase_start) >= u64::from(duration_ms) => {
            FadeTransition {
                phase: Phase::FadingOut,
                alpha,
                phase_start,
                effect: FadeEffect::None,
            }
        }
        Phase::Visible => FadeTransition {
            phase,
            alpha,
            phase_start,
            effect: FadeEffect::None,
        },
        Phase::FadingOut => {
            let alpha = alpha.saturating_sub(ALPHA_STEP);
            if alpha == 0 {
                FadeTransition {
                    phase: Phase::Hidden,
                    alpha,
                    phase_start,
                    effect: FadeEffect::HideAndStop,
                }
            } else {
                FadeTransition {
                    phase,
                    alpha,
                    phase_start,
                    effect: FadeEffect::UpdateOpacity(alpha),
                }
            }
        }
        Phase::Hidden => FadeTransition {
            phase,
            alpha: 0,
            phase_start,
            effect: FadeEffect::HideAndStop,
        },
    }
}

pub(super) struct ToastState {
    pub hwnd: HWND,
    pub text: String,
    pub phase: Phase,
    /// True only after the current staged text has completed Present1 and attach.
    pub scene_ready: bool,
    pub alpha: u8,
    pub phase_start: u64,
    pub duration_ms: u32,
    pub height: i32,
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_toast_cannot_publish_a_previous_attached_frame() {
        assert!(!publication_allowed(Phase::Hidden, false, true, true));
        assert!(!publication_allowed(Phase::FadingIn, false, true, true));
    }

    #[test]
    fn completed_toast_frame_requires_every_publication_gate() {
        assert!(publication_allowed(Phase::FadingIn, true, true, true));
        assert!(!publication_allowed(Phase::Hidden, true, true, true));
        assert!(!publication_allowed(Phase::FadingIn, true, false, true));
        assert!(!publication_allowed(Phase::FadingIn, true, true, false));
    }

    #[test]
    fn fade_in_saturates_and_starts_visible_duration_at_that_tick() {
        assert_eq!(
            advance_fade(Phase::FadingIn, 25, 10, 2_000, 40),
            FadeTransition {
                phase: Phase::FadingIn,
                alpha: 50,
                phase_start: 10,
                effect: FadeEffect::UpdateOpacity(50),
            }
        );
        assert_eq!(
            advance_fade(Phase::FadingIn, 210, 10, 2_000, 40),
            FadeTransition {
                phase: Phase::Visible,
                alpha: MAX_ALPHA,
                phase_start: 40,
                effect: FadeEffect::UpdateOpacity(MAX_ALPHA),
            }
        );
    }

    #[test]
    fn visible_duration_transitions_at_the_inclusive_threshold() {
        assert_eq!(
            advance_fade(Phase::Visible, 220, 100, 2_000, 2_099).phase,
            Phase::Visible
        );
        let transition = advance_fade(Phase::Visible, 220, 100, 2_000, 2_100);
        assert_eq!(transition.phase, Phase::FadingOut);
        assert_eq!(transition.effect, FadeEffect::None);
    }

    #[test]
    fn fade_out_updates_opacity_then_hides_and_stops_at_zero() {
        assert_eq!(
            advance_fade(Phase::FadingOut, 50, 100, 2_000, 200).effect,
            FadeEffect::UpdateOpacity(25)
        );
        assert_eq!(
            advance_fade(Phase::FadingOut, 25, 100, 2_000, 200),
            FadeTransition {
                phase: Phase::Hidden,
                alpha: 0,
                phase_start: 100,
                effect: FadeEffect::HideAndStop,
            }
        );
    }

    #[test]
    fn hidden_toast_always_requests_hide_and_timer_stop() {
        assert_eq!(
            advance_fade(Phase::Hidden, 12, 100, 2_000, 200),
            FadeTransition {
                phase: Phase::Hidden,
                alpha: 0,
                phase_start: 100,
                effect: FadeEffect::HideAndStop,
            }
        );
    }
}

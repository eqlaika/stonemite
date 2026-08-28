use std::collections::HashMap;

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::geometry::scale as pixels;
use super::labels::{Color, Rect};
use super::log_sources::pid_for_log_source;
use super::state::OverlayState;
use super::surfaces::request_redraw;
use crate::diagnostics::debug_log;

pub(super) const TIMER_ID: usize = 46;
pub(super) const TIMER_INTERVAL_MS: u32 = 50;

const HIT_FLASH_MS: u64 = 120;
const MIN_HIT_FLASH_INTERVAL_MS: u64 = 334;
const DAMAGE_EFFECT_MS: u64 = 850;
const DAMAGE_IMPACT_MS: u64 = 150;
const DAMAGE_DECAY_MS: u64 = 450;
const ATTACK_ISSUE_MS: u64 = 10_000;

const ATTACK_RED: Color = Color::from_colorref(0x003648E8);
const IMPACT_WHITE: Color = Color::from_colorref(0x00FFFFFF);
const ATTENTION_AMBER: Color = Color::from_colorref(0x002BA5F0);
const BLOOD_RED: Color = Color::from_colorref(0x002010B5);
const DEATH_RED: Color = Color::from_colorref(0x0018106B);
const ATTENTION_SURFACE: Color = Color::from_colorref(0x00101828);
const DEATH_SURFACE: Color = Color::from_colorref(0x00101050);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CombatStatus {
    OutOfRange,
    TooClose,
    LineOfSight,
    Dead,
}

impl CombatStatus {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::OutOfRange => "OUT OF RANGE",
            Self::TooClose => "TOO CLOSE",
            Self::LineOfSight => "NO LINE OF SIGHT",
            Self::Dead => "DEAD",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BorderKind {
    Attacking,
    Hit,
    Attention,
    Dead,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DamagePhase {
    None,
    Faint,
    Decay,
    Impact,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VisualKey {
    border: Option<BorderKind>,
    damage: DamagePhase,
    status: Option<CombatStatus>,
}

impl VisualKey {
    const NEUTRAL: Self = Self {
        border: None,
        damage: DamagePhase::None,
        status: None,
    };
}

#[derive(Clone, Copy, Debug)]
struct AttackIssue {
    problem: crate::log_watcher::AttackProblem,
    observed_at_ms: u64,
}

#[derive(Default)]
struct Entry {
    last_weapon_hit_ms: Option<u64>,
    hit_flash_until_ms: u64,
    last_hit_flash_ms: Option<u64>,
    last_damage_taken_ms: Option<u64>,
    attack_issue: Option<AttackIssue>,
    dead: bool,
    rendered: Option<VisualKey>,
}

impl Entry {
    fn visual_key(&self, now_ms: u64, hit_duration_ms: u64, animations: bool) -> VisualKey {
        if self.dead {
            return VisualKey {
                border: Some(BorderKind::Dead),
                damage: DamagePhase::None,
                status: Some(CombatStatus::Dead),
            };
        }

        let attack_status = self.attack_issue.and_then(|issue| {
            (now_ms.saturating_sub(issue.observed_at_ms) < ATTACK_ISSUE_MS).then_some(
                match issue.problem {
                    crate::log_watcher::AttackProblem::OutOfRange => CombatStatus::OutOfRange,
                    crate::log_watcher::AttackProblem::TooClose => CombatStatus::TooClose,
                    crate::log_watcher::AttackProblem::LineOfSight => CombatStatus::LineOfSight,
                },
            )
        });
        let status = attack_status;
        let attacking = self
            .last_weapon_hit_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < hit_duration_ms);
        let hit_flash = animations && attacking && now_ms < self.hit_flash_until_ms;
        let border = if status.is_some() {
            Some(BorderKind::Attention)
        } else if hit_flash {
            Some(BorderKind::Hit)
        } else if attacking {
            Some(BorderKind::Attacking)
        } else {
            None
        };
        let damage = self
            .last_damage_taken_ms
            .map(|last| now_ms.saturating_sub(last))
            .map_or(DamagePhase::None, |elapsed| {
                if elapsed >= DAMAGE_EFFECT_MS {
                    DamagePhase::None
                } else if !animations {
                    DamagePhase::Decay
                } else if elapsed < DAMAGE_IMPACT_MS {
                    DamagePhase::Impact
                } else if elapsed < DAMAGE_DECAY_MS {
                    DamagePhase::Decay
                } else {
                    DamagePhase::Faint
                }
            });
        VisualKey {
            border,
            damage,
            status,
        }
    }

    fn has_pending_transition(&self, now_ms: u64, hit_duration_ms: u64) -> bool {
        if self.dead {
            return false;
        }
        now_ms < self.hit_flash_until_ms
            || self
                .last_weapon_hit_ms
                .is_some_and(|last| now_ms.saturating_sub(last) < hit_duration_ms)
            || self
                .last_damage_taken_ms
                .is_some_and(|last| now_ms.saturating_sub(last) < DAMAGE_EFFECT_MS)
            || self
                .attack_issue
                .is_some_and(|issue| now_ms.saturating_sub(issue.observed_at_ms) < ATTACK_ISSUE_MS)
    }
}

pub(super) struct CombatAwarenessCenter {
    entries: HashMap<u32, Entry>,
    enabled: bool,
    hit_duration_ms: u64,
    animations_enabled: bool,
}

impl CombatAwarenessCenter {
    pub(super) fn new(cfg: &crate::config::Config, animations_enabled: bool) -> Self {
        Self {
            entries: HashMap::new(),
            enabled: cfg.combat_awareness_enabled,
            hit_duration_ms: cfg.effective_combat_hit_duration_ms(),
            animations_enabled,
        }
    }

    pub(super) fn apply_config(
        &mut self,
        cfg: &crate::config::Config,
        animations_enabled: bool,
    ) -> bool {
        let enabled = cfg.combat_awareness_enabled;
        let hit_duration_ms = cfg.effective_combat_hit_duration_ms();
        let changed = self.enabled != enabled
            || self.hit_duration_ms != hit_duration_ms
            || self.animations_enabled != animations_enabled;
        self.enabled = enabled;
        self.hit_duration_ms = hit_duration_ms;
        self.animations_enabled = animations_enabled;
        if !enabled {
            for entry in self.entries.values_mut() {
                let dead = entry.dead;
                *entry = Entry {
                    dead,
                    rendered: None,
                    ..Entry::default()
                };
            }
        } else if changed {
            for entry in self.entries.values_mut() {
                entry.last_weapon_hit_ms = None;
                entry.hit_flash_until_ms = 0;
                entry.last_hit_flash_ms = None;
                entry.last_damage_taken_ms = None;
                entry.rendered = None;
            }
        }
        changed
    }

    pub(super) fn remove(&mut self, pid: u32) {
        self.entries.remove(&pid);
    }

    fn apply_event(
        &mut self,
        pid: u32,
        _is_bard: bool,
        event: &crate::log_watcher::LogEvent,
        now_ms: u64,
    ) -> bool {
        let durable_character_event = matches!(
            event,
            crate::log_watcher::LogEvent::Character(
                crate::log_watcher::CharacterEvent::Died
                    | crate::log_watcher::CharacterEvent::Revived
            )
        );
        if !self.enabled && !durable_character_event {
            return false;
        }
        let entry = self.entries.entry(pid).or_default();
        let before = if self.enabled {
            entry.visual_key(now_ms, self.hit_duration_ms, self.animations_enabled)
        } else {
            VisualKey::NEUTRAL
        };
        match event {
            crate::log_watcher::LogEvent::Combat(
                crate::log_watcher::CombatEvent::WeaponDamageDealt,
            ) if !entry.dead => {
                entry.last_weapon_hit_ms = Some(now_ms);
                entry.attack_issue = None;
                let flash_allowed = entry
                    .last_hit_flash_ms
                    .is_none_or(|last| now_ms.saturating_sub(last) >= MIN_HIT_FLASH_INTERVAL_MS);
                if self.animations_enabled && flash_allowed {
                    entry.last_hit_flash_ms = Some(now_ms);
                    entry.hit_flash_until_ms = now_ms.saturating_add(HIT_FLASH_MS);
                }
            }
            crate::log_watcher::LogEvent::Combat(crate::log_watcher::CombatEvent::DamageTaken)
                if !entry.dead =>
            {
                entry.last_damage_taken_ms = Some(now_ms);
            }
            crate::log_watcher::LogEvent::Combat(
                crate::log_watcher::CombatEvent::AttackBlocked(problem),
            ) if !entry.dead => {
                entry.attack_issue = Some(AttackIssue {
                    problem: *problem,
                    observed_at_ms: now_ms,
                });
            }
            crate::log_watcher::LogEvent::Character(crate::log_watcher::CharacterEvent::Died) => {
                *entry = Entry {
                    dead: true,
                    rendered: entry.rendered,
                    ..Entry::default()
                };
            }
            crate::log_watcher::LogEvent::Character(
                crate::log_watcher::CharacterEvent::Revived,
            ) if entry.dead => {
                *entry = Entry {
                    rendered: entry.rendered,
                    ..Entry::default()
                };
            }
            _ => return false,
        }
        let after = if self.enabled {
            entry.visual_key(now_ms, self.hit_duration_ms, self.animations_enabled)
        } else {
            VisualKey::NEUTRAL
        };
        before != after
    }

    pub(super) fn snapshot(&mut self, pid: u32, now_ms: u64) -> Option<CombatVisualSnapshot> {
        let entry = self.entries.get_mut(&pid)?;
        let key = if self.enabled {
            entry.visual_key(now_ms, self.hit_duration_ms, self.animations_enabled)
        } else {
            VisualKey::NEUTRAL
        };
        entry.rendered = Some(key);
        (key != VisualKey::NEUTRAL).then_some(CombatVisualSnapshot { key })
    }

    fn needs_redraw(&self, pid: u32, now_ms: u64) -> bool {
        let Some(entry) = self.entries.get(&pid) else {
            return false;
        };
        let key = if self.enabled {
            entry.visual_key(now_ms, self.hit_duration_ms, self.animations_enabled)
        } else {
            VisualKey::NEUTRAL
        };
        entry.rendered != Some(key)
    }

    fn has_pending_transitions(&self, now_ms: u64) -> bool {
        self.enabled
            && self
                .entries
                .values()
                .any(|entry| entry.has_pending_transition(now_ms, self.hit_duration_ms))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CombatVisualSnapshot {
    key: VisualKey,
}

impl CombatVisualSnapshot {
    pub(super) fn status(self) -> Option<CombatStatus> {
        self.key.status
    }

    pub(super) fn claims_border(self) -> bool {
        matches!(
            self.key.border,
            Some(BorderKind::Attention | BorderKind::Dead)
        )
    }

    pub(super) fn layout(
        self,
        canvas: Rect,
        content: Rect,
        border_width: i32,
        scale: f64,
        measured_status_width: i32,
        show_border: bool,
    ) -> CombatVisualLayout {
        let dead = self.key.status == Some(CombatStatus::Dead);
        let dead_tint = dead.then_some(CombatFill {
            bounds: content,
            color: DEATH_RED,
            alpha: 166,
        });
        let (blood_tint, blood_frames) = if dead {
            (None, Vec::new())
        } else {
            blood_vignette(content, scale, self.key.damage)
        };
        let border = (show_border)
            .then(|| {
                self.key
                    .border
                    .map(|kind| combat_border(canvas, border_width, kind))
            })
            .flatten();
        let status = self
            .key
            .status
            .map(|status| combat_status_layout(content, scale, measured_status_width, status));
        CombatVisualLayout {
            dead_tint,
            blood_tint,
            blood_frames,
            border,
            status,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CombatFill {
    pub bounds: Rect,
    pub color: Color,
    pub alpha: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CombatBorderLayout {
    pub frames: Vec<Rect>,
    pub color: Color,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CombatStatusLayout {
    pub status: CombatStatus,
    pub surface: Rect,
    pub text: Rect,
    pub surface_color: Color,
    pub text_color: Color,
    pub alpha: u8,
    pub radius: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CombatVisualLayout {
    pub dead_tint: Option<CombatFill>,
    pub blood_tint: Option<CombatFill>,
    pub blood_frames: Vec<CombatFill>,
    pub border: Option<CombatBorderLayout>,
    pub status: Option<CombatStatusLayout>,
}

fn combat_border(canvas: Rect, border_width: i32, kind: BorderKind) -> CombatBorderLayout {
    let width = border_width.max(3);
    let color = match kind {
        BorderKind::Attacking => ATTACK_RED,
        BorderKind::Hit => IMPACT_WHITE,
        BorderKind::Attention => ATTENTION_AMBER,
        BorderKind::Dead => DEATH_RED,
    };
    CombatBorderLayout {
        frames: (0..width).map(|inset| canvas.inset(inset)).collect(),
        color,
    }
}

fn blood_vignette(
    content: Rect,
    scale: f64,
    phase: DamagePhase,
) -> (Option<CombatFill>, Vec<CombatFill>) {
    let (tint_alpha, edge_alpha): (u8, u8) = match phase {
        DamagePhase::None => return (None, Vec::new()),
        DamagePhase::Faint => (12, 46),
        DamagePhase::Decay => (24, 82),
        DamagePhase::Impact => (36, 122),
    };
    let depth = pixels(10, scale)
        .max(3)
        .min((content.width().min(content.height()) / 4).max(1));
    let frames = (0..depth)
        .map(|inset| CombatFill {
            bounds: content.inset(inset),
            color: BLOOD_RED,
            alpha: ((u32::from(edge_alpha) * (depth - inset) as u32) / depth as u32) as u8,
        })
        .collect();
    (
        Some(CombatFill {
            bounds: content,
            color: BLOOD_RED,
            alpha: tint_alpha,
        }),
        frames,
    )
}

fn combat_status_layout(
    content: Rect,
    scale: f64,
    measured_text_width: i32,
    status: CombatStatus,
) -> CombatStatusLayout {
    let dead = status == CombatStatus::Dead;
    let horizontal_padding = pixels(if dead { 18 } else { 12 }, scale).max(4);
    let vertical_margin = pixels(8, scale).max(2);
    let height = pixels(if dead { 48 } else { 34 }, scale)
        .max(20)
        .min(content.height().max(0));
    let width = (measured_text_width + 2 * horizontal_padding)
        .max(pixels(if dead { 104 } else { 92 }, scale))
        .min((content.width() - 2 * vertical_margin).max(0));
    let surface = if dead {
        let left = content.left + (content.width() - width) / 2;
        let top = content.top + (content.height() - height) / 2;
        Rect::new(left, top, left + width, top + height)
    } else {
        let right = content.right - vertical_margin;
        let top = content.top + vertical_margin;
        Rect::new(right - width, top, right, top + height)
    }
    .intersect(content);
    let text_inset = horizontal_padding.min(surface.width().max(0) / 2);
    CombatStatusLayout {
        status,
        surface,
        text: Rect::new(
            surface.left + text_inset,
            surface.top,
            surface.right - text_inset,
            surface.bottom,
        ),
        surface_color: if dead {
            DEATH_SURFACE
        } else {
            ATTENTION_SURFACE
        },
        text_color: if dead { IMPACT_WHITE } else { ATTENTION_AMBER },
        alpha: if dead { 246 } else { 238 },
        radius: pixels(6, scale).max(2),
    }
}

pub(super) fn apply_log_event(
    state: &mut OverlayState,
    event: &crate::log_watcher::ParsedLogEvent,
) {
    if !matches!(
        event.event,
        crate::log_watcher::LogEvent::Combat(_) | crate::log_watcher::LogEvent::Character(_)
    ) {
        return;
    }
    let Some(pid) = pid_for_log_source(&state.clients.windows, &event.source) else {
        debug_log(&format!(
            "eq_logs: combat source {} is no longer attached to an EQ window",
            event.source.id.as_str()
        ));
        return;
    };
    let now_ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    let visual_changed = state
        .combat_awareness
        .apply_event(pid, false, &event.event, now_ms);
    unsafe {
        if visual_changed {
            if let Some(pip) = state
                .presentation
                .pip_windows
                .iter()
                .find(|pip| pip.pid == pid)
            {
                request_redraw(pip.label_hwnd);
            }
        }
        if state.combat_awareness.has_pending_transitions(now_ms) {
            let _ = SetTimer(
                state.presentation.active_label_hwnd,
                TIMER_ID,
                TIMER_INTERVAL_MS,
                None,
            );
        }
    }
}

pub(super) unsafe fn tick(state: &mut OverlayState, timer_hwnd: HWND) {
    let now_ms = windows::Win32::System::SystemInformation::GetTickCount64();
    for pip in &state.presentation.pip_windows {
        if state.combat_awareness.needs_redraw(pip.pid, now_ms) {
            request_redraw(pip.label_hwnd);
        }
    }
    if !state.combat_awareness.has_pending_transitions(now_ms) {
        let _ = KillTimer(timer_hwnd, TIMER_ID);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log_watcher::{AttackProblem, CharacterEvent, CombatEvent, LogEvent};

    fn center(hit_duration_ms: u64, animations_enabled: bool) -> CombatAwarenessCenter {
        CombatAwarenessCenter {
            entries: HashMap::new(),
            enabled: true,
            hit_duration_ms,
            animations_enabled,
        }
    }

    #[test]
    fn outgoing_hits_flash_then_hold_a_red_attack_frame_for_the_configured_duration() {
        let mut center = center(3_000, true);
        assert!(center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::WeaponDamageDealt),
            1_000,
        ));
        assert_eq!(
            center.entries[&42]
                .visual_key(1_050, center.hit_duration_ms, true)
                .border,
            Some(BorderKind::Hit)
        );
        assert_eq!(
            center.entries[&42]
                .visual_key(1_200, center.hit_duration_ms, true)
                .border,
            Some(BorderKind::Attacking)
        );
        assert_eq!(
            center.entries[&42]
                .visual_key(3_999, center.hit_duration_ms, true)
                .border,
            Some(BorderKind::Attacking)
        );
        assert_eq!(
            center.entries[&42]
                .visual_key(4_000, center.hit_duration_ms, true)
                .border,
            None
        );
    }

    #[test]
    fn rapid_hits_are_coalesced_but_extend_the_attack_hold() {
        let mut center = center(3_000, true);
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::WeaponDamageDealt),
            1_000,
        );
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::WeaponDamageDealt),
            1_100,
        );
        assert_eq!(center.entries[&42].last_hit_flash_ms, Some(1_000));
        assert_eq!(
            center.entries[&42]
                .visual_key(4_099, center.hit_duration_ms, true)
                .border,
            Some(BorderKind::Attacking)
        );
    }

    #[test]
    fn incoming_damage_steps_down_through_a_bounded_blood_vignette() {
        let mut center = center(3_000, true);
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::DamageTaken),
            1_000,
        );
        let entry = &center.entries[&42];
        assert_eq!(
            entry.visual_key(1_100, center.hit_duration_ms, true).damage,
            DamagePhase::Impact
        );
        assert_eq!(
            entry.visual_key(1_300, center.hit_duration_ms, true).damage,
            DamagePhase::Decay
        );
        assert_eq!(
            entry.visual_key(1_700, center.hit_duration_ms, true).damage,
            DamagePhase::Faint
        );
        assert_eq!(
            entry.visual_key(1_850, center.hit_duration_ms, true).damage,
            DamagePhase::None
        );
    }

    #[test]
    fn successful_weapon_damage_clears_attack_problems() {
        let mut center = center(3_000, false);
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::AttackBlocked(AttackProblem::LineOfSight)),
            1_000,
        );
        assert_eq!(
            center.entries[&42]
                .visual_key(1_001, center.hit_duration_ms, false)
                .status,
            Some(CombatStatus::LineOfSight)
        );
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::WeaponDamageDealt),
            2_000,
        );
        assert_eq!(
            center.entries[&42]
                .visual_key(2_001, center.hit_duration_ms, false)
                .status,
            None
        );
    }

    #[test]
    fn dead_state_suppresses_combat_and_clears_only_on_observed_return_to_play() {
        let mut center = center(3_000, true);
        center.apply_event(
            42,
            false,
            &LogEvent::Combat(CombatEvent::WeaponDamageDealt),
            1_000,
        );
        center.apply_event(42, false, &LogEvent::Character(CharacterEvent::Died), 1_100);
        let dead = center.entries[&42].visual_key(1_101, center.hit_duration_ms, true);
        assert_eq!(dead.status, Some(CombatStatus::Dead));
        assert_eq!(dead.damage, DamagePhase::None);
        center.apply_event(
            42,
            false,
            &LogEvent::Character(CharacterEvent::Revived),
            2_000,
        );
        assert_eq!(
            center.entries[&42].visual_key(2_001, center.hit_duration_ms, true),
            VisualKey::NEUTRAL
        );
    }

    #[test]
    fn disabling_and_reenabling_presentation_preserves_durable_death_state() {
        let mut config = crate::config::Config::default();
        let mut center = CombatAwarenessCenter::new(&config, false);
        center.apply_event(42, false, &LogEvent::Character(CharacterEvent::Died), 1_000);

        config.combat_awareness_enabled = false;
        center.apply_config(&config, false);
        assert!(center.entries[&42].dead);
        assert!(center.snapshot(42, 1_001).is_none());

        config.combat_awareness_enabled = true;
        center.apply_config(&config, false);
        assert_eq!(
            center
                .snapshot(42, 1_002)
                .and_then(|snapshot| snapshot.status()),
            Some(CombatStatus::Dead)
        );
    }

    #[test]
    fn combat_layout_uses_direct_attention_copy_and_scales_inside_tiny_pips() {
        let snapshot = CombatVisualSnapshot {
            key: VisualKey {
                border: Some(BorderKind::Attention),
                damage: DamagePhase::Impact,
                status: Some(CombatStatus::LineOfSight),
            },
        };
        let canvas = Rect::new(0, 0, 160, 90);
        let layout = snapshot.layout(canvas, canvas.inset(3), 3, 1.5, 180, true);
        let status = layout.status.expect("status banner");
        assert_eq!(status.status.label(), "NO LINE OF SIGHT");
        assert_eq!(status.surface, status.surface.intersect(canvas));
        assert_eq!(status.text.top, status.surface.top);
        assert_eq!(status.text.bottom, status.surface.bottom);
        assert!(layout.border.is_some());
        assert!(!layout.blood_frames.is_empty());
    }
}

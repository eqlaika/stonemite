//! Log-only spell casting state, persona-safe timing estimates, and PiP visuals.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use super::log_sources::pid_for_log_source;
use super::state::OverlayState;
use super::surfaces::request_redraw;
use crate::{config, log_watcher};
use eqspell::{SpellCatalog, SpellDefinition, SpellId};

pub(super) const TIMER_ID: usize = 47;
pub(super) const TIMER_INTERVAL_MS: u32 = 16;

const RECENT_SAMPLE_WEIGHT: f64 = 0.90;
const UNCONFIRMED_GRACE: Duration = Duration::from_millis(750);
const UNCONFIRMED_FADE: Duration = Duration::from_millis(220);
const COMPLETION_FILL: Duration = Duration::from_millis(110);
const SUCCESS_LINGER: Duration = Duration::from_millis(420);
const FIZZLE_LINGER: Duration = Duration::from_millis(760);
const RESIST_LINGER: Duration = Duration::from_millis(660);
const INTERRUPT_LINGER: Duration = Duration::from_millis(560);
const MAX_SAMPLE: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct EstimateKey {
    server: String,
    character: String,
    class: String,
    spell_id: SpellId,
}

impl EstimateKey {
    fn new(server: &str, character: &str, class: &str, spell_id: SpellId) -> Self {
        Self {
            server: normalize(server),
            character: normalize(character),
            class: normalize(class),
            spell_id,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct LegacyEstimateKey {
    server: String,
    character: String,
    class: String,
    spell: String,
}

impl LegacyEstimateKey {
    fn new(server: &str, character: &str, class: &str, spell: &str) -> Self {
        Self {
            server: normalize(server),
            character: normalize(character),
            class: normalize(class),
            spell: normalize(spell),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Estimate {
    millis: f64,
    base_millis: u64,
}

#[derive(Serialize, Deserialize)]
struct PersistedEstimate {
    server: String,
    character: String,
    class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spell_id: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spell: Option<String>,
    millis: f64,
    base_millis: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct EstimateFile {
    #[serde(default)]
    estimates: Vec<PersistedEstimate>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CastingOutcome {
    Completed,
    Fizzled,
    Resisted,
    Interrupted,
}

impl CastingOutcome {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Completed => "COMPLETE",
            Self::Fizzled => "FIZZLE",
            Self::Resisted => "RESIST",
            Self::Interrupted => "INTERRUPTED",
        }
    }

    fn linger(self) -> Duration {
        match self {
            Self::Completed => SUCCESS_LINGER,
            Self::Fizzled => FIZZLE_LINGER,
            Self::Resisted => RESIST_LINGER,
            Self::Interrupted => INTERRUPT_LINGER,
        }
    }

    fn fills(self) -> bool {
        matches!(self, Self::Completed | Self::Resisted)
    }
}

#[derive(Clone, Debug)]
struct ActiveCast {
    spell: SpellDefinition,
    class: Option<Arc<str>>,
    estimate_key: Option<EstimateKey>,
    started_at: Instant,
    estimated_duration: Duration,
}

#[derive(Clone, Debug)]
struct TerminalCast {
    spell_name: Arc<str>,
    outcome: CastingOutcome,
    observed_at: Instant,
    start_progress: f32,
}

#[derive(Clone, Debug)]
enum CastEntry {
    Active(ActiveCast),
    Terminal(TerminalCast),
}

#[derive(Clone, Debug)]
pub(super) struct CastingVisualSnapshot {
    pub(super) spell_name: Arc<str>,
    pub(super) outcome: Option<CastingOutcome>,
    pub(super) progress: f32,
    pub(super) alpha: u8,
}

pub(super) struct CastingCenter {
    eq_dir: std::path::PathBuf,
    catalog: SpellCatalog,
    estimates: HashMap<EstimateKey, Estimate>,
    legacy_estimates: HashMap<LegacyEstimateKey, Estimate>,
    entries: HashMap<u32, CastEntry>,
    animations_enabled: bool,
    dirty: bool,
}

impl CastingCenter {
    pub(super) fn new(cfg: &config::Config, animations_enabled: bool) -> Self {
        let eq_dir = cfg.eq_directory();
        let (estimates, legacy_estimates) = load_estimates();
        Self {
            catalog: load_catalog(&eq_dir),
            eq_dir,
            estimates,
            legacy_estimates,
            entries: HashMap::new(),
            animations_enabled,
            dirty: false,
        }
    }

    pub(super) fn apply_config(&mut self, cfg: &config::Config, animations_enabled: bool) {
        let eq_dir = cfg.eq_directory();
        if self.eq_dir != eq_dir {
            self.catalog = load_catalog(&eq_dir);
            self.eq_dir = eq_dir;
            self.entries.clear();
        }
        self.animations_enabled = animations_enabled;
        self.entries.clear();
    }

    pub(super) fn remove(&mut self, pid: u32) {
        self.entries.remove(&pid);
    }

    pub(super) fn reconcile_clients(&mut self, windows: &[crate::eq_windows::EqWindow]) {
        self.entries.retain(|pid, entry| {
            let Some(window) = windows.iter().find(|window| window.pid == *pid) else {
                return false;
            };
            match entry {
                CastEntry::Active(active) => active.class.as_deref() == window.class.as_deref(),
                CastEntry::Terminal(_) => true,
            }
        });
    }

    fn start(
        &mut self,
        pid: u32,
        server: &str,
        character: &str,
        class: Option<&str>,
        spell_name: &str,
        observed_at: Instant,
    ) -> bool {
        let Some(spell) = self.catalog.resolve(spell_name, class) else {
            self.entries.remove(&pid);
            return false;
        };
        // A uniquely classed spell is stronger persona evidence than a stale
        // server/name cache entry. Shared spells retain the current class.
        let effective_class = spell.unique_class.or(class);
        let bard = effective_class.is_some_and(|class| class.eq_ignore_ascii_case("BRD"));
        let base_duration = if bard && !spell.base_cast_time.is_zero() {
            Duration::from_secs(3)
        } else {
            spell.base_cast_time
        };
        if base_duration.is_zero() {
            self.entries.insert(
                pid,
                CastEntry::Terminal(TerminalCast {
                    spell_name: spell.name,
                    outcome: CastingOutcome::Completed,
                    observed_at,
                    start_progress: 1.0,
                }),
            );
            return true;
        }
        let estimate_key = (!bard)
            .then(|| {
                effective_class.map(|class| EstimateKey::new(server, character, class, spell.id))
            })
            .flatten();
        if let (Some(key), Some(class)) = (&estimate_key, effective_class) {
            if !self.estimates.contains_key(key) {
                let legacy_key = LegacyEstimateKey::new(server, character, class, &spell.name);
                if let Some(estimate) = self.legacy_estimates.remove(&legacy_key) {
                    self.estimates.insert(key.clone(), estimate);
                    self.dirty = true;
                }
            }
        }
        let base_millis = duration_millis(base_duration);
        let estimated_duration = estimate_key
            .as_ref()
            .and_then(|key| self.estimates.get(key))
            .filter(|estimate| estimate.base_millis == base_millis)
            .map(|estimate| Duration::from_secs_f64((estimate.millis / 1000.0).max(0.001)))
            .unwrap_or(base_duration);
        self.entries.insert(
            pid,
            CastEntry::Active(ActiveCast {
                spell,
                class: effective_class.map(Arc::from),
                estimate_key,
                started_at: observed_at,
                estimated_duration,
            }),
        );
        true
    }

    fn terminal(
        &mut self,
        pid: u32,
        observed_spell: Option<&str>,
        outcome: CastingOutcome,
        observed_at: Instant,
        train: bool,
    ) -> bool {
        let Some(CastEntry::Active(active)) = self.entries.get(&pid) else {
            return false;
        };
        if observed_spell.is_some_and(|spell| !spell.eq_ignore_ascii_case(&active.spell.name)) {
            return false;
        }
        let active = active.clone();
        let elapsed = observed_at.saturating_duration_since(active.started_at);
        let progress = active_progress(&active, observed_at);
        if train && !elapsed.is_zero() && elapsed <= MAX_SAMPLE {
            if let Some(key) = active.estimate_key {
                let base_millis = duration_millis(active.spell.base_cast_time);
                let previous = self
                    .estimates
                    .get(&key)
                    .filter(|estimate| estimate.base_millis == base_millis)
                    .map_or(base_millis as f64, |estimate| estimate.millis);
                let sample = elapsed.as_secs_f64() * 1000.0;
                self.estimates.insert(
                    key,
                    Estimate {
                        millis: RECENT_SAMPLE_WEIGHT * sample
                            + (1.0 - RECENT_SAMPLE_WEIGHT) * previous,
                        base_millis,
                    },
                );
                self.dirty = true;
            }
        }
        self.entries.insert(
            pid,
            CastEntry::Terminal(TerminalCast {
                spell_name: active.spell.name,
                outcome,
                observed_at,
                start_progress: progress,
            }),
        );
        true
    }

    fn active_matches_landing(&self, pid: u32, body: &str) -> bool {
        matches!(
            self.entries.get(&pid),
            Some(CastEntry::Active(active)) if active.spell.matches_landing_message(body)
        )
    }

    fn active_spell_name(&self, pid: u32) -> Option<&str> {
        match self.entries.get(&pid) {
            Some(CastEntry::Active(active)) => Some(&active.spell.name),
            _ => None,
        }
    }

    pub(super) fn snapshot(&self, pid: u32, now: Instant) -> Option<CastingVisualSnapshot> {
        match self.entries.get(&pid)? {
            CastEntry::Active(active) => active_snapshot(active, now, self.animations_enabled),
            CastEntry::Terminal(terminal) => {
                terminal_snapshot(terminal, now, self.animations_enabled)
            }
        }
    }

    fn prune(&mut self, now: Instant) -> Vec<u32> {
        let expired = self
            .entries
            .iter()
            .filter_map(|(pid, entry)| {
                let visible = match entry {
                    CastEntry::Active(active) => {
                        active_snapshot(active, now, self.animations_enabled).is_some()
                    }
                    CastEntry::Terminal(terminal) => {
                        terminal_snapshot(terminal, now, self.animations_enabled).is_some()
                    }
                };
                (!visible).then_some(*pid)
            })
            .collect::<Vec<_>>();
        for pid in &expired {
            self.entries.remove(pid);
        }
        expired
    }

    fn has_visible_entries(&self, now: Instant) -> bool {
        self.entries
            .iter()
            .any(|(pid, _)| self.snapshot(*pid, now).is_some())
    }

    pub(super) fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = estimate_path() else { return };
        let Some(parent) = path.parent() else { return };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let mut persisted = self
            .estimates
            .iter()
            .map(|(key, estimate)| PersistedEstimate {
                server: key.server.clone(),
                character: key.character.clone(),
                class: key.class.clone(),
                spell_id: Some(key.spell_id.get()),
                spell: None,
                millis: estimate.millis,
                base_millis: estimate.base_millis,
            })
            .chain(
                self.legacy_estimates
                    .iter()
                    .map(|(key, estimate)| PersistedEstimate {
                        server: key.server.clone(),
                        character: key.character.clone(),
                        class: key.class.clone(),
                        spell_id: None,
                        spell: Some(key.spell.clone()),
                        millis: estimate.millis,
                        base_millis: estimate.base_millis,
                    }),
            )
            .collect::<Vec<_>>();
        persisted.sort_by(|left, right| {
            (
                &left.server,
                &left.character,
                &left.class,
                left.spell_id,
                &left.spell,
            )
                .cmp(&(
                    &right.server,
                    &right.character,
                    &right.class,
                    right.spell_id,
                    &right.spell,
                ))
        });
        let file = EstimateFile {
            estimates: persisted,
        };
        if toml::to_string_pretty(&file)
            .ok()
            .and_then(|contents| std::fs::write(path, contents).ok())
            .is_some()
        {
            self.dirty = false;
        }
    }
}

pub(super) fn apply_log_envelope(
    state: &mut OverlayState,
    envelope: &log_watcher::LogEnvelope,
) -> bool {
    let Some(pid) = pid_for_log_source(&state.clients.windows, &envelope.raw.source) else {
        return false;
    };
    let class = state
        .clients
        .windows
        .iter()
        .find(|window| window.pid == pid)
        .and_then(|window| window.class.clone());
    let server = envelope.raw.source.server.as_ref();
    let character = envelope.raw.source.character.as_ref();
    let mut changed = false;
    let mut terminal_seen = false;

    for parsed in envelope.events.iter() {
        let log_watcher::LogEvent::Casting(event) = &parsed.event else {
            continue;
        };
        match event {
            log_watcher::CastingEvent::Started { spell, .. } => {
                changed |= state.casting.start(
                    pid,
                    server,
                    character,
                    class.as_deref(),
                    spell,
                    envelope.observed_at,
                );
            }
            log_watcher::CastingEvent::Fizzled { spell } => {
                terminal_seen = true;
                changed |= state.casting.terminal(
                    pid,
                    spell.as_deref(),
                    CastingOutcome::Fizzled,
                    envelope.observed_at,
                    true,
                );
            }
            log_watcher::CastingEvent::Interrupted { spell } => {
                terminal_seen = true;
                changed |= state.casting.terminal(
                    pid,
                    spell.as_deref(),
                    CastingOutcome::Interrupted,
                    envelope.observed_at,
                    false,
                );
            }
            log_watcher::CastingEvent::Resisted { spell } => {
                terminal_seen = true;
                changed |= state.casting.terminal(
                    pid,
                    Some(spell),
                    CastingOutcome::Resisted,
                    envelope.observed_at,
                    true,
                );
            }
            log_watcher::CastingEvent::ConcentrationRecovered => {}
        }
    }

    if !terminal_seen {
        let owned_direct_spell = envelope.events.iter().any(|event| {
            matches!(
                &event.event,
                log_watcher::LogEvent::Combat(log_watcher::CombatEvent::Damage(damage))
                    if damage.kind == log_watcher::DamageKind::DirectSpell
                        && matches!(damage.attacker.perspective, log_watcher::Perspective::You | log_watcher::Perspective::Your)
                        && damage.ability.as_deref().zip(state.casting.active_spell_name(pid)).is_some_and(|(ability, active)| ability.eq_ignore_ascii_case(active))
            )
        });
        let owned_heal = state
            .casting
            .active_spell_name(pid)
            .is_some_and(|spell| is_owned_heal(&envelope.raw.body, spell));
        let landing = state
            .casting
            .active_matches_landing(pid, &envelope.raw.body);
        if owned_direct_spell || owned_heal || landing {
            changed |= state.casting.terminal(
                pid,
                None,
                CastingOutcome::Completed,
                envelope.observed_at,
                true,
            );
        }
    }

    unsafe {
        if changed {
            if let Some(pip) = state
                .presentation
                .pip_windows
                .iter()
                .find(|pip| pip.pid == pid)
            {
                request_redraw(pip.label_hwnd);
            }
        }
        if state.casting.has_visible_entries(envelope.observed_at) {
            let _ = SetTimer(
                state.presentation.active_label_hwnd,
                TIMER_ID,
                TIMER_INTERVAL_MS,
                None,
            );
        }
    }
    changed
}

pub(super) unsafe fn tick(state: &mut OverlayState, timer_hwnd: HWND) {
    let now = Instant::now();
    let expired = state.casting.prune(now);
    for pip in &state.presentation.pip_windows {
        if expired.contains(&pip.pid) || state.casting.snapshot(pip.pid, now).is_some() {
            request_redraw(pip.label_hwnd);
        }
    }
    if !state.casting.has_visible_entries(now) {
        let _ = KillTimer(timer_hwnd, TIMER_ID);
    }
}

fn active_snapshot(
    active: &ActiveCast,
    now: Instant,
    animations_enabled: bool,
) -> Option<CastingVisualSnapshot> {
    let elapsed = now.saturating_duration_since(active.started_at);
    let timeout = active.estimated_duration.saturating_add(UNCONFIRMED_GRACE);
    if elapsed >= timeout {
        return None;
    }
    let fade_start = timeout.saturating_sub(UNCONFIRMED_FADE);
    let alpha = if !animations_enabled || elapsed <= fade_start {
        255
    } else {
        let fade =
            elapsed.saturating_sub(fade_start).as_secs_f64() / UNCONFIRMED_FADE.as_secs_f64();
        ((1.0 - fade.clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Some(CastingVisualSnapshot {
        spell_name: active.spell.name.clone(),
        outcome: None,
        progress: active_progress(active, now),
        alpha,
    })
}

fn active_progress(active: &ActiveCast, now: Instant) -> f32 {
    let estimate = active.estimated_duration.as_secs_f64().max(0.001);
    let ratio = now
        .saturating_duration_since(active.started_at)
        .as_secs_f64()
        / estimate;
    if ratio <= 1.0 {
        (0.88 * ratio.clamp(0.0, 1.0)) as f32
    } else {
        (0.88 + 0.09 * (1.0 - (-3.0 * (ratio - 1.0)).exp())).min(0.97) as f32
    }
}

fn terminal_snapshot(
    terminal: &TerminalCast,
    now: Instant,
    animations_enabled: bool,
) -> Option<CastingVisualSnapshot> {
    let elapsed = now.saturating_duration_since(terminal.observed_at);
    let linger = terminal.outcome.linger();
    if elapsed >= linger {
        return None;
    }
    let progress = if terminal.outcome.fills() {
        if animations_enabled {
            let t = (elapsed.as_secs_f64() / COMPLETION_FILL.as_secs_f64()).clamp(0.0, 1.0);
            let eased = 1.0 - (1.0 - t).powi(3);
            terminal.start_progress + (1.0 - terminal.start_progress) * eased as f32
        } else {
            1.0
        }
    } else {
        terminal.start_progress
    };
    let fade_duration = Duration::from_millis(180).min(linger);
    let fade_start = linger.saturating_sub(fade_duration);
    let alpha = if !animations_enabled || elapsed <= fade_start {
        255
    } else {
        let fade = elapsed.saturating_sub(fade_start).as_secs_f64() / fade_duration.as_secs_f64();
        ((1.0 - fade.clamp(0.0, 1.0)) * 255.0).round() as u8
    };
    Some(CastingVisualSnapshot {
        spell_name: terminal.spell_name.clone(),
        outcome: Some(terminal.outcome),
        progress: progress.clamp(0.0, 1.0),
        alpha,
    })
}

fn is_owned_heal(body: &str, active_spell: &str) -> bool {
    let Some((prefix, suffix)) = body.rsplit_once(" hit points by ") else {
        return false;
    };
    if !prefix.contains("You healed ") {
        return false;
    }
    let spell = suffix
        .split(". (")
        .next()
        .unwrap_or(suffix)
        .trim_end_matches('.');
    spell.eq_ignore_ascii_case(active_spell)
}

fn load_catalog(eq_dir: &std::path::Path) -> SpellCatalog {
    match SpellCatalog::load(eq_dir) {
        Ok(catalog) => catalog,
        Err(error) => {
            crate::diagnostics::debug_log(&format!(
                "spell_casting: could not load spell data from {}: {error}",
                eq_dir.display()
            ));
            SpellCatalog::default()
        }
    }
}

fn load_estimates() -> (
    HashMap<EstimateKey, Estimate>,
    HashMap<LegacyEstimateKey, Estimate>,
) {
    let Some(path) = estimate_path() else {
        return (HashMap::new(), HashMap::new());
    };
    let Some(file) = std::fs::read_to_string(path)
        .ok()
        .and_then(|contents| toml::from_str::<EstimateFile>(&contents).ok())
    else {
        return (HashMap::new(), HashMap::new());
    };
    estimate_maps(file)
}

fn estimate_maps(
    file: EstimateFile,
) -> (
    HashMap<EstimateKey, Estimate>,
    HashMap<LegacyEstimateKey, Estimate>,
) {
    let mut estimates = HashMap::new();
    let mut legacy_estimates = HashMap::new();
    for entry in file
        .estimates
        .into_iter()
        .filter(|entry| entry.millis.is_finite() && entry.millis > 0.0)
    {
        let estimate = Estimate {
            millis: entry.millis,
            base_millis: entry.base_millis,
        };
        if let Some(spell_id) = entry.spell_id {
            estimates.insert(
                EstimateKey::new(
                    &entry.server,
                    &entry.character,
                    &entry.class,
                    SpellId::new(spell_id),
                ),
                estimate,
            );
        } else if let Some(spell) = entry.spell.filter(|spell| !spell.trim().is_empty()) {
            legacy_estimates.insert(
                LegacyEstimateKey::new(&entry.server, &entry.character, &entry.class, &spell),
                estimate,
            );
        }
    }
    (estimates, legacy_estimates)
}

fn estimate_path() -> Option<std::path::PathBuf> {
    config::Config::dir().map(|directory| directory.join("cast-times.toml"))
}

fn duration_millis(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active(duration: Duration, estimate: Duration, start: Instant) -> ActiveCast {
        ActiveCast {
            spell: SpellDefinition {
                id: SpellId::new(13),
                name: Arc::from("Complete Heal"),
                base_cast_time: duration,
                unique_class: Some("CLR"),
                cast_on_you: None,
                cast_on_other: None,
            },
            class: Some(Arc::from("CLR")),
            estimate_key: None,
            started_at: start,
            estimated_duration: estimate,
        }
    }

    #[test]
    fn estimated_progress_never_claims_completion() {
        let start = Instant::now();
        let cast = active(Duration::from_secs(10), Duration::from_secs(10), start);
        assert_eq!(active_progress(&cast, start), 0.0);
        assert!((active_progress(&cast, start + Duration::from_secs(10)) - 0.88).abs() < 0.001);
        assert!(active_progress(&cast, start + Duration::from_millis(10_700)) < 0.98);
    }

    #[test]
    fn unconfirmed_tail_fades_only_when_client_animations_are_enabled() {
        let start = Instant::now();
        let cast = active(Duration::from_secs(1), Duration::from_secs(1), start);
        let near_timeout = start + Duration::from_millis(1_650);
        assert!(active_snapshot(&cast, near_timeout, true).unwrap().alpha < 255);
        assert_eq!(
            active_snapshot(&cast, near_timeout, false).unwrap().alpha,
            255
        );
        assert!(active_snapshot(&cast, start + Duration::from_millis(1_750), true).is_none());
    }

    #[test]
    fn confirmed_completion_eases_to_full_and_fizzle_holds_partial_progress() {
        let start = Instant::now();
        let completed = TerminalCast {
            spell_name: Arc::from("Complete Heal"),
            outcome: CastingOutcome::Completed,
            observed_at: start,
            start_progress: 0.88,
        };
        assert_eq!(
            terminal_snapshot(&completed, start, false)
                .unwrap()
                .progress,
            1.0
        );
        assert!(
            terminal_snapshot(&completed, start + Duration::from_millis(50), true)
                .unwrap()
                .progress
                > 0.88
        );

        let fizzled = TerminalCast {
            outcome: CastingOutcome::Fizzled,
            ..completed
        };
        assert_eq!(
            terminal_snapshot(&fizzled, start, true).unwrap().progress,
            0.88
        );
    }

    #[test]
    fn exact_persona_spell_estimate_is_ninety_percent_latest_sample() {
        let start = Instant::now();
        let key = EstimateKey::new("Teek", "Bilka", "CLR", SpellId::new(13));
        let spell = SpellDefinition {
            id: SpellId::new(13),
            name: Arc::from("Complete Heal"),
            base_cast_time: Duration::from_secs(10),
            unique_class: Some("CLR"),
            cast_on_you: None,
            cast_on_other: None,
        };
        let mut center = CastingCenter {
            eq_dir: std::path::PathBuf::new(),
            catalog: SpellCatalog::default(),
            estimates: HashMap::new(),
            legacy_estimates: HashMap::new(),
            entries: HashMap::from([(
                42,
                CastEntry::Active(ActiveCast {
                    spell: spell.clone(),
                    class: Some(Arc::from("CLR")),
                    estimate_key: Some(key.clone()),
                    started_at: start,
                    estimated_duration: Duration::from_secs(10),
                }),
            )]),
            animations_enabled: true,
            dirty: false,
        };

        assert!(center.terminal(
            42,
            Some("Complete Heal"),
            CastingOutcome::Completed,
            start + Duration::from_secs(5),
            true,
        ));
        assert_eq!(center.estimates[&key].millis, 5_500.0);

        center.entries.insert(
            42,
            CastEntry::Active(ActiveCast {
                spell,
                class: Some(Arc::from("CLR")),
                estimate_key: Some(key.clone()),
                started_at: start,
                estimated_duration: Duration::from_millis(5_500),
            }),
        );
        center.terminal(
            42,
            None,
            CastingOutcome::Fizzled,
            start + Duration::from_secs(4),
            true,
        );
        assert_eq!(center.estimates[&key].millis, 4_150.0);
        assert!(!center.estimates.contains_key(&EstimateKey::new(
            "Teek",
            "Bilka",
            "WIZ",
            SpellId::new(13)
        )));
    }

    #[test]
    fn legacy_name_estimate_migrates_to_the_resolved_spell_id() {
        let mut fields = vec![String::new(); 53];
        fields[0] = "13".to_owned();
        fields[1] = "Complete Heal".to_owned();
        fields[8] = "10000".to_owned();
        fields[37] = "39".to_owned();
        let catalog = SpellCatalog::from_text(&fields.join("^"), "");
        let legacy_key = LegacyEstimateKey::new("Teek", "Bilka", "CLR", "Complete Heal");
        let mut center = CastingCenter {
            eq_dir: std::path::PathBuf::new(),
            catalog,
            estimates: HashMap::new(),
            legacy_estimates: HashMap::from([(
                legacy_key,
                Estimate {
                    millis: 5_500.0,
                    base_millis: 10_000,
                },
            )]),
            entries: HashMap::new(),
            animations_enabled: true,
            dirty: false,
        };

        assert!(center.start(
            42,
            "Teek",
            "Bilka",
            Some("CLR"),
            "Complete Heal",
            Instant::now(),
        ));
        let key = EstimateKey::new("Teek", "Bilka", "CLR", SpellId::new(13));
        assert_eq!(center.estimates[&key].millis, 5_500.0);
        assert!(center.legacy_estimates.is_empty());
        assert!(center.dirty);
        assert!(matches!(
            center.entries.get(&42),
            Some(CastEntry::Active(active))
                if active.spell.id == SpellId::new(13)
                    && active.estimated_duration == Duration::from_millis(5_500)
        ));
    }

    #[test]
    fn persisted_name_keys_remain_loadable_during_migration() {
        let file = toml::from_str::<EstimateFile>(
            r#"[[estimates]]
server = "teek"
character = "bilka"
class = "clr"
spell = "complete heal"
millis = 5500.0
base_millis = 10000
"#,
        )
        .unwrap();
        let (estimates, legacy) = estimate_maps(file);
        assert!(estimates.is_empty());
        assert_eq!(
            legacy[&LegacyEstimateKey::new("Teek", "Bilka", "CLR", "Complete Heal")].millis,
            5_500.0
        );
    }

    #[test]
    fn owned_heals_match_only_the_active_spell() {
        assert!(is_owned_heal(
            "You healed Bilka for 500 hit points by Complete Heal. (Critical)",
            "Complete Heal"
        ));
        assert!(!is_owned_heal(
            "Tolzol healed Bilka for 500 hit points by Complete Heal.",
            "Complete Heal"
        ));
    }
}

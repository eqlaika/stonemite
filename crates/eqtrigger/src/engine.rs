//! The trigger evaluation engine.
//!
//! A [`CompiledLibrary`] is an immutable, shareable compilation of a
//! [`TriggerLibrary`]. A [`TriggerEngine`] layers per-character mutable
//! state (variables, counters, lockouts, timers, previous line) on top and
//! turns log lines plus a virtual clock into presentation-only actions.
//!
//! Semantics mirror EQLP 2.3.x (`TriggerProcessor`) with the documented-
//! intent fixes called out in `docs/triggers.md`: real 750 ms repeated
//! resets (millisecond precision instead of truncated integer seconds).

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::conditions::{self, Condition};
use crate::model::*;
use crate::netregex::{CompatRegex, MatchMap, Prefilter};
use crate::pattern::{self, NumberConstraint};
use crate::substitute;
use crate::timers::{resolve_display_name, ActiveTimer, CompiledEnder, TimerSnapshot};
use crate::trace::{LineTrace, TriggerTrace};
use crate::EngineMillis;

/// Identity of the character whose log produced a line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterContext {
    /// Stable per-client key (Stonemite log-source id).
    pub key: String,
    pub character: String,
    pub server: String,
}

/// Where in a trigger's lifecycle an action was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionPhase {
    Initial,
    TimerWarning,
    TimerEnd,
    TimerEndEarly,
}

/// A presentation-only action. There is intentionally no command, script,
/// keyboard, mouse, network, or clipboard variant.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum TriggerAction {
    DisplayText {
        text: String,
        overlays: Vec<OverlayId>,
        font_color: Option<String>,
    },
    PlaySound {
        /// Managed asset name or absolute-path-free file reference.
        sound: String,
        volume: i32,
    },
    Speak {
        text: String,
        /// `None` = character/system default rate.
        rate: Option<i32>,
        volume: i32,
    },
}

/// One generated action with its provenance and routing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionEvent {
    pub character: String,
    pub trigger_id: TriggerId,
    pub trigger_name: String,
    pub phase: ActionPhase,
    pub target: PresentationTarget,
    /// Lower interrupts higher (EQLP audio priority).
    pub priority: i64,
    pub action: TriggerAction,
}

/// Output of one engine step.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActionBatch {
    pub events: Vec<ActionEvent>,
    /// True when the set of running timers changed.
    pub timers_changed: bool,
}

impl ActionBatch {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && !self.timers_changed
    }
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)] // one matcher per trigger; boxing wins nothing
enum Matcher {
    Regex {
        regex: CompatRegex,
        constraints: Vec<NumberConstraint>,
        prefilter: Prefilter,
        /// Pattern text before compilation, for `{c}` re-expansion.
        expanded_pattern: String,
        needs_character: bool,
    },
    Literal {
        text: String,
        needs_character: bool,
    },
}

#[derive(Clone, Debug)]
struct EnderTemplate {
    pattern: String,
    use_regex: bool,
}

#[derive(Clone, Debug, Default)]
struct TemplateFlags {
    counter: bool,
    repeated: bool,
    logtime: bool,
}

fn flags_for(template: &str) -> TemplateFlags {
    TemplateFlags {
        counter: substitute::contains_code(template, substitute::COUNTER_CODE),
        repeated: substitute::contains_code(template, substitute::REPEATED_CODE),
        logtime: substitute::contains_code(template, substitute::LOGTIME_CODE),
    }
}

pub(crate) struct CompiledTrigger {
    pub id: TriggerId,
    pub name: String,
    trigger: Trigger,
    matcher: Matcher,
    previous: Option<Matcher>,
    condition: Option<Condition>,
    condition_invalid: bool,
    display_template: String,
    speak_template: String,
    timer_name_template: String,
    warning_display: String,
    warning_speak: String,
    end_display: String,
    end_speak: String,
    early_display: String,
    early_speak: String,
    ender_templates: Vec<EnderTemplate>,
    speak_flags: TemplateFlags,
    display_flags: TemplateFlags,
    timer_flags: TemplateFlags,
}

/// Immutable compiled library shared across characters and threads.
pub struct CompiledLibrary {
    pub(crate) triggers: Vec<CompiledTrigger>,
    profiles: Vec<CompiledProfile>,
    /// Triggers that failed compilation, with reasons (for diagnostics).
    pub compile_errors: Vec<(TriggerId, String)>,
}

struct CompiledProfile {
    assignment: ProfileAssignment,
    /// Indexes into `triggers`.
    members: Vec<usize>,
}

impl CompiledLibrary {
    pub fn compile(library: &TriggerLibrary) -> Self {
        let mut triggers = Vec::new();
        let mut compile_errors = Vec::new();
        let mut index_by_id = HashMap::new();

        for trigger in &library.triggers {
            if !trigger.enabled || trigger.quarantine.is_some() || trigger.pattern.is_empty() {
                continue;
            }
            match compile_trigger(trigger) {
                Ok(compiled) => {
                    index_by_id.insert(trigger.id, triggers.len());
                    triggers.push(compiled);
                }
                Err(error) => compile_errors.push((trigger.id, error)),
            }
        }

        let profiles = library
            .profiles
            .iter()
            .filter(|profile| profile.enabled)
            .map(|profile| {
                let mut members: Vec<usize> = profile
                    .triggers
                    .iter()
                    .filter_map(|id| index_by_id.get(id).copied())
                    .collect();
                for folder in &profile.folders {
                    let subtree = library.folder_subtree(*folder);
                    for trigger in &library.triggers {
                        if let Some(parent) = trigger.folder {
                            if subtree.contains(&parent) {
                                if let Some(index) = index_by_id.get(&trigger.id) {
                                    members.push(*index);
                                }
                            }
                        }
                    }
                }
                members.sort_unstable();
                members.dedup();
                CompiledProfile {
                    assignment: profile.assignment.clone(),
                    members,
                }
            })
            .collect();

        Self {
            triggers,
            profiles,
            compile_errors,
        }
    }

    pub fn active_trigger_count(&self) -> usize {
        self.triggers.len()
    }

    /// Trigger indexes active for a character. With no profiles defined,
    /// every compiled (enabled) trigger is active.
    fn active_for(&self, character: &str, server: &str) -> Vec<usize> {
        if self.profiles.is_empty() {
            return (0..self.triggers.len()).collect();
        }
        let mut active: Vec<usize> = self
            .profiles
            .iter()
            .filter(|profile| match &profile.assignment {
                ProfileAssignment::Global => true,
                ProfileAssignment::Characters { characters } => characters
                    .iter()
                    .any(|selector| selector.matches(character, server)),
            })
            .flat_map(|profile| profile.members.iter().copied())
            .collect();
        active.sort_unstable();
        active.dedup();
        active
    }
}

fn compile_trigger(trigger: &Trigger) -> Result<CompiledTrigger, String> {
    let matcher = compile_matcher(&trigger.pattern)?;
    let previous = trigger
        .previous_pattern
        .as_ref()
        .filter(|pattern| !pattern.is_empty())
        .map(compile_matcher)
        .transpose()?;

    let condition = conditions::parse(&trigger.condition);
    let condition_invalid = condition.is_none() && !trigger.condition.trim().is_empty();

    let warning_seconds = trigger
        .timer
        .as_ref()
        .map(|timer| timer.warning_seconds)
        .unwrap_or(0);
    let preprocess = |text: Option<&String>| -> String {
        let text = text.map(String::as_str).unwrap_or("");
        substitute::replace_code(
            text,
            substitute::TIMER_WARN_TIME_CODE,
            &warning_seconds.to_string(),
        )
    };

    let timer = trigger.timer.as_ref();
    let timer_name_source = timer
        .map(|behavior| behavior.timer_name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| trigger.name.clone());

    let display_template = preprocess(trigger.display_text.as_ref());
    let speak_template = preprocess(trigger.speak_text.as_ref());
    let timer_name_template = preprocess(Some(&timer_name_source));

    Ok(CompiledTrigger {
        id: trigger.id,
        name: trigger.name.clone(),
        matcher,
        previous,
        condition,
        condition_invalid,
        warning_display: preprocess(timer.and_then(|t| t.warning.display_text.as_ref())),
        warning_speak: preprocess(timer.and_then(|t| t.warning.speak_text.as_ref())),
        end_display: preprocess(timer.and_then(|t| t.end.display_text.as_ref())),
        end_speak: preprocess(timer.and_then(|t| t.end.speak_text.as_ref())),
        early_display: preprocess(timer.and_then(|t| t.early_end.display_text.as_ref())),
        early_speak: preprocess(timer.and_then(|t| t.early_end.speak_text.as_ref())),
        ender_templates: timer
            .map(|t| {
                t.end_early_patterns
                    .iter()
                    .filter(|pattern| !pattern.is_empty())
                    .map(|pattern| EnderTemplate {
                        pattern: pattern.text.clone(),
                        use_regex: pattern.use_regex,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        speak_flags: flags_for(&speak_template),
        display_flags: flags_for(&display_template),
        timer_flags: flags_for(&timer_name_template),
        display_template,
        speak_template,
        timer_name_template,
        trigger: trigger.clone(),
    })
}

fn compile_matcher(pattern: &Pattern) -> Result<Matcher, String> {
    let needs_character = substitute::contains_code(&pattern.text, substitute::CHARACTER_CODE);
    if !pattern.use_regex {
        return Ok(Matcher::Literal {
            text: pattern.text.clone(),
            needs_character,
        });
    }
    let expanded = pattern::expand(&pattern.text);
    if needs_character {
        // Compiled lazily per character; validate the shape now with a
        // placeholder so broken patterns still fail at compile time.
        let probe = substitute::replace_code(&expanded.pattern, substitute::CHARACTER_CODE, "X");
        CompatRegex::compile(&probe).map_err(|error| error.to_string())?;
        return Ok(Matcher::Regex {
            regex: CompatRegex::compile(&probe).map_err(|error| error.to_string())?,
            constraints: expanded.constraints,
            prefilter: Prefilter::None,
            expanded_pattern: expanded.pattern,
            needs_character: true,
        });
    }
    let regex = CompatRegex::compile(&expanded.pattern).map_err(|error| error.to_string())?;
    Ok(Matcher::Regex {
        prefilter: Prefilter::for_pattern(&expanded.pattern),
        regex,
        constraints: expanded.constraints,
        expanded_pattern: expanded.pattern,
        needs_character: false,
    })
}

// ---------------------------------------------------------------------------
// Per-character state
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
struct RepeatedData {
    count: i64,
    last_ms: EngineMillis,
}

#[derive(Default)]
struct VariableStore {
    values: HashMap<String, String>,
    counters: HashMap<String, f64>,
    expiry: HashMap<String, EngineMillis>,
}

impl VariableStore {
    fn key(name: &str) -> String {
        name.to_ascii_lowercase()
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(&Self::key(name)).map(String::as_str)
    }

    fn set(&mut self, name: &str, value: String, expires_at: Option<EngineMillis>) {
        let key = Self::key(name);
        self.values.insert(key.clone(), value);
        match expires_at {
            Some(at) => {
                self.expiry.insert(key, at);
            }
            None => {
                self.expiry.remove(&key);
            }
        }
    }

    fn clear(&mut self, name: &str) {
        let key = Self::key(name);
        self.values.remove(&key);
        self.counters.remove(&key);
        self.expiry.remove(&key);
    }

    fn expire(&mut self, now: EngineMillis) {
        if self.expiry.is_empty() {
            return;
        }
        let expired: Vec<String> = self
            .expiry
            .iter()
            .filter(|(_, at)| now >= **at)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.values.remove(&key);
            self.counters.remove(&key);
            self.expiry.remove(&key);
        }
    }

    fn as_match_map(&self) -> MatchMap {
        let mut map = MatchMap::new();
        for (key, value) in &self.values {
            map.insert(key, value.clone());
        }
        map
    }
}

struct CharState {
    context: CharacterContext,
    active: Vec<usize>,
    /// Runtime-disabled triggers (regex budget exceeded), per EQLP policy.
    disabled: Vec<bool>,
    /// Lazily compiled per-character regexes for `{c}` patterns:
    /// (trigger index, is_previous) → compiled.
    character_regexes: HashMap<(usize, bool), Option<CompatRegex>>,
    variables: VariableStore,
    counter_times: HashMap<usize, HashMap<String, RepeatedData>>,
    repeated_text: HashMap<usize, HashMap<String, RepeatedData>>,
    repeated_timer: HashMap<usize, HashMap<String, RepeatedData>>,
    repeated_speak: HashMap<usize, HashMap<String, RepeatedData>>,
    lockout_until: HashMap<usize, EngineMillis>,
    previous_line: Option<String>,
    timers: Vec<ActiveTimer>,
}

// ---------------------------------------------------------------------------
// Engine
// ---------------------------------------------------------------------------

pub struct TriggerEngine {
    compiled: Arc<CompiledLibrary>,
    characters: HashMap<String, CharState>,
}

struct LineMatch {
    captures: MatchMap,
    spans: Vec<(usize, usize)>,
    dynamic_duration: Option<f64>,
}

impl TriggerEngine {
    pub fn new(compiled: Arc<CompiledLibrary>) -> Self {
        Self {
            compiled,
            characters: HashMap::new(),
        }
    }

    pub fn compiled(&self) -> &Arc<CompiledLibrary> {
        &self.compiled
    }

    /// Drop all state for a character (client removed).
    pub fn remove_character(&mut self, key: &str) -> bool {
        self.characters.remove(key).is_some()
    }

    /// Snapshot every running timer across characters.
    pub fn timer_snapshots(&self) -> Vec<TimerSnapshot> {
        let mut snapshots = Vec::new();
        for state in self.characters.values() {
            for timer in &state.timers {
                snapshots.push(self.snapshot_timer(state, timer));
            }
        }
        snapshots.sort_by(|a, b| {
            a.end_ms
                .cmp(&b.end_ms)
                .then_with(|| a.display_name.cmp(&b.display_name))
        });
        snapshots
    }

    fn snapshot_timer(&self, state: &CharState, timer: &ActiveTimer) -> TimerSnapshot {
        let compiled = &self.compiled.triggers[timer.trigger_index];
        let variables = state.variables.as_match_map();
        TimerSnapshot {
            character: state.context.key.clone(),
            trigger_id: timer.trigger_id,
            kind: timer.kind,
            display_name: resolve_display_name(timer, &variables),
            begin_ms: timer.begin,
            end_ms: timer.end,
            duration_ms: timer.duration_ms,
            reset_at_ms: timer.reset_at,
            warned: timer.warned,
            target: compiled.trigger.target,
            timer_overlays: compiled.trigger.timer_overlays.clone(),
            font_color: compiled.trigger.font_color.clone(),
            active_color: compiled.trigger.active_color.clone(),
            idle_color: compiled.trigger.idle_color.clone(),
            reset_color: compiled.trigger.reset_color.clone(),
        }
    }

    fn state_for(&mut self, context: &CharacterContext) -> &mut CharState {
        let compiled = &self.compiled;
        self.characters
            .entry(context.key.clone())
            .or_insert_with(|| {
                let active = compiled.active_for(&context.character, &context.server);
                CharState {
                    context: context.clone(),
                    active,
                    disabled: vec![false; compiled.triggers.len()],
                    character_regexes: HashMap::new(),
                    variables: VariableStore::default(),
                    counter_times: HashMap::new(),
                    repeated_text: HashMap::new(),
                    repeated_timer: HashMap::new(),
                    repeated_speak: HashMap::new(),
                    lockout_until: HashMap::new(),
                    previous_line: None,
                    timers: Vec::new(),
                }
            })
    }

    /// Evaluate one log line for one character.
    ///
    /// `log_time` is the display form of the log timestamp for `{logtime}`.
    pub fn process_line(
        &mut self,
        context: &CharacterContext,
        line: &str,
        log_time: Option<&str>,
        now: EngineMillis,
        mut trace: Option<&mut LineTrace>,
    ) -> ActionBatch {
        let mut batch = ActionBatch::default();
        if line.is_empty() {
            return batch;
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.line = line.to_owned();
        }
        // Take the state out so we can borrow compiled immutably alongside.
        self.state_for(context);
        let mut state = self
            .characters
            .remove(&context.key)
            .expect("state created above");
        let mut expired = false;

        let active = state.active.clone();
        for index in active {
            if state.disabled[index] {
                continue;
            }
            let mut entry = TriggerTrace {
                trigger_id: Some(self.compiled.triggers[index].id),
                trigger_name: self.compiled.triggers[index].name.clone(),
                constraints_passed: true,
                ..TriggerTrace::default()
            };
            let record_trace = |trace: &mut Option<&mut LineTrace>, entry: TriggerTrace| {
                if let Some(trace) = trace.as_deref_mut() {
                    trace.entries.push(entry);
                }
            };

            let Some(line_match) = self.check_line(&mut state, index, line, false) else {
                if trace.is_some() && self.prefilter_admits(&state, index, line) {
                    record_trace(&mut trace, entry);
                }
                continue;
            };
            entry.matched = true;
            entry.match_spans = line_match.spans.clone();
            entry.captures = line_match
                .captures
                .iter()
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
                .collect();

            let previous_matches = if self.compiled.triggers[index].previous.is_some() {
                let previous_line = state.previous_line.clone();
                let previous = previous_line
                    .as_deref()
                    .and_then(|previous| self.check_line(&mut state, index, previous, true));
                entry.previous_line_matched = Some(previous.is_some());
                match previous {
                    Some(matched) => matched.captures,
                    None => {
                        record_trace(&mut trace, entry);
                        continue;
                    }
                }
            } else {
                MatchMap::new()
            };

            if !expired {
                state.variables.expire(now);
                expired = true;
            }

            let compiled = &self.compiled.triggers[index];
            if let Some(condition) = &compiled.condition {
                let variables = &state.variables;
                let captures = &line_match.captures;
                let previous = &previous_matches;
                let passed = conditions::evaluate(condition, &|name| {
                    variables
                        .get(name)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .or_else(|| {
                            captures
                                .get(name)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        })
                        .or_else(|| {
                            previous
                                .get(name)
                                .filter(|value| !value.is_empty())
                                .map(str::to_owned)
                        })
                });
                entry.condition_passed = Some(passed);
                if !passed {
                    record_trace(&mut trace, entry);
                    continue;
                }
            } else if compiled.condition_invalid {
                entry.condition_passed = Some(false);
                record_trace(&mut trace, entry);
                continue;
            }

            self.handle_trigger(
                &mut state,
                index,
                line,
                log_time,
                &line_match.captures,
                &previous_matches,
                line_match.dynamic_duration,
                now,
                0,
                &mut batch,
                Some(&mut entry),
            );
            record_trace(&mut trace, entry);
        }

        // End-early checks against every running timer.
        self.check_timer_enders(&mut state, line, now, &mut batch);

        state.previous_line = Some(line.to_owned());
        self.characters.insert(context.key.clone(), state);
        batch
    }

    fn prefilter_admits(&self, _state: &CharState, index: usize, line: &str) -> bool {
        match &self.compiled.triggers[index].matcher {
            Matcher::Regex { prefilter, .. } => prefilter.admits(line),
            Matcher::Literal { text, .. } => crate::netregex::contains_ignore_case(line, text),
        }
    }

    fn check_line(
        &self,
        state: &mut CharState,
        index: usize,
        line: &str,
        is_previous: bool,
    ) -> Option<LineMatch> {
        let compiled = &self.compiled.triggers[index];
        let matcher = if is_previous {
            compiled.previous.as_ref()?
        } else {
            &compiled.matcher
        };
        match matcher {
            Matcher::Literal {
                text,
                needs_character,
            } => {
                let resolved;
                let text = if *needs_character {
                    resolved = substitute::replace_code(
                        text,
                        substitute::CHARACTER_CODE,
                        &state.context.character,
                    );
                    &resolved
                } else {
                    text
                };
                (!text.is_empty() && crate::netregex::contains_ignore_case(line, text)).then(|| {
                    LineMatch {
                        captures: MatchMap::new(),
                        spans: Vec::new(),
                        dynamic_duration: None,
                    }
                })
            }
            Matcher::Regex {
                regex,
                constraints,
                prefilter,
                expanded_pattern,
                needs_character,
            } => {
                if !prefilter.admits(line) {
                    return None;
                }
                let outcome = if *needs_character {
                    let character = state.context.character.clone();
                    let entry = state
                        .character_regexes
                        .entry((index, is_previous))
                        .or_insert_with(|| {
                            let pattern = substitute::replace_code(
                                expanded_pattern,
                                substitute::CHARACTER_CODE,
                                &regex::escape(&character),
                            );
                            CompatRegex::compile(&pattern).ok()
                        });
                    entry.as_ref()?.snapshot_matches(line)
                } else {
                    regex.snapshot_matches(line)
                };
                let outcome = match outcome {
                    Ok(outcome) => outcome?,
                    Err(_) => {
                        // Budget exceeded: disable the trigger for this
                        // character, as EQLP does on regex timeout.
                        state.disabled[index] = true;
                        return None;
                    }
                };
                let (passed, duration) = pattern::check_constraints(constraints, &outcome.captures);
                passed.then_some(LineMatch {
                    captures: outcome.captures,
                    spans: outcome.spans,
                    dynamic_duration: duration,
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_trigger(
        &self,
        state: &mut CharState,
        index: usize,
        line: &str,
        log_time: Option<&str>,
        captures: &MatchMap,
        previous_matches: &MatchMap,
        dynamic_duration: Option<f64>,
        now: EngineMillis,
        loop_count: u32,
        batch: &mut ActionBatch,
        mut entry: Option<&mut TriggerTrace>,
    ) {
        let compiled = &self.compiled.triggers[index];
        let trigger = &compiled.trigger;

        // Lockout (initial firings only; loop repeats bypass it).
        if loop_count == 0 && trigger.lockout_seconds > 0.0 {
            if let Some(until) = state.lockout_until.get(&index) {
                if now <= *until {
                    if let Some(entry) = entry.as_deref_mut() {
                        entry.lockout_blocked = true;
                    }
                    return;
                }
            }
            state.lockout_until.insert(
                index,
                now + (trigger.lockout_seconds * 1000.0) as EngineMillis,
            );
        }

        // {counter}: counts firings regardless of output uniqueness.
        let uses_counter = compiled.speak_flags.counter
            || compiled.display_flags.counter
            || compiled.timer_flags.counter;
        let counter_count = if uses_counter {
            update_repeated(
                &mut state.counter_times,
                index,
                "trigger-count",
                now,
                trigger.repeated_reset_seconds,
            )
        } else {
            -1
        };

        // Variable actions.
        self.apply_variable_actions(
            state,
            index,
            line,
            captures,
            previous_matches,
            now,
            &mut entry,
        );

        // Timer.
        let mut timer_name =
            substitute::replace_from_matches(&compiled.timer_name_template, Some(captures));
        timer_name = substitute::replace_from_matches(&timer_name, Some(previous_matches));
        timer_name = substitute::replace_line_code(&timer_name, line);
        timer_name = substitute::replace_code(
            &timer_name,
            substitute::CHARACTER_CODE,
            &state.context.character,
        );
        if compiled.timer_flags.repeated {
            update_repeated(
                &mut state.repeated_timer,
                index,
                &timer_name,
                now,
                trigger.repeated_reset_seconds,
            );
        }
        if let Some(behavior) = &trigger.timer {
            let dynamic = dynamic_duration
                .filter(|duration| behavior.kind.accepts_dynamic_duration() && *duration > 0.0);
            if behavior.duration_seconds > 0.0 || dynamic.is_some() {
                let template = timer_name.clone();
                let resolved_name = substitute::replace_tokens(&timer_name, |name| {
                    state.variables.get(name).map(str::to_owned)
                });
                self.start_timer(
                    state,
                    index,
                    resolved_name,
                    template,
                    dynamic,
                    line,
                    log_time,
                    captures,
                    previous_matches,
                    now,
                    loop_count,
                    counter_count,
                    batch,
                );
                batch.timers_changed = true;
            }
        }

        // Initial speak / sound.
        self.push_audio(
            state,
            index,
            ActionPhase::Initial,
            trigger.sound.as_deref(),
            &compiled.speak_template,
            line,
            Some(captures),
            None,
            Some(previous_matches),
            log_time,
            counter_count,
            now,
            batch,
            &mut entry,
        );

        // Initial display text.
        let variables = state.variables.as_match_map();
        if let Some(mut text) = substitute::resolve_template(
            &compiled.display_template,
            line,
            Some(captures),
            None,
            Some(previous_matches),
            &variables,
        ) {
            text = substitute::replace_code(
                &text,
                substitute::CHARACTER_CODE,
                &state.context.character,
            );
            if compiled.display_flags.repeated {
                let repeated = update_repeated(
                    &mut state.repeated_text,
                    index,
                    &text,
                    now,
                    trigger.repeated_reset_seconds,
                );
                text = substitute::replace_code(
                    &text,
                    substitute::REPEATED_CODE,
                    &repeated.to_string(),
                );
            }
            if compiled.display_flags.counter {
                text = substitute::replace_code(
                    &text,
                    substitute::COUNTER_CODE,
                    &counter_count.to_string(),
                );
            }
            if compiled.display_flags.logtime {
                if let Some(log_time) = log_time {
                    text = substitute::replace_code(&text, substitute::LOGTIME_CODE, log_time);
                }
            }
            if let Some(entry) = entry {
                entry.actions.push(format!("display: {text}"));
            }
            batch
                .events
                .push(self.display_event(state, index, ActionPhase::Initial, text));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_variable_actions(
        &self,
        state: &mut CharState,
        index: usize,
        line: &str,
        captures: &MatchMap,
        previous_matches: &MatchMap,
        now: EngineMillis,
        entry: &mut Option<&mut TriggerTrace>,
    ) {
        let compiled = &self.compiled.triggers[index];
        for action in &compiled.trigger.variable_actions {
            let name = action.name.trim();
            if name.is_empty() {
                continue;
            }
            let expires_at = (action.time_to_live_seconds > 0.0)
                .then(|| now + (action.time_to_live_seconds * 1000.0) as EngineMillis);
            match action.op {
                VariableOp::Clear => {
                    state.variables.clear(name);
                    if let Some(entry) = entry.as_deref_mut() {
                        entry.variable_mutations.push((name.to_owned(), None));
                    }
                }
                VariableOp::SetCounter => {
                    let key = VariableStore::key(name);
                    let current = match state.variables.counters.get(&key) {
                        Some(current) => *current,
                        None => state
                            .variables
                            .get(name)
                            .and_then(|existing| existing.parse::<f64>().ok())
                            .unwrap_or(action.initial_value),
                    };
                    let updated = current + action.step;
                    state.variables.counters.insert(key, updated);
                    let rendered = format_counter(updated);
                    state.variables.set(name, rendered.clone(), expires_at);
                    if let Some(entry) = entry.as_deref_mut() {
                        entry
                            .variable_mutations
                            .push((name.to_owned(), Some(rendered)));
                    }
                }
                VariableOp::SetValue => {
                    if action.value.is_empty() {
                        continue;
                    }
                    let mut resolved =
                        substitute::replace_from_matches(&action.value, Some(captures));
                    resolved = substitute::replace_from_matches(&resolved, Some(previous_matches));
                    resolved = substitute::replace_line_code(&resolved, line);
                    let variables = state.variables.as_match_map();
                    resolved = substitute::replace_tokens(&resolved, |name| {
                        variables.get(name).map(str::to_owned)
                    });
                    state.variables.set(name, resolved.clone(), expires_at);
                    if let Some(entry) = entry.as_deref_mut() {
                        entry
                            .variable_mutations
                            .push((name.to_owned(), Some(resolved)));
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn start_timer(
        &self,
        state: &mut CharState,
        index: usize,
        display_name: String,
        display_name_template: String,
        dynamic_duration: Option<f64>,
        line: &str,
        log_time: Option<&str>,
        captures: &MatchMap,
        previous_matches: &MatchMap,
        now: EngineMillis,
        loop_count: u32,
        counter_count: i64,
        batch: &mut ActionBatch,
    ) {
        let compiled = &self.compiled.triggers[index];
        let trigger = &compiled.trigger;
        let behavior = trigger.timer.as_ref().expect("caller verified");

        match behavior.restart_mode {
            TimerRestartMode::StartNew => {}
            TimerRestartMode::RestartAll => {
                state.timers.retain(|timer| timer.trigger_index != index);
            }
            TimerRestartMode::RestartSameName => {
                state.timers.retain(|timer| {
                    timer.trigger_index != index
                        || !timer.display_name.eq_ignore_ascii_case(&display_name)
                });
            }
            TimerRestartMode::IgnoreIfAnyRunning => {
                if state
                    .timers
                    .iter()
                    .any(|timer| timer.trigger_index == index)
                {
                    return;
                }
            }
            TimerRestartMode::IgnoreIfSameNameRunning => {
                if state.timers.iter().any(|timer| {
                    timer.trigger_index == index
                        && timer.display_name.eq_ignore_ascii_case(&display_name)
                }) {
                    return;
                }
            }
        }

        let duration_seconds = dynamic_duration.unwrap_or(behavior.duration_seconds);
        let duration_ms = (duration_seconds * 1000.0).max(0.0) as u64;
        let warning_at = (behavior.warning_seconds > 0
            && duration_seconds - behavior.warning_seconds as f64 > 0.0)
            .then(|| now + duration_ms - u64::from(behavior.warning_seconds) * 1000);

        let mut early_enders = Vec::new();
        for template in &compiled.ender_templates {
            let mut pattern = substitute::replace_from_matches(&template.pattern, Some(captures));
            pattern = substitute::replace_from_matches(&pattern, Some(previous_matches));
            pattern = substitute::replace_code(
                &pattern,
                substitute::CHARACTER_CODE,
                &state.context.character,
            );
            if template.use_regex {
                let expanded = pattern::expand(&pattern);
                if let Ok(regex) = CompatRegex::compile(&expanded.pattern) {
                    early_enders.push(CompiledEnder::Regex {
                        regex,
                        constraints: expanded.constraints,
                    });
                }
            } else {
                early_enders.push(CompiledEnder::Literal(pattern));
            }
        }

        let repeated_count = if compiled.timer_flags.repeated {
            get_repeated(&state.repeated_timer, index, &display_name_template)
        } else {
            -1
        };

        state.timers.push(ActiveTimer {
            trigger_index: index,
            trigger_id: compiled.id,
            kind: behavior.kind,
            display_name,
            display_name_template,
            begin: now,
            end: now + duration_ms,
            duration_ms,
            reset_at: (behavior.reset_duration_seconds > 0.0)
                .then(|| now + (behavior.reset_duration_seconds * 1000.0) as EngineMillis),
            warning_at,
            warned: false,
            loop_count,
            early_enders,
            original_matches: captures.clone(),
            previous_matches: previous_matches.clone(),
            source_line: line.to_owned(),
            log_time: log_time.map(str::to_owned),
            counter_count: if compiled.timer_flags.counter {
                counter_count
            } else {
                -1
            },
            repeated_count,
        });
        batch.timers_changed = true;
    }

    /// Advance the virtual clock: fire due warnings and natural ends.
    pub fn advance(&mut self, now: EngineMillis) -> ActionBatch {
        let mut batch = ActionBatch::default();
        let keys: Vec<String> = self.characters.keys().cloned().collect();
        for key in keys {
            let mut state = self.characters.remove(&key).expect("key just listed");

            // Warnings.
            for position in 0..state.timers.len() {
                if state.timers[position].warning_due(now) {
                    state.timers[position].warned = true;
                    let timer = state.timers[position].clone();
                    state.variables.expire(now);
                    let line = timer.source_line.clone();
                    self.fire_timer_stage(
                        &mut state,
                        &timer,
                        ActionPhase::TimerWarning,
                        None,
                        &line,
                        now,
                        &mut batch,
                    );
                }
            }

            // Natural ends (collect first: firing may start loop timers).
            let mut ended = Vec::new();
            state.timers.retain(|timer| {
                if timer.is_ended(now) {
                    ended.push(timer.clone());
                    false
                } else {
                    true
                }
            });
            for timer in ended {
                batch.timers_changed = true;
                state.variables.expire(now);
                let line = timer.source_line.clone();
                self.fire_timer_stage(
                    &mut state,
                    &timer,
                    ActionPhase::TimerEnd,
                    None,
                    &line,
                    now,
                    &mut batch,
                );
                self.clear_end_variables(&mut state, timer.trigger_index);
                // Looping repeat.
                let compiled = &self.compiled.triggers[timer.trigger_index];
                if let Some(behavior) = &compiled.trigger.timer {
                    if behavior.kind == TimerKind::Looping
                        && behavior.times_to_loop > timer.loop_count
                    {
                        let captures = timer.original_matches.clone();
                        let previous = timer.previous_matches.clone();
                        self.handle_trigger(
                            &mut state,
                            timer.trigger_index,
                            &timer.source_line,
                            timer.log_time.as_deref(),
                            &captures,
                            &previous,
                            None,
                            now,
                            timer.loop_count + 1,
                            &mut batch,
                            None,
                        );
                    }
                }
            }

            self.characters.insert(key, state);
        }
        batch
    }

    fn check_timer_enders(
        &self,
        state: &mut CharState,
        line: &str,
        now: EngineMillis,
        batch: &mut ActionBatch,
    ) {
        let mut ended: Vec<(ActiveTimer, MatchMap)> = Vec::new();
        let compiled_library = &self.compiled;
        let repeated_timer = &state.repeated_timer;
        let counter_times = &state.counter_times;
        state.timers.retain(|timer| {
            for ender in &timer.early_enders {
                if let Some(captures) = ender.matches(line) {
                    ended.push((timer.clone(), captures));
                    return false;
                }
            }
            // Repeated-count threshold ender.
            let compiled = &compiled_library.triggers[timer.trigger_index];
            if let Some(behavior) = &compiled.trigger.timer {
                if behavior.end_early_repeated_count > 0
                    && (compiled.timer_flags.counter || compiled.timer_flags.repeated)
                {
                    let stop = i64::from(behavior.end_early_repeated_count);
                    let repeated =
                        get_repeated(repeated_timer, timer.trigger_index, &timer.display_name);
                    let counter = get_repeated(counter_times, timer.trigger_index, "trigger-count");
                    if repeated >= stop || counter >= stop {
                        ended.push((timer.clone(), MatchMap::new()));
                        return false;
                    }
                }
            }
            true
        });

        for (timer, early_matches) in ended {
            batch.timers_changed = true;
            // Reset the repeated counters consumed by the threshold ender.
            if let Some(map) = state.repeated_timer.get_mut(&timer.trigger_index) {
                map.remove(&repeat_key(&timer.display_name));
            }
            if let Some(map) = state.counter_times.get_mut(&timer.trigger_index) {
                map.remove(&repeat_key("trigger-count"));
            }
            state.variables.expire(now);
            self.fire_timer_stage(
                state,
                &timer,
                ActionPhase::TimerEndEarly,
                Some(&early_matches),
                line,
                now,
                batch,
            );
            self.clear_end_variables(state, timer.trigger_index);
        }
    }

    fn clear_end_variables(&self, state: &mut CharState, index: usize) {
        let compiled = &self.compiled.triggers[index];
        if let Some(behavior) = &compiled.trigger.timer {
            for name in &behavior.end_clear_variables {
                let name = name
                    .trim()
                    .trim_start_matches('$')
                    .trim_start_matches('{')
                    .trim_end_matches('}')
                    .trim();
                if !name.is_empty() {
                    state.variables.clear(name);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn fire_timer_stage(
        &self,
        state: &mut CharState,
        timer: &ActiveTimer,
        phase: ActionPhase,
        early_matches: Option<&MatchMap>,
        line: &str,
        now: EngineMillis,
        batch: &mut ActionBatch,
    ) {
        let compiled = &self.compiled.triggers[timer.trigger_index];
        let trigger = &compiled.trigger;
        let Some(behavior) = &trigger.timer else {
            return;
        };

        // Early-end falls back to the normal end stage when blank (EQLP).
        let (sound, speak_template, display_template) = match phase {
            ActionPhase::TimerWarning => (
                behavior.warning.sound.as_deref(),
                compiled.warning_speak.as_str(),
                compiled.warning_display.as_str(),
            ),
            ActionPhase::TimerEnd => (
                behavior.end.sound.as_deref(),
                compiled.end_speak.as_str(),
                compiled.end_display.as_str(),
            ),
            ActionPhase::TimerEndEarly => {
                let sound = behavior
                    .early_end
                    .sound
                    .as_deref()
                    .filter(|sound| !sound.is_empty())
                    .or(behavior.end.sound.as_deref());
                let speak = if compiled.early_speak.is_empty() && sound.is_none() {
                    compiled.end_speak.as_str()
                } else {
                    compiled.early_speak.as_str()
                };
                let display = if compiled.early_display.is_empty() {
                    compiled.end_display.as_str()
                } else {
                    compiled.early_display.as_str()
                };
                (sound, speak, display)
            }
            ActionPhase::Initial => unreachable!("initial stage handled elsewhere"),
        };

        let matches = match phase {
            ActionPhase::TimerEndEarly => early_matches.cloned().unwrap_or_default(),
            _ => timer.original_matches.clone(),
        };
        let (current, original) = match phase {
            ActionPhase::TimerEndEarly => (Some(&matches), Some(&timer.original_matches)),
            _ => (Some(&matches), None),
        };

        self.push_audio(
            state,
            timer.trigger_index,
            phase,
            sound,
            speak_template,
            line,
            current,
            original,
            Some(&timer.previous_matches),
            timer.log_time.as_deref(),
            timer.counter_count,
            now,
            batch,
            &mut None,
        );

        let variables = state.variables.as_match_map();
        if let Some(mut text) = substitute::resolve_template(
            display_template,
            line,
            current,
            original,
            Some(&timer.previous_matches),
            &variables,
        ) {
            text = substitute::replace_code(
                &text,
                substitute::CHARACTER_CODE,
                &state.context.character,
            );
            batch
                .events
                .push(self.display_event(state, timer.trigger_index, phase, text));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn push_audio(
        &self,
        state: &mut CharState,
        index: usize,
        phase: ActionPhase,
        sound: Option<&str>,
        speak_template: &str,
        line: &str,
        matches: Option<&MatchMap>,
        original: Option<&MatchMap>,
        previous: Option<&MatchMap>,
        log_time: Option<&str>,
        counter_count: i64,
        now: EngineMillis,
        batch: &mut ActionBatch,
        entry: &mut Option<&mut TriggerTrace>,
    ) {
        let compiled = &self.compiled.triggers[index];
        let trigger = &compiled.trigger;

        // A configured sound file wins over TTS (EQLP GetFromDecodedSoundOrText).
        if let Some(sound) = sound.filter(|sound| is_sound_file(sound)) {
            if !sound.eq_ignore_ascii_case(substitute::NULL_CODE) {
                if let Some(entry) = entry.as_deref_mut() {
                    entry.actions.push(format!("sound: {sound}"));
                }
                batch.events.push(ActionEvent {
                    character: state.context.key.clone(),
                    trigger_id: compiled.id,
                    trigger_name: compiled.name.clone(),
                    phase,
                    target: trigger.target,
                    priority: trigger.priority,
                    action: TriggerAction::PlaySound {
                        sound: sound.to_owned(),
                        volume: trigger.volume,
                    },
                });
            }
            return;
        }

        if speak_template.is_empty() || speak_template.eq_ignore_ascii_case(substitute::NULL_CODE) {
            return;
        }

        let variables = state.variables.as_match_map();
        let Some(mut tts) = substitute::resolve_template(
            speak_template,
            line,
            matches,
            original,
            previous,
            &variables,
        ) else {
            return;
        };
        tts = substitute::replace_code(&tts, substitute::CHARACTER_CODE, &state.context.character);
        if phase == ActionPhase::Initial {
            if compiled.speak_flags.repeated {
                let repeated = update_repeated(
                    &mut state.repeated_speak,
                    index,
                    &tts,
                    now,
                    trigger.repeated_reset_seconds,
                );
                tts = substitute::replace_code(
                    &tts,
                    substitute::REPEATED_CODE,
                    &repeated.to_string(),
                );
            }
            if compiled.speak_flags.counter && counter_count > 0 {
                tts = substitute::replace_code(
                    &tts,
                    substitute::COUNTER_CODE,
                    &counter_count.to_string(),
                );
            }
            if compiled.speak_flags.logtime {
                if let Some(log_time) = log_time {
                    tts = substitute::replace_code(&tts, substitute::LOGTIME_CODE, log_time);
                }
            }
        }
        let tts = substitute::sanitize_tts(&tts);
        if tts.trim().is_empty() {
            return;
        }
        if let Some(entry) = entry.as_deref_mut() {
            entry.actions.push(format!("speak: {tts}"));
        }
        batch.events.push(ActionEvent {
            character: state.context.key.clone(),
            trigger_id: compiled.id,
            trigger_name: compiled.name.clone(),
            phase,
            target: trigger.target,
            priority: trigger.priority,
            action: TriggerAction::Speak {
                text: tts,
                rate: (trigger.voice_rate > 0).then(|| trigger.voice_rate - 1),
                volume: trigger.volume,
            },
        });
    }

    fn display_event(
        &self,
        state: &CharState,
        index: usize,
        phase: ActionPhase,
        text: String,
    ) -> ActionEvent {
        let compiled = &self.compiled.triggers[index];
        let trigger = &compiled.trigger;
        ActionEvent {
            character: state.context.key.clone(),
            trigger_id: compiled.id,
            trigger_name: compiled.name.clone(),
            phase,
            target: trigger.target,
            priority: trigger.priority,
            action: TriggerAction::DisplayText {
                text,
                overlays: trigger.text_overlays.clone(),
                font_color: trigger.font_color.clone(),
            },
        }
    }

    /// `{EQLP:CLEAR}`-style full variable reset for one character.
    pub fn clear_variables(&mut self, key: &str) {
        if let Some(state) = self.characters.get_mut(key) {
            state.variables = VariableStore::default();
        }
    }
}

fn repeat_key(value: &str) -> String {
    value.to_ascii_lowercase()
}

/// Update a repeated-count bucket. Real 750 ms semantics: the count resets
/// when more than the reset window (milliseconds) elapsed since the previous
/// firing — EQLP's truncated integer-second comparison is a documented
/// defect we do not reproduce.
fn update_repeated(
    buckets: &mut HashMap<usize, HashMap<String, RepeatedData>>,
    index: usize,
    value: &str,
    now: EngineMillis,
    reset_seconds: f64,
) -> i64 {
    if value.is_empty() || reset_seconds < 0.0 {
        return -1;
    }
    let bucket = buckets.entry(index).or_default();
    let data = bucket.entry(repeat_key(value)).or_default();
    let reset_ms = (reset_seconds * 1000.0) as u64;
    if data.count == 0 || now.saturating_sub(data.last_ms) > reset_ms {
        data.count = 1;
    } else {
        data.count += 1;
    }
    data.last_ms = now;
    data.count
}

fn get_repeated(
    buckets: &HashMap<usize, HashMap<String, RepeatedData>>,
    index: usize,
    value: &str,
) -> i64 {
    buckets
        .get(&index)
        .and_then(|bucket| bucket.get(&repeat_key(value)))
        .map_or(-1, |data| data.count)
}

fn format_counter(value: f64) -> String {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

fn is_sound_file(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.ends_with(".wav") || lower.ends_with(".mp3")
}

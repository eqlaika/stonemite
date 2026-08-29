//! Engine behavior tests on the virtual clock.

use std::sync::Arc;

use eqtrigger::*;

fn context(key: &str, name: &str) -> CharacterContext {
    CharacterContext {
        key: key.to_owned(),
        character: name.to_owned(),
        server: "teek".to_owned(),
    }
}

fn engine_for(triggers: Vec<Trigger>) -> TriggerEngine {
    let mut library = TriggerLibrary::new();
    library.triggers = triggers;
    let compiled = Arc::new(CompiledLibrary::compile(&library));
    assert!(
        compiled.compile_errors.is_empty(),
        "unexpected compile errors: {:?}",
        compiled.compile_errors
    );
    TriggerEngine::new(compiled)
}

fn display_texts(batch: &ActionBatch) -> Vec<String> {
    batch
        .events
        .iter()
        .filter_map(|event| match &event.action {
            TriggerAction::DisplayText { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn speak_texts(batch: &ActionBatch) -> Vec<String> {
    batch
        .events
        .iter()
        .filter_map(|event| match &event.action {
            TriggerAction::Speak { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn literal_matching_is_case_insensitive_and_captures_nothing() {
    let mut engine = engine_for(vec![Trigger {
        name: "Tell".to_owned(),
        enabled: true,
        pattern: Pattern::literal("tells you"),
        display_text: Some("Incoming tell".to_owned()),
        ..Trigger::default()
    }]);
    let batch = engine.process_line(
        &context("c1", "Bilka"),
        "Kafka TELLS YOU, 'hi'",
        None,
        0,
        None,
    );
    assert_eq!(display_texts(&batch), vec!["Incoming tell"]);
}

#[test]
fn regex_captures_flow_into_display_speak_and_character_code() {
    let mut engine = engine_for(vec![Trigger {
        name: "Slain".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"^You have been slain by (?<killer>.+)!"),
        display_text: Some("{c} died to {killer}".to_owned()),
        speak_text: Some("{killer} killed {c}".to_owned()),
        ..Trigger::default()
    }]);
    let batch = engine.process_line(
        &context("c1", "Bilka"),
        "You have been slain by a gnoll pup!",
        None,
        0,
        None,
    );
    assert_eq!(display_texts(&batch), vec!["Bilka died to a gnoll pup"]);
    assert_eq!(speak_texts(&batch), vec!["a gnoll pup killed Bilka"]);
}

#[test]
fn gina_macros_and_numeric_constraints_gate_firing() {
    let mut engine = engine_for(vec![Trigger {
        name: "Big hit".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"{S1} hits you for {N>=1000} points"),
        display_text: Some("{S1} hit for {N}".to_owned()),
        ..Trigger::default()
    }]);
    let small = engine.process_line(
        &context("c1", "Bilka"),
        "A dragon hits you for 999 points of damage.",
        None,
        0,
        None,
    );
    assert!(small.events.is_empty());
    let big = engine.process_line(
        &context("c1", "Bilka"),
        "A dragon hits you for 4213 points of damage.",
        None,
        1,
        None,
    );
    assert_eq!(display_texts(&big), vec!["A dragon hit for 4213"]);
}

#[test]
fn previous_line_requirement_must_match_the_immediately_preceding_line() {
    let mut engine = engine_for(vec![Trigger {
        name: "Combo".to_owned(),
        enabled: true,
        pattern: Pattern::literal("second line"),
        previous_pattern: Some(Pattern::regex("^first (?<what>.+)$")),
        display_text: Some("combo {what}".to_owned()),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");

    // No previous line yet.
    assert!(engine
        .process_line(&ctx, "second line", None, 0, None)
        .events
        .is_empty());
    engine.process_line(&ctx, "first strike", None, 1, None);
    let hit = engine.process_line(&ctx, "second line", None, 2, None);
    assert_eq!(display_texts(&hit), vec!["combo strike"]);
    // A line in between breaks the requirement.
    engine.process_line(&ctx, "unrelated", None, 3, None);
    assert!(engine
        .process_line(&ctx, "second line", None, 4, None)
        .events
        .is_empty());
}

#[test]
fn conditions_gate_on_variables_and_captures() {
    let setter = Trigger {
        name: "Setter".to_owned(),
        enabled: true,
        pattern: Pattern::literal("boss engaged"),
        variable_actions: vec![VariableAction {
            op: VariableOp::SetValue,
            name: "phase".to_owned(),
            value: "burn".to_owned(),
            ..VariableAction::default()
        }],
        ..Trigger::default()
    };
    let gated = Trigger {
        name: "Gated".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"you feel (?<mood>.+)"),
        condition: "{phase} = 'burn' and {mood} contains 'strong'".to_owned(),
        display_text: Some("go!".to_owned()),
        ..Trigger::default()
    };
    let mut engine = engine_for(vec![setter, gated]);
    let ctx = context("c1", "Bilka");

    assert!(engine
        .process_line(&ctx, "you feel very strong", None, 0, None)
        .events
        .is_empty());
    engine.process_line(&ctx, "boss engaged", None, 1, None);
    let batch = engine.process_line(&ctx, "you feel very strong", None, 2, None);
    assert_eq!(display_texts(&batch), vec!["go!"]);
    assert!(engine
        .process_line(&ctx, "you feel weak", None, 3, None)
        .events
        .is_empty());
}

#[test]
fn invalid_condition_blocks_the_trigger_entirely() {
    let mut engine = engine_for(vec![Trigger {
        name: "Broken".to_owned(),
        enabled: true,
        pattern: Pattern::literal("anything"),
        condition: "{a} >".to_owned(),
        display_text: Some("never".to_owned()),
        ..Trigger::default()
    }]);
    assert!(engine
        .process_line(&context("c1", "Bilka"), "anything", None, 0, None)
        .events
        .is_empty());
}

#[test]
fn variable_ttl_expires_on_the_virtual_clock() {
    let setter = Trigger {
        name: "Setter".to_owned(),
        enabled: true,
        pattern: Pattern::literal("buff up"),
        variable_actions: vec![VariableAction {
            op: VariableOp::SetValue,
            name: "buffed".to_owned(),
            value: "yes".to_owned(),
            time_to_live_seconds: 10.0,
            ..VariableAction::default()
        }],
        ..Trigger::default()
    };
    let reader = Trigger {
        name: "Reader".to_owned(),
        enabled: true,
        pattern: Pattern::literal("check"),
        condition: "{buffed} = 'yes'".to_owned(),
        display_text: Some("still buffed".to_owned()),
        ..Trigger::default()
    };
    let mut engine = engine_for(vec![setter, reader]);
    let ctx = context("c1", "Bilka");
    engine.process_line(&ctx, "buff up", None, 0, None);
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "check", None, 5_000, None)).len(),
        1
    );
    assert!(engine
        .process_line(&ctx, "check", None, 10_001, None)
        .events
        .is_empty());
}

#[test]
fn counters_seed_from_value_variables_and_render_in_text() {
    let mut engine = engine_for(vec![Trigger {
        name: "Stacks".to_owned(),
        enabled: true,
        pattern: Pattern::literal("gains a stack"),
        variable_actions: vec![VariableAction {
            op: VariableOp::SetCounter,
            name: "stacks".to_owned(),
            step: 1.0,
            initial_value: 0.0,
            ..VariableAction::default()
        }],
        display_text: Some("stacks: {stacks}".to_owned()),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "gains a stack", None, 0, None)),
        vec!["stacks: 1"]
    );
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "gains a stack", None, 1, None)),
        vec!["stacks: 2"]
    );
}

#[test]
fn lockout_suppresses_repeat_firings_until_the_window_passes() {
    let mut engine = engine_for(vec![Trigger {
        name: "Locked".to_owned(),
        enabled: true,
        pattern: Pattern::literal("spam line"),
        lockout_seconds: 5.0,
        display_text: Some("fired".to_owned()),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "spam line", None, 0, None)).len(),
        1
    );
    assert!(engine
        .process_line(&ctx, "spam line", None, 3_000, None)
        .events
        .is_empty());
    assert!(engine
        .process_line(&ctx, "spam line", None, 5_000, None)
        .events
        .is_empty());
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "spam line", None, 5_001, None)).len(),
        1
    );
}

#[test]
fn repeated_code_uses_real_750ms_semantics() {
    let mut engine = engine_for(vec![Trigger {
        name: "Repeat".to_owned(),
        enabled: true,
        pattern: Pattern::literal("dot tick"),
        display_text: Some("tick x{repeated}".to_owned()),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "dot tick", None, 0, None)),
        vec!["tick x1"]
    );
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "dot tick", None, 700, None)),
        vec!["tick x2"]
    );
    // 800 ms after the last firing: beyond the 750 ms window, so it resets —
    // EQLP's truncated integer-second comparison would have kept counting.
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "dot tick", None, 1_500, None)),
        vec!["tick x1"]
    );
}

#[test]
fn counter_code_counts_every_firing() {
    let mut engine = engine_for(vec![Trigger {
        name: "Count".to_owned(),
        enabled: true,
        pattern: Pattern::literal("counted"),
        // {counter} keeps counting within the reset window regardless of text.
        repeated_reset_seconds: 100.0,
        display_text: Some("n={counter} on {l}".to_owned()),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "counted once", None, 0, None)),
        vec!["n=1 on counted once"]
    );
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "counted twice", None, 1_000, None)),
        vec!["n=2 on counted twice"]
    );
}

fn timer_trigger(name: &str, duration: f64) -> Trigger {
    Trigger {
        name: name.to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"^cast (?<spell>.+)$"),
        timer: Some(TimerBehavior {
            duration_seconds: duration,
            timer_name: "{spell}".to_owned(),
            warning_seconds: 2,
            warning: TimerStageActions {
                speak_text: Some("{spell} fading".to_owned()),
                ..TimerStageActions::default()
            },
            end: TimerStageActions {
                display_text: Some("{spell} ended".to_owned()),
                ..TimerStageActions::default()
            },
            early_end: TimerStageActions::default(),
            end_early_patterns: vec![Pattern::literal("interrupted")],
            ..TimerBehavior::default()
        }),
        ..Trigger::default()
    }
}

#[test]
fn timer_lifecycle_warning_then_end_on_virtual_clock() {
    let mut engine = engine_for(vec![timer_trigger("Buff", 10.0)]);
    let ctx = context("c1", "Bilka");

    let started = engine.process_line(&ctx, "cast Haste", None, 0, None);
    assert!(started.timers_changed);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].display_name, "Haste");
    assert_eq!(snapshots[0].end_ms, 10_000);
    assert_eq!(snapshots[0].remaining_ms(4_000), 6_000);
    assert!((snapshots[0].progress(5_000) - 0.5).abs() < 1e-6);

    // Nothing due yet.
    assert!(engine.advance(7_999).events.is_empty());
    // Warning at duration - 2s.
    let warning = engine.advance(8_000);
    assert_eq!(speak_texts(&warning), vec!["Haste fading"]);
    assert_eq!(warning.events[0].phase, ActionPhase::TimerWarning);
    // Warning fires once.
    assert!(engine.advance(8_500).events.is_empty());
    // Natural end.
    let ended = engine.advance(10_000);
    assert_eq!(display_texts(&ended), vec!["Haste ended"]);
    assert_eq!(ended.events[0].phase, ActionPhase::TimerEnd);
    assert!(ended.timers_changed);
    assert!(engine.timer_snapshots().is_empty());
}

#[test]
fn timer_end_early_pattern_fires_early_stage_with_fallback_to_end() {
    let mut engine = engine_for(vec![timer_trigger("Buff", 10.0)]);
    let ctx = context("c1", "Bilka");
    engine.process_line(&ctx, "cast Haste", None, 0, None);
    let early = engine.process_line(&ctx, "the spell was interrupted", None, 3_000, None);
    // early_end stage is empty, so it falls back to the end stage text.
    assert_eq!(display_texts(&early), vec!["Haste ended"]);
    assert_eq!(early.events[0].phase, ActionPhase::TimerEndEarly);
    assert!(engine.timer_snapshots().is_empty());
    // No further end at the natural time.
    assert!(engine.advance(10_000).events.is_empty());
}

#[test]
fn dynamic_ts_duration_overrides_configured_duration() {
    let mut engine = engine_for(vec![Trigger {
        name: "TS".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"^respawn in {TS}$"),
        timer: Some(TimerBehavior {
            duration_seconds: 5.0,
            ..TimerBehavior::default()
        }),
        ..Trigger::default()
    }]);
    engine.process_line(&context("c1", "Bilka"), "respawn in 2:30", None, 0, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots[0].duration_ms, 150_000);
}

#[test]
fn restart_modes_control_deduplication() {
    let base = |mode: TimerRestartMode| Trigger {
        name: "T".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"^go (?<name>.+)$"),
        timer: Some(TimerBehavior {
            duration_seconds: 10.0,
            timer_name: "{name}".to_owned(),
            restart_mode: mode,
            ..TimerBehavior::default()
        }),
        ..Trigger::default()
    };
    let ctx = context("c1", "Bilka");

    // StartNew stacks timers.
    let mut engine = engine_for(vec![base(TimerRestartMode::StartNew)]);
    engine.process_line(&ctx, "go alpha", None, 0, None);
    engine.process_line(&ctx, "go alpha", None, 1_000, None);
    assert_eq!(engine.timer_snapshots().len(), 2);

    // RestartAll keeps exactly one.
    let mut engine = engine_for(vec![base(TimerRestartMode::RestartAll)]);
    engine.process_line(&ctx, "go alpha", None, 0, None);
    engine.process_line(&ctx, "go beta", None, 1_000, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].display_name, "beta");
    assert_eq!(snapshots[0].begin_ms, 1_000);

    // RestartSameName restarts alpha but keeps beta.
    let mut engine = engine_for(vec![base(TimerRestartMode::RestartSameName)]);
    engine.process_line(&ctx, "go alpha", None, 0, None);
    engine.process_line(&ctx, "go beta", None, 1_000, None);
    engine.process_line(&ctx, "go ALPHA", None, 2_000, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 2);
    let alpha = snapshots
        .iter()
        .find(|s| s.display_name == "ALPHA")
        .unwrap();
    assert_eq!(alpha.begin_ms, 2_000);

    // IgnoreIfAnyRunning drops the second start entirely.
    let mut engine = engine_for(vec![base(TimerRestartMode::IgnoreIfAnyRunning)]);
    engine.process_line(&ctx, "go alpha", None, 0, None);
    engine.process_line(&ctx, "go beta", None, 1_000, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].display_name, "alpha");

    // IgnoreIfSameNameRunning allows different names only.
    let mut engine = engine_for(vec![base(TimerRestartMode::IgnoreIfSameNameRunning)]);
    engine.process_line(&ctx, "go alpha", None, 0, None);
    engine.process_line(&ctx, "go alpha", None, 1_000, None);
    engine.process_line(&ctx, "go beta", None, 2_000, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 2);
    let alpha = snapshots
        .iter()
        .find(|s| s.display_name == "alpha")
        .unwrap();
    assert_eq!(alpha.begin_ms, 0);
}

#[test]
fn looping_timer_repeats_the_configured_number_of_times() {
    let mut engine = engine_for(vec![Trigger {
        name: "Loop".to_owned(),
        enabled: true,
        pattern: Pattern::literal("start loop"),
        timer: Some(TimerBehavior {
            kind: TimerKind::Looping,
            duration_seconds: 5.0,
            times_to_loop: 2,
            end: TimerStageActions {
                display_text: Some("loop end".to_owned()),
                ..TimerStageActions::default()
            },
            ..TimerBehavior::default()
        }),
        ..Trigger::default()
    }]);
    let ctx = context("c1", "Bilka");
    engine.process_line(&ctx, "start loop", None, 0, None);
    // First natural end restarts (loop 1).
    let first = engine.advance(5_000);
    assert_eq!(display_texts(&first), vec!["loop end"]);
    assert_eq!(engine.timer_snapshots().len(), 1);
    // Second end restarts once more (loop 2).
    let second = engine.advance(10_000);
    assert_eq!(display_texts(&second), vec!["loop end"]);
    assert_eq!(engine.timer_snapshots().len(), 1);
    // Third end stops for good: times_to_loop (2) reached.
    let third = engine.advance(15_000);
    assert_eq!(display_texts(&third), vec!["loop end"]);
    assert!(engine.timer_snapshots().is_empty());
}

#[test]
fn per_character_state_is_isolated() {
    let mut engine = engine_for(vec![timer_trigger("Buff", 10.0)]);
    let bilka = context("c1", "Bilka");
    let kafka = context("c2", "Kafka");
    engine.process_line(&bilka, "cast Haste", None, 0, None);
    engine.process_line(&kafka, "cast Clarity", None, 0, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 2);
    // Ending Bilka's early does not touch Kafka's.
    engine.process_line(&bilka, "interrupted", None, 1_000, None);
    let snapshots = engine.timer_snapshots();
    assert_eq!(snapshots.len(), 1);
    assert_eq!(snapshots[0].character, "c2");
    assert_eq!(snapshots[0].display_name, "Clarity");
}

#[test]
fn profiles_scope_triggers_to_characters() {
    let mut library = TriggerLibrary::new();
    let everyone = Trigger {
        name: "Everyone".to_owned(),
        enabled: true,
        pattern: Pattern::literal("shared line"),
        display_text: Some("everyone".to_owned()),
        ..Trigger::default()
    };
    let bilka_only = Trigger {
        name: "Bilka only".to_owned(),
        enabled: true,
        pattern: Pattern::literal("shared line"),
        display_text: Some("bilka".to_owned()),
        ..Trigger::default()
    };
    library.profiles.push(Profile {
        name: "Global".to_owned(),
        triggers: vec![everyone.id],
        ..Profile::default()
    });
    library.profiles.push(Profile {
        name: "Bilka".to_owned(),
        assignment: ProfileAssignment::Characters {
            characters: vec![CharacterSelector {
                character: "bilka".to_owned(),
                server: String::new(),
            }],
        },
        triggers: vec![bilka_only.id],
        ..Profile::default()
    });
    library.triggers = vec![everyone, bilka_only];
    let mut engine = TriggerEngine::new(Arc::new(CompiledLibrary::compile(&library)));

    let bilka = engine.process_line(&context("c1", "Bilka"), "shared line", None, 0, None);
    let mut texts = display_texts(&bilka);
    texts.sort();
    assert_eq!(texts, vec!["bilka", "everyone"]);

    let kafka = engine.process_line(&context("c2", "Kafka"), "shared line", None, 0, None);
    assert_eq!(display_texts(&kafka), vec!["everyone"]);
}

#[test]
fn disabled_and_quarantined_triggers_never_compile_in() {
    let mut library = TriggerLibrary::new();
    library.triggers = vec![
        Trigger {
            name: "Disabled".to_owned(),
            enabled: false,
            pattern: Pattern::literal("x"),
            ..Trigger::default()
        },
        Trigger {
            name: "Quarantined".to_owned(),
            enabled: true,
            pattern: Pattern::regex("(?<a-b>x)"),
            quarantine: Some(Quarantine {
                reason: "unsupported-regex".to_owned(),
                detail: "balancing group".to_owned(),
            }),
            ..Trigger::default()
        },
    ];
    let compiled = CompiledLibrary::compile(&library);
    assert_eq!(compiled.active_trigger_count(), 0);
    assert!(compiled.compile_errors.is_empty());
}

#[test]
fn sound_reference_beats_speak_text() {
    let mut engine = engine_for(vec![Trigger {
        name: "Sound".to_owned(),
        enabled: true,
        pattern: Pattern::literal("ding"),
        sound: Some("alert.wav".to_owned()),
        speak_text: Some("never spoken".to_owned()),
        ..Trigger::default()
    }]);
    let batch = engine.process_line(&context("c1", "Bilka"), "ding", None, 0, None);
    assert_eq!(batch.events.len(), 1);
    assert!(matches!(
        &batch.events[0].action,
        TriggerAction::PlaySound { sound, .. } if sound == "alert.wav"
    ));
}

#[test]
fn trace_records_matches_captures_and_actions() {
    let mut engine = engine_for(vec![Trigger {
        name: "Traced".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"hits you for (?<dmg>\d+)"),
        condition: "{dmg} > 100".to_owned(),
        display_text: Some("big: {dmg}".to_owned()),
        ..Trigger::default()
    }]);
    let mut trace = LineTrace::default();
    engine.process_line(
        &context("c1", "Bilka"),
        "a bear hits you for 4200 points.",
        None,
        0,
        Some(&mut trace),
    );
    assert_eq!(trace.entries.len(), 1);
    let entry = &trace.entries[0];
    assert!(entry.matched);
    assert_eq!(entry.condition_passed, Some(true));
    assert!(entry
        .captures
        .iter()
        .any(|(k, v)| k == "dmg" && v == "4200"));
    assert_eq!(entry.actions, vec!["display: big: 4200"]);
    assert!(!entry.match_spans.is_empty());

    // Condition failure is visible in the trace.
    let mut trace = LineTrace::default();
    engine.process_line(
        &context("c1", "Bilka"),
        "a bear hits you for 50 points.",
        None,
        1,
        Some(&mut trace),
    );
    assert_eq!(trace.entries[0].condition_passed, Some(false));
    assert!(trace.entries[0].actions.is_empty());
}

#[test]
fn character_code_in_pattern_matches_per_character() {
    let mut engine = engine_for(vec![Trigger {
        name: "Own death".to_owned(),
        enabled: true,
        pattern: Pattern::regex(r"^{c} has been slain"),
        display_text: Some("dead".to_owned()),
        ..Trigger::default()
    }]);
    let bilka = context("c1", "Bilka");
    let kafka = context("c2", "Kafka");
    assert_eq!(
        display_texts(&engine.process_line(
            &bilka,
            "Bilka has been slain by a rat!",
            None,
            0,
            None
        )),
        vec!["dead"]
    );
    assert!(engine
        .process_line(&bilka, "Kafka has been slain by a rat!", None, 1, None)
        .events
        .is_empty());
    assert_eq!(
        display_texts(&engine.process_line(
            &kafka,
            "Kafka has been slain by a rat!",
            None,
            2,
            None
        )),
        vec!["dead"]
    );
}

#[test]
fn end_clear_variables_run_after_timer_end() {
    let mut engine = engine_for(vec![
        Trigger {
            name: "Timer".to_owned(),
            enabled: true,
            pattern: Pattern::literal("start it"),
            variable_actions: vec![VariableAction {
                op: VariableOp::SetValue,
                name: "target".to_owned(),
                value: "boss".to_owned(),
                ..VariableAction::default()
            }],
            timer: Some(TimerBehavior {
                duration_seconds: 5.0,
                end: TimerStageActions {
                    display_text: Some("ended {target}".to_owned()),
                    ..TimerStageActions::default()
                },
                end_clear_variables: vec!["{target}".to_owned()],
                ..TimerBehavior::default()
            }),
            ..Trigger::default()
        },
        Trigger {
            name: "Check".to_owned(),
            enabled: true,
            pattern: Pattern::literal("check"),
            condition: "{target} = 'boss'".to_owned(),
            display_text: Some("still set".to_owned()),
            ..Trigger::default()
        },
    ]);
    let ctx = context("c1", "Bilka");
    engine.process_line(&ctx, "start it", None, 0, None);
    assert_eq!(
        display_texts(&engine.process_line(&ctx, "check", None, 1_000, None)).len(),
        1
    );
    // End text still sees the variable; afterwards it is cleared.
    let ended = engine.advance(5_000);
    assert_eq!(display_texts(&ended), vec!["ended boss"]);
    assert!(engine
        .process_line(&ctx, "check", None, 6_000, None)
        .events
        .is_empty());
}

#[test]
fn pathological_regex_disables_only_that_trigger() {
    let mut engine = engine_for(vec![
        Trigger {
            name: "Evil".to_owned(),
            enabled: true,
            pattern: Pattern::regex(r"(a+)+\1$"),
            display_text: Some("evil".to_owned()),
            ..Trigger::default()
        },
        Trigger {
            name: "Fine".to_owned(),
            enabled: true,
            pattern: Pattern::literal("aaaa"),
            display_text: Some("fine".to_owned()),
            ..Trigger::default()
        },
    ]);
    let ctx = context("c1", "Bilka");
    let evil_line = "a".repeat(64) + "c";
    let batch = engine.process_line(&ctx, &evil_line, None, 0, None);
    assert_eq!(display_texts(&batch), vec!["fine"]);
    // The evil trigger is now disabled and stays quiet even on benign input.
    let batch = engine.process_line(&ctx, "aa\u{31}", None, 1, None);
    assert!(display_texts(&batch).is_empty());
}

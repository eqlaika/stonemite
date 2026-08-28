use std::sync::Arc;

use eqcombat::{
    CombatEngine, CombatPolicy, CombatRecord, EncounterPhase, EngineInput, GapReason, LogEvent,
    LogSource, MonoTime, SourceQuality, SourceRecordId,
};
use eqlog::{
    CombatAttempt, CombatEvent, DamageKind, DamageModifiers, DamageObservation, DamageOutcome,
    EqSecond, ObservedCombatant, ParserProvenance, Perspective, PetEvent, TargetSlainObservation,
};

fn source(id: &str, character: &str) -> LogSource {
    LogSource::new(id, character, "xegony")
}

fn actor(name: &str, perspective: Perspective) -> ObservedCombatant {
    ObservedCombatant {
        name: Arc::from(name),
        perspective,
    }
}

fn hit(attacker: ObservedCombatant, target: &str, amount: u64) -> LogEvent {
    LogEvent::Combat(CombatEvent::Damage(DamageObservation {
        attacker,
        explicit_owner: None,
        defender: ObservedCombatant::named(target),
        amount,
        kind: DamageKind::Melee,
        ability: Some(Arc::from("slash")),
        outcome: DamageOutcome::Hit,
        modifiers: DamageModifiers::default(),
        provenance: ParserProvenance::Melee,
    }))
}

fn apply_record(
    engine: &mut CombatEngine,
    source: &LogSource,
    generation: u64,
    sequence: u64,
    second: i64,
    now: u64,
    events: Vec<LogEvent>,
) {
    engine.apply(
        MonoTime::from_millis(now),
        EngineInput::Record(CombatRecord::new(
            SourceRecordId::new(source.id.clone(), generation, sequence),
            source.clone(),
            Some(EqSecond::new(second)),
            events,
        )),
    );
}

fn register(engine: &mut CombatEngine, sources: &[LogSource]) {
    for source in sources {
        engine.apply(
            MonoTime::ZERO,
            EngineInput::SourceRegistered {
                source: source.clone(),
                generation: 0,
            },
        );
    }
}

#[test]
fn duplicate_multi_log_hits_elect_one_whole_participant_source() {
    let sources = [
        source("a", "Bilka"),
        source("b", "Saabra"),
        source("c", "Kafka"),
        source("d", "Orlov"),
    ];
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, &sources);
    for (index, source) in sources.iter().enumerate() {
        let attacker = if index == 0 {
            actor("You", Perspective::You)
        } else {
            ObservedCombatant::named("Bilka")
        };
        apply_record(
            &mut engine,
            source,
            0,
            0,
            100,
            index as u64 * 10,
            vec![hit(attacker, "Terris Thule", 100)],
        );
    }

    let book = engine.snapshot();
    assert_eq!(book.encounters.len(), 1);
    let encounter = &book.encounters[0];
    assert_eq!(encounter.raid_damage, 100);
    assert_eq!(encounter.rows.len(), 1);
    assert_eq!(encounter.rows[0].damage, 100);
    assert_eq!(
        encounter.rows[0].source_quality,
        SourceQuality::AuthoritativePersonal
    );
    assert_eq!(encounter.rows[0].elected_source.as_str(), "a");
}

#[test]
fn complete_personal_source_beats_a_larger_observer_candidate() {
    let personal = source("personal", "Bilka");
    let observer = source("observer", "Saabra");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, &[personal.clone(), observer.clone()]);

    apply_record(
        &mut engine,
        &personal,
        0,
        0,
        100,
        0,
        vec![hit(actor("You", Perspective::You), "Terris Thule", 100)],
    );
    apply_record(
        &mut engine,
        &observer,
        0,
        0,
        100,
        10,
        vec![hit(ObservedCombatant::named("Bilka"), "Terris Thule", 100)],
    );
    apply_record(
        &mut engine,
        &observer,
        0,
        1,
        101,
        20,
        vec![hit(ObservedCombatant::named("Bilka"), "Terris Thule", 900)],
    );
    apply_record(
        &mut engine,
        &personal,
        0,
        1,
        101,
        30,
        vec![hit(actor("You", Perspective::You), "Terris Thule", 50)],
    );
    apply_record(&mut engine, &personal, 0, 2, 102, 40, Vec::new());
    apply_record(&mut engine, &observer, 0, 2, 102, 50, Vec::new());

    let encounter = &engine.snapshot().encounters[0];
    assert_eq!(encounter.raid_damage, 150);
    assert_eq!(encounter.rows[0].damage, 150);
    assert_eq!(encounter.rows[0].elected_source.as_str(), "personal");
}

#[test]
fn explicit_pet_damage_merges_with_direct_damage_inside_one_candidate() {
    let owner = source("owner", "Saabra");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, std::slice::from_ref(&owner));
    apply_record(
        &mut engine,
        &owner,
        0,
        0,
        200,
        0,
        vec![hit(actor("You", Perspective::You), "a raid boss", 100)],
    );
    let mut pet = match hit(ObservedCombatant::named("Fluffy"), "a raid boss", 40) {
        LogEvent::Combat(CombatEvent::Damage(damage)) => damage,
        _ => unreachable!(),
    };
    pet.kind = DamageKind::Pet;
    pet.explicit_owner = Some(actor("Your", Perspective::Your));
    apply_record(
        &mut engine,
        &owner,
        0,
        1,
        201,
        10,
        vec![LogEvent::Combat(CombatEvent::Damage(pet))],
    );
    apply_record(&mut engine, &owner, 0, 2, 202, 20, Vec::new());

    let row = &engine.snapshot().encounters[0].rows[0];
    assert_eq!(row.damage, 140);
    assert!(row.has_pet_damage);
    assert_eq!(row.display_name.as_ref(), "Saabra");
}

#[test]
fn late_pet_mapping_rebuilds_and_merges_without_overwriting_direct_damage() {
    let owner = source("owner", "Saabra");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, std::slice::from_ref(&owner));
    apply_record(
        &mut engine,
        &owner,
        0,
        0,
        400,
        0,
        vec![hit(actor("You", Perspective::You), "a raid boss", 100)],
    );
    apply_record(
        &mut engine,
        &owner,
        0,
        1,
        401,
        10,
        vec![hit(ObservedCombatant::named("Fluffy"), "a raid boss", 40)],
    );
    apply_record(&mut engine, &owner, 0, 2, 402, 20, Vec::new());
    assert_eq!(engine.snapshot().encounters[0].rows.len(), 2);

    apply_record(
        &mut engine,
        &owner,
        0,
        3,
        403,
        30,
        vec![LogEvent::Pet(PetEvent::OwnershipClaimed {
            pet: Arc::from("Fluffy"),
            owner: Arc::from("Saabra"),
        })],
    );
    let encounter = &engine.snapshot().encounters[0];
    assert_eq!(encounter.rows.len(), 1);
    assert_eq!(encounter.rows[0].damage, 140);
    assert_eq!(encounter.rows[0].display_name.as_ref(), "Saabra");
    assert!(encounter.rows[0].has_pet_damage);
}

#[test]
fn fixed_metadata_cross_source_interleavings_finalize_identically() {
    fn replay(observer_first: bool) -> Arc<eqcombat::EncounterBookSnapshot> {
        let personal = source("a", "Bilka");
        let observer = source("b", "Saabra");
        let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
        register(&mut engine, &[personal.clone(), observer.clone()]);
        let personal_event = || hit(actor("You", Perspective::You), "Terris Thule", 100);
        let observer_event = || hit(ObservedCombatant::named("Bilka"), "Terris Thule", 100);
        if observer_first {
            apply_record(
                &mut engine,
                &observer,
                0,
                0,
                500,
                10,
                vec![observer_event()],
            );
            apply_record(&mut engine, &personal, 0, 0, 500, 0, vec![personal_event()]);
        } else {
            apply_record(&mut engine, &personal, 0, 0, 500, 0, vec![personal_event()]);
            apply_record(
                &mut engine,
                &observer,
                0,
                0,
                500,
                10,
                vec![observer_event()],
            );
        }
        apply_record(
            &mut engine,
            &personal,
            0,
            1,
            501,
            20,
            vec![LogEvent::Combat(CombatEvent::TargetSlain(
                TargetSlainObservation {
                    target: ObservedCombatant::named("Terris Thule"),
                    killer: Some(actor("You", Perspective::You)),
                },
            ))],
        );
        engine.tick(MonoTime::from_millis(2_020));
        engine.snapshot()
    }

    assert_eq!(replay(false).as_ref(), replay(true).as_ref());
}

#[test]
fn generation_replay_deduplicates_the_last_open_second_and_rejects_closed_history() {
    let source = source("solo", "Bilka");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, std::slice::from_ref(&source));
    let first = || hit(actor("You", Perspective::You), "Terris Thule", 100);
    let open = || hit(actor("You", Perspective::You), "Terris Thule", 50);
    apply_record(&mut engine, &source, 0, 0, 700, 0, vec![first()]);
    apply_record(&mut engine, &source, 0, 1, 702, 10, vec![open()]);
    engine.apply(
        MonoTime::from_millis(20),
        EngineInput::SourceGap {
            source: source.id.clone(),
            generation: 1,
            reason: GapReason::FileTruncated,
        },
    );
    apply_record(&mut engine, &source, 1, 0, 700, 30, vec![first()]);
    apply_record(&mut engine, &source, 1, 1, 702, 40, vec![open()]);
    apply_record(&mut engine, &source, 1, 2, 703, 50, Vec::new());

    let encounter = &engine.snapshot().encounters[0];
    assert_eq!(encounter.raid_damage, 150);
    assert_eq!(encounter.rows[0].damage, 150);
}

#[test]
fn rejected_amount_sustains_a_known_target_without_contributing() {
    let source = source("solo", "Bilka");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, std::slice::from_ref(&source));
    apply_record(
        &mut engine,
        &source,
        0,
        0,
        600,
        0,
        vec![hit(actor("You", Perspective::You), "Terris Thule", 100)],
    );
    apply_record(
        &mut engine,
        &source,
        0,
        1,
        620,
        29_000,
        vec![LogEvent::Combat(CombatEvent::Attempt(CombatAttempt {
            attacker: actor("You", Perspective::You),
            defender: ObservedCombatant::named("Terris Thule"),
            outcome: DamageOutcome::Rejected,
            kind: DamageKind::Melee,
            ability: Some(Arc::from("slash")),
            provenance: ParserProvenance::CombatAttempt,
        }))],
    );
    engine.tick(MonoTime::from_millis(30_000));
    let encounter = &engine.snapshot().encounters[0];
    assert_eq!(encounter.phase, EncounterPhase::Active);
    assert_eq!(encounter.raid_damage, 100);
    engine.tick(MonoTime::from_millis(59_000));
    assert_eq!(
        engine.snapshot().encounters[0].phase,
        EncounterPhase::EndingGrace
    );
}

#[test]
fn kill_finalizes_on_monotonic_grace_and_held_result_expires() {
    let source = source("solo", "Bilka");
    let mut engine = CombatEngine::new(CombatPolicy::mvp_v1()).unwrap();
    register(&mut engine, std::slice::from_ref(&source));
    apply_record(
        &mut engine,
        &source,
        0,
        0,
        300,
        0,
        vec![hit(actor("You", Perspective::You), "Terris Thule", 1_000)],
    );
    apply_record(
        &mut engine,
        &source,
        0,
        1,
        301,
        100,
        vec![LogEvent::Combat(CombatEvent::TargetSlain(
            TargetSlainObservation {
                target: ObservedCombatant::named("Terris Thule"),
                killer: Some(actor("You", Perspective::You)),
            },
        ))],
    );
    assert_eq!(
        engine.snapshot().encounters[0].phase,
        EncounterPhase::EndingGrace
    );

    engine.tick(MonoTime::from_millis(2_100));
    assert_eq!(engine.snapshot().encounters[0].phase, EncounterPhase::Held);
    assert_eq!(engine.snapshot().encounters[0].encounter_seconds, 2);

    engine.tick(MonoTime::from_millis(10_100));
    assert!(engine.snapshot().encounters.is_empty());
}

#[test]
fn managed_rows_below_top_n_are_appended_with_true_rank() {
    use eqcombat::project_visible_rows;

    let mut rows = Vec::new();
    for rank in 1..=12 {
        rows.push(eqcombat::DpsRowSnapshot {
            rank,
            participant: eqcombat::ParticipantId::new("xegony", &format!("P{rank}")),
            display_name: Arc::from(format!("P{rank}")),
            managed: rank == 12,
            has_pet_damage: false,
            provisional_pet: false,
            damage: (20 - rank) as u128,
            active_seconds: 1,
            dps: 1,
            sdps: 1,
            contribution_millionths: 0,
            source_quality: SourceQuality::CompleteObserver,
            elected_source: eqcombat::LogSourceId::new(format!("s{rank}")),
        });
    }
    let visible = project_visible_rows(&rows, 10);
    assert_eq!(visible.len(), 11);
    assert_eq!(visible.last().unwrap().rank, 12);
}

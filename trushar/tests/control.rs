use trushar::control::{
    ActivationStatus, BroadcastState, ClientTarget, CommandOutcome, Controller, ErrorCode,
    InMemoryController, InputKind, KeyCode, KeyStroke, RecordedInput, SnapshotMapper, SourceClient,
};

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn available(enabled: bool) -> BroadcastState {
    BroadcastState {
        available: true,
        enabled,
    }
}

#[test]
fn maps_zero_and_multiple_loaded_clients_without_confusing_window_number_with_layout_order() {
    let mut mapper = SnapshotMapper::default();
    let empty = mapper.map(&[], BroadcastState::UNAVAILABLE);
    assert!(empty.clients.is_empty());

    let mapped = mapper.map(
        &[
            SourceClient {
                private_key: 20,
                character: Some("Second".into()),
                server: Some("Xegony".into()),
                class_code: Some("SHM".into()),
                window_number: 6,
                active: false,
                activatable: false,
                input_ready: false,
            },
            SourceClient {
                private_key: 10,
                character: None,
                server: None,
                class_code: None,
                window_number: 2,
                active: true,
                activatable: true,
                input_ready: true,
            },
        ],
        available(false),
    );
    assert_eq!(mapped.clients[0].window_number, 2);
    assert_eq!(mapped.clients[1].window_number, 6);
    assert!(!mapped.clients[1].activatable);
    assert!(!mapped.clients[1].input_ready);
    assert!(mapped.clients[0].input_ready);
    assert!(mapped.capabilities.send_text && mapped.capabilities.send_keys);
    assert_eq!(mapped.clients[1].class_code.as_deref(), Some("SHM"));
}

#[test]
fn mapper_ids_survive_enrichment_but_not_process_lifetimes() {
    let mut mapper = SnapshotMapper::default();
    let initial = mapper.map(
        &[SourceClient {
            private_key: 7,
            character: None,
            server: None,
            class_code: None,
            window_number: 1,
            active: true,
            activatable: true,
            input_ready: false,
        }],
        BroadcastState::UNAVAILABLE,
    );
    let id = initial.clients[0].id.clone();
    let enriched = mapper.map(
        &[SourceClient {
            private_key: 7,
            character: Some("Laika".into()),
            server: Some("Xegony".into()),
            class_code: Some("SHK".into()),
            window_number: 1,
            active: true,
            activatable: true,
            input_ready: false,
        }],
        BroadcastState::UNAVAILABLE,
    );
    assert_eq!(enriched.clients[0].id, id);
    assert_eq!(enriched.clients[0].character.as_deref(), Some("Laika"));

    let changed_identity = mapper.map(
        &[SourceClient {
            private_key: 7,
            character: Some("Orlov".into()),
            server: Some("Teek".into()),
            class_code: Some("CLR".into()),
            window_number: 1,
            active: true,
            activatable: true,
            input_ready: false,
        }],
        BroadcastState::UNAVAILABLE,
    );
    assert_eq!(changed_identity.clients[0].id, id);
    assert_eq!(
        changed_identity.clients[0].character.as_deref(),
        Some("Orlov")
    );

    mapper.map(&[], BroadcastState::UNAVAILABLE);
    let reappeared = mapper.map(
        &[SourceClient {
            private_key: 7,
            character: None,
            server: None,
            class_code: None,
            window_number: 1,
            active: true,
            activatable: true,
            input_ready: false,
        }],
        BroadcastState::UNAVAILABLE,
    );
    assert_ne!(reappeared.clients[0].id, id);
}

#[test]
fn appearance_disappearance_enrichment_and_local_foreground_changes_publish_revisions() {
    let control = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let mut states = control.subscribe();
    let first = control.add_client(1, None, None, None, true, true);
    assert!(states.has_changed().unwrap());
    let appeared = states.borrow_and_update().clone();
    assert_eq!(appeared.revision, 1);

    control.enrich_client(&first, Some("Laika"), Some("Xegony"), Some("SHK"));
    let enriched = states.borrow_and_update().clone();
    assert_eq!(enriched.revision, 2);
    assert_eq!(enriched.clients[0].class_code.as_deref(), Some("SHK"));

    control.enrich_client(&first, Some("Orlov"), Some("Teek"), Some("CLR"));
    let identity_changed = states.borrow_and_update().clone();
    assert_eq!(identity_changed.revision, 3);
    assert_eq!(identity_changed.clients[0].id, first);
    assert_eq!(
        identity_changed.clients[0].character.as_deref(),
        Some("Orlov")
    );

    let second = control.add_client(2, None, None, None, false, true);
    control.set_active_locally(&second);
    let foreground = control.snapshot();
    assert!(
        foreground
            .clients
            .iter()
            .find(|c| c.id == second)
            .unwrap()
            .active
    );

    control.remove_client(&first);
    assert_eq!(control.snapshot().clients.len(), 1);
}

#[test]
fn activates_exact_id_and_reports_already_active() {
    let control = InMemoryController::new(available(false));
    let active = control.add_client(1, Some("One"), Some("A"), None, true, true);
    let target = control.add_client(2, Some("Two"), Some("A"), None, false, true);
    let result = block_on(control.activate(ClientTarget::Id(target.clone()))).unwrap();
    assert_eq!(
        result,
        CommandOutcome::Activated {
            status: ActivationStatus::Activated,
            foreground_confirmed: true,
        }
    );
    assert!(
        !control
            .snapshot()
            .clients
            .iter()
            .find(|c| c.id == active)
            .unwrap()
            .active
    );
    let again = block_on(control.activate(ClientTarget::Id(target))).unwrap();
    assert!(matches!(
        again,
        CommandOutcome::Activated {
            status: ActivationStatus::AlreadyActive,
            ..
        }
    ));
}

#[test]
fn target_errors_cover_missing_ambiguous_disappeared_and_unsupported_clients() {
    let control = InMemoryController::new(available(false));
    let first = control.add_client(1, Some("Same"), Some("A"), None, true, true);
    control.add_client(2, Some("Same"), Some("B"), None, false, true);
    let unsupported = control.add_client(7, Some("Extra"), Some("A"), None, false, false);

    let missing = block_on(control.activate(ClientTarget::WindowNumber(99))).unwrap_err();
    assert_eq!(missing.code, ErrorCode::ClientNotFound);
    let ambiguous = block_on(control.activate(ClientTarget::Identity {
        character: "same".into(),
        server: None,
    }))
    .unwrap_err();
    assert_eq!(ambiguous.code, ErrorCode::AmbiguousTarget);
    let exact = block_on(control.activate(ClientTarget::Identity {
        character: "same".into(),
        server: Some("B".into()),
    }));
    assert!(exact.is_ok());
    let unsupported = block_on(control.activate(ClientTarget::Id(unsupported))).unwrap_err();
    assert_eq!(unsupported.code, ErrorCode::ActivationFailed);

    control.disappear_on_next_activation(first.clone());
    let disappeared = block_on(control.activate(ClientTarget::Id(first))).unwrap_err();
    assert_eq!(disappeared.code, ErrorCode::TargetDisappeared);

    let removed = control.add_client(8, None, None, None, false, true);
    control.remove_client(&removed);
    let stale_id = block_on(control.activate(ClientTarget::Id(removed))).unwrap_err();
    assert_eq!(stale_id.code, ErrorCode::TargetDisappeared);
}

#[test]
fn activation_failure_is_typed_and_does_not_change_active_client() {
    let control = InMemoryController::new(available(false));
    let active = control.add_client(1, None, None, None, true, true);
    let target = control.add_client(2, None, None, None, false, true);
    control.fail_next_activation("foreground request failed");
    let error = block_on(control.activate(ClientTarget::Id(target))).unwrap_err();
    assert_eq!(error.code, ErrorCode::ActivationFailed);
    assert!(
        control
            .snapshot()
            .clients
            .iter()
            .find(|c| c.id == active)
            .unwrap()
            .active
    );
}

#[test]
fn broadcast_unavailable_disabled_enabled_and_failure_are_distinct() {
    let control = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let unavailable = block_on(control.set_broadcast_enabled(true)).unwrap_err();
    assert_eq!(unavailable.code, ErrorCode::BroadcastUnavailable);

    control.set_broadcast_availability(true);
    assert!(!control.snapshot().broadcast.enabled);
    let enabled = block_on(control.set_broadcast_enabled(true)).unwrap();
    assert_eq!(enabled, CommandOutcome::BroadcastSet { enabled: true });
    assert!(control.snapshot().broadcast.enabled);

    control.fail_next_broadcast("hook failed");
    let failed = block_on(control.set_broadcast_enabled(false)).unwrap_err();
    assert_eq!(failed.code, ErrorCode::BroadcastOperationFailed);
    assert!(control.snapshot().broadcast.enabled);
    assert!(block_on(control.set_broadcast_enabled(false)).is_ok());
    assert!(!control.snapshot().broadcast.enabled);
}

#[test]
fn equal_publication_is_deduplicated() {
    let control = InMemoryController::new(available(false));
    control.set_broadcast_locally(false);
    assert_eq!(control.snapshot().revision, 0);
    control.set_broadcast_locally(true);
    assert_eq!(control.snapshot().revision, 1);
}

#[test]
fn targeted_text_and_key_sequences_use_exact_ids_and_record_delivery() {
    let control = InMemoryController::new(available(false));
    let client = control.add_client(1, Some("Laika"), Some("Xegony"), None, true, true);
    let text = block_on(control.send_text(client.clone(), "/who".into(), true)).unwrap();
    assert_eq!(
        text,
        CommandOutcome::InputDelivered {
            kind: InputKind::Text,
            strokes: 5,
        }
    );

    let stroke = KeyStroke::new(
        vec![
            KeyCode::new("left_control").unwrap(),
            KeyCode::new("1").unwrap(),
        ],
        50,
        25,
    )
    .unwrap();
    let keys = block_on(control.send_keys(client.clone(), vec![stroke.clone()])).unwrap();
    assert_eq!(
        keys,
        CommandOutcome::InputDelivered {
            kind: InputKind::Keys,
            strokes: 1,
        }
    );
    assert_eq!(
        control.recorded_inputs(),
        vec![
            RecordedInput::Text {
                client_id: client.clone(),
                text: "/who".into(),
                submit: true,
            },
            RecordedInput::Keys {
                client_id: client,
                strokes: vec![stroke],
            },
        ]
    );
    let capabilities = control.snapshot().capabilities;
    assert!(capabilities.send_text && capabilities.send_keys);
}

#[test]
fn targeted_input_reports_unavailable_invalid_failed_and_disappeared() {
    let unavailable = InMemoryController::new(BroadcastState::UNAVAILABLE);
    let client = unavailable.add_client(1, None, None, None, true, true);
    let error = block_on(unavailable.send_text(client, "/who".into(), true)).unwrap_err();
    assert_eq!(error.code, ErrorCode::InputUnavailable);

    let control = InMemoryController::new(available(false));
    let client = control.add_client(1, None, None, None, true, true);
    control.set_input_ready(&client, false);
    let unready = block_on(control.send_text(client.clone(), "/who".into(), false)).unwrap_err();
    assert_eq!(unready.code, ErrorCode::InputUnavailable);
    let capabilities = control.snapshot().capabilities;
    assert!(!capabilities.send_text && !capabilities.send_keys);

    let invalid =
        block_on(control.send_text(client.clone(), "bad\ntext".into(), false)).unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidArgument);

    control.set_input_ready(&client, true);
    let capabilities = control.snapshot().capabilities;
    assert!(capabilities.send_text && capabilities.send_keys);
    control.fail_next_input("shared memory write failed");
    let failed = block_on(control.send_text(client.clone(), "/who".into(), true)).unwrap_err();
    assert_eq!(failed.code, ErrorCode::InputOperationFailed);

    control.disappear_on_next_input(client.clone());
    let disappeared = block_on(control.send_text(client.clone(), "/who".into(), true)).unwrap_err();
    assert_eq!(disappeared.code, ErrorCode::TargetDisappeared);
    let stale = block_on(control.send_text(client, "/who".into(), true)).unwrap_err();
    assert_eq!(stale.code, ErrorCode::TargetDisappeared);

    assert!(KeyCode::new("launch_missiles").is_err());
    assert!(KeyStroke::new(Vec::new(), 50, 50).is_err());
}

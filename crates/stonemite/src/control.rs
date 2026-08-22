//! Production bridge from the generic control API to the Win32 owner thread.

use futures_util::future::BoxFuture;
use std::cell::UnsafeCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use trushar::control::{
    BroadcastState, ClientId, ClientTarget, CommandOutcome, ControlError, Controller, EqAction,
    EqActionTargets, EqMappingName, ErrorCode, InputKind, KeyStroke, MouseClutchOperation,
    MouseClutchOwner, MouseClutchState, SnapshotMapper, SourceClient, StateHub, StateSnapshot,
    DEFAULT_KEY_HOLD_MS, DEFAULT_KEY_PAUSE_MS, EQ_KEYMAP_PAGE_SIZE,
};
use trushar::server::{ServerConfig, ServerHandle};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_USER};

pub const WM_CONTROL_COMMAND: u32 = WM_USER + 20;
pub const TIMER_CONTROL_INPUT: usize = 3;
const COMMAND_CAPACITY: usize = 32;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const INPUT_TIMEOUT: Duration = Duration::from_secs(45);
const INPUT_TICK_MS: u32 = 10;
const INPUT_ACTIVATION_DELAY: Duration = Duration::from_millis(200);
const INPUT_MODIFIER_DELAY: Duration = Duration::from_millis(30);
const INPUT_RELEASE_DELAY: Duration = Duration::from_millis(100);

enum UiCommand {
    Activate {
        target: ClientTarget,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SwapWindowNumbers {
        target: ClientTarget,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SetBroadcast {
        enabled: bool,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    UpdateMouseClutchHold {
        owner: MouseClutchOwner,
        operation: MouseClutchOperation,
        sequence: u64,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    EndMouseClutchSession {
        session_id: u64,
        sequence: u64,
        reply: oneshot::Sender<Result<(), ControlError>>,
    },
    SendText {
        client_id: ClientId,
        text: String,
        submit: bool,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SendKeys {
        client_id: ClientId,
        strokes: Vec<KeyStroke>,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SendEqAction {
        client_id: ClientId,
        action: EqAction,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    ListEqKeymapActions {
        targets: EqActionTargets,
        after: Option<EqMappingName>,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SendEqActionBatch {
        targets: EqActionTargets,
        action: EqAction,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
}

struct ResolvedStroke {
    scans: Vec<u8>,
    hold: Duration,
    pause: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InputPhase {
    Press,
    PressChord,
    Release,
    ReleaseModifiers,
    Finish,
}

enum ActiveInputKind {
    Input(InputKind),
    EqAction(EqAction),
}

enum InputCompletion {
    Single(oneshot::Sender<Result<CommandOutcome, ControlError>>),
    Batch(u64),
    Local { failure_message: &'static str },
}

struct ActiveInput {
    pid: u32,
    kind: ActiveInputKind,
    strokes: Vec<ResolvedStroke>,
    index: usize,
    phase: InputPhase,
    next_step: Instant,
    completion: InputCompletion,
}

struct PendingBatch {
    action: EqAction,
    window_numbers: Vec<usize>,
    remaining: HashSet<u32>,
    error: Option<ControlError>,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
}

struct UiState {
    receiver: mpsc::Receiver<UiCommand>,
    mapper: SnapshotMapper,
    latest_sources: Vec<SourceClient>,
    hub: Arc<StateHub>,
    tray_hwnd: HWND,
    active_inputs: HashMap<u32, ActiveInput>,
    input_batches: HashMap<u64, PendingBatch>,
    next_batch_id: u64,
    keymaps: HashMap<PathBuf, crate::eq_keymap::EqKeymapResolver>,
    default_eq_dir: PathBuf,
    pairing: Option<trushar::server::PairingHandle>,
    pairing_auth_token: Option<String>,
}

struct UiCell(UnsafeCell<Option<UiState>>);
unsafe impl Sync for UiCell {}
static UI: UiCell = UiCell(UnsafeCell::new(None));

fn ui() -> &'static mut Option<UiState> {
    // Only the tray message-loop thread accesses this cell.
    unsafe { &mut *UI.0.get() }
}

struct ProductionController {
    sender: mpsc::SyncSender<UiCommand>,
    tray_hwnd: usize,
    hub: Arc<StateHub>,
}

impl ProductionController {
    fn enqueue<T: Send + 'static>(
        &self,
        timeout: Duration,
        make_command: impl FnOnce(oneshot::Sender<Result<T, ControlError>>) -> UiCommand,
    ) -> BoxFuture<'static, Result<T, ControlError>> {
        let (reply, receiver) = oneshot::channel();
        if self.sender.try_send(make_command(reply)).is_err() {
            return Box::pin(async {
                Err(ControlError::new(
                    ErrorCode::CommandTimeout,
                    "the Stonemite UI command queue is full or unavailable",
                ))
            });
        }
        let posted = unsafe {
            PostMessageW(
                HWND(self.tray_hwnd as *mut _),
                WM_CONTROL_COMMAND,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if posted.is_err() {
            return Box::pin(async {
                Err(ControlError::new(
                    ErrorCode::InternalError,
                    "failed to notify the Stonemite UI thread",
                ))
            });
        }
        Box::pin(async move {
            match tokio::time::timeout(timeout, receiver).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Err(ControlError::new(
                    ErrorCode::InternalError,
                    "the Stonemite UI command dispatcher stopped",
                )),
                Err(_) => Err(ControlError::new(
                    ErrorCode::CommandTimeout,
                    "the Stonemite UI thread did not answer in time",
                )),
            }
        })
    }
}

impl Controller for ProductionController {
    fn snapshot(&self) -> StateSnapshot {
        self.hub.snapshot()
    }

    fn subscribe(&self) -> tokio::sync::watch::Receiver<StateSnapshot> {
        self.hub.subscribe()
    }

    fn activate(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| UiCommand::Activate {
            target,
            reply,
        })
    }

    fn swap_window_numbers(
        &self,
        target: ClientTarget,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| UiCommand::SwapWindowNumbers {
            target,
            reply,
        })
    }

    fn set_broadcast_enabled(
        &self,
        enabled: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| UiCommand::SetBroadcast {
            enabled,
            reply,
        })
    }

    fn update_mouse_clutch_hold(
        &self,
        owner: MouseClutchOwner,
        operation: MouseClutchOperation,
        sequence: u64,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| {
            UiCommand::UpdateMouseClutchHold {
                owner,
                operation,
                sequence,
                reply,
            }
        })
    }

    fn end_mouse_clutch_session(
        &self,
        session_id: u64,
        sequence: u64,
    ) -> BoxFuture<'static, Result<(), ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| {
            UiCommand::EndMouseClutchSession {
                session_id,
                sequence,
                reply,
            }
        })
    }

    fn send_text(
        &self,
        client_id: ClientId,
        text: String,
        submit: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(INPUT_TIMEOUT, move |reply| UiCommand::SendText {
            client_id,
            text,
            submit,
            reply,
        })
    }

    fn send_keys(
        &self,
        client_id: ClientId,
        strokes: Vec<KeyStroke>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(INPUT_TIMEOUT, move |reply| UiCommand::SendKeys {
            client_id,
            strokes,
            reply,
        })
    }

    fn send_eq_action(
        &self,
        client_id: ClientId,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(INPUT_TIMEOUT, move |reply| UiCommand::SendEqAction {
            client_id,
            action,
            reply,
        })
    }

    fn list_eq_keymap_actions(
        &self,
        targets: EqActionTargets,
        after: Option<EqMappingName>,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| {
            UiCommand::ListEqKeymapActions {
                targets,
                after,
                reply,
            }
        })
    }

    fn send_eq_action_batch(
        &self,
        targets: EqActionTargets,
        action: EqAction,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(INPUT_TIMEOUT, move |reply| UiCommand::SendEqActionBatch {
            targets,
            action,
            reply,
        })
    }
}

/// Initialize the bounded dispatcher and optionally start the dedicated server thread.
/// Called after the tray window exists and before its message loop starts.
pub fn start(
    hwnd: HWND,
    config: &crate::config::TrusharConfig,
    eq_dir: PathBuf,
) -> Option<ServerHandle> {
    let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
    let hub = Arc::new(StateHub::new(Default::default()));
    *ui() = Some(UiState {
        receiver,
        mapper: SnapshotMapper::default(),
        latest_sources: Vec::new(),
        hub: hub.clone(),
        tray_hwnd: hwnd,
        active_inputs: HashMap::new(),
        input_batches: HashMap::new(),
        next_batch_id: 0,
        keymaps: HashMap::new(),
        default_eq_dir: eq_dir,
        pairing: None,
        pairing_auth_token: None,
    });
    crate::overlay::publish_control_snapshot();

    if !config.enabled {
        return None;
    }
    let bind: std::net::SocketAddr = match config.bind.parse() {
        Ok(bind) => bind,
        Err(error) => {
            report_start_failure(&format!("invalid bind address: {error}"));
            return None;
        }
    };
    let pairing_auth_token = if bind.ip().is_loopback() {
        None
    } else {
        config.auth_token.clone()
    };
    let server_config = ServerConfig {
        bind,
        auth_token: config.auth_token.clone(),
    };
    let controller: Arc<dyn Controller> = Arc::new(ProductionController {
        sender,
        tray_hwnd: hwnd.0 as usize,
        hub,
    });
    match ServerHandle::start(server_config, controller) {
        Ok(server) => {
            if let Some(state) = ui().as_mut() {
                state.pairing = Some(server.pairing_handle());
                state.pairing_auth_token = pairing_auth_token;
            }
            Some(server)
        }
        Err(error) => {
            report_start_failure(&error.to_string());
            None
        }
    }
}

pub fn begin_pairing(code: u32) -> bool {
    let Some(state) = ui().as_ref() else {
        return false;
    };
    let (Some(pairing), Some(auth_token)) = (&state.pairing, &state.pairing_auth_token) else {
        return false;
    };
    pairing.begin(code, auth_token.clone())
}

pub fn cancel_pairing() {
    if let Some(pairing) = ui().as_ref().and_then(|state| state.pairing.as_ref()) {
        pairing.cancel();
    }
}

pub fn pairing_is_open() -> bool {
    ui().as_ref()
        .and_then(|state| state.pairing.as_ref())
        .is_some_and(trushar::server::PairingHandle::is_open)
}

fn report_start_failure(detail: &str) {
    let message = format!("trushar disabled: {detail}");
    eprintln!("{message}");
    crate::overlay::debug_log(&message);
    crate::overlay::show_toast("trushar could not start; see debug.log");
}

pub fn stop() {
    if let Some(mut state) = ui().take() {
        let had_active_inputs = !state.active_inputs.is_empty();
        for input in state.active_inputs.drain().map(|(_, input)| input) {
            crate::broadcast::finish_targeted_input(input.pid);
        }
        state.input_batches.clear();
        if had_active_inputs {
            unsafe {
                let _ = KillTimer(state.tray_hwnd, TIMER_CONTROL_INPUT);
            }
        }
    }
}

/// Drain all queued requests on the owner thread. Replies are non-blocking oneshot sends.
pub fn drain_commands() {
    loop {
        let command = {
            let Some(state) = ui().as_mut() else { return };
            match state.receiver.try_recv() {
                Ok(command) => command,
                Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => return,
            }
        };
        match command {
            UiCommand::Activate { target, reply } => {
                let result = activate_on_ui(target);
                let _ = reply.send(result);
            }
            UiCommand::SwapWindowNumbers { target, reply } => {
                let result = swap_window_numbers_on_ui(target);
                let _ = reply.send(result);
            }
            UiCommand::SetBroadcast { enabled, reply } => {
                let result = set_broadcast_on_ui(enabled, true);
                let _ = reply.send(result);
            }
            UiCommand::UpdateMouseClutchHold {
                owner,
                operation,
                sequence,
                reply,
            } => {
                let result = update_mouse_clutch_hold_on_ui(owner, operation, sequence);
                let _ = reply.send(result);
            }
            UiCommand::EndMouseClutchSession {
                session_id,
                sequence,
                reply,
            } => {
                let result = end_mouse_clutch_session_on_ui(session_id, sequence);
                let _ = reply.send(result);
            }
            UiCommand::SendText {
                client_id,
                text,
                submit,
                reply,
            } => start_text_input(client_id, text, submit, reply),
            UiCommand::SendKeys {
                client_id,
                strokes,
                reply,
            } => start_key_input(client_id, strokes, reply),
            UiCommand::SendEqAction {
                client_id,
                action,
                reply,
            } => start_eq_action(client_id, action, reply),
            UiCommand::ListEqKeymapActions {
                targets,
                after,
                reply,
            } => {
                let _ = reply.send(list_eq_keymap_actions_on_ui(targets, after));
            }
            UiCommand::SendEqActionBatch {
                targets,
                action,
                reply,
            } => start_eq_action_batch(targets, action, reply),
        }
    }
}

fn resolve_target_private_key(state: &UiState, target: &ClientTarget) -> Result<u64, ControlError> {
    let matches: Vec<&SourceClient> = state
        .latest_sources
        .iter()
        .filter(|source| match target {
            ClientTarget::Id(id) => state.mapper.id_for_key(source.private_key) == Some(id),
            ClientTarget::WindowNumber(number) => source.window_number == *number,
            ClientTarget::Identity { character, server } => {
                source
                    .character
                    .as_deref()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(character))
                    && server.as_ref().is_none_or(|expected| {
                        source
                            .server
                            .as_deref()
                            .is_some_and(|actual| actual.eq_ignore_ascii_case(expected))
                    })
            }
        })
        .collect();
    if matches.is_empty() {
        if matches!(target, ClientTarget::Id(id) if state.mapper.is_retired(id)) {
            return Err(ControlError::new(
                ErrorCode::TargetDisappeared,
                "the target is no longer loaded",
            ));
        }
        return Err(ControlError::new(
            ErrorCode::ClientNotFound,
            "no loaded client matches the target",
        ));
    }
    if matches.len() > 1 {
        return Err(ControlError::new(
            ErrorCode::AmbiguousTarget,
            "more than one loaded client matches the target",
        ));
    }
    Ok(matches[0].private_key)
}

fn activate_on_ui(target: ClientTarget) -> Result<CommandOutcome, ControlError> {
    let Some(state) = ui().as_ref() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    let private_key = resolve_target_private_key(state, &target)?;
    unsafe { crate::overlay::activate_pid(private_key as u32) }
}

fn swap_window_numbers_on_ui(target: ClientTarget) -> Result<CommandOutcome, ControlError> {
    let Some(state) = ui().as_ref() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    let private_key = resolve_target_private_key(state, &target)?;
    unsafe { crate::overlay::swap_active_window_numbers(private_key as u32) }
}

pub fn set_broadcast_on_ui(
    enabled: bool,
    show_notification: bool,
) -> Result<CommandOutcome, ControlError> {
    if !crate::broadcast::is_available() {
        return Err(ControlError::new(
            ErrorCode::BroadcastUnavailable,
            "broadcasting is unavailable because trusik is disabled",
        ));
    }
    crate::broadcast::set_active(enabled)
        .map_err(|message| ControlError::new(ErrorCode::BroadcastOperationFailed, message))?;
    crate::overlay::refresh_broadcast_label();
    if show_notification {
        crate::overlay::show_toast(if enabled {
            "Key broadcasting enabled"
        } else {
            "Key broadcasting disabled"
        });
    }
    crate::overlay::publish_control_snapshot();
    Ok(CommandOutcome::BroadcastSet { enabled })
}

pub fn toggle_broadcast_on_ui(show_notification: bool) {
    let _ = set_broadcast_on_ui(!crate::broadcast::is_active(), show_notification);
}

fn update_mouse_clutch_hold_on_ui(
    owner: MouseClutchOwner,
    operation: MouseClutchOperation,
    sequence: u64,
) -> Result<CommandOutcome, ControlError> {
    let held = crate::broadcast::update_remote_mouse_clutch_hold(owner, operation, sequence)
        .map_err(map_mouse_clutch_error)?;
    crate::overlay::refresh_broadcast_label();
    crate::overlay::publish_control_snapshot();
    Ok(CommandOutcome::MouseClutchHoldUpdated { held })
}

fn end_mouse_clutch_session_on_ui(session_id: u64, sequence: u64) -> Result<(), ControlError> {
    crate::broadcast::end_remote_mouse_clutch_session(session_id, sequence);
    crate::overlay::refresh_broadcast_label();
    crate::overlay::publish_control_snapshot();
    Ok(())
}

fn map_mouse_clutch_error(error: crate::broadcast::MouseClutchControlError) -> ControlError {
    let (code, message) = match error {
        crate::broadcast::MouseClutchControlError::Unavailable(message) => {
            (ErrorCode::MouseClutchUnavailable, message)
        }
        crate::broadcast::MouseClutchControlError::NotReady(message) => {
            (ErrorCode::MouseClutchNotReady, message)
        }
        crate::broadcast::MouseClutchControlError::HoldExpired(message) => {
            (ErrorCode::MouseClutchHoldExpired, message)
        }
        crate::broadcast::MouseClutchControlError::OperationFailed(message) => {
            (ErrorCode::MouseClutchOperationFailed, message)
        }
    };
    ControlError::new(code, message)
}

fn resolve_local_eq_action(pid: u32, action: &EqAction) -> Result<ResolvedStroke, ControlError> {
    action.validate()?;
    if !crate::broadcast::is_available() {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable because trusik is disabled",
        ));
    }
    if !crate::broadcast::is_target_ready(pid) {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable for this EQ client",
        ));
    }
    let Some(state) = ui().as_mut() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    if state.active_inputs.contains_key(&pid) {
        return Err(ControlError::new(
            ErrorCode::InputOperationFailed,
            "the selected target already has an input sequence in progress",
        ));
    }
    let source = state
        .latest_sources
        .iter()
        .find(|source| source.private_key as u32 == pid)
        .cloned()
        .ok_or_else(|| ControlError::new(ErrorCode::ClientNotFound, "the target is not loaded"))?;
    let binding = resolve_action_for_source(state, &source, action)?;
    Ok(ResolvedStroke {
        scans: binding.scans,
        hold: Duration::from_millis(u64::from(DEFAULT_KEY_HOLD_MS)),
        pause: Duration::ZERO,
    })
}

/// Return whether the exact EQ client can currently receive its configured
/// Invite/Follow action. This is intentionally an owner-thread-only query.
pub fn invite_follow_available(pid: u32) -> bool {
    resolve_local_eq_action(pid, &EqAction::InviteFollow).is_ok()
}

/// Start a user-confirmed Invite/Follow action for one exact EQ process without
/// activating it. Completion failures are surfaced as a toast on the UI thread.
pub fn send_invite_follow(pid: u32) -> Result<(), ControlError> {
    let action = EqAction::InviteFollow;
    let stroke = resolve_local_eq_action(pid, &action)?;
    crate::broadcast::begin_targeted_input(pid).map_err(map_targeted_input_error)?;

    let Some(state) = ui().as_mut() else {
        crate::broadcast::finish_targeted_input(pid);
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    if state.active_inputs.is_empty() {
        let timer = unsafe { SetTimer(state.tray_hwnd, TIMER_CONTROL_INPUT, INPUT_TICK_MS, None) };
        if timer == 0 {
            crate::broadcast::finish_targeted_input(pid);
            return Err(ControlError::new(
                ErrorCode::InputOperationFailed,
                "failed to start the targeted input timer",
            ));
        }
    }
    state.active_inputs.insert(
        pid,
        ActiveInput {
            pid,
            kind: ActiveInputKind::EqAction(action),
            strokes: vec![stroke],
            index: 0,
            phase: InputPhase::Press,
            next_step: Instant::now() + INPUT_ACTIVATION_DELAY,
            completion: InputCompletion::Local {
                failure_message: "Could not accept the group invitation",
            },
        },
    );
    Ok(())
}

fn start_text_input(
    client_id: ClientId,
    text: String,
    submit: bool,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
) {
    let resolved = trushar::control::validate_text_input(&text)
        .and_then(|()| resolve_text_strokes(&text, submit));
    match resolved {
        Ok(strokes) => {
            if let Err((reply, error)) = start_resolved_input(
                client_id,
                ActiveInputKind::Input(InputKind::Text),
                strokes,
                reply,
            ) {
                let _ = reply.send(Err(error));
            }
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
}

fn start_key_input(
    client_id: ClientId,
    strokes: Vec<KeyStroke>,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
) {
    let resolved = trushar::control::validate_key_strokes(&strokes)
        .and_then(|()| resolve_key_strokes(&strokes));
    match resolved {
        Ok(strokes) => {
            if let Err((reply, error)) = start_resolved_input(
                client_id,
                ActiveInputKind::Input(InputKind::Keys),
                strokes,
                reply,
            ) {
                let _ = reply.send(Err(error));
            }
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
}

fn start_eq_action(
    client_id: ClientId,
    action: EqAction,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
) {
    let resolved = action.validate().and_then(|()| {
        let source = resolve_input_source(&client_id)?;
        let Some(state) = ui().as_mut() else {
            return Err(ControlError::new(
                ErrorCode::InternalError,
                "control dispatcher is stopped",
            ));
        };
        let eq_dir = crate::eq_windows::process_eq_directory(source.private_key as u32)
            .unwrap_or_else(|| state.default_eq_dir.clone());
        let binding = state
            .keymaps
            .entry(eq_dir.clone())
            .or_insert_with(|| crate::eq_keymap::EqKeymapResolver::new(eq_dir))
            .resolve(
                &action,
                crate::eq_keymap::ClientIdentity {
                    character: source.character.as_deref(),
                    server: source.server.as_deref(),
                    class_code: source.class_code.as_deref(),
                },
            )
            .map_err(map_eq_action_error)?;
        Ok(vec![ResolvedStroke {
            scans: binding.scans,
            hold: Duration::from_millis(u64::from(DEFAULT_KEY_HOLD_MS)),
            pause: Duration::ZERO,
        }])
    });

    match resolved {
        Ok(strokes) => {
            if let Err((reply, error)) =
                start_resolved_input(client_id, ActiveInputKind::EqAction(action), strokes, reply)
            {
                let _ = reply.send(Err(error));
            }
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
}

fn list_eq_keymap_actions_on_ui(
    targets: EqActionTargets,
    after: Option<EqMappingName>,
) -> Result<CommandOutcome, ControlError> {
    targets.validate()?;
    let Some(state) = ui().as_mut() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    let sources = sources_for_targets(state, &targets, false)?;
    let mut window_numbers = sources
        .iter()
        .map(|source| source.window_number)
        .collect::<Vec<_>>();
    window_numbers.sort_unstable();

    let mut shared: Option<BTreeSet<EqMappingName>> = None;
    for source in &sources {
        let mapped = mapped_actions_for_source(state, source)?;
        shared = Some(match shared {
            Some(mut shared) => {
                shared.retain(|mapping| mapped.contains(mapping));
                shared
            }
            None => mapped,
        });
    }
    let mut mappings = shared.unwrap_or_default().into_iter().collect::<Vec<_>>();
    if let Some(after) = &after {
        mappings.retain(|mapping| mapping > after);
    }
    let next_after =
        (mappings.len() > EQ_KEYMAP_PAGE_SIZE).then(|| mappings[EQ_KEYMAP_PAGE_SIZE - 1].clone());
    mappings.truncate(EQ_KEYMAP_PAGE_SIZE);
    Ok(CommandOutcome::EqKeymapActionsListed {
        mappings,
        window_numbers,
        next_after,
    })
}

struct PreparedBatchInput {
    pid: u32,
    window_number: usize,
    stroke: ResolvedStroke,
}

fn start_eq_action_batch(
    targets: EqActionTargets,
    action: EqAction,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
) {
    if reply.is_closed() {
        return;
    }
    let result = prepare_eq_action_batch(&targets, &action);
    let prepared = match result {
        Ok(prepared) => prepared,
        Err(error) => {
            let _ = reply.send(Err(error));
            return;
        }
    };
    let Some(state) = ui().as_mut() else {
        let _ = reply.send(Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        )));
        return;
    };

    let mut acquired = Vec::with_capacity(prepared.len());
    for input in &prepared {
        if let Err(error) = crate::broadcast::begin_targeted_input(input.pid) {
            for pid in acquired {
                crate::broadcast::finish_targeted_input(pid);
            }
            let _ = reply.send(Err(map_targeted_input_error(error)));
            return;
        }
        acquired.push(input.pid);
    }
    if state.active_inputs.is_empty() {
        let timer = unsafe { SetTimer(state.tray_hwnd, TIMER_CONTROL_INPUT, INPUT_TICK_MS, None) };
        if timer == 0 {
            for pid in acquired {
                crate::broadcast::finish_targeted_input(pid);
            }
            let _ = reply.send(Err(ControlError::new(
                ErrorCode::InputOperationFailed,
                "failed to start the targeted input timer",
            )));
            return;
        }
    }

    state.next_batch_id = state.next_batch_id.saturating_add(1);
    let batch_id = state.next_batch_id;
    let remaining = prepared.iter().map(|input| input.pid).collect();
    let mut window_numbers = prepared
        .iter()
        .map(|input| input.window_number)
        .collect::<Vec<_>>();
    window_numbers.sort_unstable();
    state.input_batches.insert(
        batch_id,
        PendingBatch {
            action: action.clone(),
            window_numbers,
            remaining,
            error: None,
            reply,
        },
    );
    let next_step = Instant::now() + INPUT_ACTIVATION_DELAY;
    for input in prepared {
        state.active_inputs.insert(
            input.pid,
            ActiveInput {
                pid: input.pid,
                kind: ActiveInputKind::EqAction(action.clone()),
                strokes: vec![input.stroke],
                index: 0,
                phase: InputPhase::Press,
                next_step,
                completion: InputCompletion::Batch(batch_id),
            },
        );
    }
}

fn prepare_eq_action_batch(
    targets: &EqActionTargets,
    action: &EqAction,
) -> Result<Vec<PreparedBatchInput>, ControlError> {
    targets.validate()?;
    action.validate()?;
    if !crate::broadcast::is_available() {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable because trusik is disabled",
        ));
    }
    let Some(state) = ui().as_mut() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    let sources = sources_for_targets(state, targets, true)?;
    let mut prepared = Vec::with_capacity(sources.len());
    for source in sources {
        let pid = source.private_key as u32;
        if !crate::broadcast::is_target_ready(pid) {
            return Err(ControlError::new(
                ErrorCode::InputUnavailable,
                format!(
                    "targeted input is unavailable because Stonemite box {} is not ready",
                    source.window_number
                ),
            ));
        }
        if state.active_inputs.contains_key(&pid) {
            return Err(ControlError::new(
                ErrorCode::InputOperationFailed,
                format!(
                    "Stonemite box {} already has an input sequence in progress",
                    source.window_number
                ),
            ));
        }
        let binding = resolve_action_for_source(state, &source, action).map_err(|error| {
            if error.code == ErrorCode::EqActionUnbound {
                ControlError::new(
                    ErrorCode::EqActionUnbound,
                    format!(
                        "the selected EQ action is not mapped for Stonemite box {}",
                        source.window_number
                    ),
                )
            } else {
                error
            }
        })?;
        prepared.push(PreparedBatchInput {
            pid,
            window_number: source.window_number,
            stroke: ResolvedStroke {
                scans: binding.scans,
                hold: Duration::from_millis(u64::from(DEFAULT_KEY_HOLD_MS)),
                pause: Duration::ZERO,
            },
        });
    }
    Ok(prepared)
}

fn sources_for_targets(
    state: &UiState,
    targets: &EqActionTargets,
    require_all: bool,
) -> Result<Vec<SourceClient>, ControlError> {
    select_sources_for_targets(&state.latest_sources, targets, require_all)
}

fn select_sources_for_targets(
    latest_sources: &[SourceClient],
    targets: &EqActionTargets,
    require_all: bool,
) -> Result<Vec<SourceClient>, ControlError> {
    let active_key = matches!(
        targets,
        EqActionTargets::Active | EqActionTargets::BackgroundLoaded
    )
    .then(|| {
        latest_sources
            .iter()
            .find(|source| source.active)
            .map(|source| source.private_key)
            .ok_or_else(|| {
                ControlError::new(
                    ErrorCode::ClientNotFound,
                    "no active client is available for the dynamic EQ action target",
                )
            })
    })
    .transpose()?;
    let mut sources = latest_sources
        .iter()
        .filter(|source| match targets {
            EqActionTargets::AllLoaded => true,
            EqActionTargets::Active => active_key == Some(source.private_key),
            EqActionTargets::BackgroundLoaded => active_key != Some(source.private_key),
            EqActionTargets::WindowNumbers(numbers) => numbers.contains(&source.window_number),
        })
        .cloned()
        .collect::<Vec<_>>();
    sources.sort_by_key(|source| source.window_number);
    if !require_all {
        return Ok(sources);
    }
    match targets {
        EqActionTargets::AllLoaded if sources.is_empty() => Err(ControlError::new(
            ErrorCode::ClientNotFound,
            "no loaded clients match the all-boxes target",
        )),
        EqActionTargets::BackgroundLoaded if sources.is_empty() => Err(ControlError::new(
            ErrorCode::ClientNotFound,
            "no loaded clients match the background-boxes target",
        )),
        EqActionTargets::WindowNumbers(numbers) if sources.len() != numbers.len() => {
            let missing = numbers
                .iter()
                .find(|number| {
                    !sources
                        .iter()
                        .any(|source| source.window_number == **number)
                })
                .copied()
                .unwrap_or_default();
            Err(ControlError::new(
                ErrorCode::ClientNotFound,
                format!("Stonemite box {missing} is not loaded"),
            ))
        }
        _ => Ok(sources),
    }
}

fn mapped_actions_for_source(
    state: &mut UiState,
    source: &SourceClient,
) -> Result<BTreeSet<EqMappingName>, ControlError> {
    let eq_dir = crate::eq_windows::process_eq_directory(source.private_key as u32)
        .unwrap_or_else(|| state.default_eq_dir.clone());
    state
        .keymaps
        .entry(eq_dir.clone())
        .or_insert_with(|| crate::eq_keymap::EqKeymapResolver::new(eq_dir))
        .mapped_actions(source_identity(source))
        .map_err(map_eq_action_error)
}

fn resolve_action_for_source(
    state: &mut UiState,
    source: &SourceClient,
    action: &EqAction,
) -> Result<crate::eq_keymap::ResolvedBinding, ControlError> {
    let eq_dir = crate::eq_windows::process_eq_directory(source.private_key as u32)
        .unwrap_or_else(|| state.default_eq_dir.clone());
    state
        .keymaps
        .entry(eq_dir.clone())
        .or_insert_with(|| crate::eq_keymap::EqKeymapResolver::new(eq_dir))
        .resolve(action, source_identity(source))
        .map_err(map_eq_action_error)
}

fn source_identity(source: &SourceClient) -> crate::eq_keymap::ClientIdentity<'_> {
    crate::eq_keymap::ClientIdentity {
        character: source.character.as_deref(),
        server: source.server.as_deref(),
        class_code: source.class_code.as_deref(),
    }
}

fn map_eq_action_error(error: crate::eq_keymap::ResolveError) -> ControlError {
    match error {
        crate::eq_keymap::ResolveError::Unbound => ControlError::new(
            ErrorCode::EqActionUnbound,
            "the selected EQ action has no primary or alternate key binding",
        ),
        crate::eq_keymap::ResolveError::Read(message)
        | crate::eq_keymap::ResolveError::Malformed(message) => {
            ControlError::new(ErrorCode::InputOperationFailed, message)
        }
    }
}

type StartInputError = (
    oneshot::Sender<Result<CommandOutcome, ControlError>>,
    ControlError,
);

fn start_resolved_input(
    client_id: ClientId,
    kind: ActiveInputKind,
    strokes: Vec<ResolvedStroke>,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
) -> Result<(), StartInputError> {
    if reply.is_closed() {
        return Ok(());
    }
    let pid = match resolve_input_pid(&client_id) {
        Ok(pid) => pid,
        Err(error) => return Err((reply, error)),
    };
    let Some(state) = ui().as_mut() else {
        return Err((
            reply,
            ControlError::new(ErrorCode::InternalError, "control dispatcher is stopped"),
        ));
    };
    if state.active_inputs.contains_key(&pid) {
        return Err((
            reply,
            ControlError::new(
                ErrorCode::InputOperationFailed,
                "the selected target already has an input sequence in progress",
            ),
        ));
    }
    if let Err(error) = crate::broadcast::begin_targeted_input(pid) {
        return Err((reply, map_targeted_input_error(error)));
    }
    if state.active_inputs.is_empty() {
        let timer = unsafe { SetTimer(state.tray_hwnd, TIMER_CONTROL_INPUT, INPUT_TICK_MS, None) };
        if timer == 0 {
            crate::broadcast::finish_targeted_input(pid);
            return Err((
                reply,
                ControlError::new(
                    ErrorCode::InputOperationFailed,
                    "failed to start the targeted input timer",
                ),
            ));
        }
    }
    state.active_inputs.insert(
        pid,
        ActiveInput {
            pid,
            kind,
            strokes,
            index: 0,
            phase: InputPhase::Press,
            // trusik observes the active flag and gives a background EQ window an
            // activation notification before DirectInput consumes the first key.
            next_step: Instant::now() + INPUT_ACTIVATION_DELAY,
            completion: InputCompletion::Single(reply),
        },
    );
    Ok(())
}

fn map_targeted_input_error(error: crate::broadcast::TargetedInputError) -> ControlError {
    let (code, message) = match error {
        crate::broadcast::TargetedInputError::Unavailable(message) => {
            (ErrorCode::InputUnavailable, message)
        }
        crate::broadcast::TargetedInputError::OperationFailed(message) => {
            (ErrorCode::InputOperationFailed, message)
        }
    };
    ControlError::new(code, message)
}

fn resolve_input_source(client_id: &ClientId) -> Result<SourceClient, ControlError> {
    if !crate::broadcast::is_available() {
        return Err(ControlError::new(
            ErrorCode::InputUnavailable,
            "targeted input is unavailable because trusik is disabled",
        ));
    }
    let Some(state) = ui().as_ref() else {
        return Err(ControlError::new(
            ErrorCode::InternalError,
            "control dispatcher is stopped",
        ));
    };
    if let Some(source) = state
        .latest_sources
        .iter()
        .find(|source| state.mapper.id_for_key(source.private_key) == Some(client_id))
    {
        let source = source.clone();
        if !crate::broadcast::is_target_ready(source.private_key as u32) {
            return Err(ControlError::new(
                ErrorCode::InputUnavailable,
                "targeted input is unavailable because the selected client's trusik proxy is not ready",
            ));
        }
        return Ok(source);
    }
    let code = if state.mapper.is_retired(client_id) {
        ErrorCode::TargetDisappeared
    } else {
        ErrorCode::ClientNotFound
    };
    Err(ControlError::new(code, "the target client is not loaded"))
}

fn resolve_input_pid(client_id: &ClientId) -> Result<u32, ControlError> {
    Ok(resolve_input_source(client_id)?.private_key as u32)
}

fn resolve_text_strokes(text: &str, submit: bool) -> Result<Vec<ResolvedStroke>, ControlError> {
    let mut strokes = Vec::with_capacity(text.chars().count() + usize::from(submit));
    for character in text.chars() {
        let Some(character) = u16::try_from(character as u32).ok() else {
            return Err(input_resolution_error(
                "text contains a character unsupported by the active keyboard layout",
            ));
        };
        let mapping = unsafe { VkKeyScanW(character) };
        if mapping == -1i16 {
            return Err(input_resolution_error(
                "text contains a character unsupported by the active keyboard layout",
            ));
        }
        let vk = (mapping & 0xff) as u32;
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) as u8 };
        if scan == 0 || scan == 255 {
            return Err(input_resolution_error(
                "text contains a character without a DirectInput scan code",
            ));
        }
        let modifiers = ((mapping >> 8) & 0xff) as u8;
        let mut scans = Vec::with_capacity(4);
        if modifiers & 0x02 != 0 {
            scans.push(0x1d); // left control
        }
        if modifiers & 0x01 != 0 {
            scans.push(0x2a); // left shift
        }
        if modifiers & 0x04 != 0 {
            scans.push(0x38); // left alt
        }
        if !scans.contains(&scan) {
            scans.push(scan);
        }
        strokes.push(ResolvedStroke {
            scans,
            hold: Duration::from_millis(u64::from(DEFAULT_KEY_HOLD_MS)),
            pause: Duration::from_millis(u64::from(DEFAULT_KEY_PAUSE_MS)),
        });
    }
    if submit {
        strokes.push(ResolvedStroke {
            scans: vec![0x1c], // enter
            hold: Duration::from_millis(u64::from(DEFAULT_KEY_HOLD_MS)),
            pause: Duration::ZERO,
        });
    }
    Ok(strokes)
}

fn resolve_key_strokes(strokes: &[KeyStroke]) -> Result<Vec<ResolvedStroke>, ControlError> {
    strokes
        .iter()
        .map(|stroke| {
            let scans = stroke
                .keys
                .iter()
                .map(|key| semantic_scan_code(key.as_str()))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| input_resolution_error("key has no DirectInput scan code"))?;
            Ok(ResolvedStroke {
                scans,
                hold: Duration::from_millis(u64::from(stroke.hold_ms)),
                pause: Duration::from_millis(u64::from(stroke.pause_ms)),
            })
        })
        .collect()
}

fn input_resolution_error(message: &'static str) -> ControlError {
    ControlError::new(ErrorCode::InvalidArgument, message)
}

fn semantic_scan_code(key: &str) -> Option<u8> {
    Some(match key {
        "escape" => 0x01,
        "1" => 0x02,
        "2" => 0x03,
        "3" => 0x04,
        "4" => 0x05,
        "5" => 0x06,
        "6" => 0x07,
        "7" => 0x08,
        "8" => 0x09,
        "9" => 0x0a,
        "0" => 0x0b,
        "minus" => 0x0c,
        "equals" => 0x0d,
        "backspace" => 0x0e,
        "tab" => 0x0f,
        "q" => 0x10,
        "w" => 0x11,
        "e" => 0x12,
        "r" => 0x13,
        "t" => 0x14,
        "y" => 0x15,
        "u" => 0x16,
        "i" => 0x17,
        "o" => 0x18,
        "p" => 0x19,
        "left_bracket" => 0x1a,
        "right_bracket" => 0x1b,
        "enter" => 0x1c,
        "left_control" => 0x1d,
        "a" => 0x1e,
        "s" => 0x1f,
        "d" => 0x20,
        "f" => 0x21,
        "g" => 0x22,
        "h" => 0x23,
        "j" => 0x24,
        "k" => 0x25,
        "l" => 0x26,
        "semicolon" => 0x27,
        "apostrophe" => 0x28,
        "grave" => 0x29,
        "left_shift" => 0x2a,
        "backslash" => 0x2b,
        "z" => 0x2c,
        "x" => 0x2d,
        "c" => 0x2e,
        "v" => 0x2f,
        "b" => 0x30,
        "n" => 0x31,
        "m" => 0x32,
        "comma" => 0x33,
        "period" => 0x34,
        "slash" => 0x35,
        "right_shift" => 0x36,
        "numpad_multiply" => 0x37,
        "left_alt" => 0x38,
        "space" => 0x39,
        "caps_lock" => 0x3a,
        "f1" => 0x3b,
        "f2" => 0x3c,
        "f3" => 0x3d,
        "f4" => 0x3e,
        "f5" => 0x3f,
        "f6" => 0x40,
        "f7" => 0x41,
        "f8" => 0x42,
        "f9" => 0x43,
        "f10" => 0x44,
        "num_lock" => 0x45,
        "scroll_lock" => 0x46,
        "numpad_7" => 0x47,
        "numpad_8" => 0x48,
        "numpad_9" => 0x49,
        "numpad_subtract" => 0x4a,
        "numpad_4" => 0x4b,
        "numpad_5" => 0x4c,
        "numpad_6" => 0x4d,
        "numpad_add" => 0x4e,
        "numpad_1" => 0x4f,
        "numpad_2" => 0x50,
        "numpad_3" => 0x51,
        "numpad_0" => 0x52,
        "numpad_decimal" => 0x53,
        "f11" => 0x57,
        "f12" => 0x58,
        "numpad_enter" => 0x9c,
        "right_control" => 0x9d,
        "numpad_divide" => 0xb5,
        "right_alt" => 0xb8,
        "pause" => 0xc5,
        "home" => 0xc7,
        "arrow_up" => 0xc8,
        "page_up" => 0xc9,
        "arrow_left" => 0xcb,
        "arrow_right" => 0xcd,
        "end" => 0xcf,
        "arrow_down" => 0xd0,
        "page_down" => 0xd1,
        "insert" => 0xd2,
        "delete" => 0xd3,
        _ => return None,
    })
}

fn is_modifier_scan(scan: u8) -> bool {
    matches!(scan, 0x1d | 0x2a | 0x36 | 0x38 | 0x9d | 0xb8)
}

fn has_modifier_chord(stroke: &ResolvedStroke) -> bool {
    stroke.scans.iter().copied().any(is_modifier_scan)
        && stroke
            .scans
            .iter()
            .copied()
            .any(|scan| !is_modifier_scan(scan))
}

fn phase_scans(stroke: &ResolvedStroke, phase: InputPhase) -> Vec<u8> {
    let modifier_chord = has_modifier_chord(stroke);
    match phase {
        InputPhase::Press if modifier_chord => stroke
            .scans
            .iter()
            .copied()
            .filter(|scan| is_modifier_scan(*scan))
            .collect(),
        InputPhase::Press => stroke.scans.clone(),
        InputPhase::PressChord => stroke
            .scans
            .iter()
            .copied()
            .filter(|scan| !is_modifier_scan(*scan))
            .collect(),
        InputPhase::Release if modifier_chord => stroke
            .scans
            .iter()
            .rev()
            .copied()
            .filter(|scan| !is_modifier_scan(*scan))
            .collect(),
        InputPhase::Release => stroke.scans.iter().rev().copied().collect(),
        InputPhase::ReleaseModifiers => stroke
            .scans
            .iter()
            .rev()
            .copied()
            .filter(|scan| is_modifier_scan(*scan))
            .collect(),
        InputPhase::Finish => Vec::new(),
    }
}

fn complete_stroke(input: &mut ActiveInput, now: Instant) {
    let pause = input.strokes[input.index].pause;
    if input.index + 1 == input.strokes.len() {
        // Keep SHM active briefly with an all-up state so EQ can observe the
        // final release before the mapping is deactivated.
        input.phase = InputPhase::Finish;
        input.next_step = now + INPUT_RELEASE_DELAY;
    } else {
        input.index += 1;
        input.phase = InputPhase::Press;
        input.next_step = now + pause;
    }
}

/// Advance every active target-specific sequence from the Win32 timer callback.
pub fn advance_input() {
    let Some(state) = ui().as_mut() else { return };
    if state.active_inputs.is_empty() {
        unsafe {
            let _ = KillTimer(state.tray_hwnd, TIMER_CONTROL_INPUT);
        }
        return;
    }

    let inputs = std::mem::take(&mut state.active_inputs);
    for input in inputs.into_values() {
        advance_one_input(state, input);
    }
    if state.active_inputs.is_empty() {
        unsafe {
            let _ = KillTimer(state.tray_hwnd, TIMER_CONTROL_INPUT);
        }
    }
}

fn advance_one_input(state: &mut UiState, mut input: ActiveInput) {
    if input_completion_is_closed(state, &input.completion) {
        finish_input(state, input, None);
        return;
    }
    if !state
        .latest_sources
        .iter()
        .any(|source| source.private_key as u32 == input.pid)
    {
        finish_input(
            state,
            input,
            Some(Err(ControlError::new(
                ErrorCode::TargetDisappeared,
                "the target disappeared during input delivery",
            ))),
        );
        return;
    }
    let now = Instant::now();
    if now < input.next_step {
        state.active_inputs.insert(input.pid, input);
        return;
    }
    let phase = input.phase;
    if phase == InputPhase::Finish {
        let result = Ok(match &input.kind {
            ActiveInputKind::Input(kind) => CommandOutcome::InputDelivered {
                kind: *kind,
                strokes: input.strokes.len(),
            },
            ActiveInputKind::EqAction(action) => CommandOutcome::EqActionDelivered {
                action: action.clone(),
            },
        });
        finish_input(state, input, Some(result));
        return;
    }

    let modifier_chord = has_modifier_chord(&input.strokes[input.index]);
    let hold = input.strokes[input.index].hold;
    let pressed = matches!(phase, InputPhase::Press | InputPhase::PressChord);
    for scan in phase_scans(&input.strokes[input.index], phase) {
        if let Err(message) = crate::broadcast::set_targeted_key(input.pid, scan, pressed) {
            finish_input(
                state,
                input,
                Some(Err(ControlError::new(
                    ErrorCode::InputOperationFailed,
                    message,
                ))),
            );
            return;
        }
    }

    match phase {
        InputPhase::Press if modifier_chord => {
            // Give EQ at least one input frame to observe the modifier before
            // exposing the primary key. Simultaneous shared-memory changes can
            // otherwise be consumed in scan-code order (for Ctrl+I, I first).
            input.phase = InputPhase::PressChord;
            input.next_step = now + INPUT_MODIFIER_DELAY;
        }
        InputPhase::Press | InputPhase::PressChord => {
            input.phase = InputPhase::Release;
            input.next_step = now + hold;
        }
        InputPhase::Release if modifier_chord => {
            // Release the primary key while the modifier is still held, then
            // give EQ another frame before releasing the modifier.
            input.phase = InputPhase::ReleaseModifiers;
            input.next_step = now + INPUT_MODIFIER_DELAY;
        }
        InputPhase::Release | InputPhase::ReleaseModifiers => complete_stroke(&mut input, now),
        InputPhase::Finish => unreachable!(),
    }
    state.active_inputs.insert(input.pid, input);
}

fn input_completion_is_closed(state: &UiState, completion: &InputCompletion) -> bool {
    match completion {
        InputCompletion::Single(reply) => reply.is_closed(),
        InputCompletion::Batch(batch_id) => state
            .input_batches
            .get(batch_id)
            .is_none_or(|batch| batch.reply.is_closed()),
        InputCompletion::Local { .. } => false,
    }
}

fn finish_input(
    state: &mut UiState,
    input: ActiveInput,
    result: Option<Result<CommandOutcome, ControlError>>,
) {
    crate::broadcast::finish_targeted_input(input.pid);
    match input.completion {
        InputCompletion::Single(reply) => {
            if let Some(result) = result {
                let _ = reply.send(result);
            }
        }
        InputCompletion::Batch(batch_id) => {
            let complete = if let Some(batch) = state.input_batches.get_mut(&batch_id) {
                batch.remaining.remove(&input.pid);
                if let Some(Err(mut error)) = result {
                    error.message = format!(
                        "{}; one or more targets may have received the action",
                        error.message
                    );
                    batch.error.get_or_insert(error);
                }
                batch.remaining.is_empty()
            } else {
                false
            };
            if complete {
                let batch = state
                    .input_batches
                    .remove(&batch_id)
                    .expect("completed input batch disappeared");
                if !batch.reply.is_closed() {
                    let result = match batch.error {
                        Some(error) => Err(error),
                        None => Ok(CommandOutcome::EqActionBatchDelivered {
                            action: batch.action,
                            window_numbers: batch.window_numbers,
                        }),
                    };
                    let _ = batch.reply.send(result);
                }
            }
        }
        InputCompletion::Local { failure_message } => {
            if let Some(Err(error)) = result {
                crate::overlay::show_toast(&format!("{failure_message}: {}", error.message));
            }
        }
    }
}

/// Publish owner-thread state after any known local or remote change.
pub fn publish(
    sources: Vec<SourceClient>,
    broadcast: BroadcastState,
    mouse_clutch: MouseClutchState,
) {
    let Some(state) = ui().as_mut() else { return };
    let mut data = state.mapper.map(&sources, broadcast);
    data.mouse_clutch = mouse_clutch;
    data.capabilities.set_mouse_clutch = crate::broadcast::is_available();
    state.latest_sources = sources;
    state.hub.publish(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_keys_map_to_directinput_codes() {
        assert_eq!(semantic_scan_code("a"), Some(0x1e));
        assert_eq!(semantic_scan_code("enter"), Some(0x1c));
        assert_eq!(semantic_scan_code("arrow_up"), Some(0xc8));
        assert_eq!(semantic_scan_code("right_control"), Some(0x9d));
        assert_eq!(semantic_scan_code("unknown"), None);
    }

    #[test]
    fn text_is_fully_resolved_before_delivery() {
        let strokes = resolve_text_strokes("/who", true).unwrap();
        assert_eq!(strokes.len(), 5);
        assert_eq!(strokes.last().unwrap().scans, vec![0x1c]);
    }

    #[test]
    fn modifier_chords_press_and_release_the_modifier_around_the_primary_key() {
        let stroke = ResolvedStroke {
            scans: vec![0x1d, 0x17], // left control + I
            hold: Duration::from_millis(50),
            pause: Duration::from_millis(40),
        };

        assert_eq!(phase_scans(&stroke, InputPhase::Press), vec![0x1d]);
        assert_eq!(phase_scans(&stroke, InputPhase::PressChord), vec![0x17]);
        assert_eq!(phase_scans(&stroke, InputPhase::Release), vec![0x17]);
        assert_eq!(
            phase_scans(&stroke, InputPhase::ReleaseModifiers),
            vec![0x1d]
        );
    }

    #[test]
    fn dynamic_batch_targets_use_the_current_active_source() {
        let sources = vec![source(3, false), source(1, true), source(2, false)];
        let active = select_sources_for_targets(&sources, &EqActionTargets::Active, true).unwrap();
        assert_eq!(
            active
                .iter()
                .map(|source| source.window_number)
                .collect::<Vec<_>>(),
            [1]
        );
        let background =
            select_sources_for_targets(&sources, &EqActionTargets::BackgroundLoaded, true).unwrap();
        assert_eq!(
            background
                .iter()
                .map(|source| source.window_number)
                .collect::<Vec<_>>(),
            [2, 3]
        );

        let solo = [source(1, true)];
        let error = select_sources_for_targets(&solo, &EqActionTargets::BackgroundLoaded, true)
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ClientNotFound);

        let inactive = [source(1, false)];
        let error =
            select_sources_for_targets(&inactive, &EqActionTargets::Active, false).unwrap_err();
        assert_eq!(error.code, ErrorCode::ClientNotFound);
    }

    fn source(window_number: usize, active: bool) -> SourceClient {
        SourceClient {
            private_key: window_number as u64,
            character: None,
            server: None,
            class_code: None,
            window_number,
            active,
            activatable: true,
            input_ready: true,
        }
    }

    #[test]
    fn live_readiness_failures_are_reported_as_input_unavailable() {
        let error = map_targeted_input_error(crate::broadcast::TargetedInputError::Unavailable(
            "proxy is not ready".into(),
        ));
        assert_eq!(error.code, ErrorCode::InputUnavailable);
    }
}

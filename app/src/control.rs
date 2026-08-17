//! Production bridge from the generic control API to the Win32 owner thread.

use futures_util::future::BoxFuture;
use std::cell::UnsafeCell;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use trushar::control::{
    BroadcastState, ClientId, ClientTarget, CommandOutcome, ControlError, Controller, ErrorCode,
    InputKind, KeyStroke, SnapshotMapper, SourceClient, StateHub, StateSnapshot,
    DEFAULT_KEY_HOLD_MS, DEFAULT_KEY_PAUSE_MS,
};
use trushar::server::{ServerConfig, ServerHandle};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{MapVirtualKeyW, VkKeyScanW, MAPVK_VK_TO_VSC};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, PostMessageW, SetTimer, WM_USER};

pub const WM_CONTROL_COMMAND: u32 = WM_USER + 20;
pub const TIMER_CONTROL_INPUT: usize = 2;
const COMMAND_CAPACITY: usize = 32;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
const INPUT_TIMEOUT: Duration = Duration::from_secs(45);
const INPUT_TICK_MS: u32 = 10;
const INPUT_ACTIVATION_DELAY: Duration = Duration::from_millis(200);
const INPUT_RELEASE_DELAY: Duration = Duration::from_millis(100);

enum UiCommand {
    Activate {
        target: ClientTarget,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
    },
    SetBroadcast {
        enabled: bool,
        reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
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
}

struct ResolvedStroke {
    scans: Vec<u8>,
    hold: Duration,
    pause: Duration,
}

enum InputPhase {
    Press,
    Release,
    Finish,
}

struct ActiveInput {
    pid: u32,
    kind: InputKind,
    strokes: Vec<ResolvedStroke>,
    index: usize,
    phase: InputPhase,
    next_step: Instant,
    reply: oneshot::Sender<Result<CommandOutcome, ControlError>>,
}

struct UiState {
    receiver: mpsc::Receiver<UiCommand>,
    mapper: SnapshotMapper,
    latest_sources: Vec<SourceClient>,
    hub: Arc<StateHub>,
    tray_hwnd: HWND,
    active_input: Option<ActiveInput>,
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
    fn enqueue(
        &self,
        timeout: Duration,
        make_command: impl FnOnce(oneshot::Sender<Result<CommandOutcome, ControlError>>) -> UiCommand,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
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

    fn set_broadcast_enabled(
        &self,
        enabled: bool,
    ) -> BoxFuture<'static, Result<CommandOutcome, ControlError>> {
        self.enqueue(COMMAND_TIMEOUT, move |reply| UiCommand::SetBroadcast {
            enabled,
            reply,
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
}

/// Initialize the bounded dispatcher and optionally start the dedicated server thread.
/// Called after the tray window exists and before its message loop starts.
pub fn start(hwnd: HWND, config: &crate::config::TrusharConfig) -> Option<ServerHandle> {
    let (sender, receiver) = mpsc::sync_channel(COMMAND_CAPACITY);
    let hub = Arc::new(StateHub::new(Default::default()));
    *ui() = Some(UiState {
        receiver,
        mapper: SnapshotMapper::default(),
        latest_sources: Vec::new(),
        hub: hub.clone(),
        tray_hwnd: hwnd,
        active_input: None,
    });
    crate::overlay::publish_control_snapshot();

    if !config.enabled {
        return None;
    }
    let bind = match config.bind.parse() {
        Ok(bind) => bind,
        Err(error) => {
            report_start_failure(&format!("invalid bind address: {error}"));
            return None;
        }
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
        Ok(server) => Some(server),
        Err(error) => {
            report_start_failure(&error.to_string());
            None
        }
    }
}

fn report_start_failure(detail: &str) {
    let message = format!("trushar disabled: {detail}");
    eprintln!("{message}");
    crate::overlay::debug_log(&message);
    crate::overlay::show_toast("trushar could not start; see debug.log");
}

pub fn stop() {
    if let Some(mut state) = ui().take() {
        if let Some(input) = state.active_input.take() {
            crate::broadcast::finish_targeted_input(input.pid);
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
            UiCommand::SetBroadcast { enabled, reply } => {
                let result = set_broadcast_on_ui(enabled, true);
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
        }
    }
}

fn activate_on_ui(target: ClientTarget) -> Result<CommandOutcome, ControlError> {
    let private_key = {
        let Some(state) = ui().as_ref() else {
            return Err(ControlError::new(
                ErrorCode::InternalError,
                "control dispatcher is stopped",
            ));
        };
        let matches: Vec<&SourceClient> = state
            .latest_sources
            .iter()
            .filter(|source| match &target {
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
            if matches!(&target, ClientTarget::Id(id) if state.mapper.is_retired(id)) {
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
        matches[0].private_key
    };
    unsafe { crate::overlay::activate_pid(private_key as u32) }
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
            if let Err((reply, error)) =
                start_resolved_input(client_id, InputKind::Text, strokes, reply)
            {
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
            if let Err((reply, error)) =
                start_resolved_input(client_id, InputKind::Keys, strokes, reply)
            {
                let _ = reply.send(Err(error));
            }
        }
        Err(error) => {
            let _ = reply.send(Err(error));
        }
    }
}

type StartInputError = (
    oneshot::Sender<Result<CommandOutcome, ControlError>>,
    ControlError,
);

fn start_resolved_input(
    client_id: ClientId,
    kind: InputKind,
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
    if state.active_input.is_some() {
        return Err((
            reply,
            ControlError::new(
                ErrorCode::InputOperationFailed,
                "another targeted input sequence is already in progress",
            ),
        ));
    }
    if let Err(message) = crate::broadcast::begin_targeted_input(pid) {
        return Err((
            reply,
            ControlError::new(ErrorCode::InputOperationFailed, message),
        ));
    }
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
    state.active_input = Some(ActiveInput {
        pid,
        kind,
        strokes,
        index: 0,
        phase: InputPhase::Press,
        // trusik observes the active flag and gives a background EQ window an
        // activation notification before DirectInput consumes the first key.
        next_step: Instant::now() + INPUT_ACTIVATION_DELAY,
        reply,
    });
    Ok(())
}

fn resolve_input_pid(client_id: &ClientId) -> Result<u32, ControlError> {
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
        return Ok(source.private_key as u32);
    }
    let code = if state.mapper.is_retired(client_id) {
        ErrorCode::TargetDisappeared
    } else {
        ErrorCode::ClientNotFound
    };
    Err(ControlError::new(code, "the target client is not loaded"))
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

/// Advance one target-specific sequence phase from the Win32 timer callback.
pub fn advance_input() {
    let Some(state) = ui().as_mut() else { return };
    let Some(mut input) = state.active_input.take() else {
        unsafe {
            let _ = KillTimer(state.tray_hwnd, TIMER_CONTROL_INPUT);
        }
        return;
    };
    if input.reply.is_closed() {
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
        state.active_input = Some(input);
        return;
    }
    let stroke = &input.strokes[input.index];
    let result = match input.phase {
        InputPhase::Press => {
            for &scan in &stroke.scans {
                if let Err(message) = crate::broadcast::set_targeted_key(input.pid, scan, true) {
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
            input.phase = InputPhase::Release;
            input.next_step = now + stroke.hold;
            None
        }
        InputPhase::Release => {
            for &scan in stroke.scans.iter().rev() {
                if let Err(message) = crate::broadcast::set_targeted_key(input.pid, scan, false) {
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
            if input.index + 1 == input.strokes.len() {
                // Keep SHM active briefly with an all-up state so EQ can
                // observe the final release before the mapping is deactivated.
                input.phase = InputPhase::Finish;
                input.next_step = now + INPUT_RELEASE_DELAY;
                None
            } else {
                input.index += 1;
                input.phase = InputPhase::Press;
                input.next_step = now + stroke.pause;
                None
            }
        }
        InputPhase::Finish => Some(Ok(CommandOutcome::InputDelivered {
            kind: input.kind,
            strokes: input.strokes.len(),
        })),
    };
    if let Some(result) = result {
        finish_input(state, input, Some(result));
    } else {
        state.active_input = Some(input);
    }
}

fn finish_input(
    state: &mut UiState,
    input: ActiveInput,
    result: Option<Result<CommandOutcome, ControlError>>,
) {
    crate::broadcast::finish_targeted_input(input.pid);
    unsafe {
        let _ = KillTimer(state.tray_hwnd, TIMER_CONTROL_INPUT);
    }
    if let Some(result) = result {
        let _ = input.reply.send(result);
    }
}

/// Publish owner-thread state after any known local or remote change.
pub fn publish(sources: Vec<SourceClient>, broadcast: BroadcastState) {
    let Some(state) = ui().as_mut() else { return };
    let data = state.mapper.map(&sources, broadcast);
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
}

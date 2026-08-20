# trushar WebSocket protocol

`trushar` is Stonemite's generic semantic control/state interface. It provides current EQ-client state, pushed changes, exact activation, active-to-selected window-number swaps, explicit broadcast enable/disable operations, bounded input delivery, and keybinding-independent EverQuest actions for one exact loaded client. It has no client-hardware, layout, icon, button, or vendor protocol model. A future device-specific integration can use it as one ordinary client.

## Endpoint and security

Protocol version 1 uses JSON text messages over RFC 6455 WebSocket at:

```text
ws://127.0.0.1:19720/trushar/v1
```

Stonemite starts the server on IPv4 loopback by default, so a trusted native client on the same PC can connect without setup or authentication. In **Settings > General > Integrations**, **This PC only** keeps that default boundary. Disable integrations there, or configure it manually, to stop the API:

```toml
[trushar]
enabled = false
bind = "127.0.0.1:19720"
```

`bind` must be a numeric IPv4 or bracketed IPv6 socket address. Port `0` is supported for tests. IPv4 `127.0.0.0/8` and IPv6 `::1` are recognized as loopback. Wildcard and non-loopback addresses fail closed unless `auth_token` is configured. Settings take effect after restart and are preserved when the graphical settings dialog saves other settings.

For guided LAN access, choose **Devices on my local network**, save, restart, and allow Stonemite on Private networks if Windows Defender Firewall asks. Stonemite generates the long credential automatically. The equivalent advanced configuration is:

```toml
[trushar]
enabled = true
bind = "0.0.0.0:19720"
auth_token = "replace-with-a-long-random-token"
```

Authenticated clients send the long credential in the HTTP upgrade request, never in the URL:

```http
Authorization: Bearer replace-with-a-long-random-token
```

When a token is configured, every connection must supply it. A version 1 token authorizes every API operation; token scopes are reserved for a future protocol version if distinct trust levels become necessary. A request containing an `Origin` header is rejected unless it is authenticated. Native localhost clients normally omit `Origin`; browsers send it, so an arbitrary web page cannot use unauthenticated loopback access. Tokens are never included in state, protocol errors, or server diagnostics.

Authentication failures reject the HTTP upgrade with status 401 (or 403 for the unauthenticated `Origin` policy) and an `application/json` body using the stable `unauthorized` error code. No WebSocket connection is established.

### Six-digit device pairing

While LAN mode is running, **Pair a device** in Settings opens this endpoint for five minutes:

```text
ws://<stonemite-host>:19720/trushar/v1/pair
```

The native client connects without an `Authorization` or `Origin` header and sends the six digits without spaces:

```json
{ "type": "pair", "version": 1, "code": "482731" }
```

A correct code receives the long credential exactly once:

```json
{ "type": "paired", "version": 1, "auth_token": "long-random-credential" }
```

The client stores that credential securely, closes the pairing connection, and reconnects to `/trushar/v1` using `Authorization: Bearer ...`. A code is generated from cryptographic randomness, expires after five minutes, is invalidated by one successful exchange, and is disabled after five failed attempts. Starting a new code invalidates the previous one; closing Settings cancels a code still in progress. Pairing requests with an `Origin` header are rejected. Neither the six-digit code nor long credential is logged or included in normal state and error messages.

The server currently provides `ws://`, not TLS. A token authenticates a LAN client but offers no confidentiality against traffic observation or modification. Because an authenticated client can deliver input to EQ, use LAN binding only on a trusted network or provide an encrypted tunnel. Certificate management and native `wss://` are intentionally outside this small subsystem.

Leaving `trushar` enabled makes its complete API available to trusted native clients on this PC, including targeted input when trusik is also enabled; there is no separate input-enable switch. LAN exposure remains explicit and authenticated. Disable integrations if no trusted local client should control EQ. The input operations are intended for bounded, user-initiated actions rather than unattended gameplay automation; users remain responsible for the game's rules.

## State and identifiers

The server sends a complete `state` message immediately after a successful upgrade. Later known changes are pushed as more complete `state` messages; reconnecting therefore recovers without a replay log. `revision` increases monotonically for each distinct public state during one Stonemite run. Equal snapshots are deduplicated and slow clients coalesce naturally to the latest snapshot.

```json
{
  "type": "state",
  "version": 1,
  "state": {
    "revision": 4,
    "clients": [
      {
        "id": "client-0000000000000001",
        "character": "Laika",
        "server": "Xegony",
        "class_code": "SHK",
        "window_number": 1,
        "active": true,
        "activatable": true,
        "input_ready": true
      }
    ],
    "active_client_id": "client-0000000000000001",
    "broadcast": { "available": true, "enabled": false },
    "capabilities": {
      "activate": true,
      "swap_window_numbers": true,
      "set_broadcast": true,
      "send_text": true,
      "send_keys": true,
      "eq_actions": {
        "use_center_screen": true,
        "invite_follow": true,
        "hotbars": 11,
        "hotbar_buttons": 12,
        "spell_gems": 14,
        "keymap_actions": true
      }
    }
  }
}
```

`id` is opaque and stable only while that EQ process remains loaded in the current Stonemite run. It does not expose the PID or HWND. A stale recently issued ID produces `target_disappeared`; an ID never known to this run produces `client_not_found`.

`window_number` is Stonemite's one-based, user-visible window number. It is not the PiP layout index. `activatable` is false if a discovered EQ window is outside the existing active-plus-five-PiP swap set; state does not claim that such a window can be activated. `input_ready` is true only after that EQ process's compatible trusik proxy acknowledges the per-process shared-memory channel.

`character`, `server`, and `class_code` are omitted when unknown. Character/server identity can appear after initial process discovery and can change when trusik observes the same EQ process open a different character log after camping or changing servers. `class_code` is the actual abbreviation Stonemite learned (for example `SHK` or `SHM`), not a fabricated full class name. Log-file assignment candidates are not exposed as loaded clients.

`broadcast.available` is false when trusik/broadcast support was not initialized. In that state `enabled` is false, `set_broadcast` capability is false, and mutation requests return `broadcast_unavailable`.

`swap_window_numbers` is true when the server supports exchanging the active client's stable window number with one selected client without changing the foreground client. `send_text`, `send_keys`, and the typed delivery ranges under `eq_actions` are available when at least one loaded client has `input_ready: true`. They require `trusik = true` but do not depend on whether broadcasting is enabled. `eq_actions.hotbars`, `hotbar_buttons`, and `spell_gems` are zero when unavailable; otherwise they advertise the supported one-based ranges. `keymap_actions` advertises generic mapping discovery and batch-target preflight and remains true while no input channel is currently ready, so clients can distinguish temporary readiness from an older server. Consumers must still check the selected client's `input_ready`; command-time validation is authoritative because readiness, identity, and effective keymaps can change after a snapshot. Clients talking to older servers must treat missing `eq_actions` fields as unsupported.

## Client requests

Every request carries version 1 and a nonempty caller-chosen `request_id` of at most 128 bytes. Version 1 state and result objects may gain additive fields; clients must ignore fields they do not recognize. Up to 16 commands may be in flight on one connection. Results can complete in a different order, and pushed state can arrive between them, so clients must correlate by `request_id`.

Request current state explicitly:

```json
{ "type": "get_state", "version": 1, "request_id": "state-1" }
```

Activate using the exact opaque ID from a current snapshot:

```json
{
  "type": "activate",
  "version": 1,
  "request_id": "activate-1",
  "target": { "type": "client_id", "client_id": "client-0000000000000001" }
}
```

Window-number and identity targets are also available when useful:

```json
{"type":"activate","version":1,"request_id":"activate-2","target":{"type":"window_number","window_number":2}}
{"type":"activate","version":1,"request_id":"activate-3","target":{"type":"identity","character":"Laika","server":"Xegony"}}
```

Identity matching is case-insensitive. Omitting `server` can be ambiguous when the same character name occurs on multiple servers; the command then returns `ambiguous_target`. Character name is never the only implicit activation key.

Swap the active client's user-visible window number with an exact selected client. This does not activate either client or change the foreground window:

```json
{
  "type": "swap_window_numbers",
  "version": 1,
  "request_id": "swap-numbers-1",
  "target": { "type": "client_id", "client_id": "client-0000000000000002" }
}
```

The same exact target forms accepted by `activate` are supported. Selecting the already-active client is a successful no-op. If there is no active client, the command returns `window_number_swap_failed`.

Set broadcast state explicitly (there is no required read-modify-write toggle):

```json
{
  "type": "set_broadcast",
  "version": 1,
  "request_id": "broadcast-1",
  "enabled": true
}
```

Type text into exactly one current opaque client ID, optionally followed by Enter:

```json
{
  "type": "send_text",
  "version": 1,
  "request_id": "who-1",
  "client_id": "client-0000000000000001",
  "text": "/who",
  "submit": true
}
```

`text` must contain 1–256 printable characters and at most 1024 UTF-8 bytes. Stonemite resolves the complete string against the active Windows keyboard layout before delivering anything; an unsupported character returns `invalid_argument`. Text contents are not logged. `submit` defaults to false.

Deliver semantic key strokes to one exact client. Keys in one `keys` array form a chord; strokes run in array order:

```json
{
  "type": "send_keys",
  "version": 1,
  "request_id": "hotkey-1",
  "client_id": "client-0000000000000001",
  "strokes": [{ "keys": ["left_control", "1"], "hold_ms": 50, "pause_ms": 40 }]
}
```

`send_keys` accepts 1–64 strokes, each containing 1–8 distinct keys. `hold_ms` defaults to 75 and must be 1–1000; `pause_ms` defaults to 75 and must be 0–1000. Total requested duration is limited to 15 seconds. Supported names are `a`–`z`, `0`–`9`, `f1`–`f12`, `numpad_0`–`numpad_9`, and: `escape`, `minus`, `equals`, `backspace`, `tab`, `left_bracket`, `right_bracket`, `enter`, `left_control`, `semicolon`, `apostrophe`, `grave`, `left_shift`, `backslash`, `comma`, `period`, `slash`, `right_shift`, `numpad_multiply`, `left_alt`, `space`, `caps_lock`, `num_lock`, `scroll_lock`, `numpad_subtract`, `numpad_add`, `numpad_decimal`, `numpad_divide`, `numpad_enter`, `right_control`, `right_alt`, `home`, `arrow_up`, `page_up`, `arrow_left`, `arrow_right`, `end`, `arrow_down`, `page_down`, `insert`, `delete`, and `pause`.

Input uses the target process's trusik shared memory. A selected client whose compatible proxy has not acknowledged that channel returns `input_unavailable`; Stonemite never reports successful delivery solely because it created a mapping. Input does not activate the window, call global `SendInput`, or send keys to other EQ clients. One targeted sequence may run per client, so independent clients can receive input concurrently; a second sequence for the same client is rejected until the first completes. All API-held keys are released on success, error, request timeout/disconnect, target disappearance, or Stonemite shutdown. Physical broadcast keys and targeted keys are tracked separately so one source cannot release the other's held key.

Invoke a semantic EverQuest action without assuming the user's physical keybinding:

```json
{
  "type": "send_eq_action",
  "version": 1,
  "request_id": "use-1",
  "client_id": "client-0000000000000001",
  "action": { "type": "use_center_screen" }
}
```

Supported action objects are:

```json
{"type":"use_center_screen"}
{"type":"invite_follow"}
{"type":"hotbar","bar":1,"button":1}
{"type":"spell_gem","gem":1}
{"type":"keymap","mapping":"SIT_STAND"}
```

A generic `keymap` mapping is the canonical stem between `KEYMAPPING_` and the final `_1` or `_2`. It contains 1–128 ASCII letters, numbers, or underscores and is canonicalized to uppercase. Physical scan codes are never exposed through the protocol.

Hotbars are 1–11, each with buttons 1–12; spell gems are 1–14. Stonemite resolves `USE`, `INVITE_FOLLOW`, `HOT{bar}_{button}`, or `CAST{gem}` from the selected client's effective EQ keymap beside that process's own `eqgame.exe`, then delivers the configured primary binding or a configured alternate. Current live EQ character/persona keymaps in `<Character>_<server>_<class>.ini` take precedence for that exact identity; the legacy `<Character>_<server>.ini` form and shared `eqclient.ini` remain supported. Known EQ defaults cover Use Center Screen, Invite/Follow, Hotbar 1, and all 14 spell gems when no override is stored. Explicitly unbound primary and alternate mappings return `eq_action_unbound`.

An action result has the exact resolved semantic action:

```json
{
  "type": "eq_action_delivered",
  "action": { "type": "hotbar", "bar": 1, "button": 3 }
}
```

Delivery means the resolved chord was written and released through the selected process's acknowledged input channel. It does not prove that EQ performed the action. Normal EQ multibinds still apply: if the same chord is bound to several actions, EQ may perform all of them according to its own rules.

Discover generic mappings shared by the selected loaded boxes:

```json
{
  "type": "list_eq_keymap_actions",
  "version": 1,
  "request_id": "mapped-1",
  "targets": { "type": "window_numbers", "window_numbers": [1, 2, 4] }
}
```

Targets are `{"type":"all_loaded"}`, `{"type":"active"}`, `{"type":"background_loaded"}`, or `{"type":"window_numbers","window_numbers":[...]}` with 1–6 unique Stonemite window numbers from 1 through 6. Active resolves the current foreground EQ client. Background resolves every loaded EQ client except that active client. Dynamic targets are evaluated from authoritative owner-thread state for each request. Discovery intersects the effective mappings of the currently resolved boxes; requested empty numeric positions are omitted and reported through `window_numbers`. A missing active client rejects active or background discovery. Results are sorted pages of at most 64 mapping names:

```json
{
  "type": "eq_keymap_actions_listed",
  "mappings": ["DUCK", "SIT_STAND"],
  "window_numbers": [1, 2],
  "next_after": "SIT_STAND"
}
```

When `next_after` is present, send another request with `"after":"SIT_STAND"`. A mapping is discoverable when its effective profile has a decodable nonzero primary or alternate entry, or Stonemite knows its EQ default. Explicit zero mappings are absent. Malformed relevant entries return `input_operation_failed` rather than being mislabeled as unbound.

Deliver one action to a preflighted target set:

```json
{
  "type": "send_eq_action_batch",
  "version": 1,
  "request_id": "burn-1",
  "targets": { "type": "all_loaded" },
  "action": { "type": "keymap", "mapping": "HOT2_1" }
}
```

For `all_loaded`, `active`, and `background_loaded`, Stonemite resolves and freezes the target set when the command reaches its owner thread. Active requires a current active client. Background additionally requires at least one loaded background client. For explicit window numbers, every requested position must be loaded. Before exposing any key, Stonemite validates every target's readiness and effective mapping, rejects targets with another sequence in progress, and acquires every trusik input channel. Any preflight or acquisition failure rolls back and sends nothing. Once admitted, target chords start concurrently and success is returned only after all keys are released:

```json
{
  "type": "eq_action_batch_delivered",
  "action": { "type": "keymap", "mapping": "HOT2_1" },
  "window_numbers": [1, 2, 3, 4, 5, 6]
}
```

A process can still disappear or fail after admission. The resulting error states that one or more targets may already have received the action because delivered input cannot be rolled back.

## Results and errors

Every successful request returns the authoritative current snapshot as well as a typed result:

```json
{
  "type": "result",
  "version": 1,
  "request_id": "activate-1",
  "result": {
    "type": "activated",
    "status": "activated",
    "foreground_confirmed": true
  },
  "state": {
    "revision": 5,
    "clients": [],
    "active_client_id": null,
    "broadcast": { "available": false, "enabled": false },
    "capabilities": {
      "activate": true,
      "swap_window_numbers": true,
      "set_broadcast": false,
      "send_text": false,
      "send_keys": false,
      "eq_actions": {
        "use_center_screen": false,
        "invite_follow": false,
        "hotbars": 0,
        "hotbar_buttons": 0,
        "spell_gems": 0,
        "keymap_actions": false
      }
    }
  }
}
```

Activation status is `activated` or `already_active`. A successful activation is returned only after Windows identifies the requested HWND as foreground and its window tree owns keyboard focus, so production success results set `foreground_confirmed` to `true`; the field remains in protocol v1 for compatibility. If Windows denies foreground or keyboard-focus acquisition, or the target is unresponsive, Stonemite returns `activation_failed` without applying a speculative active/PiP exchange. A target HWND that disappears during acquisition returns `target_disappeared`.

A completed window-number swap returns `{"type":"window_numbers_swapped","active_previous_number":1,"selected_previous_number":3}` plus the authoritative state showing the exchanged numbers. The active client remains active. Equal previous numbers indicate the selected target was already active and no state changed.

A completed input operation returns `{"type":"input_delivered","input":"text|keys","strokes":N}`. This confirms that Stonemite wrote and released every requested stroke through the intended live process's shared-memory channel. It does not claim that EQ accepted the input or performed a resulting action.

Errors have stable machine codes and concise messages:

```json
{
  "type": "error",
  "version": 1,
  "request_id": "activate-1",
  "error": {
    "code": "client_not_found",
    "message": "no loaded client matches the target"
  }
}
```

Version 1 defines: `malformed_request`, `unsupported_protocol_version`, `unauthorized` (HTTP upgrade failures use HTTP 401), `invalid_argument`, `client_not_found`, `ambiguous_target`, `target_disappeared`, `broadcast_unavailable`, `activation_failed`, `window_number_swap_failed`, `broadcast_operation_failed`, `input_unavailable`, `input_operation_failed`, `eq_action_unbound`, `command_timeout`, and `internal_error`.

Unknown request types, invalid fields, and malformed JSON receive `malformed_request` without affecting other clients. Binary data messages receive a structured `malformed_request`; only UTF-8 JSON text is accepted. Text messages and individual frames are limited to 16 KiB. Oversized input closes the connection with WebSocket code 1009. Tungstenite handles matching Pong responses to Ping frames. On client Close the server completes the close exchange; on application shutdown it sends code 1001, stops accepting, drains connection tasks for a bounded interval, and joins the dedicated runtime thread before overlay/broadcast teardown.

## Hardware-free manual validation

The repository includes a generic interactive client. Start Stonemite, then from a source checkout run:

```powershell
cargo run -p trushar --example client -- ws://127.0.0.1:19720/trushar/v1
```

The initial snapshot prints immediately. Paste any request above as one line to get state, activate a currently listed ID, swap the active and selected window numbers, type `/who` into one exact current ID, send a bounded key chord, change broadcast state, and continue watching pushed updates.

For an authenticated endpoint, set the token without putting it in the URL:

```powershell
$env:TRUSHAR_TOKEN = Read-Host "trushar token"
cargo run -p trushar --example client -- ws://127.0.0.1:19720/trushar/v1
```

After configuring an authenticated LAN bind and allowing the selected TCP port through the host firewall, run this on another trusted LAN machine, replacing the address only:

```powershell
$env:TRUSHAR_TOKEN = Read-Host "trushar token"
cargo run -p trushar --example client -- ws://192.168.1.50:19720/trushar/v1
```

Windows Defender Firewall normally asks whether to allow Stonemite when LAN mode first starts. Allow **Private networks** only. If no prompt appears and a LAN client cannot connect, create a narrowly scoped rule from an elevated PowerShell prompt:

```powershell
New-NetFirewallRule -DisplayName "Stonemite trushar (Private LAN)" `
  -Direction Inbound -Action Allow `
  -Program "$env:LOCALAPPDATA\Programs\Stonemite\stonemite.exe" `
  -Protocol TCP -LocalPort 19720 -Profile Private -RemoteAddress LocalSubnet
```

Remove it when disabling LAN access or before changing the program/port scope:

```powershell
Remove-NetFirewallRule -DisplayName "Stonemite trushar (Private LAN)"
```

The per-user installer does not silently elevate or open the firewall. The normal Windows firewall prompt owns any rule it creates. The commands above are only the manual fallback; remove a manual rule when disabling LAN access or uninstalling. Loopback-only use never needs a rule.

Do not commit a real token or place it in a query string.

For development without a running Stonemite UI, `cargo run -p trushar --example server -- 127.0.0.1:19720 300` starts the same real server around an in-memory controller for five minutes. Non-loopback binds still require `TRUSHAR_TOKEN` and enforce the same authentication and Origin policies.

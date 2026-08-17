# trushar WebSocket protocol

`trushar` is Stonemite's generic semantic control/state interface. It provides current EQ-client state, pushed changes, exact activation, explicit broadcast enable/disable operations, and bounded input delivery to one exact loaded client. It has no client-hardware, layout, icon, button, or vendor protocol model. A future device-specific integration can use it as one ordinary client.

## Endpoint and security

Protocol version 1 uses JSON text messages over RFC 6455 WebSocket at:

```text
ws://127.0.0.1:19720/trushar/v1
```

The backward-compatible default configuration is:

```toml
[trushar]
enabled = true
bind = "127.0.0.1:19720"
```

`bind` must be a numeric IPv4 or bracketed IPv6 socket address. Port `0` is supported for tests. IPv4 `127.0.0.0/8` and IPv6 `::1` are recognized as loopback. Wildcard and non-loopback addresses fail closed unless `auth_token` is configured. Settings take effect after restart and are preserved when the graphical settings dialog saves other settings.

To opt into LAN access:

```toml
[trushar]
enabled = true
bind = "0.0.0.0:19720"
auth_token = "replace-with-a-long-random-token"
```

Authenticated clients send the token in the HTTP upgrade request, never in the URL:

```http
Authorization: Bearer replace-with-a-long-random-token
```

When a token is configured, every connection must supply it. A version 1 token authorizes every API operation; token scopes are reserved for a future protocol version if distinct trust levels become necessary. A request containing an `Origin` header is rejected unless it is authenticated. Native localhost clients normally omit `Origin`; browsers send it, so an arbitrary web page cannot use unauthenticated loopback access. Tokens are never included in state, protocol errors, or server diagnostics.

Authentication failures reject the HTTP upgrade with status 401 (or 403 for the unauthenticated `Origin` policy) and an `application/json` body using the stable `unauthorized` error code. No WebSocket connection is established.

The server currently provides `ws://`, not TLS. A token authenticates a LAN client but offers no confidentiality against traffic observation or modification. Because an authenticated client can deliver input to EQ, use LAN binding only on a trusted network or provide an encrypted tunnel. Certificate management and native `wss://` are intentionally outside this small subsystem.

Enabling `trushar` is the opt-in for its complete API, including targeted input; there is no second input-enable switch. Disable the server if no trusted client should control EQ. The input operations are intended for bounded, user-initiated actions rather than unattended gameplay automation; users remain responsible for the game's rules.

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
        "activatable": true
      }
    ],
    "active_client_id": "client-0000000000000001",
    "broadcast": { "available": true, "enabled": false },
    "capabilities": { "activate": true, "set_broadcast": true, "send_text": true, "send_keys": true }
  }
}
```

`id` is opaque and stable only while that EQ process remains loaded in the current Stonemite run. It does not expose the PID or HWND. A stale recently issued ID produces `target_disappeared`; an ID never known to this run produces `client_not_found`.

`window_number` is Stonemite's one-based, user-visible window number. It is not the PiP layout index. `activatable` is false if a discovered EQ window is outside the existing active-plus-five-PiP swap set; state does not claim that such a window can be activated.

`character`, `server`, and `class_code` are omitted when unknown. Character/server identity can appear after initial process discovery and can change when trusik observes the same EQ process open a different character log after camping or changing servers. `class_code` is the actual abbreviation Stonemite learned (for example `SHK` or `SHM`), not a fabricated full class name. Log-file assignment candidates are not exposed as loaded clients.

`broadcast.available` is false when trusik/broadcast support was not initialized. In that state `enabled` is false, `set_broadcast` capability is false, and mutation requests return `broadcast_unavailable`.

`send_text` and `send_keys` capabilities are true when trusik shared-memory input is available. This requires `trusik = true`, the proxy DLL loaded in the target EQ process, and a current per-process shared-memory target. These capabilities do not depend on whether broadcasting is enabled.

## Client requests

Every request carries version 1 and a nonempty caller-chosen `request_id` of at most 128 bytes. Up to 16 commands may be in flight on one connection. Results can complete in a different order, and pushed state can arrive between them, so clients must correlate by `request_id`.

Request current state explicitly:

```json
{"type":"get_state","version":1,"request_id":"state-1"}
```

Activate using the exact opaque ID from a current snapshot:

```json
{"type":"activate","version":1,"request_id":"activate-1","target":{"type":"client_id","client_id":"client-0000000000000001"}}
```

Window-number and identity targets are also available when useful:

```json
{"type":"activate","version":1,"request_id":"activate-2","target":{"type":"window_number","window_number":2}}
{"type":"activate","version":1,"request_id":"activate-3","target":{"type":"identity","character":"Laika","server":"Xegony"}}
```

Identity matching is case-insensitive. Omitting `server` can be ambiguous when the same character name occurs on multiple servers; the command then returns `ambiguous_target`. Character name is never the only implicit activation key.

Set broadcast state explicitly (there is no required read-modify-write toggle):

```json
{"type":"set_broadcast","version":1,"request_id":"broadcast-1","enabled":true}
```

Type text into exactly one current opaque client ID, optionally followed by Enter:

```json
{"type":"send_text","version":1,"request_id":"who-1","client_id":"client-0000000000000001","text":"/who","submit":true}
```

`text` must contain 1–256 printable characters and at most 1024 UTF-8 bytes. Stonemite resolves the complete string against the active Windows keyboard layout before delivering anything; an unsupported character returns `invalid_argument`. Text contents are not logged. `submit` defaults to false.

Deliver semantic key strokes to one exact client. Keys in one `keys` array form a chord; strokes run in array order:

```json
{"type":"send_keys","version":1,"request_id":"hotkey-1","client_id":"client-0000000000000001","strokes":[{"keys":["left_control","1"],"hold_ms":50,"pause_ms":40}]}
```

`send_keys` accepts 1–64 strokes, each containing 1–8 distinct keys. `hold_ms` defaults to 75 and must be 1–1000; `pause_ms` defaults to 75 and must be 0–1000. Total requested duration is limited to 15 seconds. Supported names are `a`–`z`, `0`–`9`, `f1`–`f12`, `numpad_0`–`numpad_9`, and: `escape`, `minus`, `equals`, `backspace`, `tab`, `left_bracket`, `right_bracket`, `enter`, `left_control`, `semicolon`, `apostrophe`, `grave`, `left_shift`, `backslash`, `comma`, `period`, `slash`, `right_shift`, `numpad_multiply`, `left_alt`, `space`, `caps_lock`, `num_lock`, `scroll_lock`, `numpad_subtract`, `numpad_add`, `numpad_decimal`, `numpad_divide`, `numpad_enter`, `right_control`, `right_alt`, `home`, `arrow_up`, `page_up`, `arrow_left`, `arrow_right`, `end`, `arrow_down`, `page_down`, `insert`, `delete`, and `pause`.

Input uses the target process's trusik shared memory. It does not activate the window, call global `SendInput`, or send keys to other EQ clients. Only one targeted sequence runs at a time. All API-held keys are released on success, error, request timeout/disconnect, target disappearance, or Stonemite shutdown. Physical broadcast keys and targeted keys are tracked separately so one source cannot release the other's held key.

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
  "state": { "revision": 5, "clients": [], "active_client_id": null, "broadcast": { "available": false, "enabled": false }, "capabilities": { "activate": true, "set_broadcast": false, "send_text": false, "send_keys": false } }
}
```

Activation status is `activated` or `already_active`. `foreground_confirmed` reports whether Windows identified the requested HWND as foreground immediately after the existing asynchronous foreground request; `false` does not falsely claim OS confirmation even though Stonemite applied its internal active/PiP exchange.

A completed input operation returns `{"type":"input_delivered","input":"text|keys","strokes":N}`. This confirms that Stonemite wrote and released every requested stroke through the intended live process's shared-memory channel. It does not claim that EQ accepted the input or performed a resulting action.

Errors have stable machine codes and concise messages:

```json
{"type":"error","version":1,"request_id":"activate-1","error":{"code":"client_not_found","message":"no loaded client matches the target"}}
```

Version 1 defines: `malformed_request`, `unsupported_protocol_version`, `unauthorized` (HTTP upgrade failures use HTTP 401), `invalid_argument`, `client_not_found`, `ambiguous_target`, `target_disappeared`, `broadcast_unavailable`, `activation_failed`, `broadcast_operation_failed`, `input_unavailable`, `input_operation_failed`, `command_timeout`, and `internal_error`.

Unknown request types, invalid fields, and malformed JSON receive `malformed_request` without affecting other clients. Binary data messages receive a structured `malformed_request`; only UTF-8 JSON text is accepted. Text messages and individual frames are limited to 16 KiB. Oversized input closes the connection with WebSocket code 1009. Tungstenite handles matching Pong responses to Ping frames. On client Close the server completes the close exchange; on application shutdown it sends code 1001, stops accepting, drains connection tasks for a bounded interval, and joins the dedicated runtime thread before overlay/broadcast teardown.

## Hardware-free manual validation

The repository includes a generic interactive client. Start Stonemite, then from a source checkout run:

```powershell
cargo run -p trushar --example client -- ws://127.0.0.1:19720/trushar/v1
```

The initial snapshot prints immediately. Paste any request above as one line to get state, activate a currently listed ID, type `/who` into one exact current ID, send a bounded key chord, change broadcast state, and continue watching pushed updates.

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

Windows Defender Firewall normally needs an inbound exception for LAN mode. From an elevated PowerShell prompt, scope it to the installed executable, Private networks, the configured TCP port, and the local subnet:

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

The current per-user installer does not silently elevate or open the firewall. A future settings/installer flow should make this an explicit opt-in UAC action, apply the same narrow scope, and remove its rule on opt-out/uninstall. Loopback-only use never needs this rule.

Do not commit a real token or place it in a query string.

For development without a running Stonemite UI, `cargo run -p trushar --example server -- 127.0.0.1:19720 300` starts the same real server around an in-memory controller for five minutes. Non-loopback binds still require `TRUSHAR_TOKEN` and enforce the same authentication and Origin policies.

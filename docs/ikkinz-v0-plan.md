# Ikkinz v0 implementation plan

## Approved scope

Build Ikkinz inside this repository as a reusable Windows/macOS Stream Deck Desktop plugin with individually placeable controls and an editable 5×3 Stream Deck Mobile default profile. The plugin connects locally or over a private LAN to Stonemite v0.5.0, pairs with Stonemite's existing six-digit flow, and ships the core live dashboard only.

V0 includes:

- one-time host/code pairing and persisted host/token;
- automatic authenticated reconnect and authoritative state recovery;
- a bounded, position-independent boot/handoff into live controls;
- up to six live character tiles with Stonemite slot colors, number badges, class artwork, identity/readiness, and active state;
- exact activation by current opaque client ID;
- a two-step active-to-selected window-number swap;
- explicit broadcast enable/disable with solid red on-state;
- separate group, follow, assist, Bcast, and Swap actions, an inert Stonemite logo action, and three intentionally blank cells in the default profile;
- clear pairing, disconnected, unavailable, in-flight, success, and error feedback;
- an installable `.streamDeckPlugin` package.

V0 excludes Burn, Camp, heals, spells, in-game target/buff/cooldown state, and a generic action editor.

## Architecture

Create `ikkinz/` with:

- `src/plugin.ts` — composition root; register every separate key action before connecting to Stream Deck.
- `src/actions/dashboard-key.ts` and `dashboard-controller.ts` — register action identities, track visible instances independent of coordinates, and dispatch presses from the current logical surface.
- `src/actions/key-definitions.ts` — canonical key names, UUIDs, and action-list copy.
- `src/trushar/client.ts` — one `ws` client for pairing and authenticated protocol v1; bounded backoff, request correlation, message validation, and latest snapshot.
- `src/state/store.ts` — connection/pairing/dashboard state and subscriptions.
- `src/state/layout.ts` — pure identity-based key model with at most six sorted clients, a two-step swap mode, and the shipped default-profile mapping.
- `src/render/key-svg.ts` — deterministic 72×72 SVG data URLs with escaped dynamic text and embedded class art.
- `src/render/assets.generated.ts` — generated app/class PNG data URLs sourced from `app/assets/`.
- `src/types/trushar.ts` — strict wire types and runtime parsers that tolerate additive fields but reject invalid required fields.
- `src/ui/` plus the compiled plugin `ui/` — property inspector for address, six-digit pairing, reconnect, forget-device, and status.
- `co.laikasoft.ikkinz.sdPlugin/` — manifest, built bundle, local `sdpi-components.js`, icons, and the bundled Mobile profile.

Use plugin-wide global settings for `{ address, authToken }`. A six-digit code is sent from the property inspector to the Node application layer and is never persisted. The Node layer performs pairing because browser-originated pairing is correctly rejected by Stonemite and authenticated WebSocket upgrades need an `Authorization` header.

## Dashboard behavior

- Resolve character actions from `window_number` slots 1–6; the default profile uses the same two-row arrangement as the mockup, while custom placements behave identically anywhere.
- Use the exact `LABEL_COLORS` and `BADGE_COLORS` from `app/src/overlay.rs`.
- Character press sends `activate` only when the current client is activatable and the connection is ready.
- Swap press arms a selection mode; the next non-active character press exchanges its window number with the active client's number without changing the active client. Pressing Swap again or selecting the active character cancels.
- Bcast press sends explicit `enabled: !current.enabled`, disables while its request is in flight, and reconciles from returned/pushed state. Its label remains `Bcast`; solid red means enabled.
- Assist press identifies the active named main box and concurrently sends `/assist <main>` to every other input-ready client.
- Group uses blue, Follow green, Assist gold, and Swap cyan. Their dark idle surfaces fill with the action color only while Group/Follow/Assist runs or Swap is armed.
- Empty character slots explain absence without looking actionable.
- The default profile places the inert Stonemite logo at bottom left and leaves the other three former utility cells blank.
- Temporary key feedback says delivered/failed/activated, never cast/succeeded in EQ.
- Boot motion is synchronized but position-independent: a small number of whole-key SVG stages, not an unbounded 15 fps stream.

## Packaging and profile

- Plugin UUID: `co.laikasoft.ikkinz`; action UUIDs cover character slots 1–6, Group, Broadcast, Follow, Assist, Swap, and Logo.
- Bundle editable, auto-installed 5×3 preset profiles for Stream Deck (device type 0) and Stream Deck Mobile (device type 3), each containing the 12 individual actions and three empty positions.
- Node runtime 24, SDK version 3, Stream Deck minimum 7.4 (latest schema currently documented), OS entries for macOS and Windows because Stream Deck Desktop may run on either.
- Each key's behavior follows its action UUID rather than physical coordinates; logical view/state changes redraw visible instances and never switch Stream Deck profiles.
- Keep every action visible in the action list so users can rearrange, duplicate, or repair controls manually.
- Generate both profile archives deterministically from the tracked manifest and profile source data.

## Validation contract

### Automated

- `npm run format:check`
- `npm run lint`
- `npm run typecheck`
- `npm test`
- `npm run build`
- `npm run validate` with current Elgato CLI
- Protocol fixture tests: initial/pushed state, results between state messages, out-of-order results, errors, additive fields, malformed/oversized frames, and stale IDs.
- Layout/render tests: all separate key identities, the 12-action default profile, arbitrary physical placement, zero/six/more-than-six clients, unknown identity, active/disabled/input-unready states, distinct Group/Follow/Assist colors and filled running states, stable Bcast wording across off/on, Swap idle/filled-armed/unavailable, Logo and blank cells, rail-free surfaces, XML escaping, and deterministic SVG.
- Connection tests against a local `ws` fixture for pairing, Authorization header, reconnect, startup-order inversion, and token clearing/re-pairing.

### Hardware-free integration

Run Stonemite's real in-memory trushar example server and connect the built Ikkinz client to it where practical. Confirm exact outgoing requests and authoritative state reconciliation.

### Mac + Stream Deck Mobile

- Link the plugin into Stream Deck Desktop 7.5.1 on this Mac.
- Select/configure the 5×3 Mobile profile, verify all 12 controls, and move several actions to confirm position-independent behavior.
- Inspect property inspector keyboard flow and status changes.
- Validate disconnected and mock-server states before testing against a live Stonemite host.

### Live Windows host test

- Enable LAN integrations in Stonemite v0.5.0, restart, and allow Private networks.
- Enter the address Stonemite displays, such as `stonemite-pc.local:19720`, plus its six-digit code in Ikkinz.
- Confirm the token is exchanged once and later reconnects need no code.
- Observe real six-client identity/readiness.
- Activate each available character and confirm pushed active state.
- Arm Swap, select a background character, and confirm the two window numbers exchange while the active character does not.
- Toggle broadcast off/on explicitly and confirm desktop/deck stay synchronized.
- Restart Stonemite, Stream Deck, Mac networking, and Mobile in different orders.
- Confirm no token or pairing code appears in logs or packaged files.

## Review and finish

After implementation, run fresh-context reviews for protocol/security correctness, Stream Deck lifecycle/rendering behavior, and test/packaging quality. Apply accepted fixes with one writer, rerun affected gates, inspect the final 72×72 SVG output and property inspector once in a bounded visual pass, run the Impeccable detector over changed web UI, and record residual hardware-only risks.

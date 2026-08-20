# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

- Stonemite desktop app: Rust workspace targeting Windows 10+.
- Stonemite Stream Deck plugin: Node.js 24, TypeScript, Elgato Stream Deck SDK, Rollup, Vitest, and `ws` for automatic loopback or authenticated LAN WebSocket upgrades.

## Users

EverQuest multiboxers who run several EQ clients through Stonemite and want a compact, glanceable control surface on Stream Deck hardware or Stream Deck Mobile.

A typical setup runs Stonemite on a Windows PC and Stream Deck Desktop on the same computer or another device paired to Stream Deck hardware or Mobile. Cross-device installations connect to Stonemite over a trusted private LAN.

## Product purpose

Stonemite makes a six-client EverQuest setup visible and controllable without repeatedly navigating Windows. The Stream Deck plugin extends the same live character identity and narrowly bounded controls onto the deck.

The Stream Deck plugin v0 succeeds when a same-PC installation connects without setup, a cross-PC installation pairs once, and the user can see up to six current clients and their readiness at a glance, activate an exact client, swap the active and selected clients' window numbers, explicitly enable or disable broadcast, survive either app reconnecting, and understand failures without looking at logs.

## Positioning

Stonemite derives character identity from the running EQ clients and controls an exact loaded process through a small semantic API. The Stream Deck plugin reflects that live state rather than asking the user to maintain a second manual roster or sending blind desktop hotkeys.

## Operating context

- Stonemite v0.5.0 runs on Windows with loopback integrations enabled by default. When Stream Deck Desktop runs on that PC, the plugin connects automatically to `127.0.0.1:19720` without authentication.
- Cross-PC use is explicit: Stonemite exposes the API to the private LAN, displays a `hostname.local:port` address and single-use six-digit pairing code, and requires the resulting credential.
- Stream Deck Desktop runs the Stonemite Node.js plugin. Stream Deck Mobile is only the paired key surface; it does not connect to Stonemite directly.
- Every character slot and control is independently placeable; the plugin does not currently bundle a preset profile.
- Users may start Stonemite, Stream Deck, the Mac, and the mobile device in any order, so reconnection is normal rather than exceptional.

## Capabilities and constraints

- Protocol v1 provides complete pushed state, exact client activation, active-to-selected window-number swaps, explicit broadcast state, exact-client text/key delivery, keymap-aware EQ actions, shared mapping discovery, preflighted all-loaded, active, background, and stable-number targets, request correlation, and six-digit LAN pairing.
- The Stream Deck plugin v0 is the core deck only: position-independent boot/handoff, six separately placeable character slots, window-number swaps, broadcast, configurable mapped Hotkey tiles, a Stonemite setup key with connection status, automatic loopback, LAN pairing, and honest error states.
- A Hotkey tile can name, icon, and color one mapped EQ action and target all loaded boxes, the active box, every background box, or stable box numbers. User-authored EverQuest socials own grouping, following, assisting, burns, camps, healing, and other multi-step gameplay policy. Dynamic target modes select recipients but do not synthesize commands or substitute character names.
- An input result proves delivery to the intended process, not an observed spell, target, buff, or combat result. The interface must never claim otherwise.
- Client IDs are opaque and valid only for the current Stonemite run. Reconnect state is authoritative.
- Character, server, and class can be temporarily unknown. Clients can be absent, extra, unactivatable, or not ready for targeted input.
- LAN transport is authenticated `ws://`, not encrypted. It is intended only for a trusted private network.

## Brand commitments

- The desktop app and Stream Deck plugin share the product name **Stonemite**.
- UI copy uses sentence case.
- The approved visual reference is `target/tmp/stonemite-stream-deck-demo.html`: 72×72 key artwork, restrained dark utility surfaces, Stonemite slot colors and number badges, class identity, direct state feedback, and solid red broadcast-on treatment.
- The Stream Deck plugin should reuse the existing Stonemite app and class assets rather than inventing a separate game-themed brand.

## Evidence on hand

- `target/tmp/stonemite-stream-deck-demo.html` demonstrates the intended key composition and state language.
- `crates/stonemite/assets/app.png` and `crates/stonemite/assets/class_icons/` contain the real product and class artwork.
- `crates/stonemite/src/overlay.rs` contains the six slot and badge palettes used by Stonemite.
- `crates/trushar/README.md` is the normative integration contract.
- `crates/trushar/tests/control.rs` and `crates/trushar/tests/network.rs` exercise the state/control and transport seams.
- There are no user testimonials, performance claims, or observed in-game outcome signals to present.

## Product principles

1. Show what Stonemite knows; never imply game state it cannot observe.
2. Make exact-client control obvious and recoverable at a glance.
3. Connect locally without setup, pair LAN clients once, reconnect automatically, and treat startup order as irrelevant.
4. Preserve one roster and one identity system across the desktop overlay and deck.
5. Keep gameplay controls one-to-one with existing EQ key mappings; multi-step policy belongs in user-authored in-game socials.

## Accessibility & inclusion

- Key meaning must not rely on color alone; use number, name, class, border, and text state together.
- Property-inspector controls must be keyboard accessible and expose explicit labels, progress, success, and error copy.
- Motion must be bounded and nonessential; steady-state information cannot depend on animation.

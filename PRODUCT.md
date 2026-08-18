# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Stack

- Stonemite: Rust workspace targeting Windows 10+.
- Ikkinz: Node.js 24, TypeScript, Elgato Stream Deck SDK, Rollup, Vitest, and `ws` for authenticated LAN WebSocket upgrades.

## Users

EverQuest multiboxers who run several EQ clients through Stonemite and want a compact, glanceable control surface on Stream Deck hardware or Stream Deck Mobile.

The first Ikkinz tester runs Stonemite on the Windows PC `jaggedpine` and Stream Deck Desktop on a Mac paired to a 5×3 Stream Deck Mobile device. Ikkinz therefore connects to Stonemite across a trusted private LAN.

## Product purpose

Stonemite makes a six-client EverQuest setup visible and controllable without repeatedly navigating Windows. Ikkinz extends the same live character identity and narrowly bounded controls onto Stream Deck.

Ikkinz v0 succeeds when the user can pair once, see up to six current clients and their readiness at a glance, activate an exact client, explicitly enable or disable broadcast, survive either app reconnecting, and understand failures without looking at logs.

## Positioning

Stonemite derives character identity from the running EQ clients and controls an exact loaded process through a small semantic API. Ikkinz reflects that live state rather than asking the user to maintain a second manual roster or sending blind desktop hotkeys.

## Operating context

- Stonemite v0.5.0 runs on Windows with **Integrations** enabled and **Devices on my local network** selected.
- Stonemite displays a `hostname.local:port` address and a single-use six-digit pairing code.
- Stream Deck Desktop runs the Ikkinz Node plugin. Stream Deck Mobile is only the paired key surface; it does not connect to Stonemite directly.
- The intended steady-state layout is 5 columns × 3 rows.
- Users may start Stonemite, Stream Deck, the Mac, and the mobile device in any order, so reconnection is normal rather than exceptional.

## Capabilities and constraints

- Protocol v1 provides complete pushed state, exact client activation, explicit broadcast state, exact-client text/key delivery, request correlation, and six-digit LAN pairing.
- Ikkinz v0 is the core deck only: boot/handoff, live character tiles, exact activation, broadcast, connection/group/input status, pairing, and honest error states.
- Semantic actions such as Assist, Follow, Burn, Camp, heals, and spells are deliberately outside v0 because their EQ commands, hotkeys, timing, and target policy are not yet defined.
- An input result proves delivery to the intended process, not an observed spell, target, buff, or combat result. The interface must never claim otherwise.
- Client IDs are opaque and valid only for the current Stonemite run. Reconnect state is authoritative.
- Character, server, and class can be temporarily unknown. Clients can be absent, extra, unactivatable, or not ready for targeted input.
- LAN transport is authenticated `ws://`, not encrypted. It is intended only for a trusted private network.
- A 15-key layout requires Stream Deck Mobile Pro.

## Brand commitments

- Product names are **Stonemite** and **Ikkinz**.
- UI copy uses sentence case.
- The approved visual reference is `target/tmp/stonemite-stream-deck-demo.html`: 72×72 key artwork, restrained dark utility surfaces, Stonemite slot colors and number badges, class identity, direct state feedback, and solid red broadcast-on treatment.
- Ikkinz should reuse the existing Stonemite app and class assets rather than inventing a separate game-themed brand.

## Evidence on hand

- `target/tmp/stonemite-stream-deck-demo.html` demonstrates the intended key composition and state language.
- `app/assets/app.png` and `app/assets/class_icons/` contain the real product and class artwork.
- `app/src/overlay.rs` contains the six slot and badge palettes used by Stonemite.
- `docs/trushar-protocol.md` is the normative integration contract.
- `trushar/tests/control.rs` and `trushar/tests/network.rs` exercise the state/control and transport seams.
- There are no user testimonials, performance claims, or observed in-game outcome signals to present.

## Product principles

1. Show what Stonemite knows; never imply game state it cannot observe.
2. Make exact-client control obvious and recoverable at a glance.
3. Pair once, reconnect automatically, and treat startup order as irrelevant.
4. Preserve one roster and one identity system across the desktop overlay and deck.
5. Add semantic actions only when their commands and failure policy are explicit.

## Accessibility & inclusion

- Key meaning must not rely on color alone; use number, name, class, border, and text state together.
- Property-inspector controls must be keyboard accessible and expose explicit labels, progress, success, and error copy.
- Motion must be bounded and nonessential; steady-state information cannot depend on animation.

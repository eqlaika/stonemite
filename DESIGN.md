---
name: Stonemite / Stream Deck
description: A direct, glanceable instrument panel for exact EverQuest client control.
colors:
  key-deep: "#171a1f"
  key-high: "#20242b"
  key-black: "#080a0d"
  empty-gray: "#3c424b"
  ambient-gray: "#3d4650"
  boot-line: "#38414b"
  pi-canvas: "#24262a"
  pi-surface: "#30343a"
  pi-line: "#4a5059"
  pi-inset: "#1c1e22"
  text: "#f5f7fa"
  key-muted: "#a7b0bb"
  pi-muted: "#b8c0ca"
  key-quiet: "#77818d"
  active-white: "#ffffff"
  on-accent: "#101615"
  setup-muted: "#9ba5b2"
  server-olive: "#9bbf73"
  link-cyan: "#59d8d0"
  ready-green: "#80df89"
  recovery-amber: "#ffc75c"
  error-coral: "#ff826f"
  error-copy: "#ffc6bd"
  broadcast-red: "#cc3020"
  broadcast-off: "#cc5040"
  broadcast-unavailable: "#6d737c"
  broadcast-soft: "#ffd6cf"
  slot-blue: "#4a86d4"
  slot-green: "#6ab060"
  slot-rose: "#d85858"
  slot-amber: "#e0b848"
  slot-orchid: "#a07cc8"
  slot-teal: "#58c8a8"
  badge-blue: "#3068b0"
  badge-green: "#489040"
  badge-rose: "#b83838"
  badge-amber: "#c09828"
  badge-orchid: "#805ca8"
  badge-teal: "#38a888"
typography:
  key-number:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "20px"
    fontWeight: 800
    lineHeight: 1
  key-value:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "18px"
    fontWeight: 800
    lineHeight: 1
  key-label:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "15px"
    fontWeight: 800
    lineHeight: 1
  key-micro:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "15px"
    fontWeight: 750
    lineHeight: 1
  pi-title:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "13px"
    fontWeight: 700
    lineHeight: 1.4
  body:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "12px"
    fontWeight: 400
    lineHeight: 1.4
  pi-help:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1.4
  pi-privacy:
    fontFamily: "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif"
    fontSize: "10px"
    fontWeight: 400
    lineHeight: 1.4
rounded:
  key: "7px"
  control: "8px"
  panel: "12px"
spacing:
  micro: "5px"
  key-gap: "6px"
  sm: "8px"
  control-gap: "10px"
  panel: "12px"
  field: "14px"
  section: "16px"
  status: "18px"
components:
  button-primary:
    backgroundColor: "{colors.link-cyan}"
    textColor: "{colors.on-accent}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    height: "34px"
    width: "100%"
  input:
    backgroundColor: "{colors.pi-inset}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.key}"
    padding: "6px 9px"
    height: "34px"
    width: "100%"
  status-panel:
    backgroundColor: "{colors.pi-surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.panel}"
    padding: "12px"
    width: "100%"
  key-tile:
    backgroundColor: "{colors.key-deep}"
    textColor: "{colors.text}"
    typography: "{typography.key-label}"
    rounded: "{rounded.key}"
    height: "72px"
    width: "72px"
---

# Design System: Stonemite / Stream Deck

## Overview

**Creative North Star: "The exact-client instrument panel"**

The exact-client instrument panel is a dark, compact utility system that turns Stonemite's observed client state into direct, glanceable controls. It feels like trustworthy system instrumentation rather than fantasy-game chrome: identity is vivid, state is explicit, and decoration never outruns what the software knows.

Across the Stonemite overlay, Stream Deck keys, and the property inspector, the system uses the same slot colors, number badges, class identity, system UI typography, and restrained tonal depth. It does not present delivered input as an observed in-game result, and it does not invent targets, buffs, combat outcomes, or other game state.

**Key Characteristics:**

- Compact dark utility surfaces with high-contrast text.
- Exact identity through slot number, character name, class, and color together.
- State communicated with words, shapes, borders, and color rather than color alone.
- Restrained motion that confirms transitions without carrying steady-state meaning.

## Colors

Charcoal surfaces keep the interface quiet while cyan, green, amber, and coral report link and operation state; six paired slot-and-badge hues preserve identity between the desktop overlay and deck.

### Primary

- **Link cyan** (`colors.link-cyan`): The connection accent, input caret, Swap action color, selection color, primary property-inspector action, focus outline, and boot/link signal.

### Secondary

- **Ready green** (`colors.ready-green`): Connected, input-ready, complete, and success states plus the Follow action.
- **Recovery amber** (`colors.recovery-amber`): Pairing, connecting, reconnecting, pending, unsupported-layout, and not-ready states.
- **Error coral** (`colors.error-coral`): Failed feedback, connection errors, and invalid-field borders. Error copy uses the softer `colors.error-copy` on dark property-inspector surfaces.
- **Server olive** (`colors.server-olive`): The detected-server utility accent; it is informational rather than a readiness signal.

### Tertiary

- **Slots 1–6** (`colors.slot-blue`, `colors.slot-green`, `colors.slot-rose`, `colors.slot-amber`, `colors.slot-orchid`, `colors.slot-teal`): Full identity surfaces in fixed Stonemite order. Group reuses slot blue, Assist slot amber, and Use slot orchid so action colors stay within the established palette.
- **Badges 1–6** (`colors.badge-blue`, `colors.badge-green`, `colors.badge-rose`, `colors.badge-amber`, `colors.badge-orchid`, `colors.badge-teal`): Darker circular number fields paired one-to-one with the slot surfaces.

### Neutral

- **Key depth** (`colors.key-high` to `colors.key-deep`): The restrained diagonal tonal gradient behind utility, boot, broadcast-off, and feedback keys. `colors.key-black` veils unavailable clients and backs class-code fallbacks.
- **Key structure** (`colors.empty-gray`, `colors.ambient-gray`, `colors.boot-line`): Empty and reserved dark surfaces plus quiet boot separators.
- **Property-inspector depth** (`colors.pi-canvas`, `colors.pi-surface`, `colors.pi-inset`): Canvas, status/help panels, and inset fields. One-pixel `colors.pi-line` borders separate controls without adding shadow.
- **Text hierarchy** (`colors.text`, `colors.key-muted`, `colors.pi-muted`, `colors.key-quiet`): Primary, secondary, and tertiary information. `colors.active-white` fills the active-client tile, dark `colors.on-accent` labels that light surface and sits on cyan fills, and `colors.setup-muted` marks setup or unknown utility states.

### Named Rules

**The Broadcast-Red Rule.** Solid broadcast red is reserved for confirmed broadcast-on state; broadcast-off and unavailable states stay on dark tiles. The stable “Bcast” label does not change, so the full-surface inversion carries the toggle state without adding copy.

**The Redundancy Rule.** Color never carries identity, readiness, progress, success, or failure by itself; pair it with a number, name, class, border, icon, or explicit status text.

## Typography

**Display Font:** System UI (with -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif)
**Body Font:** System UI (with -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif)

**Character:** A single native UI stack keeps the surfaces immediate and legible across Windows, macOS, Stream Deck, and Stream Deck Mobile. Weight, size, alignment, and short labels create hierarchy without a decorative display face.

### Hierarchy

- **Key number** (800, 20px, 1): Heavy centered slot numerals inside the circular badges.
- **Key value** (800, 15–18px, 1): Short central values stay large; longer text fits horizontally without dropping below the 15px floor.
- **Key label** (750–850, 15px minimum, 1): Uppercase deck copy never renders smaller than the standard character-name size. Long strings fit horizontally within the 72px canvas instead of reducing font size.
- **Action label** (800, 15px, 1): A single large title-case verb—“Group,” “Follow,” “Assist,” “Use,” “Swap,” or “Bcast”—or a user-configured Hotkey name is anchored below the action icon. Hotkey adds only a compact `ALL` or stable-number target summary.
- **Character name** (800, 15px, 1): Centered near the bottom edge; long names fit horizontally without shrinking below the deck-wide floor.
- **Property-inspector title** (700, 13px, 1.4): The leading status statement.
- **Property-inspector body** (400–700, 10–12px, 1.4): Labels, help, actions, instructions, privacy copy, and explicit errors. User-facing copy remains sentence case.

### Named Rules

**The Two-Registers Rule.** Deck action labels and property-inspector copy use sentence case; compact status microcopy may remain uppercase for scan speed.

## Layout

Stream Deck key art is authored at exactly 72 × 72 pixels. Character slots 1–6, Group, Follow, Assist, Use, Bcast, Swap, configurable Hotkey, and Stonemite setup are separate, position-independent actions. No preset profile currently ships; users place and duplicate the actions they want, while Hotkey remains in the action list until configured.

Dark key surfaces have no colored top rails. Character keys use a 13px-radius badge centered at (17, 18), a 29 × 29 class image at the top right when known, a bottom-centered name, and a 3px readiness dot at the lower right. The active character replaces its slot gradient with an active-white surface, keeps a 3px slot-color outline, switches its labels to dark ink, and states “ACTIVE” explicitly. Missing clients and unavailable controls remain in place rather than collapsing the grid.

Stonemite setup and Hotkey own focused property inspectors. Both are compact single columns with 12px outer padding and a 260px minimum width. Setup identifies **This PC** or **LAN** and keeps cross-PC pairing behind one disclosure. Hotkey keeps targeting, shared mapped-action search, a 14-character tile name, Stonemite color swatches plus a custom color picker, and its searchable Lucide Animated picker in one linear form. Full-width 34px fields and buttons, explicit labels, one-pixel separators, and the established 5–18px spacing rhythm keep both inspectors consistent.

## Elevation & Depth

The production system is flat and tonal: key gradients move only from `colors.key-high` to `colors.key-deep`, while property-inspector panels and inset fields step through three charcoal levels. Icons, type, and state color provide structure without decorative top rails. The approved browser study adds ambient shadows only to simulate physical Stream Deck hardware; those preview shadows are not production surface tokens.

### Named Rules

**The Tonal-First Rule.** Build hierarchy with adjacent charcoal levels, one-pixel lines, icon color, and type; do not add decorative shadows, top rails, or glass effects to production controls.

## Shapes

Corners are compact and consistent: Stream Deck artwork and property-inspector inputs use the key radius (7px), buttons and Stonemite overlay labels use the control radius (8px), and property-inspector status and help panels use the panel radius (12px). Number badges and status dots are true circles. Active-client and focus treatments follow the existing silhouette instead of changing it.

## Components

### Buttons

- **Shape:** Full-width primary and paired secondary actions are at least 34px high with the control radius.
- **Primary:** Link-cyan fill, dark on-accent text, 700 weight, and a cyan border; its shipped hover fill is `#78e4de`.
- **Secondary:** Property-inspector surface fill with a one-pixel line border; hover moves to `#3a3f46`. The destructive quiet action uses `#ffd0c9` text without becoming a filled danger button.
- **Focus / Disabled:** Keyboard focus is a 2px link-cyan outline with a 2px offset. Disabled controls retain their label, use 0.55 opacity, and show a wait cursor while work is in progress.

### Inputs / Fields

- **Style:** Full-width 34px inset fields use a one-pixel line border, the key radius, 6px × 9px padding, tabular numerals, and a link-cyan caret.
- **Focus:** The same 2px link-cyan outline and 2px offset used by buttons.
- **Error / Disabled:** Invalid fields change the border to error coral, set `aria-invalid`, and expose adjacent role-alert copy. Busy fields are disabled; a paired LAN connection replaces the pairing form with its saved address and management actions.

### Status Panels

- **Style:** The property-inspector connection panel is a three-column status header with a 9px dot, explicit title and detail, and a compact **This PC** or **LAN** badge. It uses 12px padding and the panel radius.
- **State:** Muted means idle, ready green means connected, recovery amber means pairing/connecting/reconnecting, and error coral means error. The dot always accompanies live title and detail text.
- **Recovery and disclosure:** A compact tonal recovery panel appears only while idle, reconnecting, or failed. LAN setup and management use one native disclosure separated by one-pixel lines; privacy copy remains inside that disclosed LAN context.

### Key Tiles

- **Base grammar:** Every tile is a 72px square with the key radius. Non-character tiles use the restrained key gradient with no colored top rail and deck copy no smaller than 15px; long labels fit horizontally instead of shrinking.
- **Identity:** Character tiles combine the fixed slot surface, darker number badge, class icon or explicit code fallback, character name, readiness dot, and optional unavailable veil. The active tile becomes active-white with dark labels, a slot-color outline, and an explicit “ACTIVE” marker.
- **Bcast:** Off uses the dark base, broadcast-off icon, and one “Bcast” label; unavailable switches to broadcast-unavailable and says “UNAVAILABLE.” On replaces the entire surface with solid broadcast red while keeping the white lightning mark and the same “Bcast” label.
- **Group / Follow / Assist / Use:** Actions use vendored Lucide Animated geometry—Users Round, Route, Target, and Mouse Pointer Click—above one large “Group,” “Follow,” “Assist,” or “Use” label. Group is blue, Follow is green, Assist is gold, and Use is orchid. Idle tiles keep a dark surface with the action color on the icon; while an action is running, the entire tile fills with its action color and the icon and fixed action word invert to dark ink. Ready-box counts and lower qualifiers remain omitted for hardware-scale legibility.
- **Window-number swap:** Lucide’s Arrow Left Right sits above one large “Swap” label in both idle and armed modes. Swap is cyan: idle keeps a dark surface with the cyan icon, while armed mode fills the tile cyan, inverts its icon and word to dark ink, staggers the two source arrow motions, and temporarily relabels character tiles as “CURRENT” or “SELECT.”
- **Configurable Hotkey:** An unconfigured tile uses an amber Keyboard icon and “Configure.” A configured tile uses the chosen Lucide Animated icon in its per-tile color, the user’s name below it, and `ALL`, `1–6`, or selected stable numbers at upper right. Running fills the tile with that color, animates the authored icon, and automatically chooses white or dark icon/text content by contrast. Missing or unready targets preserve the configured icon/name but replace the target summary with an explicit uppercase status; command-time unbound and delivery failures use the normal error language.
- **Stonemite setup:** The setup action centers the real Stonemite app artwork on the standard dark key surface while connected. Pairing and connection recovery add an amber or coral dot plus explicit 15px status copy. Pressing it never triggers a command; selecting it in Stream Deck opens the one plugin-wide connection inspector.
- **Feedback / Error:** Pending Group, Follow, Assist, Use, and Hotkey actions preserve their large action word while filling with the relevant action color and animating in dark ink. Other pending and error states replace only the operated key with recovery amber or error coral iconography plus 15px “WORKING” or “FAILED” copy and a fitted 15px message. Empty character actions show only their slot number in a centered 13px-radius circle filled with Stonemite’s notification blue-gray (`#203040`).

### Motion and Accessibility States

- **Boot:** Visible non-setup actions advance through synchronized, position-independent static boot frames at 0ms, 160ms, 620ms, and 1050ms; motion is a bounded handoff into authoritative live state, not a source of information. Setup remains on its real app artwork and adds explicit connection status when recovery is needed.
- **Feedback:** Group, Follow, Assist, Use, and Hotkey redraw one filled active 72px key at 8fps only while their bounded input operation is running. Hotkey uses its configured fill plus a contrast-selected white or dark foreground and selects from the complete pinned 466-icon Lucide Animated catalog, precompiled into eight Stream Deck-safe SVG frames. Armed Swap uses the same filled-state treatment and animation budget until the user picks a character or cancels. The large action word remains stable throughout motion.
- **Connection:** Pairing, connecting, and reconnecting dots breathe over 1.2s with `ease-in-out`; reduced-motion preference collapses the animation to 1ms.
- **Access:** Property-inspector fields have explicit labels and described-by help/error relationships; status regions are live, busy state is exposed, errors use role alerts, and steady-state meaning remains available without animation or color perception.

## Do's and Don'ts

### Do:

- Do reuse the shipped Stonemite app and class artwork; use the class-code fallback only when identity artwork is unavailable.
- Do preserve the 72px key grammar, fixed slot/badge pairings, and exact-client labels wherever the plugin renders deck state.
- Do show unavailable, unknown, reconnecting, pending, success, and error states with explicit words and redundant visual cues.
- Do reserve animation for bounded boot, progress, and feedback transitions, with a reduced-motion path.

### Don't:

- Don't imply targets, buffs, spells, combat outcomes, or other game state Stonemite does not observe.
- Don't use solid broadcast red for any state except confirmed broadcast on.
- Don't introduce a separate fantasy-game palette, invented icons, or decorative assets when Stonemite identity already exists.
- Don't rely on color alone, hide failures in logs, or replace actionable error copy with a generic warning mark.
- Don't make an action's behavior depend on its physical row or column; no preset profile dictates the user's arrangement.

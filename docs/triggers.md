# EQLP Trigger Support Plan

## 1. Scope contract

### Included

- Round-trip EQLP:
  - `.tgf.gz` trigger packages
  - `.ogf.gz` overlay packages
- GINA `.gtp` import
- Case-insensitive text and practical `.NET` regex compatibility
- `{S}`, `{N}`, numeric constraints, `{TS}`, named captures, and substitutions
- Immediate previous-line matching
- Match-variable expressions
- Stateful values, counters, TTLs, lockouts, `{COUNTER}`, and `{REPEATED}`
- Full timer lifecycle:
  - countdown, fast countdown, progress, looping
  - restart/deduplication modes
  - warning, normal end, and early-end actions
- Managed WAV/MP3 sounds
- Windows system TTS voices with rate, volume, priority, and interruption
- Text and timer overlays with reusable presets
- Profiles assigned globally or to selected characters/clients
- Source, active-client, all-client, and global presentation targeting
- Full manager and test bench

### Excluded

- Webhooks, chat sending, clipboard sharing, Quick Share networking
- Commands, scripts, keyboard/mouse actions, or gameplay automation
- Piper TTS initially
- Pixel-identical WPF rendering
- NAG database import

Unsupported EQLP fields will be retained for re-export but never executed.

## 2. Architecture

### Portable trigger domain

Create `crates/eqtrigger/` containing:

- Versioned native schema and migrations
- Trigger/profile/category/preset models
- EQLP and GINA codecs
- Pattern macro expansion
- Regex compatibility adapter
- Condition parser/evaluator
- Variable and counter state
- Lockout/repeat handling
- Virtual-clock timer state machine
- Action generation
- Test-bench tracing

This keeps compatibility logic platform-independent and thoroughly testable outside Windows.

### Persistence

Store triggers separately from `config.toml`:

```text
%APPDATA%\Stonemite\triggers\
├── library.json
├── assets\
└── backups\
```

The store will use stable UUIDs, schema versions, strict validation, atomic replacement, backups, and per-record quarantine. A malformed trigger must not reset the entire library.

A native `.stonemite-triggers` ZIP will contain selected triggers, dependent presets, and content-addressed assets. EQLP exports cannot embed media, so they will reference managed asset paths and warn about portability.

### Runtime integration

Replace the prototype in `crates/stonemite/src/log_watcher/triggers.rs` with an `eqtrigger` adapter.

The log worker will:

1. Receive immutable compiled-library snapshots.
2. Evaluate active profiles synchronously per source line.
3. Produce presentation-only activations.
4. Hot-reload after the existing `WM_SETTINGS_CHANGED` path without restarting.

No trigger executor will have access to input broadcasting or control APIs.

### Presentation services

Add independent dispatchers for:

- Audio/TTS queue
- Text overlay entries
- Multiple concurrent timer entries
- Warning/end/early-end actions

The current timer implementation only consumes `StartTimer` and displays one applicable timer. It will become a multi-entry overlay model with deterministic ordering, replacement, expiry, and per-client scope.

## 3. Compatibility policy

Target current EQLP 2.3.60 behavior, with these rules:

- Preserve unknown JSON fields during import/export.
- Import overlays before resolving trigger overlay references.
- Report dangling overlays and missing media instead of silently dropping them.
- Use current EQLP’s case-insensitive runtime behavior.
- Support common `.NET` regex constructs through a bounded compatibility layer.
- Quarantine unsupported constructs such as balancing groups rather than activating a changed expression.
- Enforce match/backtracking limits against pathological patterns.
- Follow documented intent rather than obvious EQLP defects—for example, real 750 ms repeat resets and true-only GINA booleans.
- Import packages disabled by default after showing a compatibility report; users can enable them during confirmation.

## 4. Trigger Manager UX

Add a top-level **Triggers** destination in `packages/desktop/src/App.tsx`. It will use a full-bleed workbench rather than the existing narrow `SettingsPage`.

### Layout

- **Left:** profiles and category tree
- **Center:** searchable, virtualized trigger list with selection and status
- **Right:** detailed editor
- **Bottom drawer:** test bench and trace

At the current 900 px window width, it becomes master/detail with a collapsible tree. Wider windows show all three panes. The 560 px minimum remains usable through drawers and single-pane navigation.

### Editor sections

- Match
- Previous-line requirement
- Conditions
- Initial actions
- Timer and retrigger behavior
- Warning/end/early-end actions
- Variables and counters
- Targeting
- Presentation preset and overrides
- Notes and compatibility diagnostics

### Library operations

- Search, filters, and result counts
- Create, duplicate, rename, reorder, move, delete
- Multi-select and bulk enable/disable/move/delete
- Import preview with merge/new-folder/conflict choices
- Export trigger/category/profile/full-library scopes
- Keyboard alternatives and announcements for every drag operation
- Reusable audio, text-overlay, and timer-overlay preset management

### Test bench

- Paste timestamped EQ log lines
- Immediate or real-time replay
- Select a simulated character/profile
- Show match spans, captures, previous-line result, condition result, variable mutations, lockout state, and generated actions
- Preview overlays virtually
- Play sound/TTS only after an explicit preview action
- Optional live-tail mode using an independent bounded log tailer

## 5. Implementation milestones

1. **Compatibility fixtures and schema**
   - Capture representative EQLP/GINA packages.
   - Define native schema, field mappings, limits, and compatibility reports.

2. **Storage and interchange**
   - Add trigger store, migrations, managed assets, EQLP round-trip, GINA import, and native packages.

3. **Matching and state engine**
   - Implement macros, regex adapter, substitutions, conditions, variables, repeats, lockouts, timers, and deterministic traces.

4. **Runtime integration**
   - Load active profiles into the log worker, hot-reload safely, and dispatch immutable actions.

5. **Audio and native overlays**
   - Add WAV/MP3 playback, Windows TTS, queue priority, text overlays, and multi-timer overlays.

6. **Trigger Manager**
   - Add the full library workspace, presets, import/export dialogs, targeting, and test bench.

7. **Hardening and documentation**
   - Import limits, ZIP traversal protection, regex resource limits, recovery, accessibility, compatibility documentation, and changelog updates.

Each milestone will be built and reviewed before beginning the next.

## 6. Verification

- Golden EQLP and GINA import fixtures
- Export/re-import semantic equivalence
- Virtual-clock timer lifecycle tests
- Multi-client targeting and profile isolation tests
- Regex timeout and malformed-package tests
- Audio queue and missing-voice fallback tests
- React interaction/accessibility tests
- Local `eqtrigger` and desktop test suites
- Windows-host `just build` and native Rust tests
- Manual Windows validation for:
  - WAV/MP3 and TTS
  - concurrent timers
  - multiple EQ clients
  - DPI and monitor changes
  - live reload
  - large trigger libraries

## Principal risks

1. `.NET` regex cannot be reproduced perfectly in Rust; unsupported expressions need explicit quarantine.
2. EQLP packages are unversioned and omit profile/enable state.
3. Native multi-overlay rendering is substantially larger than the existing single-timer scaffold.
4. Audio/TTS ordering and device changes require dedicated Windows testing.

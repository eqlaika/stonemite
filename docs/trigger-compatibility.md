# Trigger compatibility

Stonemite's trigger system targets current EQLP (EQLogParser) 2.3.x
behavior and imports GINA shares. This document records exactly what is
compatible, what is approximated, and what is deliberately different.
The implementation lives in `crates/eqtrigger` (platform-independent) with
runtime integration in `crates/stonemite`.

## Supported interchange formats

| Format | Read | Write | Notes |
| --- | --- | --- | --- |
| EQLP triggers `.tgf.gz` | ✔ | ✔ | gzip JSON `ExportTriggerNode[]`, PascalCase |
| EQLP overlays `.ogf.gz` | ✔ | ✔ | same container, overlay records |
| GINA `.gtp` | ✔ | — | ZIP holding `ShareData.xml` |
| Stonemite `.stonemite-triggers` | ✔ | ✔ | ZIP: native manifest + embedded media |

Imports always arrive **disabled** and show a compatibility report first;
users enable them during confirmation. Unknown JSON fields on EQLP
triggers and overlays are preserved verbatim (per-record passthrough) and
merged back on export, so a round-trip keeps fields Stonemite never
executes.

## Executed EQLP behavior

- Case-insensitive matching everywhere (text contains and regex), matching
  EQLP's `OrdinalIgnoreCase` / `RegexOptions.IgnoreCase` runtime.
- GINA macros in regex patterns: `{S}`/`{S1}`–`{S9}` → `(?<S1>.+)`;
  `{N}`, `{N>=50}`, `{50<N<100}` → `(?<N>\d+)` plus numeric constraints;
  `{TS}` → duration capture that also supplies a dynamic timer duration
  (countdown and progress timers only, like EQLP).
- Immediate previous-line requirement (text or regex, with captures).
- Match-variable conditions with EQLP's grammar and semantics:
  `=`, `!=`, `<>`, `<`, `<=`, `>`, `>=`, `contains`, `and/or/not`, word
  operators (`eq`, `gte`, …), `{var}` truthiness, null checks, unset
  variables comparing as 0 numerically, invalid expressions blocking the
  trigger entirely.
- Variable actions: set value (captures → previous captures → `{l}` →
  variables resolution order), counters (seeded from a numeric value
  variable, else the initial value, then stepped), clear, and TTLs that
  expire on the engine clock when a trigger matches.
- Substitution codes: `{c}`, `{l}`, `{counter}`, `{repeated}`,
  `{logtime}`, `{null}`, `{timer-warn-time-value}`, plus token modifiers
  `.upper`, `.lower`, `.capitalize`, `.number`, `.padleft:n`,
  `.padright:n`, `.center:n`. Unresolved tokens stay verbatim.
- Timer lifecycle: countdown, fast countdown, progress, looping
  (`TimesToLoop` repeats), the five restart/deduplication modes
  (`TriggerAgainOption` 0–4), warnings at N seconds before end, normal
  end and early-end stages with EQLP's blank-early-end fallback to the
  end stage, up to three end-early patterns (with match values
  substituted at timer start), the repeated-count early ender, reset/
  cooldown durations, and end-of-timer variable clearing.
- Lockouts (`LockoutTime`) suppress refires; looping repeats bypass them
  as in EQLP.
- Sound-vs-speak selection: a `SoundToPlay` ending in `.wav`/`.mp3` wins
  over the speak text; `{null}` suppresses output.
- Audio priority: lower values interrupt queued and in-flight speech.
  Voice rate 0 uses the character/system default; N > 0 maps to SAPI
  rate N−1. The EQLP volume code (4 = no change) maps to a percentage
  reduction; increases beyond 100% are not possible with SAPI and clamp.

## Deliberate deviations (documented intent over defects)

- **Real 750 ms repeated resets.** EQLP truncates the elapsed time to
  whole seconds before comparing with `RepeatedResetTime` (default
  0.75 s), which effectively makes the window one second. Stonemite
  compares in milliseconds.
- **True-only GINA booleans.** EQLP's GINA importer enables features via
  `bool.TryParse(...)`, which treats a literal `False` as enabling.
  Stonemite honors only `true`.
- **Regex budget instead of a wall-clock timeout.** EQLP disables a
  trigger whose regex exceeds 50 ms. Stonemite compiles through a
  backtracking engine with an explicit backtrack budget (comparable cost
  ceiling, deterministic) and disables the trigger per character when the
  budget is exceeded.

## Not executed (retained for re-export)

Webhooks (`ChatWebhook`, `TextToSendToChat`), clipboard sharing
(`TextToShare`), Quick Share networking, commands/scripts, keyboard or
mouse actions, and gameplay automation are never executed. The fields are
kept in each trigger's passthrough and re-export unchanged; the import
report calls them out. There is intentionally no code path from trigger
evaluation to input broadcasting or control APIs.

## .NET regex compatibility

Common constructs translate directly: named groups (`(?<n>…)`),
backreferences (`\1`, `\k<n>`), lookahead/lookbehind, atomic groups,
inline `i`/`m`/`s`/`x` options, `\A`/`\z`, and `\Z` (approximated as
`\z`, identical for single log lines).

Unsupported constructs are **quarantined**, never reinterpreted:
balancing groups (`(?<a-b>…)`), conditionals (`(?(…)…)`), character-class
subtraction (`[a-[b]]`), possessive quantifiers (`*+`), and the `(?n)`
explicit-capture option. A quarantined trigger imports, displays its
reason in the manager, never activates, and re-exports its original
pattern byte-for-byte.

Limits: patterns over 4096 characters are quarantined; each match attempt
has a 250,000-step backtracking budget.

## Storage

```text
%APPDATA%\Stonemite\triggers\
├── library.json      versioned native schema (currently v1)
├── assets\           managed WAV/MP3 media (content-addressed names)
├── backups\          rotated copies (10) written before each save
└── quarantine\       records that failed validation, preserved as files
```

Saves are atomic (temp file + rename) with backup rotation. Loading is
salvage-oriented: a malformed record moves to `quarantine\` and the rest
of the library loads; a fully corrupt file falls back to the newest
readable backup. A malformed trigger never resets the library.

`.stonemite-triggers` packages embed managed media with SHA-256 digests;
import verifies digests, enforces entry-count/size limits, and rejects
ZIP path traversal (`..`, absolute paths, drive letters, backslashes).

## EQLP field mapping

Native fields map 1:1 onto these EQLP `Trigger` fields; everything else
passes through untouched:

`Comments`, `Pattern`/`UseRegex`, `PreviousPattern`/`PreviousUseRegex`,
`MatchVariableCondition`, `LockoutTime`, `RepeatedResetTime`,
`VariableActions`, `TextToDisplay`, `TextToSpeak`, `SoundToPlay`,
`Priority`, `VoiceRate`, `Volume`, `SelectedOverlays`, `FontColor`,
`ActiveColor`, `IdleColor`, `ResetColor`, `EnableTimer`, `TimerType`,
`DurationSeconds`, `ResetDurationSeconds`, `TimesToLoop`,
`TriggerAgainOption`, `WarningSeconds`, `AltTimerName`, the nine
warning/end/early-end display/speak/sound fields, `EndEarlyPattern`(2, 3)
with `EndUseRegex`(2, 3), `EndEarlyRepeatedCount`, and
`EndTimerClearVariables`.

Legacy exports with `EnableTimer: true, TimerType: 0` import as countdown
timers, matching EQLP's migration.

EQLP packages are unversioned and carry no profile or enable state; on
export those Stonemite concepts are simply omitted, and on import
triggers arrive disabled with no profile membership.

## Known gaps

- Piper TTS is not integrated (Windows SAPI voices only).
- Overlay rendering is not pixel-identical to EQLP's WPF windows; the
  timer/text overlay models are multi-entry with deterministic ordering,
  and the native renderer currently shows the most urgent timer per
  client label plus a banner for trigger text.
- The test bench replays pasted lines (immediate or timestamp-paced); the
  optional live-tail mode is not implemented yet.
- NAG database import is out of scope.

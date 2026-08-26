# eqlog

Reusable, platform-neutral EverQuest log parsing and telemetry primitives.

`eqlog` is the semantic core extracted from Stonemite. It is suitable for other passive EQ log tools, including EQLP-style dashboards, notifications, trigger frontends, analytics, and read-only telemetry services.

It provides:

- canonical `RawLogLine` records with application-defined source attribution;
- EQ timestamp/body decoding;
- an extensible parser registry;
- typed `/who` identity, pet ownership, incoming-Tell chat, invitation and trade lifecycles, resurrection, death, level, and AA-point events;
- persistent, case-insensitive per-character telemetry reduction;
- explicit reset behavior for truncated, recreated, or removed sources.

Trade lifecycle coverage recognizes EQ's incoming `is interested in making a trade` line and its named, system, and self-cancellation lines.

It deliberately does **not** provide filesystem watching, file offsets, async runtimes, presentation, networking, or gameplay input. The host application owns those policies and supplies complete newline-terminated records in source order.

```rust
use eqlog::{LogSource, ParserRegistry, RawLogLine, TelemetryReducer};

let source = LogSource::new("client-1", "Bilka", "teek");
let line = RawLogLine::decode(
    source,
    b"[Wed Mar 25 11:15:35 2026] Players in EverQuest:",
)
.line;

let mut parsers = ParserRegistry::default();
let mut telemetry = TelemetryReducer::new();
let parsed = parsers.parse(&line);
for event in &parsed.events {
    let _change = telemetry.apply(event);
}
```

Applications should retain unmatched `RawLogLine` values if they support user-defined text or regex triggers. Unknown lines are normal and are not parser errors.

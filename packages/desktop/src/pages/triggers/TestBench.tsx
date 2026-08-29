import { Play, Volume2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import {
  Button,
  CheckboxOption,
  InlineStatus,
} from "../../components/Controls";
import { previewTriggerSound, runTriggerTest } from "../../triggers/api";
import type {
  ActionEvent,
  TestBenchLine,
  TestBenchResult,
} from "../../triggers/types";

const SAMPLE = `[Tue Aug 25 21:10:03 2026] A giant rat bites YOU for 12 points of damage.
[Tue Aug 25 21:10:05 2026] Kafka begins to cast a spell.
[Tue Aug 25 21:10:08 2026] Kafka tells you, 'incoming!'`;

export function TestBench() {
  const [lines, setLines] = useState(SAMPLE);
  const [character, setCharacter] = useState("Tester");
  const [server, setServer] = useState("");
  const [includeDisabled, setIncludeDisabled] = useState(true);
  const [realtime, setRealtime] = useState(false);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<TestBenchResult | null>(null);
  const [revealed, setRevealed] = useState(0);
  const replayTimer = useRef<number | null>(null);

  useEffect(
    () => () => {
      if (replayTimer.current !== null)
        window.clearTimeout(replayTimer.current);
    },
    [],
  );

  async function run() {
    setRunning(true);
    setError(null);
    setResult(null);
    if (replayTimer.current !== null) window.clearTimeout(replayTimer.current);
    try {
      const outcome = await runTriggerTest({
        lines: lines.split("\n").filter((line) => line.trim().length > 0),
        character,
        server,
        includeDisabled,
      });
      setResult(outcome);
      if (realtime && outcome.lines.length > 0) {
        setRevealed(0);
        const reveal = (index: number) => {
          setRevealed(index + 1);
          const next = outcome.lines[index + 1];
          if (next) {
            const delay = Math.min(
              5_000,
              Math.max(50, next.atMs - outcome.lines[index].atMs),
            );
            replayTimer.current = window.setTimeout(
              () => reveal(index + 1),
              delay,
            );
          }
        };
        reveal(0);
      } else {
        setRevealed(outcome.lines.length);
      }
    } catch (problem) {
      setError(String(problem));
    } finally {
      setRunning(false);
    }
  }

  const visibleLines = result ? result.lines.slice(0, revealed) : [];
  const lastTimers =
    visibleLines.length > 0
      ? visibleLines[visibleLines.length - 1].timersAfter
      : [];

  return (
    <section className="tw-bench" aria-label="Test bench">
      <div className="tw-bench-config">
        <textarea
          rows={5}
          value={lines}
          aria-label="Test log lines"
          placeholder="Paste timestamped EQ log lines…"
          onChange={(event) => setLines(event.target.value)}
        />
        <div className="tw-bench-controls">
          <label>
            <span>Character</span>
            <input
              value={character}
              onChange={(event) => setCharacter(event.target.value)}
            />
          </label>
          <label>
            <span>Server</span>
            <input
              value={server}
              onChange={(event) => setServer(event.target.value)}
            />
          </label>
          <CheckboxOption
            label="Include disabled triggers"
            checked={includeDisabled}
            onChange={setIncludeDisabled}
          />
          <CheckboxOption
            label="Real-time replay"
            checked={realtime}
            onChange={setRealtime}
          />
          <Button
            variant="primary"
            disabled={running}
            onClick={() => void run()}
          >
            <Play size={14} aria-hidden="true" />
            {running ? "Running…" : "Run"}
          </Button>
        </div>
      </div>

      {error ? (
        <InlineStatus tone="error" title="The test run failed">
          {error}
        </InlineStatus>
      ) : null}
      {result ? (
        <div className="tw-bench-results">
          <p className="tw-bench-summary" aria-live="polite">
            {result.activeTriggers} trigger(s) evaluated.
            {result.compileErrors.length > 0
              ? ` ${result.compileErrors.length} failed to compile.`
              : ""}
          </p>
          {result.compileErrors.map((compileError, index) => (
            <InlineStatus key={index} tone="warning" title="Compile problem">
              {compileError}
            </InlineStatus>
          ))}
          <div className="tw-bench-columns">
            <ol className="tw-bench-lines">
              {visibleLines.map((line, index) => (
                <BenchLine key={index} line={line} />
              ))}
            </ol>
            <aside
              className="tw-bench-overlay"
              aria-label="Virtual overlay preview"
            >
              <h4>Timers after last line</h4>
              {lastTimers.length === 0 ? (
                <p>No running timers.</p>
              ) : (
                <ul>
                  {lastTimers.map((timer, index) => (
                    <li key={index}>
                      <strong>{timer.displayName}</strong>{" "}
                      {(
                        (timer.endMs -
                          visibleLines[visibleLines.length - 1].atMs) /
                        1000
                      ).toFixed(1)}
                      s remaining ({timer.kind})
                    </li>
                  ))}
                </ul>
              )}
            </aside>
          </div>
        </div>
      ) : null}
    </section>
  );
}

function BenchLine({ line }: { line: TestBenchLine }) {
  const matched = line.trace.entries.filter((entry) => entry.matched);
  return (
    <li className={matched.length > 0 ? "tw-bench-line hit" : "tw-bench-line"}>
      <code>{line.line}</code>
      {line.trace.entries.map((entry, index) => (
        <div key={index} className="tw-bench-entry">
          <span className={entry.matched ? "tw-badge hit" : "tw-badge"}>
            {entry.triggerName}
          </span>
          {!entry.matched ? <span> did not match</span> : null}
          {entry.lockoutBlocked ? <span> · locked out</span> : null}
          {entry.previousLineMatched !== null ? (
            <span>
              {" "}
              · previous line {entry.previousLineMatched ? "matched" : "failed"}
            </span>
          ) : null}
          {entry.conditionPassed !== null ? (
            <span>
              {" "}
              · condition {entry.conditionPassed ? "passed" : "failed"}
            </span>
          ) : null}
          {entry.captures.length > 0 ? (
            <span className="tw-captures">
              {" "}
              {entry.captures
                .map(([name, value]) => `{${name}}=${value}`)
                .join(" ")}
            </span>
          ) : null}
          {entry.variableMutations.length > 0 ? (
            <span className="tw-captures">
              {" "}
              {entry.variableMutations
                .map(([name, value]) =>
                  value === null ? `clear ${name}` : `${name}←${value}`,
                )
                .join(" ")}
            </span>
          ) : null}
        </div>
      ))}
      {line.events.map((event, index) => (
        <BenchEvent key={index} event={event} />
      ))}
    </li>
  );
}

function BenchEvent({ event }: { event: ActionEvent }) {
  const phase =
    event.phase === "initial"
      ? ""
      : ` (${event.phase.replace("timer", "timer ")})`;
  switch (event.action.kind) {
    case "displayText":
      return (
        <div className="tw-bench-action">
          🗨 {event.action.text}
          {phase}
        </div>
      );
    case "speak":
      return (
        <div className="tw-bench-action">
          🔈 “{event.action.text}”{phase}
        </div>
      );
    case "playSound": {
      const sound = event.action.sound;
      return (
        <div className="tw-bench-action">
          ♪ {sound}
          {phase}{" "}
          <button
            type="button"
            className="tw-icon-button"
            aria-label={`Play ${sound}`}
            onClick={() => void previewTriggerSound(sound)}
          >
            <Volume2 size={12} aria-hidden="true" />
          </button>
        </div>
      );
    }
  }
}

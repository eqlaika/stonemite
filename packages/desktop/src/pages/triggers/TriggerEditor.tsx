import { Play, Plus, Trash2 } from "lucide-react";

import {
  Button,
  CheckboxOption,
  InlineStatus,
} from "../../components/Controls";
import { previewTriggerSound } from "../../triggers/api";
import { defaultTimer, emptyPattern } from "../../triggers/model";
import type {
  BuiltinSoundOption,
  Pattern,
  PresentationTarget,
  TimerBehavior,
  TimerKind,
  TimerRestartMode,
  TimerStageActions,
  Trigger,
  TriggerLibrary,
  VariableAction,
  VariableOp,
} from "../../triggers/types";

const TARGET_OPTIONS: [PresentationTarget, string][] = [
  ["source", "Source client"],
  ["activeClient", "Active client"],
  ["allClients", "All clients"],
  ["global", "Global"],
];

const TIMER_KINDS: [TimerKind, string][] = [
  ["countdown", "Countdown"],
  ["fastCountdown", "Fast countdown"],
  ["progress", "Progress"],
  ["looping", "Looping"],
];

const RESTART_MODES: [TimerRestartMode, string][] = [
  ["startNew", "Start an additional timer"],
  ["restartAll", "Restart every timer for this trigger"],
  ["restartSameName", "Restart the timer with the same name"],
  ["ignoreIfAnyRunning", "Do nothing while any timer runs"],
  ["ignoreIfSameNameRunning", "Do nothing while the same name runs"],
];

export function TriggerEditor({
  library,
  trigger,
  builtinSounds,
  onChange,
}: {
  library: TriggerLibrary;
  trigger: Trigger;
  builtinSounds: BuiltinSoundOption[];
  onChange: (trigger: Trigger) => void;
}) {
  const set = <K extends keyof Trigger>(key: K, value: Trigger[K]) =>
    onChange({ ...trigger, [key]: value });

  const soundOptions = [
    ...library.assets.map((asset) => asset.name),
    ...builtinSounds.map((sound) => sound.id),
  ];

  return (
    <div className="tw-editor-form">
      <section>
        <h3>Trigger</h3>
        <div className="tw-grid">
          <label>
            <span>Name</span>
            <input
              value={trigger.name}
              onChange={(event) => set("name", event.target.value)}
            />
          </label>
          <label>
            <span>Show on</span>
            <select
              value={trigger.target}
              onChange={(event) =>
                set("target", event.target.value as PresentationTarget)
              }
            >
              {TARGET_OPTIONS.map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <CheckboxOption
          label={
            trigger.quarantine
              ? "Enabled (unavailable while quarantined)"
              : "Enabled"
          }
          checked={trigger.enabled}
          disabled={trigger.quarantine !== null}
          onChange={(checked) => set("enabled", checked)}
        />
        {trigger.quarantine ? (
          <InlineStatus tone="warning" title="Quarantined">
            <p>
              {trigger.quarantine.detail} The original definition is kept and
              will re-export unchanged.
            </p>
          </InlineStatus>
        ) : null}
      </section>

      <section>
        <h3>Match</h3>
        <PatternEditor
          label="Pattern"
          pattern={trigger.pattern}
          onChange={(pattern) => set("pattern", pattern)}
        />
        <details open={trigger.previousPattern !== null}>
          <summary>Previous-line requirement</summary>
          {trigger.previousPattern ? (
            <>
              <PatternEditor
                label="Previous line"
                pattern={trigger.previousPattern}
                onChange={(pattern) => set("previousPattern", pattern)}
              />
              <Button onClick={() => set("previousPattern", null)}>
                Remove requirement
              </Button>
            </>
          ) : (
            <Button onClick={() => set("previousPattern", emptyPattern())}>
              Require the previous line to match
            </Button>
          )}
        </details>
        <label>
          <span>Condition (match-variable expression)</span>
          <input
            value={trigger.condition}
            placeholder="e.g. {S1} != null and {stacks} >= 3"
            onChange={(event) => set("condition", event.target.value)}
          />
        </label>
        <div className="tw-grid">
          <label>
            <span>Lockout (seconds)</span>
            <input
              type="number"
              min={0}
              step={0.1}
              value={trigger.lockoutSeconds}
              onChange={(event) =>
                set("lockoutSeconds", Number(event.target.value) || 0)
              }
            />
          </label>
          <label>
            <span>{"{repeated}"} reset window (seconds)</span>
            <input
              type="number"
              min={0}
              step={0.05}
              value={trigger.repeatedResetSeconds}
              onChange={(event) =>
                set("repeatedResetSeconds", Number(event.target.value) || 0)
              }
            />
          </label>
        </div>
      </section>

      <section>
        <h3>Initial actions</h3>
        <label>
          <span>Display text</span>
          <input
            value={trigger.displayText ?? ""}
            placeholder="Shown on the text overlay; {S1}, {c}, {counter}…"
            onChange={(event) => set("displayText", event.target.value || null)}
          />
        </label>
        <label>
          <span>Speak</span>
          <input
            value={trigger.speakText ?? ""}
            placeholder="Spoken unless a sound is set"
            onChange={(event) => set("speakText", event.target.value || null)}
          />
        </label>
        <SoundPicker
          label="Sound"
          value={trigger.sound}
          options={soundOptions}
          onChange={(sound) => set("sound", sound)}
        />
        <div className="tw-grid">
          <label>
            <span>Audio priority (lower interrupts)</span>
            <input
              type="number"
              min={1}
              max={10}
              value={trigger.priority}
              onChange={(event) =>
                set("priority", Number(event.target.value) || 3)
              }
            />
          </label>
          <label>
            <span>Voice rate (0 = default)</span>
            <input
              type="number"
              min={0}
              max={11}
              value={trigger.voiceRate}
              onChange={(event) =>
                set("voiceRate", Number(event.target.value) || 0)
              }
            />
          </label>
        </div>
      </section>

      <section>
        <h3>Timer</h3>
        {trigger.timer ? (
          <TimerEditor
            timer={trigger.timer}
            soundOptions={soundOptions}
            onChange={(timer) => set("timer", timer)}
            onRemove={() => set("timer", null)}
          />
        ) : (
          <Button onClick={() => set("timer", defaultTimer())}>
            Add a timer
          </Button>
        )}
      </section>

      <section>
        <h3>Variables and counters</h3>
        <VariableActionsEditor
          actions={trigger.variableActions}
          onChange={(variableActions) =>
            set("variableActions", variableActions)
          }
        />
      </section>

      <section>
        <h3>Presentation</h3>
        <OverlaySelection
          label="Text overlays"
          all={library.textOverlays.map((preset) => [preset.id, preset.name])}
          selected={trigger.textOverlays}
          onChange={(textOverlays) => set("textOverlays", textOverlays)}
        />
        <OverlaySelection
          label="Timer overlays"
          all={library.timerOverlays.map((preset) => [preset.id, preset.name])}
          selected={trigger.timerOverlays}
          onChange={(timerOverlays) => set("timerOverlays", timerOverlays)}
        />
        <label>
          <span>Font color override</span>
          <input
            value={trigger.fontColor ?? ""}
            placeholder="#AARRGGBB"
            onChange={(event) => set("fontColor", event.target.value || null)}
          />
        </label>
      </section>

      <section>
        <h3>Notes</h3>
        <textarea
          rows={3}
          value={trigger.comments}
          aria-label="Comments"
          onChange={(event) => set("comments", event.target.value)}
        />
        {Object.keys(trigger.passthrough).length > 0 ? (
          <p className="tw-passthrough-note">
            {Object.keys(trigger.passthrough).length} imported field(s) are
            retained for re-export but never executed (
            {Object.keys(trigger.passthrough).slice(0, 5).join(", ")}
            …).
          </p>
        ) : null}
      </section>
    </div>
  );
}

function PatternEditor({
  label,
  pattern,
  onChange,
}: {
  label: string;
  pattern: Pattern;
  onChange: (pattern: Pattern) => void;
}) {
  return (
    <div className="tw-pattern">
      <label>
        <span>{label}</span>
        <input
          value={pattern.text}
          placeholder={
            pattern.useRegex
              ? "^{S1} begins to cast {S2}\\.$"
              : "text the line must contain"
          }
          onChange={(event) =>
            onChange({ ...pattern, text: event.target.value })
          }
        />
      </label>
      <CheckboxOption
        label="Regular expression ({S1}, {N>=50}, {TS} macros expand)"
        checked={pattern.useRegex}
        onChange={(useRegex) => onChange({ ...pattern, useRegex })}
      />
    </div>
  );
}

function SoundPicker({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: string | null;
  options: string[];
  onChange: (value: string | null) => void;
}) {
  return (
    <div className="tw-sound-picker">
      <label>
        <span>{label}</span>
        <select
          value={value ?? ""}
          onChange={(event) => onChange(event.target.value || null)}
        >
          <option value="">None (use Speak text)</option>
          {options.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
      </label>
      {value ? (
        <button
          type="button"
          className="tw-icon-button"
          aria-label={`Preview ${value}`}
          onClick={() => void previewTriggerSound(value)}
        >
          <Play size={13} aria-hidden="true" />
        </button>
      ) : null}
    </div>
  );
}

function StageEditor({
  title,
  stage,
  soundOptions,
  onChange,
}: {
  title: string;
  stage: TimerStageActions;
  soundOptions: string[];
  onChange: (stage: TimerStageActions) => void;
}) {
  return (
    <fieldset className="tw-stage">
      <legend>{title}</legend>
      <label>
        <span>Display</span>
        <input
          value={stage.displayText ?? ""}
          onChange={(event) =>
            onChange({ ...stage, displayText: event.target.value || null })
          }
        />
      </label>
      <label>
        <span>Speak</span>
        <input
          value={stage.speakText ?? ""}
          onChange={(event) =>
            onChange({ ...stage, speakText: event.target.value || null })
          }
        />
      </label>
      <SoundPicker
        label="Sound"
        value={stage.sound}
        options={soundOptions}
        onChange={(sound) => onChange({ ...stage, sound })}
      />
    </fieldset>
  );
}

function TimerEditor({
  timer,
  soundOptions,
  onChange,
  onRemove,
}: {
  timer: TimerBehavior;
  soundOptions: string[];
  onChange: (timer: TimerBehavior) => void;
  onRemove: () => void;
}) {
  const set = <K extends keyof TimerBehavior>(
    key: K,
    value: TimerBehavior[K],
  ) => onChange({ ...timer, [key]: value });

  return (
    <div className="tw-timer-editor">
      <div className="tw-grid">
        <label>
          <span>Type</span>
          <select
            value={timer.kind}
            onChange={(event) => set("kind", event.target.value as TimerKind)}
          >
            {TIMER_KINDS.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Timer name (empty = trigger name)</span>
          <input
            value={timer.timerName}
            placeholder="{S1} mez"
            onChange={(event) => set("timerName", event.target.value)}
          />
        </label>
        <label>
          <span>Duration (seconds)</span>
          <input
            type="number"
            min={0}
            step={0.1}
            value={timer.durationSeconds}
            onChange={(event) =>
              set("durationSeconds", Number(event.target.value) || 0)
            }
          />
        </label>
        <label>
          <span>Reset / cooldown (seconds)</span>
          <input
            type="number"
            min={0}
            step={0.1}
            value={timer.resetDurationSeconds}
            onChange={(event) =>
              set("resetDurationSeconds", Number(event.target.value) || 0)
            }
          />
        </label>
        <label>
          <span>When it fires again</span>
          <select
            value={timer.restartMode}
            onChange={(event) =>
              set("restartMode", event.target.value as TimerRestartMode)
            }
          >
            {RESTART_MODES.map(([value, label]) => (
              <option key={value} value={value}>
                {label}
              </option>
            ))}
          </select>
        </label>
        {timer.kind === "looping" ? (
          <label>
            <span>Times to loop</span>
            <input
              type="number"
              min={0}
              value={timer.timesToLoop}
              onChange={(event) =>
                set("timesToLoop", Number(event.target.value) || 0)
              }
            />
          </label>
        ) : null}
        <label>
          <span>Warn before end (seconds, 0 = off)</span>
          <input
            type="number"
            min={0}
            value={timer.warningSeconds}
            onChange={(event) =>
              set("warningSeconds", Number(event.target.value) || 0)
            }
          />
        </label>
      </div>

      {timer.warningSeconds > 0 ? (
        <StageEditor
          title="Warning actions"
          stage={timer.warning}
          soundOptions={soundOptions}
          onChange={(warning) => set("warning", warning)}
        />
      ) : null}
      <StageEditor
        title="Normal end actions"
        stage={timer.end}
        soundOptions={soundOptions}
        onChange={(end) => set("end", end)}
      />
      <StageEditor
        title="Early-end actions (falls back to normal end)"
        stage={timer.earlyEnd}
        soundOptions={soundOptions}
        onChange={(earlyEnd) => set("earlyEnd", earlyEnd)}
      />

      <fieldset className="tw-stage">
        <legend>End early when a line matches</legend>
        {timer.endEarlyPatterns.map((pattern, index) => (
          <div key={index} className="tw-ender-row">
            <PatternEditor
              label={`Pattern ${index + 1}`}
              pattern={pattern}
              onChange={(next) =>
                set(
                  "endEarlyPatterns",
                  timer.endEarlyPatterns.map((existing, other) =>
                    other === index ? next : existing,
                  ),
                )
              }
            />
            <button
              type="button"
              className="tw-icon-button"
              aria-label={`Remove end-early pattern ${index + 1}`}
              onClick={() =>
                set(
                  "endEarlyPatterns",
                  timer.endEarlyPatterns.filter((_, other) => other !== index),
                )
              }
            >
              <Trash2 size={13} aria-hidden="true" />
            </button>
          </div>
        ))}
        {timer.endEarlyPatterns.length < 3 ? (
          <Button
            onClick={() =>
              set("endEarlyPatterns", [
                ...timer.endEarlyPatterns,
                emptyPattern(),
              ])
            }
          >
            <Plus size={14} aria-hidden="true" /> Add end-early pattern
          </Button>
        ) : null}
        <label>
          <span>Also end after N repeats (0 = off)</span>
          <input
            type="number"
            min={0}
            value={timer.endEarlyRepeatedCount}
            onChange={(event) =>
              set("endEarlyRepeatedCount", Number(event.target.value) || 0)
            }
          />
        </label>
        <label>
          <span>Clear variables at end (comma-separated)</span>
          <input
            value={timer.endClearVariables.join(", ")}
            onChange={(event) =>
              set(
                "endClearVariables",
                event.target.value
                  .split(",")
                  .map((name) => name.trim())
                  .filter(Boolean),
              )
            }
          />
        </label>
      </fieldset>
      <Button onClick={onRemove}>Remove timer</Button>
    </div>
  );
}

function VariableActionsEditor({
  actions,
  onChange,
}: {
  actions: VariableAction[];
  onChange: (actions: VariableAction[]) => void;
}) {
  const update = (index: number, action: VariableAction) =>
    onChange(
      actions.map((existing, other) => (other === index ? action : existing)),
    );

  return (
    <div className="tw-variables">
      {actions.map((action, index) => (
        <div key={index} className="tw-variable-row">
          <select
            aria-label={`Variable action ${index + 1} type`}
            value={action.op}
            onChange={(event) =>
              update(index, { ...action, op: event.target.value as VariableOp })
            }
          >
            <option value="setValue">Set value</option>
            <option value="setCounter">Counter</option>
            <option value="clear">Clear</option>
          </select>
          <input
            aria-label={`Variable ${index + 1} name`}
            placeholder="name"
            value={action.name}
            onChange={(event) =>
              update(index, { ...action, name: event.target.value })
            }
          />
          {action.op === "setValue" ? (
            <input
              aria-label={`Variable ${index + 1} value`}
              placeholder="{S1} or literal"
              value={action.value}
              onChange={(event) =>
                update(index, { ...action, value: event.target.value })
              }
            />
          ) : null}
          {action.op === "setCounter" ? (
            <input
              aria-label={`Variable ${index + 1} step`}
              type="number"
              step={1}
              title="Step"
              value={action.step}
              onChange={(event) =>
                update(index, {
                  ...action,
                  step: Number(event.target.value) || 1,
                })
              }
            />
          ) : null}
          {action.op !== "clear" ? (
            <input
              aria-label={`Variable ${index + 1} time to live seconds`}
              type="number"
              min={0}
              title="TTL seconds (0 = forever)"
              value={action.timeToLiveSeconds}
              onChange={(event) =>
                update(index, {
                  ...action,
                  timeToLiveSeconds: Number(event.target.value) || 0,
                })
              }
            />
          ) : null}
          <button
            type="button"
            className="tw-icon-button"
            aria-label={`Remove variable action ${index + 1}`}
            onClick={() =>
              onChange(actions.filter((_, other) => other !== index))
            }
          >
            <Trash2 size={13} aria-hidden="true" />
          </button>
        </div>
      ))}
      <Button
        onClick={() =>
          onChange([
            ...actions,
            {
              op: "setValue",
              name: "",
              value: "",
              step: 1,
              initialValue: 0,
              timeToLiveSeconds: 0,
            },
          ])
        }
      >
        <Plus size={14} aria-hidden="true" /> Add variable action
      </Button>
    </div>
  );
}

function OverlaySelection({
  label,
  all,
  selected,
  onChange,
}: {
  label: string;
  all: [string, string][];
  selected: string[];
  onChange: (ids: string[]) => void;
}) {
  if (all.length === 0) {
    return (
      <p className="tw-overlay-note">
        {label}: no presets yet — create them in the Presets panel.
      </p>
    );
  }
  return (
    <fieldset className="tw-overlays">
      <legend>{label}</legend>
      {all.map(([id, name]) => (
        <CheckboxOption
          key={id}
          label={name || "(unnamed preset)"}
          checked={selected.includes(id)}
          onChange={(checked) =>
            onChange(
              checked
                ? [...selected, id]
                : selected.filter((other) => other !== id),
            )
          }
        />
      ))}
    </fieldset>
  );
}

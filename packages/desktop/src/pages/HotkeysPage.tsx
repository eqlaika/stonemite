import { ArrowLeft, ArrowRight, Plus, Repeat2, Trash2 } from "lucide-react";
import { useId, useMemo } from "react";

import { Button, Field, FormSection, TextInput } from "../components/Controls";
import { HotkeyCapture } from "../components/HotkeyCapture";
import { SettingsPage } from "../components/SettingsPage";
import { useSettings } from "../settings/SettingsContext";
import type { BoxCycle, BoxIdentity } from "../settings/types";
import "./HotkeysPage.css";

const WINDOW_SLOTS = Array.from({ length: 6 }, (_, index) => index);
const MAX_BOX_CYCLES = 16;

function identityKey(identity: BoxIdentity): string {
  return `${identity.server.trim().toLowerCase()}\u001f${identity.character
    .trim()
    .toLowerCase()}`;
}

interface CycleCandidate {
  identity: BoxIdentity;
  boxNumber: number | null;
}

export function cycleMemberCandidates(
  boxOrder: BoxIdentity[],
  cycles: BoxCycle[],
): CycleCandidate[] {
  const seen = new Set<string>();
  const candidates: CycleCandidate[] = [];
  boxOrder.forEach((identity, index) => {
    const key = identityKey(identity);
    if (seen.has(key)) return;
    seen.add(key);
    candidates.push({ identity, boxNumber: index + 1 });
  });
  for (const cycle of cycles) {
    for (const identity of cycle.members) {
      const key = identityKey(identity);
      if (seen.has(key)) continue;
      seen.add(key);
      candidates.push({ identity, boxNumber: null });
    }
  }
  return candidates;
}

function unusedCycleName(cycles: BoxCycle[]): string {
  const names = new Set(cycles.map((cycle) => cycle.name.trim().toLowerCase()));
  let number = 1;
  while (names.has(`cycle ${number}`)) number += 1;
  return `Cycle ${number}`;
}

interface BoxCyclesEditorProps {
  value: BoxCycle[];
  boxOrder: BoxIdentity[];
  onChange: (value: BoxCycle[]) => void;
}

export function BoxCyclesEditor({
  value,
  boxOrder,
  onChange,
}: BoxCyclesEditorProps) {
  const editorId = useId();
  const candidates = useMemo(
    () => cycleMemberCandidates(boxOrder, value),
    [boxOrder, value],
  );

  const updateCycle = (index: number, cycle: BoxCycle) => {
    onChange(
      value.map((candidate, candidateIndex) =>
        candidateIndex === index ? cycle : candidate,
      ),
    );
  };

  const addCycle = () => {
    onChange([
      ...value,
      {
        name: unusedCycleName(value),
        nextHotkey: "",
        previousHotkey: "",
        members: [],
      },
    ]);
  };

  return (
    <>
      <div className="box-cycle-toolbar">
        <p>
          Each press activates one available member. Missing characters are
          skipped automatically.
        </p>
        <Button
          type="button"
          disabled={value.length >= MAX_BOX_CYCLES}
          onClick={addCycle}
        >
          <Plus size={15} aria-hidden="true" />
          Add cycle
        </Button>
      </div>

      {value.length === 0 ? (
        <div className="box-cycle-empty">
          <Repeat2 size={22} aria-hidden="true" />
          <div>
            <strong>No box cycles yet</strong>
            <p>
              Add a named ring for melee positioning, buffs, or any roster you
              move through repeatedly.
            </p>
          </div>
        </div>
      ) : (
        <div className="box-cycle-list">
          {value.map((cycle, cycleIndex) => {
            const nameId = `${editorId}-cycle-${cycleIndex}-name`;
            const memberKeys = new Set(cycle.members.map(identityKey));
            const displayName = cycle.name.trim() || `Cycle ${cycleIndex + 1}`;
            return (
              <article
                className="box-cycle-editor"
                key={cycleIndex}
                aria-label={`Box cycle ${displayName}`}
              >
                <div className="box-cycle-header">
                  <Repeat2 size={18} aria-hidden="true" />
                  <div className="box-cycle-name">
                    <label htmlFor={nameId}>Cycle name</label>
                    <TextInput
                      id={nameId}
                      value={cycle.name}
                      maxLength={64}
                      onChange={(event) =>
                        updateCycle(cycleIndex, {
                          ...cycle,
                          name: event.currentTarget.value,
                        })
                      }
                    />
                  </div>
                  <Button
                    type="button"
                    variant="quiet"
                    className="box-cycle-delete"
                    aria-label={`Delete ${displayName} cycle`}
                    onClick={() =>
                      onChange(
                        value.filter(
                          (_candidate, index) => index !== cycleIndex,
                        ),
                      )
                    }
                  >
                    <Trash2 size={16} aria-hidden="true" />
                  </Button>
                </div>

                <div className="box-cycle-bindings">
                  <div className="box-cycle-binding">
                    <div className="box-cycle-binding-copy">
                      <ArrowRight size={16} aria-hidden="true" />
                      <span>
                        <strong>Next</strong>
                        <small>Advance through the ring</small>
                      </span>
                    </div>
                    <HotkeyCapture
                      value={cycle.nextHotkey}
                      ariaLabel={`Next hotkey for ${displayName}`}
                      onChange={(nextHotkey) =>
                        updateCycle(cycleIndex, { ...cycle, nextHotkey })
                      }
                    />
                  </div>
                  <div className="box-cycle-binding">
                    <div className="box-cycle-binding-copy">
                      <ArrowLeft size={16} aria-hidden="true" />
                      <span>
                        <strong>Previous</strong>
                        <small>Step back after an overshoot</small>
                      </span>
                    </div>
                    <HotkeyCapture
                      value={cycle.previousHotkey}
                      ariaLabel={`Previous hotkey for ${displayName}`}
                      onChange={(previousHotkey) =>
                        updateCycle(cycleIndex, { ...cycle, previousHotkey })
                      }
                    />
                  </div>
                </div>

                <fieldset className="box-cycle-members">
                  <legend>
                    Ring members
                    <span>{cycle.members.length}</span>
                  </legend>
                  <p>
                    Characters follow Box order. Include your driver to make the
                    ring return home.
                  </p>
                  {candidates.length === 0 ? (
                    <div className="box-cycle-members-empty">
                      Add characters to Box order before building this ring.
                    </div>
                  ) : (
                    <div className="box-cycle-member-list">
                      {candidates.map(({ identity, boxNumber }) => {
                        const key = identityKey(identity);
                        const checked = memberKeys.has(key);
                        return (
                          <label className="box-cycle-member" key={key}>
                            <input
                              type="checkbox"
                              checked={checked}
                              onChange={(event) => {
                                const selected = new Set(memberKeys);
                                if (event.currentTarget.checked)
                                  selected.add(key);
                                else selected.delete(key);
                                updateCycle(cycleIndex, {
                                  ...cycle,
                                  members: candidates
                                    .filter((candidate) =>
                                      selected.has(
                                        identityKey(candidate.identity),
                                      ),
                                    )
                                    .map((candidate) => candidate.identity),
                                });
                              }}
                            />
                            <span className="box-cycle-member-number">
                              {boxNumber ?? "—"}
                            </span>
                            <span className="box-cycle-member-identity">
                              <strong>{identity.character}</strong>
                              <small>
                                {boxNumber === null
                                  ? `${identity.server} · not in Box order`
                                  : identity.server}
                              </small>
                            </span>
                          </label>
                        );
                      })}
                    </div>
                  )}
                </fieldset>
              </article>
            );
          })}
        </div>
      )}

      <p className="help-text">
        Keyboard-emulating foot pedals and F13–F24 are supported. Cycle bindings
        do not repeat when a pedal is held.
      </p>
    </>
  );
}

export function HotkeysPage() {
  const { draft, setDraft } = useSettings();
  if (!draft) return null;

  const updateWindowHotkey = (slot: number, value: string) => {
    setDraft((current) => {
      if (!current) return current;
      const swapHotkeys = current.hotkeys.swapHotkeys.slice();
      swapHotkeys[slot] = value;
      return {
        ...current,
        hotkeys: { ...current.hotkeys, swapHotkeys },
      };
    });
  };

  return (
    <SettingsPage
      title="Hotkeys"
      description="Activate exact EverQuest windows or move through a named character ring."
    >
      <FormSection
        title="Box cycles"
        description="Create reusable groups with independent next and previous shortcuts."
      >
        <BoxCyclesEditor
          value={draft.hotkeys.boxCycles}
          boxOrder={draft.boxOrder}
          onChange={(boxCycles) =>
            setDraft((current) =>
              current
                ? {
                    ...current,
                    hotkeys: { ...current.hotkeys, boxCycles },
                  }
                : current,
            )
          }
        />
      </FormSection>

      <FormSection
        title="Swap to window"
        description="Assign a shortcut to each exact EverQuest window slot."
      >
        <div className="window-hotkey-list">
          {WINDOW_SLOTS.map((slot) => (
            <Field key={slot} label={`Window ${slot + 1}`}>
              <HotkeyCapture
                value={draft.hotkeys.swapHotkeys[slot] ?? ""}
                ariaLabel={`Window ${slot + 1} hotkey`}
                onChange={(value) => updateWindowHotkey(slot, value)}
              />
            </Field>
          ))}
        </div>
        <p className="help-text">
          Shortcuts may use Ctrl, Alt, and Shift. Press Escape while capturing
          to keep the current binding.
        </p>
      </FormSection>
    </SettingsPage>
  );
}

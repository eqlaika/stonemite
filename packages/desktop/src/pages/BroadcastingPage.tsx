import { useEffect, useId, useRef, useState } from "react";

import {
  Button,
  Field,
  FormSection,
  InlineStatus,
  SelectInput,
  TextArea,
} from "../components/Controls";
import {
  HotkeyCapture,
  isSupportedConfigKey,
} from "../components/HotkeyCapture";
import { SettingsPage } from "../components/SettingsPage";
import { useSettings } from "../settings/SettingsContext";
import type { BroadcastFilterMode, SettingsDraft } from "../settings/types";
import "./BroadcastingPage.css";

const CANONICAL_KEY_NAMES: Readonly<Record<string, string>> = {
  INSERT: "Insert",
  DELETE: "Delete",
  HOME: "Home",
  END: "End",
  PAGEUP: "PageUp",
  PAGE_UP: "PageUp",
  PAGEDOWN: "PageDown",
  PAGE_DOWN: "PageDown",
  PAUSE: "Pause",
  SCROLLLOCK: "ScrollLock",
  SCROLL_LOCK: "ScrollLock",
  SPACE: "Space",
  TAB: "Tab",
  MINUS: "Minus",
  PLUS: "Plus",
  EQUALS: "Equals",
  BACKTICK: "Backtick",
  OPENBRACKET: "OpenBracket",
  CLOSEBRACKET: "CloseBracket",
  BACKSLASH: "Backslash",
  SEMICOLON: "Semicolon",
  QUOTE: "Quote",
  COMMA: "Comma",
  PERIOD: "Period",
  SLASH: "Slash",
};

function canonicalConfigKey(value: string): string | null {
  const upper = value.trim().toUpperCase();
  if (/^F(?:[1-9]|1\d|2[0-4])$/.test(upper)) return upper;
  if (/^[A-Z0-9]$/.test(upper)) return upper;
  return CANONICAL_KEY_NAMES[upper] ?? null;
}

function hotkeyKey(value: string): string | null {
  const parts = value.split("+");
  return canonicalConfigKey(parts[parts.length - 1] ?? "");
}

function mouseClutchError(draft: SettingsDraft): string | null {
  const binding = draft.broadcasting.mouseClutchKey.trim();
  if (!binding) return null;
  if (binding.includes("+")) {
    return "Mouse Clutch uses one key without modifiers";
  }

  const key = canonicalConfigKey(binding);
  if (!key || !isSupportedConfigKey(key)) {
    return `Mouse Clutch key '${binding}' is not supported`;
  }

  for (const [index, hotkey] of draft.hotkeys.swapHotkeys.entries()) {
    if (hotkeyKey(hotkey) === key) {
      return `Mouse Clutch conflicts with the Window ${index + 1} hotkey (${hotkey})`;
    }
  }
  if (hotkeyKey(draft.pip.hideHotkey) === key) {
    return `Mouse Clutch conflicts with Hide overlay (${draft.pip.hideHotkey})`;
  }
  if (hotkeyKey(draft.broadcasting.toggleHotkey) === key) {
    return `Mouse Clutch conflicts with Broadcast toggle (${draft.broadcasting.toggleHotkey})`;
  }
  return null;
}

function parseFilterKeys(value: string): string[] {
  return value
    .split(",")
    .map((key) => key.trim())
    .filter(Boolean);
}

export function BroadcastingPage() {
  const { draft, setDraft, options, runtime } = useSettings();
  const filterKeysId = useId();
  const currentFilterSignature =
    draft?.broadcasting.filterKeys.join("\u0000") ?? "";
  const lastEditedSignature = useRef(currentFilterSignature);
  const [filterKeysText, setFilterKeysText] = useState(
    draft?.broadcasting.filterKeys.join(", ") ?? "",
  );

  useEffect(() => {
    if (currentFilterSignature !== lastEditedSignature.current) {
      lastEditedSignature.current = currentFilterSignature;
      setFilterKeysText(draft?.broadcasting.filterKeys.join(", ") ?? "");
    }
  }, [currentFilterSignature, draft?.broadcasting.filterKeys]);

  if (!draft || !options) return null;

  const updateBroadcasting = (
    update: (
      broadcasting: SettingsDraft["broadcasting"],
    ) => SettingsDraft["broadcasting"],
  ) => {
    setDraft((current) =>
      current
        ? { ...current, broadcasting: update(current.broadcasting) }
        : current,
    );
  };

  const updateFilterText = (value: string) => {
    const filterKeys = parseFilterKeys(value);
    lastEditedSignature.current = filterKeys.join("\u0000");
    setFilterKeysText(value);
    updateBroadcasting((broadcasting) => ({ ...broadcasting, filterKeys }));
  };

  const clutchError = mouseClutchError(draft);
  const trusikEnabled = runtime?.trusikEnabled ?? false;

  return (
    <SettingsPage
      title="Broadcasting"
      description="Keyboard broadcasting, Mouse Clutch, and background-key filtering."
    >
      <FormSection
        title="Broadcast toggle hotkey"
        description="Toggle key broadcasting on or off."
      >
        <Field label="Shortcut">
          <HotkeyCapture
            value={draft.broadcasting.toggleHotkey}
            ariaLabel="Broadcast toggle hotkey"
            onChange={(toggleHotkey) =>
              updateBroadcasting((broadcasting) => ({
                ...broadcasting,
                toggleHotkey,
              }))
            }
          />
        </Field>
      </FormSection>

      <FormSection
        title="Mouse Clutch"
        description="Hold one key to send the complete physical mouse to ready background EQ clients."
      >
        {!trusikEnabled ? (
          <div className="broadcasting-inline-status">
            <InlineStatus tone="warning" title="Mouse Clutch unavailable">
              Enable the DirectInput proxy (trusik), then restart Stonemite and
              EverQuest.
            </InlineStatus>
          </div>
        ) : null}
        <p className="help-text">
          Keyboard-emulating foot pedals are supported, including F13–F24.
        </p>
        <Field label="Hold key">
          <div className="mouse-clutch-control">
            <HotkeyCapture
              mode="single"
              value={draft.broadcasting.mouseClutchKey}
              ariaLabel="Mouse Clutch key"
              disabled={!trusikEnabled}
              onChange={(mouseClutchKey) =>
                updateBroadcasting((broadcasting) => ({
                  ...broadcasting,
                  mouseClutchKey,
                }))
              }
            />
            <Button
              variant="quiet"
              disabled={!draft.broadcasting.mouseClutchKey}
              onClick={() =>
                updateBroadcasting((broadcasting) => ({
                  ...broadcasting,
                  mouseClutchKey: "",
                }))
              }
            >
              Clear
            </Button>
          </div>
        </Field>
        {clutchError ? (
          <p className="broadcasting-error" role="alert">
            {clutchError}
          </p>
        ) : null}
        <p className="help-text">
          Defaults to F13; Clear leaves it unbound. Requires matching EQ window
          geometry and DPI.
        </p>
      </FormSection>

      <FormSection
        title="Key filter"
        description="Choose which keys are broadcast to background windows."
      >
        <Field label="Mode" htmlFor="broadcast-filter-mode">
          <SelectInput<BroadcastFilterMode>
            id="broadcast-filter-mode"
            value={draft.broadcasting.filterMode}
            options={options.filterModes}
            onChange={(event) =>
              updateBroadcasting((broadcasting) => ({
                ...broadcasting,
                filterMode: event.currentTarget.value as BroadcastFilterMode,
              }))
            }
          />
        </Field>
        <div className="filter-keys-field">
          <label htmlFor={filterKeysId}>
            Keys <span>(comma-separated, e.g. Enter, Escape, Tab)</span>
          </label>
          <TextArea
            id={filterKeysId}
            rows={3}
            value={filterKeysText}
            onChange={(event) => updateFilterText(event.currentTarget.value)}
          />
        </div>
      </FormSection>
    </SettingsPage>
  );
}

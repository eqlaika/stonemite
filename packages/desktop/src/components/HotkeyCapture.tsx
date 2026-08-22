import { useEffect, useId, useRef, useState } from "react";

import "./HotkeyCapture.css";

type CaptureMode = "hotkey" | "single";

type KeyboardEventLike = Pick<
  KeyboardEvent,
  "key" | "code" | "ctrlKey" | "altKey" | "shiftKey" | "metaKey"
>;

const NAMED_KEYS: Readonly<Record<string, string>> = {
  Insert: "Insert",
  Delete: "Delete",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  Pause: "Pause",
  ScrollLock: "ScrollLock",
  Spacebar: "Space",
  " ": "Space",
  Tab: "Tab",
};

const CODE_KEYS: Readonly<Record<string, string>> = {
  Minus: "Minus",
  Equal: "Equals",
  NumpadAdd: "Plus",
  NumpadSubtract: "Minus",
  Backquote: "Backtick",
  BracketLeft: "OpenBracket",
  BracketRight: "CloseBracket",
  Backslash: "Backslash",
  Semicolon: "Semicolon",
  Quote: "Quote",
  Comma: "Comma",
  Period: "Period",
  Slash: "Slash",
};

const PUNCTUATION_KEYS: Readonly<Record<string, string>> = {
  "-": "Minus",
  "=": "Equals",
  "+": "Plus",
  "`": "Backtick",
  "[": "OpenBracket",
  "]": "CloseBracket",
  "\\": "Backslash",
  ";": "Semicolon",
  "'": "Quote",
  ",": "Comma",
  ".": "Period",
  "/": "Slash",
};

const SUPPORTED_CONFIG_KEYS = new Set([
  ...Array.from({ length: 24 }, (_, index) => `F${index + 1}`),
  "Insert",
  "Delete",
  "Home",
  "End",
  "PageUp",
  "PageDown",
  "Pause",
  "ScrollLock",
  ...Array.from({ length: 26 }, (_, index) =>
    String.fromCharCode("A".charCodeAt(0) + index),
  ),
  ...Array.from({ length: 10 }, (_, index) => String(index)),
  "Space",
  "Tab",
  "Minus",
  "Plus",
  "Equals",
  "Backtick",
  "OpenBracket",
  "CloseBracket",
  "Backslash",
  "Semicolon",
  "Quote",
  "Comma",
  "Period",
  "Slash",
]);

/** Convert a browser keyboard event to the canonical key name used by Rust config. */
export function keyboardEventToConfigKey(
  event: KeyboardEventLike,
): string | null {
  if (/^F(?:[1-9]|1\d|2[0-4])$/.test(event.key)) return event.key;
  if (NAMED_KEYS[event.key]) return NAMED_KEYS[event.key];
  if (/^[a-z]$/i.test(event.key)) return event.key.toUpperCase();
  if (/^[0-9]$/.test(event.key)) return event.key;
  return CODE_KEYS[event.code] ?? PUNCTUATION_KEYS[event.key] ?? null;
}

export function isSupportedConfigKey(value: string): boolean {
  return SUPPORTED_CONFIG_KEYS.has(value);
}

function modifierNames(event: KeyboardEventLike): string[] {
  const modifiers: string[] = [];
  if (event.ctrlKey) modifiers.push("Ctrl");
  if (event.altKey) modifiers.push("Alt");
  if (event.shiftKey) modifiers.push("Shift");
  return modifiers;
}

const CAPTURE_STARTED_EVENT = "stonemite-hotkey-capture-started";

export function HotkeyCapture({
  value,
  onChange,
  ariaLabel,
  mode = "hotkey",
  disabled = false,
}: {
  value: string;
  onChange: (value: string) => void;
  ariaLabel: string;
  mode?: CaptureMode;
  disabled?: boolean;
}) {
  const captureId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [capturing, setCapturing] = useState(false);
  const [prompt, setPrompt] = useState("");

  useEffect(() => {
    if (disabled) setCapturing(false);
  }, [disabled]);

  useEffect(() => {
    if (!capturing) return;
    triggerRef.current?.focus();

    const stopForAnotherCapture = (event: Event) => {
      if ((event as CustomEvent<string>).detail !== captureId) {
        setCapturing(false);
        setPrompt("");
      }
    };

    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopImmediatePropagation();

      if (event.key === "Escape") {
        setCapturing(false);
        setPrompt("");
        return;
      }

      const modifiers = modifierNames(event);
      if (event.metaKey) {
        setPrompt("The Windows key is not supported. Try another combination.");
        return;
      }
      if (mode === "single" && modifiers.length > 0) {
        setPrompt("Modifiers are not allowed. Press one key by itself.");
        return;
      }

      const key = keyboardEventToConfigKey(event);
      if (!key) {
        if (["Control", "Alt", "Shift", "Meta"].includes(event.key)) {
          if (mode === "hotkey" && modifiers.length > 0) {
            setPrompt(`${modifiers.join("+")}+…`);
          }
          return;
        }
        setPrompt("That key is not supported. Try another key.");
        return;
      }

      onChange(mode === "hotkey" ? [...modifiers, key].join("+") : key);
      setCapturing(false);
      setPrompt("");
    };

    window.addEventListener(CAPTURE_STARTED_EVENT, stopForAnotherCapture);
    window.addEventListener("keydown", onKeyDown, true);
    return () => {
      window.removeEventListener(CAPTURE_STARTED_EVENT, stopForAnotherCapture);
      window.removeEventListener("keydown", onKeyDown, true);
    };
  }, [captureId, capturing, mode, onChange]);

  const beginCapture = () => {
    window.dispatchEvent(
      new CustomEvent<string>(CAPTURE_STARTED_EVENT, { detail: captureId }),
    );
    setPrompt("");
    setCapturing(true);
  };

  const cancelCapture = () => {
    setCapturing(false);
    setPrompt("");
    triggerRef.current?.focus();
  };

  const emptyLabel = mode === "single" ? "Unbound" : "None";
  const captureInstruction =
    mode === "single"
      ? "Press one key without modifiers…"
      : "Press a key combination…";

  return (
    <div className="hotkey-capture">
      <div className="hotkey-capture-actions">
        <button
          ref={triggerRef}
          type="button"
          className={`hotkey-capture-trigger${capturing ? " is-capturing" : ""}`}
          disabled={disabled}
          aria-label={ariaLabel}
          aria-pressed={capturing}
          onClick={capturing ? undefined : beginCapture}
        >
          {capturing ? prompt || captureInstruction : value || emptyLabel}
        </button>
        {capturing ? (
          <button
            type="button"
            className="hotkey-capture-cancel"
            onClick={cancelCapture}
          >
            Cancel
          </button>
        ) : null}
      </div>
      <span className="hotkey-capture-help" aria-live="polite">
        {capturing
          ? "Escape cancels without changing the binding."
          : "Click to change"}
      </span>
    </div>
  );
}

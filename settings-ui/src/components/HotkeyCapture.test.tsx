import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { HotkeyCapture, keyboardEventToConfigKey } from "./HotkeyCapture";

function keyboardEvent(key: string, code = key): KeyboardEvent {
  return new KeyboardEvent("keydown", { key, code });
}

describe("keyboardEventToConfigKey", () => {
  it.each([
    ["F13", "F13", "F13"],
    ["F24", "F24", "F24"],
    ["Pause", "Pause", "Pause"],
    ["ScrollLock", "ScrollLock", "ScrollLock"],
    ["PageUp", "PageUp", "PageUp"],
    ["PageDown", "PageDown", "PageDown"],
    ["a", "KeyA", "A"],
    ["7", "Digit7", "7"],
    [" ", "Space", "Space"],
    ["-", "Minus", "Minus"],
    ["+", "Equal", "Equals"],
    ["+", "NumpadAdd", "Plus"],
    ["[", "BracketLeft", "OpenBracket"],
    ["'", "Quote", "Quote"],
    ["/", "Slash", "Slash"],
  ])("converts %s (%s) to %s", (key, code, expected) => {
    expect(keyboardEventToConfigKey(keyboardEvent(key, code))).toBe(expected);
  });

  it("rejects keys that the Rust config does not support", () => {
    expect(keyboardEventToConfigKey(keyboardEvent("ArrowLeft"))).toBeNull();
    expect(keyboardEventToConfigKey(keyboardEvent("Enter"))).toBeNull();
    expect(keyboardEventToConfigKey(keyboardEvent("F25"))).toBeNull();
  });
});

describe("HotkeyCapture", () => {
  it("captures modifiers in Rust config order", () => {
    const onChange = vi.fn();
    render(
      <HotkeyCapture
        value="Ctrl+F1"
        ariaLabel="Window 1 hotkey"
        onChange={onChange}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Window 1 hotkey" }));
    expect(
      screen.getByText("Escape cancels without changing the binding."),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeVisible();

    fireEvent.keyDown(window, {
      key: "F13",
      code: "F13",
      ctrlKey: true,
      altKey: true,
      shiftKey: true,
    });

    expect(onChange).toHaveBeenCalledWith("Ctrl+Alt+Shift+F13");
    expect(
      screen.queryByRole("button", { name: "Cancel" }),
    ).not.toBeInTheDocument();
  });

  it("cancels with Escape and retains the current value", () => {
    const onChange = vi.fn();
    render(
      <HotkeyCapture
        value="Pause"
        ariaLabel="Broadcast toggle hotkey"
        onChange={onChange}
      />,
    );

    const trigger = screen.getByRole("button", {
      name: "Broadcast toggle hotkey",
    });
    fireEvent.click(trigger);
    expect(trigger).toHaveAttribute("aria-pressed", "true");

    fireEvent.keyDown(window, { key: "Escape", code: "Escape" });

    expect(onChange).not.toHaveBeenCalled();
    expect(trigger).toHaveTextContent("Pause");
    expect(trigger).toHaveAttribute("aria-pressed", "false");
  });

  it("requires one unmodified key in single-key mode", () => {
    const onChange = vi.fn();
    render(
      <HotkeyCapture
        mode="single"
        value="F13"
        ariaLabel="Mouse Clutch key"
        onChange={onChange}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Mouse Clutch key" }));
    fireEvent.keyDown(window, {
      key: "F14",
      code: "F14",
      ctrlKey: true,
    });

    expect(onChange).not.toHaveBeenCalled();
    expect(
      screen.getByText("Modifiers are not allowed. Press one key by itself."),
    ).toBeVisible();

    fireEvent.keyDown(window, { key: "F14", code: "F14" });
    expect(onChange).toHaveBeenCalledWith("F14");
  });
});

import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";

import type { BoxIdentity, RunningCharacter } from "../settings/types";
import { BoxOrderEditor } from "./BoxOrderPage";

const initial: BoxIdentity[] = [
  { server: "xegony", character: "Laika" },
  { server: "xegony", character: "Bilka" },
];

const known: BoxIdentity[] = [
  ...initial,
  { server: "xegony", character: "Kafka" },
  { server: "teek", character: "Rook" },
];

const running: RunningCharacter[] = [
  { server: "xegony", character: "Kafka", windowNumber: 3 },
  { server: "teek", character: "Orlov", windowNumber: 1 },
];

afterEach(cleanup);

function EditorHarness() {
  const [value, setValue] = useState(initial);
  return (
    <BoxOrderEditor
      value={value}
      knownCharacters={known}
      runningCharacters={running}
      onChange={setValue}
    />
  );
}

function orderedCharacters(): string[] {
  const list = screen.getByRole("list", { name: "Preferred box order" });
  return within(list)
    .getAllByRole("listitem")
    .map((item) => item.querySelector("strong")?.textContent ?? "");
}

describe("BoxOrderEditor", () => {
  it("reorders and removes identities with accessible controls", () => {
    render(<EditorHarness />);

    fireEvent.click(screen.getByRole("button", { name: "Move Bilka up" }));
    expect(orderedCharacters()).toEqual(["Bilka", "Laika"]);
    expect(screen.getByText("Moved Bilka to box 1.")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Remove Laika" }));
    expect(orderedCharacters()).toEqual(["Bilka"]);
  });

  it("groups running characters first in current box order", () => {
    render(<EditorHarness />);

    fireEvent.focus(
      screen.getByRole("combobox", { name: "Detected character" }),
    );

    const runningGroup = screen.getByRole("group", { name: "Running now" });
    expect(
      within(runningGroup)
        .getAllByRole("option")
        .map((option) => option.textContent),
    ).toEqual(["OrlovteekBox 1", "KafkaxegonyBox 3"]);
    expect(
      within(screen.getByRole("group", { name: "Known characters" })).getByRole(
        "option",
        { name: /Rook/ },
      ),
    ).toBeVisible();
  });

  it("supports keyboard selection", () => {
    render(<EditorHarness />);

    const combobox = screen.getByRole("combobox", {
      name: "Detected character",
    });
    fireEvent.focus(combobox);
    fireEvent.keyDown(combobox, { key: "Enter" });

    expect(combobox).toHaveValue("Orlov — teek");
    expect(
      screen.getByRole("button", { name: "Add detected character" }),
    ).toBeEnabled();
  });

  it("searches and adds detected and manually entered identities", () => {
    render(<EditorHarness />);

    const combobox = screen.getByRole("combobox", {
      name: "Detected character",
    });
    fireEvent.focus(combobox);
    fireEvent.change(combobox, { target: { value: "kaf" } });
    fireEvent.click(screen.getByRole("option", { name: /Kafka.*Box 3/i }));
    fireEvent.click(
      screen.getByRole("button", { name: "Add detected character" }),
    );
    expect(orderedCharacters()).toEqual(["Laika", "Bilka", "Kafka"]);

    fireEvent.change(screen.getByLabelText("Character"), {
      target: { value: " Foo " },
    });
    fireEvent.change(screen.getByLabelText("Server"), {
      target: { value: " Bristlebane " },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Add manually entered character" }),
    );

    expect(orderedCharacters()).toEqual(["Laika", "Bilka", "Kafka", "Foo"]);
    expect(screen.getByText("Bristlebane")).toBeInTheDocument();
  });

  it("rejects duplicate manual identities case-insensitively", () => {
    render(<EditorHarness />);

    fireEvent.change(screen.getByLabelText("Character"), {
      target: { value: "laika" },
    });
    fireEvent.change(screen.getByLabelText("Server"), {
      target: { value: "XEGONY" },
    });

    expect(
      screen.getByRole("button", { name: "Add manually entered character" }),
    ).toBeDisabled();
    expect(
      screen.getByText("That character is already in the preferred order."),
    ).toBeVisible();
  });
});

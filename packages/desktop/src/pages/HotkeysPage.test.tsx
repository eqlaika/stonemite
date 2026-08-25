import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { afterEach, describe, expect, it } from "vitest";

import type { BoxCycle, BoxIdentity } from "../settings/types";
import { BoxCyclesEditor, cycleMemberCandidates } from "./HotkeysPage";

const boxOrder: BoxIdentity[] = [
  { server: "xegony", character: "Tank" },
  { server: "xegony", character: "Rogue" },
  { server: "xegony", character: "Monk" },
];

afterEach(cleanup);

function EditorHarness({ initial = [] }: { initial?: BoxCycle[] }) {
  const [value, setValue] = useState(initial);
  return (
    <>
      <BoxCyclesEditor value={value} boxOrder={boxOrder} onChange={setValue} />
      <output data-testid="cycles-state">{JSON.stringify(value)}</output>
    </>
  );
}

describe("BoxCyclesEditor", () => {
  it("keeps Box order and appends configured members that are no longer listed", () => {
    const candidates = cycleMemberCandidates(boxOrder, [
      {
        name: "Melee",
        nextHotkey: "F14",
        previousHotkey: "F15",
        members: [
          { server: "XEGONY", character: "Rogue" },
          { server: "teek", character: "Bard" },
        ],
      },
    ]);

    expect(
      candidates.map(({ identity, boxNumber }) => [
        identity.character,
        boxNumber,
      ]),
    ).toEqual([
      ["Tank", 1],
      ["Rogue", 2],
      ["Monk", 3],
      ["Bard", null],
    ]);
  });

  it("creates a named cycle with directional bindings and ordered members", () => {
    render(<EditorHarness />);

    expect(screen.getByText("No box cycles yet")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Add cycle" }));

    const name = screen.getByLabelText("Cycle name");
    fireEvent.change(name, { target: { value: "Melee" } });

    fireEvent.click(
      screen.getByRole("button", { name: "Next hotkey for Melee" }),
    );
    fireEvent.keyDown(window, { key: "F14" });
    fireEvent.click(
      screen.getByRole("button", { name: "Previous hotkey for Melee" }),
    );
    fireEvent.keyDown(window, { key: "F15" });

    fireEvent.click(screen.getByRole("checkbox", { name: /Tank/ }));
    fireEvent.click(screen.getByRole("checkbox", { name: /Rogue/ }));

    expect(screen.getByTestId("cycles-state")).toHaveTextContent(
      JSON.stringify([
        {
          name: "Melee",
          nextHotkey: "F14",
          previousHotkey: "F15",
          members: boxOrder.slice(0, 2),
        },
      ]),
    );
  });

  it("removes cycles with an explicit accessible action", () => {
    render(
      <EditorHarness
        initial={[
          {
            name: "Melee",
            nextHotkey: "F14",
            previousHotkey: "F15",
            members: boxOrder.slice(0, 2),
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Delete Melee cycle" }));
    expect(screen.getByText("No box cycles yet")).toBeVisible();
    expect(screen.getByTestId("cycles-state")).toHaveTextContent("[]");
  });
});

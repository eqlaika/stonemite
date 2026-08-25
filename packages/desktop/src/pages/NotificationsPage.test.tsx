import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { NotificationSettings } from "../settings/types";
import { CombatAwarenessControls } from "./NotificationsPage";

const initial: NotificationSettings = {
  visualEnabled: true,
  soundEnabled: true,
  sound: "tell.wav",
  tells: true,
  groupInvites: true,
  raidInvites: true,
  tradeProposals: true,
  resurrections: true,
  deaths: true,
  combatAwarenessEnabled: true,
  combatHitDurationSeconds: 3,
};

function CombatAwarenessHarness() {
  const [value, setValue] = useState(initial);
  return (
    <CombatAwarenessControls
      value={value}
      onChange={(update) => setValue((current) => ({ ...current, ...update }))}
    />
  );
}

describe("CombatAwarenessControls", () => {
  it("updates the hit duration and disables timing when awareness is off", () => {
    render(<CombatAwarenessHarness />);

    const toggle = screen.getByRole("switch", {
      name: "Show combat awareness",
    });
    const duration = screen.getByLabelText("Attack highlight duration");
    expect(toggle).toBeChecked();
    expect(duration).toHaveValue("3");
    expect(duration).toHaveAttribute("min", "0.5");
    expect(duration).toHaveAttribute("max", "10");
    expect(duration).toHaveAccessibleDescription(
      "How long a successful melee or archery hit keeps the red combat frame visible.",
    );

    fireEvent.change(duration, { target: { value: "2.5" } });
    expect(duration).toHaveValue("2.5");
    expect(screen.getByText("2.5 s")).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(toggle).not.toBeChecked();
    expect(duration).toBeDisabled();
  });
});

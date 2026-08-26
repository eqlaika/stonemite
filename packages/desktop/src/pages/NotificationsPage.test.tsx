import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type { NotificationSettings } from "../settings/types";
import {
  CombatAwarenessControls,
  NotificationEventControls,
} from "./NotificationsPage";

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
  levelGains: true,
  aaGains: true,
  aaPointsPerNotification: 1,
  combatAwarenessEnabled: true,
  combatHitDurationSeconds: 3,
};

function NotificationEventHarness() {
  const [value, setValue] = useState(initial);
  return (
    <NotificationEventControls
      value={value}
      onChange={(update) => setValue((current) => ({ ...current, ...update }))}
    />
  );
}

function CombatAwarenessHarness() {
  const [value, setValue] = useState(initial);
  return (
    <CombatAwarenessControls
      value={value}
      onChange={(update) => setValue((current) => ({ ...current, ...update }))}
    />
  );
}

describe("NotificationEventControls", () => {
  it("configures independent level and AA notifications", () => {
    render(<NotificationEventHarness />);

    expect(screen.getByLabelText("Level gains")).toBeChecked();
    const aaToggle = screen.getByLabelText("AA point gains");
    const interval = screen.getByLabelText("AA points per notification");
    expect(aaToggle).toBeChecked();
    expect(interval).toHaveValue("1");
    expect(interval).toHaveAttribute("min", "1");
    expect(interval).toHaveAttribute("max", "100");
    expect(interval).toHaveAccessibleDescription(
      "Count earned AA points separately for each box, even when available points are spent.",
    );

    fireEvent.change(interval, { target: { value: "25" } });
    expect(interval).toHaveValue("25");
    expect(screen.getByText("25 points")).toBeInTheDocument();

    fireEvent.click(aaToggle);
    expect(aaToggle).not.toBeChecked();
    expect(interval).toBeDisabled();
  });
});

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

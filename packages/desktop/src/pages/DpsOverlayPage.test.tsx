import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { SettingsProvider } from "../settings/SettingsContext";
import { DpsOverlayPage } from "./DpsOverlayPage";

function renderPage() {
  render(
    <SettingsProvider>
      <DpsOverlayPage />
    </SettingsProvider>,
  );
}

describe("DpsOverlayPage", () => {
  it("edits enablement and the finite top-row options accessibly", async () => {
    renderPage();

    const enabled = await screen.findByRole("switch", {
      name: "Show DPS overlay",
    });
    expect(enabled).toBeChecked();
    expect(enabled).toHaveAccessibleDescription(
      "Keep the meter topmost and click-through while you play. It never takes focus from EverQuest.",
    );
    fireEvent.click(enabled);
    expect(enabled).not.toBeChecked();

    const rows = screen.getByLabelText("Top participants");
    expect(rows).toHaveValue("10");
    expect(screen.getAllByRole("option")).toHaveLength(3);
    fireEvent.change(rows, { target: { value: "15" } });
    expect(rows).toHaveValue("15");
    expect(rows).toHaveAccessibleDescription(
      "Participating managed boxes below the cutoff are always appended with their true raid rank.",
    );
  });

  it("resets placement through its dedicated immediate action", async () => {
    renderPage();

    const reset = await screen.findByRole("button", {
      name: "Reset placement",
    });
    fireEvent.click(reset);

    expect(await screen.findByText("Placement reset")).toBeInTheDocument();
    expect(
      screen.getByText("The meter will use its default position immediately."),
    ).toBeInTheDocument();
  });
});

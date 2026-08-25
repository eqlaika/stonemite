import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { describe, expect, it } from "vitest";

import type {
  LabelFontWeight,
  OptionItem,
  PipSettings,
} from "../settings/types";
import {
  InGameAccessControl,
  LabelTypographyControls,
  ThumbnailOpacityControl,
} from "./PipPage";

const weights: OptionItem<LabelFontWeight>[] = [
  { value: "regular", label: "Regular" },
  { value: "semibold", label: "Semibold" },
  { value: "bold", label: "Bold" },
  { value: "heavy", label: "Heavy" },
];

const fontFamilies = [
  "Arial",
  "Consolas",
  "Segoe UI",
  "Tahoma",
  "Verdana",
  "Wingdings",
];

const initial: PipSettings = {
  edge: "right",
  showStonemiteButton: true,
  thumbnailOpacity: 80,
  labelHeight: 48,
  labelOpacity: 80,
  fontFamily: "Segoe UI",
  fontScale: 100,
  fontWeight: "bold",
  autoOrder: true,
  hideHotkey: "F9",
};

function TypographyHarness() {
  const [value, setValue] = useState(initial);
  return (
    <LabelTypographyControls
      value={value}
      fontFamilies={fontFamilies}
      weightOptions={weights}
      onChange={(update) => setValue((current) => ({ ...current, ...update }))}
    />
  );
}

function ThumbnailOpacityHarness() {
  const [value, setValue] = useState(initial.thumbnailOpacity);
  return <ThumbnailOpacityControl value={value} onChange={setValue} />;
}

function InGameAccessHarness() {
  const [value, setValue] = useState(initial.showStonemiteButton);
  return <InGameAccessControl value={value} onChange={setValue} />;
}

describe("InGameAccessControl", () => {
  it("updates the draft value with accessible product copy", () => {
    render(<InGameAccessHarness />);

    const toggle = screen.getByRole("switch", {
      name: "Show Stonemite button",
    });
    expect(toggle).toBeChecked();
    expect(
      screen.getByText(
        "Keep Stonemite controls available over EverQuest. Drag the logo to move it; left-click opens Settings; right-click opens the tray menu.",
      ),
    ).toBeInTheDocument();

    fireEvent.click(toggle);
    expect(toggle).not.toBeChecked();
  });
});

describe("ThumbnailOpacityControl", () => {
  it("updates the independent live-thumbnail opacity accessibly", () => {
    render(<ThumbnailOpacityHarness />);

    const opacity = screen.getByLabelText("PiP thumbnail opacity");
    expect(opacity).toHaveAttribute("min", "10");
    expect(opacity).toHaveAttribute("max", "100");
    expect(opacity).toHaveValue("80");
    expect(opacity).toHaveAccessibleDescription(
      "Set the normal transparency of the live EQ preview. Hovering a PiP temporarily reveals it at full opacity.",
    );

    fireEvent.change(opacity, { target: { value: "65" } });
    expect(opacity).toHaveValue("65");
    expect(screen.getByText("65%")).toBeInTheDocument();
  });
});

describe("LabelTypographyControls", () => {
  it("updates the family, weight, and text scale accessibly", () => {
    render(<TypographyHarness />);

    const family = screen.getByLabelText("Font family");
    fireEvent.change(family, { target: { value: "Wingdings" } });
    expect(family).toHaveValue("Wingdings");
    expect(family).not.toHaveAttribute("style");
    expect(screen.getAllByRole("option")).toHaveLength(10);
    for (const familyOption of fontFamilies) {
      expect(
        screen.getByRole("option", { name: familyOption }),
      ).toBeInTheDocument();
    }
    expect(family).toHaveAccessibleDescription(
      "Choose from the font families installed on this PC.",
    );

    fireEvent.change(screen.getByLabelText("Weight"), {
      target: { value: "semibold" },
    });
    expect(screen.getByLabelText("Weight")).toHaveValue("semibold");
    expect(screen.getByLabelText("Weight")).toHaveAccessibleDescription(
      "Set the visual emphasis of character names.",
    );
    expect(family).not.toHaveAttribute("style");

    fireEvent.change(screen.getByLabelText("Character name font size"), {
      target: { value: "115" },
    });
    expect(screen.getByText("115%")).toBeInTheDocument();
    expect(
      screen.getByLabelText("Character name font size"),
    ).toHaveAccessibleDescription(
      "Scale character names within the current label height.",
    );
  });
});

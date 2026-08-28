import { useState } from "react";

import {
  Button,
  Field,
  FormSection,
  InlineStatus,
  SelectInput,
  Toggle,
} from "../components/Controls";
import { SettingsPage } from "../components/SettingsPage";
import { resetDpsOverlayPlacement } from "../settings/api";
import { useSettings } from "../settings/SettingsContext";
import type { DpsOverlaySettings, OptionItem } from "../settings/types";

type TopRows = DpsOverlaySettings["topRows"];
type ResetState = "idle" | "resetting" | "success" | "error";

const rowOptions: OptionItem<`${TopRows}`>[] = [
  { value: "5", label: "5 participants" },
  { value: "10", label: "10 participants" },
  { value: "15", label: "15 participants" },
];

export function DpsOverlayPage() {
  const { draft, setDraft } = useSettings();
  const [resetState, setResetState] = useState<ResetState>("idle");
  const [resetError, setResetError] = useState<string | null>(null);
  if (!draft) return null;

  const update = (change: Partial<DpsOverlaySettings>) => {
    setDraft((current) =>
      current
        ? {
            ...current,
            dpsOverlay: { ...current.dpsOverlay, ...change },
          }
        : current,
    );
  };

  const resetPlacement = async () => {
    setResetState("resetting");
    setResetError(null);
    try {
      await resetDpsOverlayPlacement();
      setResetState("success");
    } catch (error) {
      setResetError(
        error instanceof Error
          ? error.message
          : typeof error === "string"
            ? error
            : "Stonemite could not reset the DPS overlay placement.",
      );
      setResetState("error");
    }
  };

  return (
    <SettingsPage
      title="DPS overlay"
      description="A passive live damage meter for group and raid encounters."
    >
      <FormSection
        title="Live meter"
        description="The panel appears automatically when combat starts and briefly holds the final result."
      >
        <Toggle
          label="Show DPS overlay"
          description="Keep the meter topmost and click-through while you play. It never takes focus from EverQuest."
          checked={draft.dpsOverlay.enabled}
          onChange={(enabled) => update({ enabled })}
        />
      </FormSection>

      <FormSection
        title="Rows"
        description="Choose the global ranking cutoff shown during combat."
      >
        <Field
          label="Top participants"
          description="Participating managed boxes below the cutoff are always appended with their true raid rank."
          htmlFor="dps-overlay-top-rows"
          descriptionId="dps-overlay-top-rows-help"
        >
          <SelectInput<`${TopRows}`>
            id="dps-overlay-top-rows"
            aria-describedby="dps-overlay-top-rows-help"
            value={String(draft.dpsOverlay.topRows) as `${TopRows}`}
            options={rowOptions}
            onChange={(event) =>
              update({ topRows: Number(event.currentTarget.value) as TopRows })
            }
          />
        </Field>
      </FormSection>

      <FormSection
        title="Placement"
        description="Use Edit overlay from the Stonemite tray menu to drag the panel or resize its width."
      >
        <Field
          label="Saved placement"
          description="Reset to place the meter 24 pixels from the upper-left of the active EverQuest monitor."
        >
          <Button
            type="button"
            onClick={() => void resetPlacement()}
            disabled={resetState === "resetting"}
          >
            {resetState === "resetting" ? "Resetting…" : "Reset placement"}
          </Button>
        </Field>
        <div aria-live="polite">
          {resetState === "success" ? (
            <InlineStatus tone="success" title="Placement reset">
              The meter will use its default position immediately.
            </InlineStatus>
          ) : null}
          {resetState === "error" ? (
            <InlineStatus tone="error" title="Placement was not reset">
              {resetError}
            </InlineStatus>
          ) : null}
        </div>
      </FormSection>
    </SettingsPage>
  );
}

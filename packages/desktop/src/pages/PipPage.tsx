import {
  Field,
  FormSection,
  RangeInput,
  SelectInput,
  Toggle,
} from "../components/Controls";
import { HotkeyCapture } from "../components/HotkeyCapture";
import { SettingsPage } from "../components/SettingsPage";
import { useSettings } from "../settings/SettingsContext";
import type { PipEdge, SettingsDraft } from "../settings/types";
import "./PipPage.css";

export function PipPage() {
  const { draft, setDraft, options } = useSettings();
  if (!draft || !options) return null;

  const updatePip = (
    update: (pip: SettingsDraft["pip"]) => SettingsDraft["pip"],
  ) => {
    setDraft((current) =>
      current ? { ...current, pip: update(current.pip) } : current,
    );
  };

  return (
    <SettingsPage
      title="PiP overlay"
      description="Placement, labels, ordering, and overlay visibility."
    >
      <FormSection
        title="Layout"
        description="Choose where PiP thumbnails appear and how they are ordered."
      >
        <Field
          label="Screen edge"
          description="The edge where PiP thumbnails are anchored."
          htmlFor="pip-screen-edge"
        >
          <SelectInput<PipEdge>
            id="pip-screen-edge"
            value={draft.pip.edge}
            options={options.pipEdges}
            onChange={(event) =>
              updatePip((pip) => ({
                ...pip,
                edge: event.currentTarget.value as PipEdge,
              }))
            }
          />
        </Field>
        <Toggle
          label="Auto order windows"
          description="Keep PiP thumbnails automatically arranged along the selected edge."
          checked={draft.pip.autoOrder}
          onChange={(autoOrder) => updatePip((pip) => ({ ...pip, autoOrder }))}
        />
      </FormSection>

      <FormSection
        title="Labels"
        description="Adjust the size and transparency of labels on PiP thumbnails."
      >
        <Field label="Height">
          <RangeInput
            value={draft.pip.labelHeight}
            min={24}
            max={64}
            suffix=" px"
            ariaLabel="PiP label height"
            onChange={(labelHeight) =>
              updatePip((pip) => ({ ...pip, labelHeight }))
            }
          />
        </Field>
        <Field label="Opacity">
          <RangeInput
            value={draft.pip.labelOpacity}
            min={10}
            max={100}
            suffix="%"
            ariaLabel="PiP label opacity"
            onChange={(labelOpacity) =>
              updatePip((pip) => ({ ...pip, labelOpacity }))
            }
          />
        </Field>
      </FormSection>

      <FormSection
        title="Hide overlay hotkey"
        description="Toggle PiP overlay visibility while EverQuest is focused."
      >
        <Field label="Shortcut">
          <div className="pip-hotkey-control">
            <HotkeyCapture
              value={draft.pip.hideHotkey}
              ariaLabel="Hide PiP overlay hotkey"
              onChange={(hideHotkey) =>
                updatePip((pip) => ({ ...pip, hideHotkey }))
              }
            />
          </div>
        </Field>
        <p className="help-text">
          Shortcuts may use Ctrl, Alt, and Shift. Press Escape while capturing
          to keep the current binding.
        </p>
      </FormSection>
    </SettingsPage>
  );
}

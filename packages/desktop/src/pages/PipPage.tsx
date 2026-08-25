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
import type {
  LabelFontWeight,
  OptionItem,
  PipEdge,
  PipSettings,
  SettingsDraft,
} from "../settings/types";
import "./PipPage.css";

export function LabelTypographyControls({
  value,
  fontFamilies,
  weightOptions,
  onChange,
}: {
  value: PipSettings;
  fontFamilies: string[];
  weightOptions: OptionItem<LabelFontWeight>[];
  onChange: (update: Partial<PipSettings>) => void;
}) {
  return (
    <>
      <Field
        label="Font family"
        description="Choose from the font families installed on this PC."
        htmlFor="pip-label-font-family"
        descriptionId="pip-label-font-family-help"
      >
        <SelectInput<string>
          id="pip-label-font-family"
          value={value.fontFamily}
          aria-describedby="pip-label-font-family-help"
          options={fontFamilies.map((family) => ({
            value: family,
            label: family,
          }))}
          onChange={(event) =>
            onChange({ fontFamily: event.currentTarget.value })
          }
        />
      </Field>
      <Field
        label="Weight"
        description="Set the visual emphasis of character names."
        htmlFor="pip-label-font-weight"
        descriptionId="pip-label-font-weight-help"
      >
        <SelectInput<LabelFontWeight>
          id="pip-label-font-weight"
          value={value.fontWeight}
          aria-describedby="pip-label-font-weight-help"
          options={weightOptions}
          onChange={(event) =>
            onChange({
              fontWeight: event.currentTarget.value as LabelFontWeight,
            })
          }
        />
      </Field>
      <Field
        label="Text size"
        description="Scale character names within the current label height."
        descriptionId="pip-label-font-size-help"
      >
        <RangeInput
          value={value.fontScale}
          min={60}
          max={120}
          step={5}
          suffix="%"
          ariaLabel="Character name font size"
          ariaDescribedBy="pip-label-font-size-help"
          onChange={(fontScale) => onChange({ fontScale })}
        />
      </Field>
    </>
  );
}

export function InGameAccessControl({
  value,
  onChange,
}: {
  value: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <Toggle
      label="Show Stonemite button"
      description="Keep Stonemite controls available over EverQuest. Drag the logo to move it; left-click opens Settings; right-click opens the tray menu."
      checked={value}
      onChange={onChange}
    />
  );
}

export function ThumbnailOpacityControl({
  value,
  onChange,
}: {
  value: number;
  onChange: (value: number) => void;
}) {
  return (
    <Field
      label="Opacity"
      description="Set the normal transparency of the live EQ preview. Hovering a PiP temporarily reveals it at full opacity."
      descriptionId="pip-thumbnail-opacity-help"
    >
      <RangeInput
        value={value}
        min={10}
        max={100}
        suffix="%"
        ariaLabel="PiP thumbnail opacity"
        ariaDescribedBy="pip-thumbnail-opacity-help"
        onChange={onChange}
      />
    </Field>
  );
}

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
      description="Placement, thumbnail transparency, labels, ordering, and overlay visibility."
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
        title="In-game access"
        description="Choose whether Stonemite controls remain available over EverQuest."
      >
        <InGameAccessControl
          value={draft.pip.showStonemiteButton}
          onChange={(showStonemiteButton) =>
            updatePip((pip) => ({ ...pip, showStonemiteButton }))
          }
        />
      </FormSection>

      <FormSection
        title="Thumbnails"
        description="Adjust live EQ previews independently from labels and notifications."
      >
        <ThumbnailOpacityControl
          value={draft.pip.thumbnailOpacity}
          onChange={(thumbnailOpacity) =>
            updatePip((pip) => ({ ...pip, thumbnailOpacity }))
          }
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
        title="Typography"
        description="Choose how character names appear on active and PiP labels."
      >
        <LabelTypographyControls
          value={draft.pip}
          fontFamilies={options.labelFontFamilies}
          weightOptions={options.labelFontWeights}
          onChange={(update) => updatePip((pip) => ({ ...pip, ...update }))}
        />
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

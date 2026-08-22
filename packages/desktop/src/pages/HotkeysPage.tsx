import { Field, FormSection } from "../components/Controls";
import { HotkeyCapture } from "../components/HotkeyCapture";
import { SettingsPage } from "../components/SettingsPage";
import { useSettings } from "../settings/SettingsContext";
import "./HotkeysPage.css";

const WINDOW_SLOTS = Array.from({ length: 6 }, (_, index) => index);

export function HotkeysPage() {
  const { draft, setDraft } = useSettings();
  if (!draft) return null;

  const updateWindowHotkey = (slot: number, value: string) => {
    setDraft((current) => {
      if (!current) return current;
      const swapHotkeys = current.hotkeys.swapHotkeys.slice();
      swapHotkeys[slot] = value;
      return {
        ...current,
        hotkeys: { ...current.hotkeys, swapHotkeys },
      };
    });
  };

  return (
    <SettingsPage
      title="Hotkeys"
      description="Direct shortcuts for activating exact EverQuest windows."
    >
      <FormSection
        title="Swap to window"
        description="Assign a shortcut to each exact EverQuest window slot."
      >
        <div className="window-hotkey-list">
          {WINDOW_SLOTS.map((slot) => (
            <Field key={slot} label={`Window ${slot + 1}`}>
              <HotkeyCapture
                value={draft.hotkeys.swapHotkeys[slot] ?? ""}
                ariaLabel={`Window ${slot + 1} hotkey`}
                onChange={(value) => updateWindowHotkey(slot, value)}
              />
            </Field>
          ))}
        </div>
        <p className="help-text">
          Shortcuts may use Ctrl, Alt, and Shift. Press Escape while capturing
          to keep the current binding.
        </p>
      </FormSection>
    </SettingsPage>
  );
}

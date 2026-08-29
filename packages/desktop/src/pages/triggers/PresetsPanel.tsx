import { Plus, Trash2, Volume2 } from "lucide-react";

import { Button, CheckboxOption } from "../../components/Controls";
import { addTriggerMedia, previewTriggerSound } from "../../triggers/api";
import { newId } from "../../triggers/model";
import type {
  TextOverlayPreset,
  TimerOverlayPreset,
  TriggerLibrary,
} from "../../triggers/types";

export function PresetsPanel({
  library,
  onChange,
}: {
  library: TriggerLibrary;
  onChange: (library: TriggerLibrary) => void;
}) {
  return (
    <div className="tw-side-panel">
      <h4>Text overlay presets</h4>
      {library.textOverlays.map((preset) => (
        <TextPresetEditor
          key={preset.id}
          preset={preset}
          onChange={(next) =>
            onChange({
              ...library,
              textOverlays: library.textOverlays.map((existing) =>
                existing.id === next.id ? next : existing,
              ),
            })
          }
          onDelete={() =>
            onChange({
              ...library,
              textOverlays: library.textOverlays.filter(
                (existing) => existing.id !== preset.id,
              ),
              triggers: library.triggers.map((trigger) => ({
                ...trigger,
                textOverlays: trigger.textOverlays.filter(
                  (id) => id !== preset.id,
                ),
              })),
            })
          }
        />
      ))}
      <Button
        onClick={() =>
          onChange({
            ...library,
            textOverlays: [
              ...library.textOverlays,
              {
                id: newId(),
                name: `Text overlay ${library.textOverlays.length + 1}`,
                isDefault: library.textOverlays.length === 0,
                fontSize: "12pt",
                fontColor: "#FFFFFFFF",
                backgroundColor: "#5F000000",
                fadeDelaySeconds: 10,
                passthrough: {},
              },
            ],
          })
        }
      >
        <Plus size={14} aria-hidden="true" /> Text overlay preset
      </Button>

      <h4>Timer overlay presets</h4>
      {library.timerOverlays.map((preset) => (
        <TimerPresetEditor
          key={preset.id}
          preset={preset}
          onChange={(next) =>
            onChange({
              ...library,
              timerOverlays: library.timerOverlays.map((existing) =>
                existing.id === next.id ? next : existing,
              ),
            })
          }
          onDelete={() =>
            onChange({
              ...library,
              timerOverlays: library.timerOverlays.filter(
                (existing) => existing.id !== preset.id,
              ),
              triggers: library.triggers.map((trigger) => ({
                ...trigger,
                timerOverlays: trigger.timerOverlays.filter(
                  (id) => id !== preset.id,
                ),
              })),
            })
          }
        />
      ))}
      <Button
        onClick={() =>
          onChange({
            ...library,
            timerOverlays: [
              ...library.timerOverlays,
              {
                id: newId(),
                name: `Timer overlay ${library.timerOverlays.length + 1}`,
                isDefault: library.timerOverlays.length === 0,
                mode: "standard",
                sortBy: 0,
                fontColor: "#FFFFFFFF",
                activeColor: "#FF1D397E",
                idleColor: "#FF8F1515",
                resetColor: "#FF8F1515",
                backgroundColor: "#5F000000",
                showMillis: false,
                passthrough: {},
              },
            ],
          })
        }
      >
        <Plus size={14} aria-hidden="true" /> Timer overlay preset
      </Button>

      <h4>Managed sounds</h4>
      <ul className="tw-asset-list">
        {library.assets.map((asset) => (
          <li key={asset.fileName}>
            <span>{asset.name}</span>
            <button
              type="button"
              className="tw-icon-button"
              aria-label={`Preview ${asset.name}`}
              onClick={() => void previewTriggerSound(asset.name)}
            >
              <Volume2 size={13} aria-hidden="true" />
            </button>
          </li>
        ))}
      </ul>
      <Button
        onClick={() =>
          void addTriggerMedia().then((record) => {
            if (
              record &&
              !library.assets.some((a) => a.fileName === record.fileName)
            ) {
              onChange({ ...library, assets: [...library.assets, record] });
            }
          })
        }
      >
        <Plus size={14} aria-hidden="true" /> Add WAV/MP3
      </Button>
    </div>
  );
}

function TextPresetEditor({
  preset,
  onChange,
  onDelete,
}: {
  preset: TextOverlayPreset;
  onChange: (preset: TextOverlayPreset) => void;
  onDelete: () => void;
}) {
  return (
    <details className="tw-preset">
      <summary>{preset.name || "(unnamed)"}</summary>
      <label>
        <span>Name</span>
        <input
          value={preset.name}
          onChange={(event) =>
            onChange({ ...preset, name: event.target.value })
          }
        />
      </label>
      <div className="tw-grid">
        <label>
          <span>Font size</span>
          <input
            value={preset.fontSize}
            onChange={(event) =>
              onChange({ ...preset, fontSize: event.target.value })
            }
          />
        </label>
        <label>
          <span>Visible seconds</span>
          <input
            type="number"
            min={1}
            value={preset.fadeDelaySeconds}
            onChange={(event) =>
              onChange({
                ...preset,
                fadeDelaySeconds: Number(event.target.value) || 10,
              })
            }
          />
        </label>
        <label>
          <span>Font color</span>
          <input
            value={preset.fontColor}
            onChange={(event) =>
              onChange({ ...preset, fontColor: event.target.value })
            }
          />
        </label>
        <label>
          <span>Background</span>
          <input
            value={preset.backgroundColor}
            onChange={(event) =>
              onChange({ ...preset, backgroundColor: event.target.value })
            }
          />
        </label>
      </div>
      <Button variant="danger" onClick={onDelete}>
        <Trash2 size={13} aria-hidden="true" /> Delete preset
      </Button>
    </details>
  );
}

function TimerPresetEditor({
  preset,
  onChange,
  onDelete,
}: {
  preset: TimerOverlayPreset;
  onChange: (preset: TimerOverlayPreset) => void;
  onDelete: () => void;
}) {
  return (
    <details className="tw-preset">
      <summary>{preset.name || "(unnamed)"}</summary>
      <label>
        <span>Name</span>
        <input
          value={preset.name}
          onChange={(event) =>
            onChange({ ...preset, name: event.target.value })
          }
        />
      </label>
      <div className="tw-grid">
        <label>
          <span>Mode</span>
          <select
            value={preset.mode}
            onChange={(event) =>
              onChange({
                ...preset,
                mode: event.target.value as TimerOverlayPreset["mode"],
              })
            }
          >
            <option value="standard">Standard</option>
            <option value="cooldown">Cooldown</option>
          </select>
        </label>
        <label>
          <span>Sort by</span>
          <select
            value={String(preset.sortBy)}
            onChange={(event) =>
              onChange({ ...preset, sortBy: Number(event.target.value) })
            }
          >
            <option value="0">Trigger order</option>
            <option value="1">Time remaining</option>
          </select>
        </label>
        <label>
          <span>Active color</span>
          <input
            value={preset.activeColor}
            onChange={(event) =>
              onChange({ ...preset, activeColor: event.target.value })
            }
          />
        </label>
        <label>
          <span>Font color</span>
          <input
            value={preset.fontColor}
            onChange={(event) =>
              onChange({ ...preset, fontColor: event.target.value })
            }
          />
        </label>
      </div>
      <CheckboxOption
        label="Show milliseconds"
        checked={preset.showMillis}
        onChange={(showMillis) => onChange({ ...preset, showMillis })}
      />
      <Button variant="danger" onClick={onDelete}>
        <Trash2 size={13} aria-hidden="true" /> Delete preset
      </Button>
    </details>
  );
}

import { Copy, ShieldAlert } from "lucide-react";
import { useEffect, useRef, useState } from "react";

import { Button } from "../../components/Controls";
import { describeTriggerActions } from "../../triggers/model";
import type { Trigger, TriggerLibrary } from "../../triggers/types";

const ROW_HEIGHT = 56;
const OVERSCAN = 8;

export function TriggerList({
  library,
  triggers,
  selection,
  focusedId,
  onFocus,
  onSelectionChange,
  onToggleEnabled,
  onBulkEnable,
  onBulkMove,
  onBulkDelete,
  onDuplicate,
}: {
  library: TriggerLibrary;
  triggers: Trigger[];
  selection: string[];
  focusedId: string | null;
  onFocus: (id: string) => void;
  onSelectionChange: (ids: string[]) => void;
  onToggleEnabled: (id: string, enabled: boolean) => void;
  onBulkEnable: (enabled: boolean) => void;
  onBulkMove: (folder: string | null) => void;
  onBulkDelete: () => void;
  onDuplicate: (id: string) => void;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [viewport, setViewport] = useState({ top: 0, height: 600 });

  useEffect(() => {
    const element = scroller.current;
    if (!element) return;
    const measure = () =>
      setViewport({
        top: element.scrollTop,
        // jsdom reports zero heights; keep a sane virtual window there.
        height: element.clientHeight || 600,
      });
    measure();
    element.addEventListener("scroll", measure);
    const observer =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(measure)
        : null;
    observer?.observe(element);
    return () => {
      element.removeEventListener("scroll", measure);
      observer?.disconnect();
    };
  }, []);

  // Simple windowing keeps ten-thousand-trigger libraries responsive
  // without a virtualization dependency.
  const first = Math.max(0, Math.floor(viewport.top / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(
    triggers.length,
    Math.ceil((viewport.top + viewport.height) / ROW_HEIGHT) + OVERSCAN,
  );
  const window_ = triggers.slice(first, last);

  const toggleSelected = (id: string, checked: boolean) => {
    onSelectionChange(
      checked ? [...selection, id] : selection.filter((other) => other !== id),
    );
  };
  const allVisibleSelected =
    triggers.length > 0 &&
    triggers.every((trigger) => selection.includes(trigger.id));

  const folderName = (id: string | null) =>
    id === null
      ? "Library root"
      : (library.folders.find((folder) => folder.id === id)?.name ??
        "Unknown folder");

  return (
    <div className="tw-trigger-list">
      <div className="tw-list-header">
        <label className="tw-select-all">
          <input
            type="checkbox"
            checked={allVisibleSelected}
            aria-label="Select all visible triggers"
            onChange={(event) =>
              onSelectionChange(
                event.target.checked
                  ? triggers.map((trigger) => trigger.id)
                  : [],
              )
            }
          />
          <span>Select all</span>
        </label>
        {selection.length > 0 ? (
          <div className="tw-bulk-bar" role="toolbar" aria-label="Bulk actions">
            <span aria-live="polite">{selection.length} selected</span>
            <Button onClick={() => onBulkEnable(true)}>Enable</Button>
            <Button onClick={() => onBulkEnable(false)}>Disable</Button>
            <select
              aria-label="Move selection to folder"
              value=""
              onChange={(event) => {
                if (event.target.value === "") return;
                onBulkMove(
                  event.target.value === "__root__" ? null : event.target.value,
                );
                event.target.value = "";
              }}
            >
              <option value="">Move to…</option>
              <option value="__root__">Library root</option>
              {library.folders.map((folder) => (
                <option key={folder.id} value={folder.id}>
                  {folder.name}
                </option>
              ))}
            </select>
            <Button onClick={onBulkDelete}>Delete</Button>
          </div>
        ) : null}
      </div>
      <div
        className="tw-list-scroll"
        ref={scroller}
        role="listbox"
        aria-label="Trigger list"
        tabIndex={0}
      >
        <div
          style={{ height: triggers.length * ROW_HEIGHT, position: "relative" }}
        >
          {window_.map((trigger, offset) => {
            const index = first + offset;
            const checked = selection.includes(trigger.id);
            return (
              <div
                key={trigger.id}
                role="option"
                aria-selected={focusedId === trigger.id}
                className={
                  focusedId === trigger.id
                    ? "tw-trigger-row focused"
                    : "tw-trigger-row"
                }
                style={{ top: index * ROW_HEIGHT, height: ROW_HEIGHT }}
              >
                <input
                  type="checkbox"
                  checked={checked}
                  aria-label={`Select ${trigger.name}`}
                  onChange={(event) =>
                    toggleSelected(trigger.id, event.target.checked)
                  }
                />
                <button
                  type="button"
                  className="tw-trigger-main"
                  onClick={() => onFocus(trigger.id)}
                >
                  <span className="tw-trigger-name">
                    {trigger.quarantine ? (
                      <ShieldAlert
                        size={13}
                        aria-label="Quarantined"
                        className="tw-quarantine-icon"
                      />
                    ) : null}
                    {trigger.name || "(unnamed)"}
                  </span>
                  <span className="tw-trigger-meta">
                    {folderName(trigger.folder)} ·{" "}
                    {describeTriggerActions(trigger)}
                  </span>
                </button>
                <button
                  type="button"
                  className="tw-icon-button"
                  aria-label={`Duplicate ${trigger.name}`}
                  onClick={() => onDuplicate(trigger.id)}
                >
                  <Copy size={13} aria-hidden="true" />
                </button>
                <label className="tw-enable">
                  <input
                    type="checkbox"
                    checked={trigger.enabled}
                    disabled={trigger.quarantine !== null}
                    aria-label={`Enable ${trigger.name}`}
                    onChange={(event) =>
                      onToggleEnabled(trigger.id, event.target.checked)
                    }
                  />
                  <span>On</span>
                </label>
              </div>
            );
          })}
        </div>
        {triggers.length === 0 ? (
          <p className="tw-list-empty">
            No triggers match the current filters.
          </p>
        ) : null}
      </div>
    </div>
  );
}

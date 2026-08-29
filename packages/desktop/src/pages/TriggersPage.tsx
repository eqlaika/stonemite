import {
  Download,
  FlaskConical,
  FolderPlus,
  Plus,
  RefreshCw,
  Save,
  Upload,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { Button, InlineStatus } from "../components/Controls";
import { loadTriggerLibrary, saveTriggerLibrary } from "../triggers/api";
import {
  emptyLibrary,
  filterTriggers,
  newFolder,
  newTrigger,
  removeFolder,
  removeTriggers,
  duplicateTrigger,
  moveTriggers,
  setTriggersEnabled,
  withTrigger,
  type StatusFilter,
} from "../triggers/model";
import type {
  BuiltinSoundOption,
  CompatReport,
  Trigger,
  TriggerLibrary,
} from "../triggers/types";
import { ExportDialog, ImportDialog } from "./triggers/ImportExportDialogs";
import { LibraryTree } from "./triggers/LibraryTree";
import { PresetsPanel } from "./triggers/PresetsPanel";
import { ProfilesPanel } from "./triggers/ProfilesPanel";
import { TestBench } from "./triggers/TestBench";
import { TriggerEditor } from "./triggers/TriggerEditor";
import { TriggerList } from "./triggers/TriggerList";
import "./TriggersPage.css";

type LeftTab = "library" | "profiles" | "presets";

export function TriggersPage() {
  const [library, setLibrary] = useState<TriggerLibrary>(emptyLibrary());
  const [builtinSounds, setBuiltinSounds] = useState<BuiltinSoundOption[]>([]);
  const [loadReport, setLoadReport] = useState<CompatReport | null>(null);
  const [loadState, setLoadState] = useState<"loading" | "ready" | "error">(
    "loading",
  );
  const [loadError, setLoadError] = useState<string | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saveState, setSaveState] = useState<"idle" | "saving">("idle");
  const [saveError, setSaveError] = useState<string | null>(null);

  const [leftTab, setLeftTab] = useState<LeftTab>("library");
  const [treeCollapsed, setTreeCollapsed] = useState(false);
  const [selectedFolder, setSelectedFolder] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<StatusFilter>("all");
  const [selection, setSelection] = useState<string[]>([]);
  const [focusedId, setFocusedId] = useState<string | null>(null);
  const [benchOpen, setBenchOpen] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [exportOpen, setExportOpen] = useState(false);

  async function reload() {
    setLoadState("loading");
    try {
      const payload = await loadTriggerLibrary();
      setLibrary(payload.library);
      setBuiltinSounds(payload.builtinSounds);
      setLoadReport(payload.report.issues.length > 0 ? payload.report : null);
      setDirty(false);
      setLoadState("ready");
    } catch (error) {
      setLoadError(String(error));
      setLoadState("error");
    }
  }

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function update(next: TriggerLibrary) {
    setLibrary(next);
    setDirty(true);
  }

  async function save() {
    setSaveState("saving");
    setSaveError(null);
    try {
      await saveTriggerLibrary(library);
      setDirty(false);
    } catch (error) {
      setSaveError(String(error));
    } finally {
      setSaveState("idle");
    }
  }

  const visible = useMemo(
    () => filterTriggers(library, selectedFolder, search, status),
    [library, selectedFolder, search, status],
  );
  const focused: Trigger | null =
    library.triggers.find((trigger) => trigger.id === focusedId) ?? null;

  function addTrigger() {
    const trigger = newTrigger(selectedFolder, library.triggers.length);
    update({ ...library, triggers: [...library.triggers, trigger] });
    setFocusedId(trigger.id);
    setSelection([trigger.id]);
  }

  function addFolder() {
    const folder = newFolder(selectedFolder, library.folders.length);
    update({ ...library, folders: [...library.folders, folder] });
    setSelectedFolder(folder.id);
  }

  function deleteSelection() {
    if (selection.length === 0) return;
    if (
      !window.confirm(
        selection.length === 1
          ? "Delete the selected trigger?"
          : `Delete ${selection.length} selected triggers?`,
      )
    )
      return;
    update(removeTriggers(library, selection));
    setSelection([]);
    if (focusedId && selection.includes(focusedId)) setFocusedId(null);
  }

  if (loadState === "loading") {
    return (
      <main className="launch-state" aria-busy="true">
        <RefreshCw className="spin" aria-hidden="true" />
        <h1>Opening triggers</h1>
        <p>Reading your trigger library…</p>
      </main>
    );
  }
  if (loadState === "error") {
    return (
      <main className="launch-state">
        <h1>Triggers could not open</h1>
        <p>{loadError}</p>
        <Button variant="primary" onClick={() => void reload()}>
          Try again
        </Button>
      </main>
    );
  }

  return (
    <div className="triggers-workbench">
      <header className="tw-toolbar">
        <div className="tw-toolbar-search">
          <input
            type="search"
            value={search}
            placeholder="Search triggers…"
            aria-label="Search triggers"
            onChange={(event) => setSearch(event.target.value)}
          />
          <select
            value={status}
            aria-label="Status filter"
            onChange={(event) => setStatus(event.target.value as StatusFilter)}
          >
            <option value="all">All</option>
            <option value="enabled">Enabled</option>
            <option value="disabled">Disabled</option>
            <option value="quarantined">Quarantined</option>
          </select>
          <span className="tw-result-count" aria-live="polite">
            {visible.length} of {library.triggers.length}
          </span>
        </div>
        <div className="tw-toolbar-actions">
          <Button onClick={addTrigger}>
            <Plus size={15} aria-hidden="true" /> Trigger
          </Button>
          <Button onClick={addFolder}>
            <FolderPlus size={15} aria-hidden="true" /> Folder
          </Button>
          <Button onClick={() => setImportOpen(true)}>
            <Download size={15} aria-hidden="true" /> Import
          </Button>
          <Button onClick={() => setExportOpen(true)}>
            <Upload size={15} aria-hidden="true" /> Export
          </Button>
          <Button
            onClick={() => setBenchOpen((open) => !open)}
            aria-pressed={benchOpen}
          >
            <FlaskConical size={15} aria-hidden="true" /> Test bench
          </Button>
          <Button
            variant="primary"
            disabled={!dirty || saveState === "saving"}
            onClick={() => void save()}
          >
            <Save size={15} aria-hidden="true" />
            {saveState === "saving" ? "Saving…" : dirty ? "Save" : "Saved"}
          </Button>
        </div>
      </header>

      {loadReport ? (
        <div className="tw-report" role="status">
          <InlineStatus tone="warning" title="Library loaded with notes">
            <ul>
              {loadReport.issues.slice(0, 6).map((issue, index) => (
                <li key={index}>
                  <strong>{issue.subject}</strong>: {issue.detail}
                </li>
              ))}
            </ul>
          </InlineStatus>
        </div>
      ) : null}
      {saveError ? (
        <div className="tw-report" role="alert">
          <InlineStatus tone="error" title="The library was not saved">
            {saveError}
          </InlineStatus>
        </div>
      ) : null}

      <div className={`tw-panes${treeCollapsed ? " tree-collapsed" : ""}`}>
        <aside className="tw-tree" aria-label="Library organization">
          <div className="tw-tabs" role="tablist" aria-label="Left panel">
            {(
              [
                ["library", "Library"],
                ["profiles", "Profiles"],
                ["presets", "Presets"],
              ] as [LeftTab, string][]
            ).map(([id, label]) => (
              <button
                key={id}
                role="tab"
                type="button"
                aria-selected={leftTab === id}
                className={leftTab === id ? "tw-tab active" : "tw-tab"}
                onClick={() => setLeftTab(id)}
              >
                {label}
              </button>
            ))}
            <button
              type="button"
              className="tw-tree-collapse"
              aria-expanded={!treeCollapsed}
              aria-label={treeCollapsed ? "Expand panel" : "Collapse panel"}
              onClick={() => setTreeCollapsed((collapsed) => !collapsed)}
            >
              {treeCollapsed ? "»" : "«"}
            </button>
          </div>
          {!treeCollapsed && leftTab === "library" ? (
            <LibraryTree
              library={library}
              selected={selectedFolder}
              onSelect={setSelectedFolder}
              onRename={(id, name) =>
                update({
                  ...library,
                  folders: library.folders.map((folder) =>
                    folder.id === id ? { ...folder, name } : folder,
                  ),
                })
              }
              onDelete={(id) => {
                if (
                  window.confirm(
                    "Delete this folder? Its triggers move to the library root.",
                  )
                ) {
                  update(removeFolder(library, id));
                  if (selectedFolder === id) setSelectedFolder(null);
                }
              }}
            />
          ) : null}
          {!treeCollapsed && leftTab === "profiles" ? (
            <ProfilesPanel library={library} onChange={update} />
          ) : null}
          {!treeCollapsed && leftTab === "presets" ? (
            <PresetsPanel library={library} onChange={update} />
          ) : null}
        </aside>

        <section className="tw-list" aria-label="Triggers">
          <TriggerList
            library={library}
            triggers={visible}
            selection={selection}
            focusedId={focusedId}
            onFocus={setFocusedId}
            onSelectionChange={setSelection}
            onToggleEnabled={(id, enabled) =>
              update(setTriggersEnabled(library, [id], enabled))
            }
            onBulkEnable={(enabled) =>
              update(setTriggersEnabled(library, selection, enabled))
            }
            onBulkMove={(folder) =>
              update(moveTriggers(library, selection, folder))
            }
            onBulkDelete={deleteSelection}
            onDuplicate={(id) => update(duplicateTrigger(library, id))}
          />
        </section>

        <section className="tw-editor" aria-label="Trigger editor">
          {focused ? (
            <TriggerEditor
              key={focused.id}
              library={library}
              trigger={focused}
              builtinSounds={builtinSounds}
              onChange={(trigger) => update(withTrigger(library, trigger))}
            />
          ) : (
            <div className="tw-editor-empty">
              <p>Select a trigger to edit it, or create a new one.</p>
            </div>
          )}
        </section>
      </div>

      {benchOpen ? <TestBench /> : null}
      {importOpen ? (
        <ImportDialog
          onClose={() => setImportOpen(false)}
          onImported={() => {
            setImportOpen(false);
            void reload();
          }}
        />
      ) : null}
      {exportOpen ? (
        <ExportDialog
          library={library}
          selection={selection}
          selectedFolder={selectedFolder}
          onClose={() => setExportOpen(false)}
        />
      ) : null}
    </div>
  );
}

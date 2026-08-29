import { useState } from "react";

import {
  Button,
  CheckboxOption,
  InlineStatus,
} from "../../components/Controls";
import {
  chooseTriggerImportFile,
  commitTriggerImport,
  exportTriggerSelection,
  previewTriggerImport,
} from "../../triggers/api";
import type {
  CompatReport,
  ExportFormat,
  ImportPreview,
  ImportSummary,
  TriggerLibrary,
} from "../../triggers/types";

function ReportList({ report }: { report: CompatReport }) {
  if (report.issues.length === 0) return null;
  return (
    <ul className="tw-report-list">
      {report.issues.slice(0, 20).map((issue, index) => (
        <li key={index} className={`tw-issue ${issue.severity}`}>
          <strong>{issue.subject}</strong> — {issue.detail}
        </li>
      ))}
      {report.issues.length > 20 ? (
        <li>…and {report.issues.length - 20} more</li>
      ) : null}
    </ul>
  );
}

export function ImportDialog({
  onClose,
  onImported,
}: {
  onClose: () => void;
  onImported: () => void;
}) {
  const [path, setPath] = useState<string | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [summary, setSummary] = useState<ImportSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [wrapInFolder, setWrapInFolder] = useState(true);
  const [folderName, setFolderName] = useState("");
  const [replaceSameNames, setReplaceSameNames] = useState(false);
  const [enable, setEnable] = useState(false);

  async function pick() {
    setError(null);
    const chosen = await chooseTriggerImportFile();
    if (!chosen) return;
    setPath(chosen);
    setBusy(true);
    try {
      const result = await previewTriggerImport(chosen);
      setPreview(result);
      setFolderName(
        result.fileName.replace(
          /\.(tgf\.gz|ogf\.gz|gtp|stonemite-triggers)$/i,
          "",
        ),
      );
    } catch (problem) {
      setError(String(problem));
      setPreview(null);
    } finally {
      setBusy(false);
    }
  }

  async function commit() {
    if (!path) return;
    setBusy(true);
    setError(null);
    try {
      const result = await commitTriggerImport(path, {
        newFolderName: wrapInFolder ? folderName.trim() || "Imported" : null,
        replaceSameNames,
        enable,
      });
      setSummary(result);
    } catch (problem) {
      setError(String(problem));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="tw-dialog-backdrop" role="presentation">
      <div
        className="tw-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Import triggers"
      >
        <h3>Import triggers</h3>
        {summary ? (
          <>
            <InlineStatus tone="success" title="Import complete">
              <p>
                Added {summary.triggersAdded} trigger(s), {summary.foldersAdded}{" "}
                folder(s), {summary.overlaysAdded} overlay preset(s),{" "}
                {summary.assetsAdded} sound(s).
                {summary.triggersReplaced > 0
                  ? ` Replaced ${summary.triggersReplaced}.`
                  : ""}
              </p>
            </InlineStatus>
            <div className="tw-dialog-actions">
              <Button variant="primary" onClick={onImported}>
                Done
              </Button>
            </div>
          </>
        ) : (
          <>
            <p>
              Stonemite imports EQLP trigger packages (.tgf.gz), EQLP overlay
              packages (.ogf.gz), GINA shares (.gtp), and Stonemite packages
              (.stonemite-triggers).
            </p>
            <Button onClick={() => void pick()} disabled={busy}>
              Choose a file…
            </Button>
            {error ? (
              <InlineStatus tone="error" title="Import failed">
                {error}
              </InlineStatus>
            ) : null}
            {preview ? (
              <div className="tw-import-preview">
                <p>
                  <strong>{preview.fileName}</strong>: {preview.triggerCount}{" "}
                  trigger(s) in {preview.folderCount} folder(s),{" "}
                  {preview.overlayCount} overlay preset(s), {preview.assetCount}{" "}
                  embedded sound(s).
                  {preview.quarantined > 0
                    ? ` ${preview.quarantined} will be quarantined.`
                    : ""}
                </p>
                <ReportList report={preview.report} />
                <CheckboxOption
                  label="Put everything in a new folder"
                  checked={wrapInFolder}
                  onChange={setWrapInFolder}
                />
                {wrapInFolder ? (
                  <label>
                    <span>Folder name</span>
                    <input
                      value={folderName}
                      onChange={(event) => setFolderName(event.target.value)}
                    />
                  </label>
                ) : null}
                <CheckboxOption
                  label="Replace triggers with the same name"
                  checked={replaceSameNames}
                  onChange={setReplaceSameNames}
                />
                <CheckboxOption
                  label="Enable the imported triggers now (imports arrive disabled)"
                  checked={enable}
                  onChange={setEnable}
                />
              </div>
            ) : null}
            <div className="tw-dialog-actions">
              <Button onClick={onClose}>Cancel</Button>
              <Button
                variant="primary"
                disabled={!preview || busy}
                onClick={() => void commit()}
              >
                {busy ? "Importing…" : "Import"}
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

export function ExportDialog({
  library,
  selection,
  selectedFolder,
  onClose,
}: {
  library: TriggerLibrary;
  selection: string[];
  selectedFolder: string | null;
  onClose: () => void;
}) {
  const [format, setFormat] = useState<ExportFormat>("stonemite");
  const [scopeKind, setScopeKind] = useState<
    "selection" | "folder" | "library"
  >(selection.length > 0 ? "selection" : selectedFolder ? "folder" : "library");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<CompatReport | null>(null);
  const [savedTo, setSavedTo] = useState<string | null>(null);

  async function run() {
    setBusy(true);
    setError(null);
    try {
      const outcome = await exportTriggerSelection({
        format,
        fullLibrary: scopeKind === "library",
        folderIds:
          scopeKind === "folder" && selectedFolder ? [selectedFolder] : [],
        triggerIds: scopeKind === "selection" ? selection : [],
      });
      setReport(outcome.report.issues.length > 0 ? outcome.report : null);
      setSavedTo(outcome.path);
      if (outcome.path && outcome.report.issues.length === 0) onClose();
    } catch (problem) {
      setError(String(problem));
    } finally {
      setBusy(false);
    }
  }

  const folderName = selectedFolder
    ? (library.folders.find((folder) => folder.id === selectedFolder)?.name ??
      "folder")
    : null;

  return (
    <div className="tw-dialog-backdrop" role="presentation">
      <div
        className="tw-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Export triggers"
      >
        <h3>Export triggers</h3>
        <label>
          <span>Scope</span>
          <select
            value={scopeKind}
            onChange={(event) =>
              setScopeKind(event.target.value as typeof scopeKind)
            }
          >
            <option value="selection" disabled={selection.length === 0}>
              Selected triggers ({selection.length})
            </option>
            <option value="folder" disabled={!selectedFolder}>
              Current folder{folderName ? ` (${folderName})` : ""}
            </option>
            <option value="library">Full library</option>
          </select>
        </label>
        <label>
          <span>Format</span>
          <select
            value={format}
            onChange={(event) => setFormat(event.target.value as ExportFormat)}
          >
            <option value="stonemite">
              Stonemite package (.stonemite-triggers, includes sounds)
            </option>
            <option value="eqlpTriggers">EQLP triggers (.tgf.gz)</option>
            <option value="eqlpOverlays">EQLP overlays (.ogf.gz)</option>
          </select>
        </label>
        {format !== "stonemite" ? (
          <p className="tw-panel-hint">
            EQLP packages cannot embed media; managed sounds must be shared
            separately.
          </p>
        ) : null}
        {error ? (
          <InlineStatus tone="error" title="Export failed">
            {error}
          </InlineStatus>
        ) : null}
        {savedTo ? (
          <InlineStatus tone="success" title="Exported">
            <p>Saved to {savedTo}</p>
          </InlineStatus>
        ) : null}
        {report ? <ReportList report={report} /> : null}
        <div className="tw-dialog-actions">
          <Button onClick={onClose}>Close</Button>
          <Button variant="primary" disabled={busy} onClick={() => void run()}>
            {busy ? "Exporting…" : "Export"}
          </Button>
        </div>
      </div>
    </div>
  );
}

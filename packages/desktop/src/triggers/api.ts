import { invoke } from "@tauri-apps/api/core";

import { emptyLibrary } from "./model";
import type {
  ExportOutcome,
  ExportScope,
  ImportOptions,
  ImportPreview,
  ImportSummary,
  AssetRecord,
  TestBenchRequest,
  TestBenchResult,
  TriggerLibrary,
  TriggerLibraryPayload,
} from "./types";

const browserPreview =
  import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);

let mockLibrary: TriggerLibrary | null = null;

export function loadTriggerLibrary(): Promise<TriggerLibraryPayload> {
  if (browserPreview) {
    return Promise.resolve({
      library: mockLibrary ?? emptyLibrary(),
      report: {
        issues: [],
        triggersImported: 0,
        foldersImported: 0,
        overlaysImported: 0,
        triggersQuarantined: 0,
      },
      builtinSounds: [
        { id: "alert.wav", label: "Alert" },
        { id: "tell.wav", label: "Tell" },
      ],
    });
  }
  return invoke<TriggerLibraryPayload>("load_trigger_library");
}

export function saveTriggerLibrary(library: TriggerLibrary): Promise<void> {
  if (browserPreview) {
    mockLibrary = library;
    return Promise.resolve();
  }
  return invoke("save_trigger_library", { library });
}

export function chooseTriggerImportFile(): Promise<string | null> {
  return browserPreview
    ? Promise.resolve(null)
    : invoke<string | null>("choose_trigger_import_file");
}

export function previewTriggerImport(path: string): Promise<ImportPreview> {
  return invoke<ImportPreview>("preview_trigger_import", { path });
}

export function commitTriggerImport(
  path: string,
  options: ImportOptions,
): Promise<ImportSummary> {
  return invoke<ImportSummary>("commit_trigger_import", { path, options });
}

export function exportTriggerSelection(
  scope: ExportScope,
): Promise<ExportOutcome> {
  if (browserPreview) {
    return Promise.resolve({
      path: null,
      report: {
        issues: [],
        triggersImported: 0,
        foldersImported: 0,
        overlaysImported: 0,
        triggersQuarantined: 0,
      },
    });
  }
  return invoke<ExportOutcome>("export_trigger_selection", { scope });
}

export function addTriggerMedia(): Promise<AssetRecord | null> {
  return browserPreview
    ? Promise.resolve(null)
    : invoke<AssetRecord | null>("add_trigger_media");
}

export function previewTriggerSound(reference: string): Promise<void> {
  return browserPreview
    ? Promise.resolve()
    : invoke("preview_trigger_sound", { reference });
}

export function runTriggerTest(
  request: TestBenchRequest,
): Promise<TestBenchResult> {
  if (browserPreview) {
    return Promise.resolve({
      lines: request.lines.map((line, index) => ({
        line,
        atMs: index * 1000,
        trace: { line, entries: [] },
        events: [],
        timersAfter: [],
      })),
      compileErrors: [],
      activeTriggers: 0,
    });
  }
  return invoke<TestBenchResult>("run_trigger_test", { request });
}

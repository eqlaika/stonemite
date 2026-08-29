// Pure helpers for the Trigger Manager: construction, tree shaping,
// filtering, and library mutations kept out of the React components so
// they stay unit-testable.

import type {
  Folder,
  Pattern,
  TimerBehavior,
  TimerStageActions,
  Trigger,
  TriggerLibrary,
} from "./types";

export function newId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) {
    return crypto.randomUUID();
  }
  // Test environments without WebCrypto: RFC4122-shaped fallback.
  return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    return (c === "x" ? r : (r & 0x3) | 0x8).toString(16);
  });
}

export function emptyLibrary(): TriggerLibrary {
  return {
    schemaVersion: 1,
    folders: [],
    triggers: [],
    profiles: [],
    textOverlays: [],
    timerOverlays: [],
    assets: [],
  };
}

export function emptyPattern(): Pattern {
  return { text: "", useRegex: false };
}

export function emptyStage(): TimerStageActions {
  return { displayText: null, speakText: null, sound: null };
}

export function defaultTimer(): TimerBehavior {
  return {
    kind: "countdown",
    timerName: "",
    durationSeconds: 30,
    resetDurationSeconds: 0,
    timesToLoop: 0,
    restartMode: "startNew",
    warningSeconds: 0,
    warning: emptyStage(),
    end: emptyStage(),
    earlyEnd: emptyStage(),
    endEarlyPatterns: [],
    endEarlyRepeatedCount: 0,
    endClearVariables: [],
  };
}

export function newTrigger(folder: string | null, index: number): Trigger {
  return {
    id: newId(),
    name: "New trigger",
    folder,
    index,
    enabled: false,
    comments: "",
    pattern: emptyPattern(),
    previousPattern: null,
    condition: "",
    lockoutSeconds: 0,
    repeatedResetSeconds: 0.75,
    variableActions: [],
    displayText: null,
    speakText: null,
    sound: null,
    timer: null,
    priority: 3,
    voiceRate: 0,
    volume: 4,
    textOverlays: [],
    timerOverlays: [],
    fontColor: null,
    activeColor: null,
    idleColor: null,
    resetColor: null,
    target: "source",
    quarantine: null,
    passthrough: {},
  };
}

export function newFolder(parent: string | null, index: number): Folder {
  return { id: newId(), name: "New folder", parent, index, expanded: true };
}

export interface FolderNode {
  folder: Folder;
  depth: number;
  children: FolderNode[];
}

/** Root-first folder tree ordered by index then name. */
export function folderTree(library: TriggerLibrary): FolderNode[] {
  const byParent = new Map<string | null, Folder[]>();
  for (const folder of library.folders) {
    const key = folder.parent ?? null;
    const list = byParent.get(key) ?? [];
    list.push(folder);
    byParent.set(key, list);
  }
  const build = (parent: string | null, depth: number): FolderNode[] => {
    if (depth > 32) return [];
    const children = byParent.get(parent) ?? [];
    children.sort((a, b) => a.index - b.index || a.name.localeCompare(b.name));
    return children.map((folder) => ({
      folder,
      depth,
      children: build(folder.id, depth + 1),
    }));
  };
  return build(null, 0);
}

/** Ids of a folder and everything transitively under it. */
export function folderSubtree(
  library: TriggerLibrary,
  folderId: string,
): string[] {
  const result = [folderId];
  for (let cursor = 0; cursor < result.length; cursor += 1) {
    for (const folder of library.folders) {
      if (folder.parent === result[cursor] && !result.includes(folder.id)) {
        result.push(folder.id);
      }
    }
  }
  return result;
}

export type StatusFilter = "all" | "enabled" | "disabled" | "quarantined";

export function filterTriggers(
  library: TriggerLibrary,
  selectedFolder: string | null,
  search: string,
  status: StatusFilter,
): Trigger[] {
  const scope =
    selectedFolder === null ? null : folderSubtree(library, selectedFolder);
  const query = search.trim().toLowerCase();
  const matches = library.triggers.filter((trigger) => {
    if (scope && (trigger.folder === null || !scope.includes(trigger.folder)))
      return false;
    if (status === "enabled" && !trigger.enabled) return false;
    if (status === "disabled" && trigger.enabled) return false;
    if (status === "quarantined" && !trigger.quarantine) return false;
    if (!query) return true;
    return (
      trigger.name.toLowerCase().includes(query) ||
      trigger.pattern.text.toLowerCase().includes(query) ||
      (trigger.displayText ?? "").toLowerCase().includes(query) ||
      (trigger.speakText ?? "").toLowerCase().includes(query) ||
      trigger.comments.toLowerCase().includes(query)
    );
  });
  matches.sort(
    (a, b) =>
      (a.folder ?? "").localeCompare(b.folder ?? "") ||
      a.index - b.index ||
      a.name.localeCompare(b.name),
  );
  return matches;
}

/** Replace one trigger by id, returning a new library. */
export function withTrigger(
  library: TriggerLibrary,
  trigger: Trigger,
): TriggerLibrary {
  return {
    ...library,
    triggers: library.triggers.map((existing) =>
      existing.id === trigger.id ? trigger : existing,
    ),
  };
}

export function removeTriggers(
  library: TriggerLibrary,
  ids: string[],
): TriggerLibrary {
  return {
    ...library,
    triggers: library.triggers.filter((trigger) => !ids.includes(trigger.id)),
    profiles: library.profiles.map((profile) => ({
      ...profile,
      triggers: profile.triggers.filter((id) => !ids.includes(id)),
    })),
  };
}

export function duplicateTrigger(
  library: TriggerLibrary,
  id: string,
): TriggerLibrary {
  const source = library.triggers.find((trigger) => trigger.id === id);
  if (!source) return library;
  const copy: Trigger = {
    ...structuredCloneSafe(source),
    id: newId(),
    name: `${source.name} copy`,
    enabled: false,
    index: source.index + 1,
  };
  return { ...library, triggers: [...library.triggers, copy] };
}

export function moveTriggers(
  library: TriggerLibrary,
  ids: string[],
  folder: string | null,
): TriggerLibrary {
  return {
    ...library,
    triggers: library.triggers.map((trigger) =>
      ids.includes(trigger.id) ? { ...trigger, folder } : trigger,
    ),
  };
}

export function setTriggersEnabled(
  library: TriggerLibrary,
  ids: string[],
  enabled: boolean,
): TriggerLibrary {
  return {
    ...library,
    triggers: library.triggers.map((trigger) =>
      ids.includes(trigger.id) && !(enabled && trigger.quarantine)
        ? { ...trigger, enabled }
        : trigger,
    ),
  };
}

export function removeFolder(
  library: TriggerLibrary,
  id: string,
): TriggerLibrary {
  // Deleting a folder deletes its subtree; contained triggers move to root
  // rather than silently disappearing.
  const subtree = folderSubtree(library, id);
  return {
    ...library,
    folders: library.folders.filter((folder) => !subtree.includes(folder.id)),
    triggers: library.triggers.map((trigger) =>
      trigger.folder && subtree.includes(trigger.folder)
        ? { ...trigger, folder: null }
        : trigger,
    ),
    profiles: library.profiles.map((profile) => ({
      ...profile,
      folders: profile.folders.filter(
        (folderId) => !subtree.includes(folderId),
      ),
    })),
  };
}

function structuredCloneSafe<T>(value: T): T {
  return typeof structuredClone === "function"
    ? structuredClone(value)
    : (JSON.parse(JSON.stringify(value)) as T);
}

export function describeTriggerActions(trigger: Trigger): string {
  const parts: string[] = [];
  if (trigger.displayText) parts.push("text");
  if (trigger.speakText) parts.push("speech");
  if (trigger.sound) parts.push("sound");
  if (trigger.timer) parts.push("timer");
  return parts.length > 0 ? parts.join(" · ") : "no actions";
}

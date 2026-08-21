import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type Dispatch,
  type ReactNode,
  type SetStateAction,
} from "react";

import { loadSettings, saveSettings } from "./api";
import type {
  SaveOutcome,
  SettingsDraft,
  SettingsOptions,
  SettingsRuntime,
} from "./types";

type LoadState = "loading" | "ready" | "error";
type SaveState = "idle" | "saving" | "error";

interface SettingsContextValue {
  loadState: LoadState;
  loadError: string | null;
  saveState: SaveState;
  saveError: string | null;
  draft: SettingsDraft | null;
  setDraft: Dispatch<SetStateAction<SettingsDraft | null>>;
  options: SettingsOptions | null;
  runtime: SettingsRuntime | null;
  dirty: boolean;
  save: () => Promise<SaveOutcome | null>;
  reset: () => void;
}

const SettingsContext = createContext<SettingsContextValue | null>(null);

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function cloneDraft(draft: SettingsDraft): SettingsDraft {
  return structuredClone(draft);
}

export function SettingsProvider({ children }: { children: ReactNode }) {
  const [loadState, setLoadState] = useState<LoadState>("loading");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saveState, setSaveState] = useState<SaveState>("idle");
  const [saveError, setSaveError] = useState<string | null>(null);
  const [initialDraft, setInitialDraft] = useState<SettingsDraft | null>(null);
  const [draft, setDraft] = useState<SettingsDraft | null>(null);
  const [options, setOptions] = useState<SettingsOptions | null>(null);
  const [runtime, setRuntime] = useState<SettingsRuntime | null>(null);

  useEffect(() => {
    let active = true;
    void loadSettings()
      .then((payload) => {
        if (!active) return;
        setInitialDraft(cloneDraft(payload.draft));
        setDraft(cloneDraft(payload.draft));
        setOptions(payload.options);
        setRuntime(payload.runtime);
        setLoadState("ready");
      })
      .catch((error: unknown) => {
        if (!active) return;
        setLoadError(errorMessage(error));
        setLoadState("error");
      });
    return () => {
      active = false;
    };
  }, []);

  const dirty = useMemo(
    () =>
      draft !== null &&
      initialDraft !== null &&
      JSON.stringify(draft) !== JSON.stringify(initialDraft),
    [draft, initialDraft],
  );

  const save = useCallback(async () => {
    if (!draft) return null;
    setSaveState("saving");
    setSaveError(null);
    try {
      const outcome = await saveSettings(draft);
      setInitialDraft(cloneDraft(draft));
      setSaveState("idle");
      return outcome;
    } catch (error: unknown) {
      setSaveError(errorMessage(error));
      setSaveState("error");
      return null;
    }
  }, [draft]);

  const reset = useCallback(() => {
    if (initialDraft) setDraft(cloneDraft(initialDraft));
    setSaveState("idle");
    setSaveError(null);
  }, [initialDraft]);

  const value = useMemo<SettingsContextValue>(
    () => ({
      loadState,
      loadError,
      saveState,
      saveError,
      draft,
      setDraft,
      options,
      runtime,
      dirty,
      save,
      reset,
    }),
    [
      loadState,
      loadError,
      saveState,
      saveError,
      draft,
      options,
      runtime,
      dirty,
      save,
      reset,
    ],
  );

  return <SettingsContext value={value}>{children}</SettingsContext>;
}

export function useSettings(): SettingsContextValue {
  const value = useContext(SettingsContext);
  if (!value)
    throw new Error("useSettings must be used inside SettingsProvider");
  return value;
}

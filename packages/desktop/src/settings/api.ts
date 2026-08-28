import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";

import {
  beginMockPairing,
  loadMockRunningCharacters,
  loadMockSettings,
  resetMockDpsOverlayPlacement,
  saveMockSettings,
} from "./mock";
import type {
  PairingSession,
  RunningCharacter,
  SaveOutcome,
  SettingsDraft,
  SettingsPayload,
} from "./types";

const browserPreview =
  import.meta.env.DEV && !("__TAURI_INTERNALS__" in window);

export function loadSettings(): Promise<SettingsPayload> {
  return browserPreview
    ? loadMockSettings()
    : invoke<SettingsPayload>("load_settings");
}

export function loadRunningCharacters(): Promise<RunningCharacter[]> {
  return browserPreview
    ? loadMockRunningCharacters()
    : invoke<RunningCharacter[]>("load_running_characters");
}

export function saveSettings(draft: SettingsDraft): Promise<SaveOutcome> {
  return browserPreview
    ? saveMockSettings()
    : invoke<SaveOutcome>("save_settings", { draft });
}

export function resetDpsOverlayPlacement(): Promise<void> {
  return browserPreview
    ? resetMockDpsOverlayPlacement()
    : invoke("reset_dps_overlay_placement");
}

export function chooseEqDirectory(
  currentDirectory: string,
): Promise<string | null> {
  return browserPreview
    ? Promise.resolve(currentDirectory)
    : invoke<string | null>("choose_eq_directory", { currentDirectory });
}

export function previewNotificationSound(sound: string): Promise<void> {
  return browserPreview
    ? Promise.resolve()
    : invoke("preview_notification_sound", { sound });
}

export function beginPairing(): Promise<PairingSession> {
  return browserPreview
    ? beginMockPairing()
    : invoke<PairingSession>("begin_pairing");
}

export function pairingIsOpen(): Promise<boolean> {
  return browserPreview
    ? Promise.resolve(true)
    : invoke<boolean>("pairing_is_open");
}

export function cancelPairing(): Promise<void> {
  return browserPreview ? Promise.resolve() : invoke("cancel_pairing");
}

export function requestRestart(): Promise<void> {
  return browserPreview ? Promise.resolve() : invoke("request_restart");
}

export function openExternal(target: string): Promise<void> {
  if (browserPreview) {
    window.open(target, "_blank", "noopener,noreferrer");
    return Promise.resolve();
  }
  return invoke("open_external", { target });
}

export async function closeSettingsWindow(): Promise<void> {
  if (!browserPreview) await getCurrentWindow().close();
}

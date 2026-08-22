import type { PairingSession, SaveOutcome, SettingsPayload } from "./types";

const mockPayload: SettingsPayload = {
  draft: {
    general: {
      eqDirectory:
        "C:\\Users\\Public\\Daybreak Game Company\\Installed Games\\EverQuest",
      hideFromAltTab: true,
      integrations: { enabled: true, lanEnabled: false },
      toast: { enabled: true, height: 64, durationSeconds: 2 },
      updates: { automatic: true, intervalDays: 7 },
    },
    accounts: {
      server: "Xegony - Druzzil Ro",
      accounts: [
        { username: "laika_primary", password: "preview-password" },
        { username: "laika_box", password: "preview-password" },
      ],
    },
    pip: {
      edge: "right",
      labelHeight: 48,
      labelOpacity: 80,
      autoOrder: true,
      hideHotkey: "F9",
    },
    notifications: {
      visualEnabled: true,
      soundEnabled: true,
      sound: "tell.wav",
      tells: true,
      groupInvites: true,
      raidInvites: true,
      resurrections: true,
      deaths: true,
    },
    hotkeys: {
      swapHotkeys: [
        "Ctrl+F1",
        "Ctrl+F2",
        "Ctrl+F3",
        "Ctrl+F4",
        "Ctrl+F5",
        "Ctrl+F6",
      ],
    },
    broadcasting: {
      toggleHotkey: "Pause",
      mouseClutchKey: "F13",
      filterMode: "blacklist",
      filterKeys: ["Enter", "Escape", "Tab"],
    },
  },
  options: {
    servers: [
      { value: "", label: "None" },
      { value: "Teek", label: "Teek" },
      { value: "Xegony - Druzzil Ro", label: "Xegony - Druzzil Ro" },
    ],
    pipEdges: [
      { value: "right", label: "Right" },
      { value: "left", label: "Left" },
      { value: "top", label: "Top" },
      { value: "bottom", label: "Bottom" },
    ],
    notificationSounds: [
      { value: "tell.wav", label: "Tell" },
      { value: "alert.wav", label: "Alert" },
      { value: "thump.wav", label: "Thump" },
    ],
    filterModes: [
      { value: "blacklist", label: "Blacklist" },
      { value: "whitelist", label: "Whitelist" },
    ],
  },
  runtime: {
    version: "2026.08.22-dev",
    trusikEnabled: true,
    integrationAddress: "gaming-pc.local:19720",
  },
};

export function loadMockSettings(): Promise<SettingsPayload> {
  return Promise.resolve(structuredClone(mockPayload));
}

export function saveMockSettings(): Promise<SaveOutcome> {
  return Promise.resolve({ restartRequired: false });
}

export function beginMockPairing(): Promise<PairingSession> {
  return Promise.resolve({
    code: "042 731",
    address: mockPayload.runtime.integrationAddress,
    expiresInSeconds: 300,
  });
}

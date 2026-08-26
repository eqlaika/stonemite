import type {
  PairingSession,
  RunningCharacter,
  SaveOutcome,
  SettingsPayload,
} from "./types";

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
    boxOrder: [
      { server: "xegony", character: "Laika" },
      { server: "xegony", character: "Bilka" },
      { server: "bristlebane", character: "Foo" },
    ],
    pip: {
      edge: "right",
      showStonemiteButton: true,
      thumbnailOpacity: 80,
      labelHeight: 48,
      labelOpacity: 80,
      fontFamily: "Segoe UI",
      fontScale: 100,
      fontWeight: "bold",
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
      tradeProposals: true,
      resurrections: true,
      deaths: true,
      levelGains: true,
      aaGains: true,
      aaPointsPerNotification: 1,
      combatAwarenessEnabled: true,
      combatHitDurationSeconds: 3,
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
      boxCycles: [
        {
          name: "Melee",
          nextHotkey: "F14",
          previousHotkey: "F15",
          members: [
            { server: "xegony", character: "Laika" },
            { server: "xegony", character: "Bilka" },
          ],
        },
      ],
    },
    broadcasting: {
      toggleHotkey: "Pause",
      disableWhenClientsExit: true,
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
    knownCharacters: [
      { server: "bristlebane", character: "Foo" },
      { server: "teek", character: "Orlov" },
      { server: "xegony", character: "Bilka" },
      { server: "xegony", character: "Kafka" },
      { server: "xegony", character: "Laika" },
    ],
    pipEdges: [
      { value: "right", label: "Right" },
      { value: "left", label: "Left" },
      { value: "top", label: "Top" },
      { value: "bottom", label: "Bottom" },
    ],
    labelFontFamilies: [
      "Arial",
      "Calibri",
      "Consolas",
      "Georgia",
      "Segoe UI",
      "Tahoma",
      "Trebuchet MS",
      "Verdana",
    ],
    labelFontWeights: [
      { value: "regular", label: "Regular" },
      { value: "semibold", label: "Semibold" },
      { value: "bold", label: "Bold" },
      { value: "heavy", label: "Heavy" },
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

export function loadMockRunningCharacters(): Promise<RunningCharacter[]> {
  return Promise.resolve([
    { server: "xegony", character: "Laika", windowNumber: 1 },
    { server: "xegony", character: "Bilka", windowNumber: 2 },
    { server: "xegony", character: "Kafka", windowNumber: 3 },
  ]);
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

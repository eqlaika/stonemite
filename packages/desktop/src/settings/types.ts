export type PipEdge = "right" | "left" | "top" | "bottom";
export type LabelFontWeight = "regular" | "semibold" | "bold" | "heavy";
export type BroadcastFilterMode = "blacklist" | "whitelist";

export interface OptionItem<T extends string = string> {
  value: T;
  label: string;
}

export interface AccountDraft {
  username: string;
  password: string;
}

export interface BoxIdentity {
  server: string;
  character: string;
}

export interface RunningCharacter extends BoxIdentity {
  windowNumber: number | null;
}

export interface IntegrationSettings {
  enabled: boolean;
  lanEnabled: boolean;
}

export interface ToastSettings {
  enabled: boolean;
  height: number;
  durationSeconds: number;
}

export interface UpdateSettings {
  automatic: boolean;
  intervalDays: number;
}

export interface GeneralSettings {
  eqDirectory: string;
  hideFromAltTab: boolean;
  integrations: IntegrationSettings;
  toast: ToastSettings;
  updates: UpdateSettings;
}

export interface AccountsSettings {
  server: string;
  accounts: AccountDraft[];
}

export interface PipSettings {
  edge: PipEdge;
  thumbnailOpacity: number;
  labelHeight: number;
  labelOpacity: number;
  fontFamily: string;
  fontScale: number;
  fontWeight: LabelFontWeight;
  autoOrder: boolean;
  hideHotkey: string;
}

export interface NotificationSettings {
  visualEnabled: boolean;
  soundEnabled: boolean;
  sound: string;
  tells: boolean;
  groupInvites: boolean;
  raidInvites: boolean;
  resurrections: boolean;
  deaths: boolean;
}

export interface HotkeySettings {
  swapHotkeys: string[];
}

export interface BroadcastingSettings {
  toggleHotkey: string;
  mouseClutchKey: string;
  filterMode: BroadcastFilterMode;
  filterKeys: string[];
}

export interface SettingsDraft {
  general: GeneralSettings;
  accounts: AccountsSettings;
  boxOrder: BoxIdentity[];
  pip: PipSettings;
  notifications: NotificationSettings;
  hotkeys: HotkeySettings;
  broadcasting: BroadcastingSettings;
}

export interface SettingsOptions {
  servers: OptionItem[];
  knownCharacters: BoxIdentity[];
  pipEdges: OptionItem<PipEdge>[];
  labelFontFamilies: string[];
  labelFontWeights: OptionItem<LabelFontWeight>[];
  notificationSounds: OptionItem[];
  filterModes: OptionItem<BroadcastFilterMode>[];
}

export interface SettingsRuntime {
  version: string;
  trusikEnabled: boolean;
  integrationAddress: string;
}

export interface SettingsPayload {
  draft: SettingsDraft;
  options: SettingsOptions;
  runtime: SettingsRuntime;
}

export interface SaveOutcome {
  restartRequired: boolean;
}

export interface PairingSession {
  code: string;
  address: string;
  expiresInSeconds: number;
}

import type { DashboardKey } from "../state/layout";

export interface DashboardActionDefinition {
  key: DashboardKey;
  name: string;
  configurableCharacter?: boolean;
  tooltip: string;
  uuid: string;
}

const UUID_PREFIX = "co.laikasoft.stonemite";

export const CHARACTER_ACTION_DEFINITION = {
  key: "character-1",
  name: "Character",
  tooltip:
    "Activate one configured Stonemite box, or select it while Swap is armed.",
  uuid: `${UUID_PREFIX}.character`,
  configurableCharacter: true,
} as const satisfies DashboardActionDefinition;

export const XTARGET_ACTION_DEFINITION = {
  name: "XTarget",
  tooltip:
    "Select saved Extended Target slots, consider the target, and activate one configured Stonemite box.",
  uuid: `${UUID_PREFIX}.xtarget`,
} as const;

export const HOTKEY_ACTION_DEFINITION = {
  name: "Hotkey",
  tooltip:
    "Send one configured EverQuest keymap action to all loaded, active, background, or selected boxes.",
  uuid: `${UUID_PREFIX}.hotkey`,
} as const;

export const DASHBOARD_ACTION_DEFINITIONS = [
  ...([1, 2, 3, 4, 5, 6] as const).map((slot): DashboardActionDefinition => ({
    key: `character-${slot}`,
    name: `Character slot ${slot}`,
    tooltip: `Activate the character in Stonemite slot ${slot}, or select it while Swap is armed.`,
    uuid: `${UUID_PREFIX}.character-slot-${slot}`,
  })),
  CHARACTER_ACTION_DEFINITION,
  {
    key: "broadcast",
    name: "Broadcast",
    tooltip: "Turn Stonemite key broadcasting on or off.",
    uuid: `${UUID_PREFIX}.broadcast`,
  },
  {
    key: "mouse-clutch",
    name: "Mouse Clutch",
    tooltip:
      "Hold to send the foreground EQ client's physical mouse to compatible background boxes.",
    uuid: `${UUID_PREFIX}.mouse-clutch`,
  },
  {
    key: "swap",
    name: "Swap",
    tooltip: "Arm a window-number swap, then choose a character slot.",
    uuid: `${UUID_PREFIX}.swap`,
  },
  {
    key: "logo",
    name: "Stonemite setup",
    tooltip: "Show connection status and configure access from another PC.",
    uuid: `${UUID_PREFIX}.logo`,
  },
] as const satisfies ReadonlyArray<DashboardActionDefinition>;

const KEYS_BY_UUID = new Map(
  DASHBOARD_ACTION_DEFINITIONS.map(({ key, uuid }) => [uuid, key]),
);

export function keyForManifestId(manifestId: string): DashboardKey | undefined {
  return KEYS_BY_UUID.get(manifestId);
}

export function definitionForKey(key: DashboardKey): DashboardActionDefinition {
  const definition = DASHBOARD_ACTION_DEFINITIONS.find(
    (candidate) => candidate.key === key,
  );
  if (!definition) throw new Error(`Missing Stream Deck action for ${key}.`);
  return definition;
}

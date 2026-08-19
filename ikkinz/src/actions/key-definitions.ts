import type { DashboardKey } from "../state/layout";

export interface DashboardActionDefinition {
  key: DashboardKey;
  name: string;
  tooltip: string;
  uuid: string;
}

const UUID_PREFIX = "co.laikasoft.ikkinz";

export const DASHBOARD_ACTION_DEFINITIONS = [
  ...([1, 2, 3, 4, 5, 6] as const).map((slot): DashboardActionDefinition => ({
    key: `character-${slot}`,
    name: `Character slot ${slot}`,
    tooltip: `Activate the character in Stonemite slot ${slot}, or select it while Swap is armed.`,
    uuid: `${UUID_PREFIX}.character-slot-${slot}`,
  })),
  {
    key: "group",
    name: "Group",
    tooltip: "Invite every ready character and accept the invitations.",
    uuid: `${UUID_PREFIX}.group`,
  },
  {
    key: "broadcast",
    name: "Broadcast",
    tooltip: "Turn Stonemite key broadcasting on or off.",
    uuid: `${UUID_PREFIX}.broadcast`,
  },
  {
    key: "follow",
    name: "Follow",
    tooltip:
      "Tell every ready background character to follow the active character.",
    uuid: `${UUID_PREFIX}.follow`,
  },
  {
    key: "use",
    name: "Use",
    tooltip: "Invoke Use Center Screen on every ready character.",
    uuid: `${UUID_PREFIX}.use`,
  },
  {
    key: "assist",
    name: "Assist",
    tooltip:
      "Tell every ready background character to assist the active character.",
    uuid: `${UUID_PREFIX}.assist`,
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

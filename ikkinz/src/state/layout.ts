import type { DashboardView, Feedback } from "./store";
import type { TrusharClient, TrusharState } from "../types/trushar";

export const GRID_COLUMNS = 5;
export const GRID_ROWS = 3;

export type CharacterSlot = 1 | 2 | 3 | 4 | 5 | 6;
export type DashboardKey =
  | `character-${CharacterSlot}`
  | "group"
  | "broadcast"
  | "follow"
  | "assist"
  | "swap"
  | "logo";

export const DEFAULT_LAYOUT = [
  ["character-1", "character-2", "character-3", "group", "broadcast"],
  ["character-4", "character-5", "character-6", "follow", "blank"],
  ["logo", "blank", "blank", "assist", "swap"],
] as const satisfies ReadonlyArray<ReadonlyArray<DashboardKey | "blank">>;

export const SLOT_COLORS = [
  "#4a86d4",
  "#6ab060",
  "#d85858",
  "#e0b848",
  "#a07cc8",
  "#58c8a8",
] as const;
export const BADGE_COLORS = [
  "#3068b0",
  "#489040",
  "#b83838",
  "#c09828",
  "#805ca8",
  "#38a888",
] as const;

export type KeyCell = { row?: number; column?: number } & (
  | { type: "boot"; stage: number }
  | { type: "feedback"; feedback: Feedback }
  | {
      type: "character";
      client: TrusharClient;
      slot: CharacterSlot;
      enabled: boolean;
      interaction: "activate" | "swap";
    }
  | { type: "empty"; slot: CharacterSlot }
  | { type: "blank" }
  | { type: "logo" }
  | {
      type: "group";
      available: boolean;
      ready: number;
      status: string;
    }
  | {
      type: "follow";
      available: boolean;
      ready: number;
      status: string;
    }
  | {
      type: "assist";
      available: boolean;
      ready: number;
      status: string;
    }
  | {
      type: "broadcast";
      available: boolean;
      enabled: boolean;
    }
  | {
      type: "swap";
      available: boolean;
      armed: boolean;
      status: string;
    }
);

export interface SwapPlan {
  active: TrusharClient | null;
  available: boolean;
  status: string;
}

export function buildSwapPlan(view: DashboardView): SwapPlan {
  const snapshot = view.snapshot;
  const active = activeClient(snapshot);
  const capabilityAvailable = Boolean(
    snapshot?.capabilities.swap_window_numbers,
  );
  const hasVisibleTarget = Boolean(
    active &&
    snapshot?.clients.some(
      (client) =>
        client.id !== active.id &&
        client.window_number >= 1 &&
        client.window_number <= 6,
    ),
  );
  const available =
    view.connection.state === "connected" &&
    capabilityAvailable &&
    Boolean(active) &&
    hasVisibleTarget;

  let status: string;
  if (view.connection.state !== "connected") status = "OFFLINE";
  else if (!snapshot) status = "NO STATE";
  else if (!capabilityAvailable) status = "UPDATE STONEMITE";
  else if (!active) status = "NO ACTIVE BOX";
  else if (snapshot.clients.length < 2) status = "ONE CLIENT";
  else if (!hasVisibleTarget) status = "NO VISIBLE TARGET";
  else status = "PRESS THEN PICK";

  return { active, available, status };
}

export type GroupInvitee = TrusharClient & { character: string };

export interface GroupPlan {
  active: TrusharClient | null;
  invitees: GroupInvitee[];
  available: boolean;
  status: string;
}

export function buildGroupPlan(view: DashboardView): GroupPlan {
  const snapshot = view.snapshot;
  const active = activeClient(snapshot);
  const invitees =
    snapshot && active
      ? snapshot.clients.filter(
          (client): client is GroupInvitee =>
            client.id !== active.id &&
            client.input_ready &&
            typeof client.character === "string" &&
            client.character.trim().length > 0,
        )
      : [];
  const inputAvailable = Boolean(
    snapshot?.capabilities.send_text && snapshot.capabilities.send_keys,
  );
  const available =
    view.connection.state === "connected" &&
    inputAvailable &&
    Boolean(active?.input_ready) &&
    invitees.length > 0;

  let status: string;
  if (view.connection.state !== "connected") status = "OFFLINE";
  else if (!snapshot) status = "NO STATE";
  else if (!inputAvailable) status = "INPUT UNAVAILABLE";
  else if (!active) status = "NO ACTIVE BOX";
  else if (!active.input_ready) status = "ACTIVE NOT READY";
  else if (invitees.length === 0) status = "NO READY BOXES";
  else
    status = `${invitees.length} ${invitees.length === 1 ? "BOX" : "BOXES"} READY`;

  return { active, invitees, available, status };
}

export type FollowLeader = TrusharClient & { character: string };

export interface FollowPlan {
  leader: FollowLeader | null;
  followers: TrusharClient[];
  available: boolean;
  status: string;
}

export function buildFollowPlan(view: DashboardView): FollowPlan {
  const snapshot = view.snapshot;
  const active = activeClient(snapshot);
  const leader =
    active &&
    typeof active.character === "string" &&
    active.character.trim().length > 0
      ? (active as FollowLeader)
      : null;
  const followers =
    snapshot && leader
      ? snapshot.clients.filter(
          (client) => client.id !== leader.id && client.input_ready,
        )
      : [];
  const inputAvailable = Boolean(snapshot?.capabilities.send_text);
  const available =
    view.connection.state === "connected" &&
    inputAvailable &&
    Boolean(leader) &&
    followers.length > 0;

  let status: string;
  if (view.connection.state !== "connected") status = "OFFLINE";
  else if (!snapshot) status = "NO STATE";
  else if (!inputAvailable) status = "INPUT UNAVAILABLE";
  else if (!active) status = "NO ACTIVE BOX";
  else if (!leader) status = "LEADER UNKNOWN";
  else if (followers.length === 0) status = "NO READY BOXES";
  else
    status = `${followers.length} ${followers.length === 1 ? "BOX" : "BOXES"} READY`;

  return { leader, followers, available, status };
}

export type AssistMain = FollowLeader;

export interface AssistPlan {
  main: AssistMain | null;
  assistants: TrusharClient[];
  available: boolean;
  status: string;
}

export function buildAssistPlan(view: DashboardView): AssistPlan {
  const followPlan = buildFollowPlan(view);
  return {
    main: followPlan.leader,
    assistants: followPlan.followers,
    available: followPlan.available,
    status:
      followPlan.status === "LEADER UNKNOWN"
        ? "MAIN UNKNOWN"
        : followPlan.status,
  };
}

function activeClient(snapshot: TrusharState | null): TrusharClient | null {
  if (!snapshot) return null;
  return (
    snapshot.clients.find(
      (client) => client.id === snapshot.active_client_id,
    ) ??
    snapshot.clients.find((client) => client.active) ??
    null
  );
}

export function cellKey(row: number, column: number): string {
  return `${row},${column}`;
}

export function buildGrid(view: DashboardView): KeyCell[] {
  const cells: KeyCell[] = [];
  for (let row = 0; row < GRID_ROWS; row += 1) {
    for (let column = 0; column < GRID_COLUMNS; column += 1) {
      cells.push(buildCell(view, row, column));
    }
  }
  return cells;
}

export function buildCell(
  view: DashboardView,
  row: number,
  column: number,
  swapArmed = false,
): KeyCell {
  if (!isCoordinate(row, column)) {
    throw new RangeError(`Grid coordinate is outside 5 by 3: ${row},${column}`);
  }

  const key = DEFAULT_LAYOUT[row]?.[column];
  if (!key || key === "blank") return { type: "blank", row, column };
  return {
    ...buildKey(view, key, cellKey(row, column), swapArmed),
    row,
    column,
  };
}

export function buildKey(
  view: DashboardView,
  key: DashboardKey,
  feedbackKey: string = key,
  swapArmed = false,
): KeyCell {
  if (key === "logo") return { type: "logo" };

  const feedback = view.feedback.get(feedbackKey);
  if (feedback && feedback.until > Date.now())
    return { type: "feedback", feedback };

  if (view.bootStage < 3) return { type: "boot", stage: view.bootStage };

  const slot = characterSlot(key);
  if (slot !== null) {
    const swapPlan = buildSwapPlan(view);
    const swapMode = swapArmed && swapPlan.available;
    const client = view.snapshot?.clients.find(
      (candidate) => candidate.window_number === slot,
    );
    if (!client) return { type: "empty", slot };
    return {
      type: "character",
      client,
      slot,
      enabled:
        view.connection.state === "connected" &&
        (swapMode ? swapPlan.available : client.activatable),
      interaction: swapMode ? "swap" : "activate",
    };
  }

  switch (key) {
    case "group": {
      const plan = buildGroupPlan(view);
      return {
        type: "group",
        available: plan.available,
        ready: plan.invitees.length,
        status: plan.status,
      };
    }
    case "broadcast":
      return {
        type: "broadcast",
        available:
          view.connection.state === "connected" &&
          Boolean(view.snapshot?.broadcast.available),
        enabled: Boolean(view.snapshot?.broadcast.enabled),
      };
    case "follow": {
      const plan = buildFollowPlan(view);
      return {
        type: "follow",
        available: plan.available,
        ready: plan.followers.length,
        status: plan.status,
      };
    }
    case "assist": {
      const plan = buildAssistPlan(view);
      return {
        type: "assist",
        available: plan.available,
        ready: plan.assistants.length,
        status: plan.status,
      };
    }
    case "swap": {
      const plan = buildSwapPlan(view);
      return {
        type: "swap",
        available: plan.available,
        armed: swapArmed && plan.available,
        status: plan.status,
      };
    }
    default:
      throw new Error(`Unsupported dashboard key: ${key}.`);
  }
}

function characterSlot(key: DashboardKey): CharacterSlot | null {
  switch (key) {
    case "character-1":
      return 1;
    case "character-2":
      return 2;
    case "character-3":
      return 3;
    case "character-4":
      return 4;
    case "character-5":
      return 5;
    case "character-6":
      return 6;
    default:
      return null;
  }
}

function isCoordinate(row: number, column: number): boolean {
  return (
    Number.isInteger(row) &&
    Number.isInteger(column) &&
    row >= 0 &&
    row < GRID_ROWS &&
    column >= 0 &&
    column < GRID_COLUMNS
  );
}

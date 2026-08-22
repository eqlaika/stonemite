import type { ConnectionPhase, DashboardView, Feedback } from "./store";
import type { TrusharClient, TrusharState } from "../types/trushar";

export type CharacterSlot = 1 | 2 | 3 | 4 | 5 | 6;
export type DashboardKey =
  `character-${CharacterSlot}` | "broadcast" | "swap" | "logo";

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

export type KeyCell =
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
  | { type: "logo"; connection: ConnectionPhase }
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
    };

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

export function buildKey(
  view: DashboardView,
  key: DashboardKey,
  feedbackKey: string = key,
  swapArmed = false,
): KeyCell {
  if (key === "logo")
    return { type: "logo", connection: view.connection.state };

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
    case "broadcast":
      return {
        type: "broadcast",
        available:
          view.connection.state === "connected" &&
          Boolean(view.snapshot?.broadcast.available),
        enabled: Boolean(view.snapshot?.broadcast.enabled),
      };
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

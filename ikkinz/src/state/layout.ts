import type { DashboardView, Feedback } from "./store";
import type { TrusharClient } from "../types/trushar";

export const GRID_COLUMNS = 5;
export const GRID_ROWS = 3;

const CHARACTER_POSITIONS = [
  [0, 0],
  [0, 1],
  [0, 2],
  [1, 0],
  [1, 1],
  [1, 2],
] as const;

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

export type GridCell =
  | { type: "unsupported"; row: number; column: number }
  | { type: "boot"; row: number; column: number; stage: number }
  | { type: "feedback"; row: number; column: number; feedback: Feedback }
  | {
      type: "character";
      row: number;
      column: number;
      client: TrusharClient;
      slot: number;
      enabled: boolean;
    }
  | { type: "empty"; row: number; column: number; slot: number }
  | {
      type: "utility";
      row: number;
      column: number;
      top: string;
      main: string;
      bottom: string;
      accent: string;
    }
  | {
      type: "group";
      row: number;
      column: number;
      available: boolean;
      ready: number;
      status: string;
      accent: string;
    }
  | {
      type: "broadcast";
      row: number;
      column: number;
      available: boolean;
      enabled: boolean;
    }
  | {
      type: "ambient";
      row: number;
      column: number;
      label: string;
      position: "left" | "right";
    };

export type GroupInvitee = TrusharClient & { character: string };

export interface GroupPlan {
  active: TrusharClient | null;
  invitees: GroupInvitee[];
  available: boolean;
  status: string;
}

export function buildGroupPlan(view: DashboardView): GroupPlan {
  const snapshot = view.snapshot;
  const active = snapshot
    ? (snapshot.clients.find(
        (client) => client.id === snapshot.active_client_id,
      ) ??
      snapshot.clients.find((client) => client.active) ??
      null)
    : null;
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

export function cellKey(row: number, column: number): string {
  return `${row},${column}`;
}

export function unsupportedCell(row: number, column: number): GridCell {
  return { type: "unsupported", row, column };
}

export function buildGrid(view: DashboardView): GridCell[] {
  const cells: GridCell[] = [];
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
): GridCell {
  if (!isCoordinate(row, column)) {
    throw new RangeError(`Grid coordinate is outside 5 by 3: ${row},${column}`);
  }

  const feedback = view.feedback.get(cellKey(row, column));
  if (feedback && feedback.until > Date.now())
    return { type: "feedback", row, column, feedback };

  if (view.bootStage < 3)
    return { type: "boot", row, column, stage: view.bootStage };

  const characterIndex = CHARACTER_POSITIONS.findIndex(
    ([r, c]) => r === row && c === column,
  );
  if (characterIndex >= 0) {
    const slot = characterIndex + 1;
    const client = view.snapshot?.clients.find(
      (candidate) => candidate.window_number === slot,
    );
    if (!client) return { type: "empty", row, column, slot };
    return {
      type: "character",
      row,
      column,
      client,
      slot,
      enabled: view.connection.state === "connected" && client.activatable,
    };
  }

  if (row === 0 && column === 3) {
    const plan = buildGroupPlan(view);
    return {
      type: "group",
      row,
      column,
      available: plan.available,
      ready: plan.invitees.length,
      status: plan.status,
      accent: plan.available
        ? "#80df89"
        : view.connection.state === "error"
          ? "#ff826f"
          : "#ffc75c",
    };
  }

  if (row === 0 && column === 4) {
    return {
      type: "broadcast",
      row,
      column,
      available:
        view.connection.state === "connected" &&
        Boolean(view.snapshot?.broadcast.available),
      enabled: Boolean(view.snapshot?.broadcast.enabled),
    };
  }

  if (row === 1 && column === 3) {
    const clients = view.snapshot?.clients ?? [];
    const ready = clients.filter((client) => client.input_ready).length;
    return {
      type: "utility",
      row,
      column,
      top: "INPUT",
      main: `${ready} / ${clients.length}`,
      bottom:
        clients.length > 0 && ready === clients.length
          ? "READY"
          : "EXACT CLIENTS",
      accent:
        clients.length > 0 && ready === clients.length ? "#80df89" : "#ffc75c",
    };
  }

  if (row === 1 && column === 4) {
    return {
      type: "utility",
      row,
      column,
      top: "STONEMITE",
      main: view.connection.state === "connected" ? "PAIRED" : "SETUP",
      bottom:
        view.connection.state === "connected" ? "PRIVATE LAN" : "SELECT A KEY",
      accent: view.connection.state === "connected" ? "#80df89" : "#9ba5b2",
    };
  }

  const clients = view.snapshot?.clients ?? [];
  if (row === 2 && column === 0) {
    return {
      type: "utility",
      row,
      column,
      top: "GROUP",
      main: String(clients.length),
      bottom: clients.length === 1 ? "CLIENT" : "CLIENTS",
      accent: clients.length > 0 ? "#80df89" : "#9ba5b2",
    };
  }

  if (row === 2 && column === 1) {
    const active = clients.find((client) => client.active);
    return {
      type: "utility",
      row,
      column,
      top: "ACTIVE",
      main: active?.character ?? (active ? `#${active.window_number}` : "—"),
      bottom: active?.class_code ?? "CURRENT CLIENT",
      accent: active
        ? (SLOT_COLORS[(active.window_number - 1) % SLOT_COLORS.length] ??
          SLOT_COLORS[0])
        : "#9ba5b2",
    };
  }

  if (row === 2 && column === 2) {
    const servers = [
      ...new Set(
        clients.flatMap((client) => (client.server ? [client.server] : [])),
      ),
    ];
    return {
      type: "utility",
      row,
      column,
      top: "SERVER",
      main:
        servers.length === 1
          ? (servers[0] ?? "—")
          : servers.length > 1
            ? "MIXED"
            : "—",
      bottom: servers.length > 0 ? "DETECTED" : "UNKNOWN",
      accent: "#9bbf73",
    };
  }

  return {
    type: "ambient",
    row,
    column,
    label: column === 3 ? "STONE" : "MITE",
    position: column === 3 ? "left" : "right",
  };
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

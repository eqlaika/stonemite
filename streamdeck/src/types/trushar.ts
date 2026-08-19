export const PROTOCOL_VERSION = 1 as const;
export const MAX_MESSAGE_BYTES = 16 * 1024;

export interface TrusharClient {
  id: string;
  character?: string;
  server?: string;
  class_code?: string;
  window_number: number;
  active: boolean;
  activatable: boolean;
  input_ready: boolean;
}

export interface TrusharState {
  revision: number;
  clients: TrusharClient[];
  active_client_id: string | null;
  broadcast: { available: boolean; enabled: boolean };
  capabilities: {
    activate: boolean;
    swap_window_numbers: boolean;
    set_broadcast: boolean;
    send_text: boolean;
    send_keys: boolean;
    eq_actions: EqActionCapabilities;
  };
}

export interface EqActionCapabilities {
  use_center_screen: boolean;
  invite_follow: boolean;
  hotbars: number;
  hotbar_buttons: number;
  spell_gems: number;
  keymap_actions: boolean;
}

export type EqAction =
  | { type: "use_center_screen" }
  | { type: "invite_follow" }
  | { type: "hotbar"; bar: number; button: number }
  | { type: "spell_gem"; gem: number }
  | { type: "keymap"; mapping: string };

export type EqActionTargets =
  { type: "all_loaded" } | { type: "window_numbers"; window_numbers: number[] };

export interface WireError {
  code: string;
  message: string;
}

export type ActivationStatus = "activated" | "already_active";

export type Success =
  | { type: "state" }
  | {
      type: "activated";
      status: ActivationStatus;
      foreground_confirmed: boolean;
    }
  | {
      type: "window_numbers_swapped";
      active_previous_number: number;
      selected_previous_number: number;
    }
  | { type: "broadcast_set"; enabled: boolean }
  | { type: "input_delivered"; input: "text" | "keys"; strokes: number }
  | { type: "eq_action_delivered"; action: EqAction }
  | {
      type: "eq_keymap_actions_listed";
      mappings: string[];
      window_numbers: number[];
      next_after?: string;
    }
  | {
      type: "eq_action_batch_delivered";
      action: EqAction;
      window_numbers: number[];
    };

export type ServerMessage =
  | { type: "state"; version: 1; state: TrusharState }
  | {
      type: "result";
      version: 1;
      request_id: string;
      result: Success;
      state: TrusharState;
    }
  | { type: "error"; version: 1; request_id?: string; error: WireError }
  | { type: "paired"; version: 1; auth_token: string };

export interface KeyStroke {
  keys: string[];
  hold_ms?: number;
  pause_ms?: number;
}

export type ClientMessage =
  | { type: "get_state"; version: 1; request_id: string }
  | {
      type: "activate";
      version: 1;
      request_id: string;
      target: { type: "client_id"; client_id: string };
    }
  | {
      type: "swap_window_numbers";
      version: 1;
      request_id: string;
      target: { type: "client_id"; client_id: string };
    }
  | { type: "set_broadcast"; version: 1; request_id: string; enabled: boolean }
  | {
      type: "send_text";
      version: 1;
      request_id: string;
      client_id: string;
      text: string;
      submit: boolean;
    }
  | {
      type: "send_keys";
      version: 1;
      request_id: string;
      client_id: string;
      strokes: KeyStroke[];
    }
  | {
      type: "send_eq_action";
      version: 1;
      request_id: string;
      client_id: string;
      action: EqAction;
    }
  | {
      type: "list_eq_keymap_actions";
      version: 1;
      request_id: string;
      targets: EqActionTargets;
      after?: string;
    }
  | {
      type: "send_eq_action_batch";
      version: 1;
      request_id: string;
      targets: EqActionTargets;
      action: EqAction;
    };

export class ProtocolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProtocolError";
  }
}

export function parseServerMessage(input: string): ServerMessage {
  if (Buffer.byteLength(input, "utf8") > MAX_MESSAGE_BYTES) {
    throw new ProtocolError("Message exceeds the 16384-byte limit.");
  }

  let value: unknown;
  try {
    value = JSON.parse(input);
  } catch {
    throw new ProtocolError("Message is not valid JSON.");
  }
  if (!isRecord(value) || typeof value.type !== "string") {
    throw new ProtocolError("Message has no valid type.");
  }
  if (value.version !== PROTOCOL_VERSION) {
    throw new ProtocolError("Message uses an unsupported protocol version.");
  }

  switch (value.type) {
    case "state":
      return { type: "state", version: 1, state: parseState(value.state) };
    case "result":
      if (typeof value.request_id !== "string") {
        throw new ProtocolError("Result message is malformed.");
      }
      return {
        type: "result",
        version: 1,
        request_id: value.request_id,
        result: parseSuccess(value.result),
        state: parseState(value.state),
      };
    case "error": {
      if (
        !isRecord(value.error) ||
        typeof value.error.code !== "string" ||
        typeof value.error.message !== "string"
      ) {
        throw new ProtocolError("Error message is malformed.");
      }
      const message: ServerMessage = {
        type: "error",
        version: 1,
        error: { code: value.error.code, message: value.error.message },
      };
      if (typeof value.request_id === "string")
        message.request_id = value.request_id;
      return message;
    }
    case "paired":
      if (
        typeof value.auth_token !== "string" ||
        value.auth_token.length === 0
      ) {
        throw new ProtocolError("Pairing response has no credential.");
      }
      return { type: "paired", version: 1, auth_token: value.auth_token };
    default:
      throw new ProtocolError("Message type is not supported.");
  }
}

export function parseSuccess(value: unknown): Success {
  if (!isRecord(value) || typeof value.type !== "string") {
    throw new ProtocolError("Result payload is malformed.");
  }
  switch (value.type) {
    case "state":
      return { type: "state" };
    case "activated":
      if (
        (value.status !== "activated" && value.status !== "already_active") ||
        !isBoolean(value.foreground_confirmed)
      ) {
        throw new ProtocolError("Activation result is malformed.");
      }
      return {
        type: "activated",
        status: value.status,
        foreground_confirmed: value.foreground_confirmed,
      };
    case "window_numbers_swapped":
      if (
        !Number.isSafeInteger(value.active_previous_number) ||
        (value.active_previous_number as number) < 1 ||
        !Number.isSafeInteger(value.selected_previous_number) ||
        (value.selected_previous_number as number) < 1
      ) {
        throw new ProtocolError("Window-number swap result is malformed.");
      }
      return {
        type: "window_numbers_swapped",
        active_previous_number: value.active_previous_number as number,
        selected_previous_number: value.selected_previous_number as number,
      };
    case "broadcast_set":
      if (!isBoolean(value.enabled)) {
        throw new ProtocolError("Broadcast result is malformed.");
      }
      return { type: "broadcast_set", enabled: value.enabled };
    case "input_delivered":
      if (
        (value.input !== "text" && value.input !== "keys") ||
        !Number.isSafeInteger(value.strokes) ||
        (value.strokes as number) < 0
      ) {
        throw new ProtocolError("Input result is malformed.");
      }
      return {
        type: "input_delivered",
        input: value.input,
        strokes: value.strokes as number,
      };
    case "eq_action_delivered":
      return {
        type: "eq_action_delivered",
        action: parseEqAction(value.action),
      };
    case "eq_keymap_actions_listed": {
      if (
        !Array.isArray(value.mappings) ||
        !value.mappings.every(isEqMappingName) ||
        !isWindowNumberResultList(value.window_numbers) ||
        (value.next_after !== undefined && !isEqMappingName(value.next_after))
      ) {
        throw new ProtocolError("EQ keymap action list is malformed.");
      }
      const result: Extract<Success, { type: "eq_keymap_actions_listed" }> = {
        type: "eq_keymap_actions_listed",
        mappings: value.mappings,
        window_numbers: value.window_numbers,
      };
      if (typeof value.next_after === "string")
        result.next_after = value.next_after;
      return result;
    }
    case "eq_action_batch_delivered":
      if (!isWindowNumberResultList(value.window_numbers)) {
        throw new ProtocolError("EQ action batch result is malformed.");
      }
      return {
        type: "eq_action_batch_delivered",
        action: parseEqAction(value.action),
        window_numbers: value.window_numbers,
      };
    default:
      throw new ProtocolError("Result type is not supported.");
  }
}

export function parseState(value: unknown): TrusharState {
  if (
    !isRecord(value) ||
    !Number.isSafeInteger(value.revision) ||
    (value.revision as number) < 0
  ) {
    throw new ProtocolError("State revision is malformed.");
  }
  if (!Array.isArray(value.clients))
    throw new ProtocolError("State clients are malformed.");
  if (
    value.active_client_id !== null &&
    typeof value.active_client_id !== "string"
  ) {
    throw new ProtocolError("Active client id is malformed.");
  }
  if (
    !isRecord(value.broadcast) ||
    !isBoolean(value.broadcast.available) ||
    !isBoolean(value.broadcast.enabled)
  ) {
    throw new ProtocolError("Broadcast state is malformed.");
  }
  if (!isRecord(value.capabilities))
    throw new ProtocolError("Capabilities are malformed.");
  const capabilityNames = [
    "activate",
    "set_broadcast",
    "send_text",
    "send_keys",
  ] as const;
  for (const name of capabilityNames) {
    if (!isBoolean(value.capabilities[name]))
      throw new ProtocolError("Capabilities are malformed.");
  }

  return {
    revision: value.revision as number,
    clients: value.clients
      .map(parseClient)
      .sort((a, b) => a.window_number - b.window_number),
    active_client_id: value.active_client_id,
    broadcast: {
      available: value.broadcast.available as boolean,
      enabled: value.broadcast.enabled as boolean,
    },
    capabilities: {
      activate: value.capabilities.activate as boolean,
      swap_window_numbers: isBoolean(value.capabilities.swap_window_numbers)
        ? value.capabilities.swap_window_numbers
        : false,
      set_broadcast: value.capabilities.set_broadcast as boolean,
      send_text: value.capabilities.send_text as boolean,
      send_keys: value.capabilities.send_keys as boolean,
      eq_actions: parseEqActionCapabilities(value.capabilities.eq_actions),
    },
  };
}

function parseEqActionCapabilities(value: unknown): EqActionCapabilities {
  if (value === undefined) {
    return {
      use_center_screen: false,
      invite_follow: false,
      hotbars: 0,
      hotbar_buttons: 0,
      spell_gems: 0,
      keymap_actions: false,
    };
  }
  if (
    !isRecord(value) ||
    !isBoolean(value.use_center_screen) ||
    !isBoolean(value.invite_follow)
  ) {
    throw new ProtocolError("EQ action capabilities are malformed.");
  }
  for (const name of ["hotbars", "hotbar_buttons", "spell_gems"] as const) {
    if (!Number.isSafeInteger(value[name]) || (value[name] as number) < 0)
      throw new ProtocolError("EQ action capabilities are malformed.");
  }
  return {
    use_center_screen: value.use_center_screen,
    invite_follow: value.invite_follow,
    hotbars: value.hotbars as number,
    hotbar_buttons: value.hotbar_buttons as number,
    spell_gems: value.spell_gems as number,
    keymap_actions: isBoolean(value.keymap_actions)
      ? value.keymap_actions
      : false,
  };
}

function parseEqAction(value: unknown): EqAction {
  if (!isRecord(value) || typeof value.type !== "string")
    throw new ProtocolError("EQ action is malformed.");
  switch (value.type) {
    case "use_center_screen":
      return { type: "use_center_screen" };
    case "invite_follow":
      return { type: "invite_follow" };
    case "hotbar":
      if (
        !Number.isSafeInteger(value.bar) ||
        (value.bar as number) < 1 ||
        (value.bar as number) > 11 ||
        !Number.isSafeInteger(value.button) ||
        (value.button as number) < 1 ||
        (value.button as number) > 12
      ) {
        throw new ProtocolError("EQ hotbar action is malformed.");
      }
      return {
        type: "hotbar",
        bar: value.bar as number,
        button: value.button as number,
      };
    case "spell_gem":
      if (
        !Number.isSafeInteger(value.gem) ||
        (value.gem as number) < 1 ||
        (value.gem as number) > 14
      ) {
        throw new ProtocolError("EQ spell-gem action is malformed.");
      }
      return { type: "spell_gem", gem: value.gem as number };
    case "keymap":
      if (!isEqMappingName(value.mapping)) {
        throw new ProtocolError("EQ keymap action is malformed.");
      }
      return { type: "keymap", mapping: value.mapping };
    default:
      throw new ProtocolError("EQ action type is not supported.");
  }
}

function parseClient(value: unknown): TrusharClient {
  if (
    !isRecord(value) ||
    typeof value.id !== "string" ||
    value.id.length === 0
  ) {
    throw new ProtocolError("Client id is malformed.");
  }
  if (
    !Number.isSafeInteger(value.window_number) ||
    (value.window_number as number) < 1
  ) {
    throw new ProtocolError("Client window number is malformed.");
  }
  for (const name of ["active", "activatable"] as const) {
    if (!isBoolean(value[name]))
      throw new ProtocolError(`Client ${name} is malformed.`);
  }
  const client: TrusharClient = {
    id: value.id,
    window_number: value.window_number as number,
    active: value.active as boolean,
    activatable: value.activatable as boolean,
    input_ready: isBoolean(value.input_ready) ? value.input_ready : false,
  };
  for (const name of ["character", "server", "class_code"] as const) {
    if (value[name] !== undefined) {
      if (typeof value[name] !== "string")
        throw new ProtocolError(`Client ${name} is malformed.`);
      client[name] = value[name];
    }
  }
  return client;
}

function isEqMappingName(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length >= 1 &&
    value.length <= 128 &&
    /^[A-Z0-9_]+$/u.test(value)
  );
}

function isWindowNumberResultList(value: unknown): value is number[] {
  return (
    Array.isArray(value) &&
    value.every((number) => Number.isSafeInteger(number) && number >= 1) &&
    new Set(value).size === value.length
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

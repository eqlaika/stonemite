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
    set_broadcast: boolean;
    send_text: boolean;
    send_keys: boolean;
  };
}

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
  | { type: "broadcast_set"; enabled: boolean }
  | { type: "input_delivered"; input: "text" | "keys"; strokes: number };

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

export type ClientMessage =
  | { type: "get_state"; version: 1; request_id: string }
  | {
      type: "activate";
      version: 1;
      request_id: string;
      target: { type: "client_id"; client_id: string };
    }
  | { type: "set_broadcast"; version: 1; request_id: string; enabled: boolean };

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
      set_broadcast: value.capabilities.set_broadcast as boolean,
      send_text: value.capabilities.send_text as boolean,
      send_keys: value.capabilities.send_keys as boolean,
    },
  };
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

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isBoolean(value: unknown): value is boolean {
  return typeof value === "boolean";
}

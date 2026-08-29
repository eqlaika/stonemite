import WebSocket, { type RawData } from "ws";
import {
  MAX_MESSAGE_BYTES,
  PROTOCOL_VERSION,
  ProtocolError,
  parseServerMessage,
  type ClientMessage,
  type EqAction,
  type EqActionTargets,
  type ServerMessage,
  type Success,
  type TrusharState,
} from "../types/trushar";
import type { ConnectionStatus } from "../state/store";

interface TrusharTiming {
  commandTimeoutMs: number;
  pairTimeoutMs: number;
  reconnectBaseMs: number;
  reconnectMaxMs: number;
  heartbeatIntervalMs: number;
  heartbeatTimeoutMs: number;
}

const DEFAULT_TIMING: TrusharTiming = {
  commandTimeoutMs: 8_000,
  pairTimeoutMs: 10_000,
  reconnectBaseMs: 500,
  reconnectMaxMs: 15_000,
  heartbeatIntervalMs: 12_000,
  heartbeatTimeoutMs: 5_000,
} as const;

export const DEFAULT_LOCAL_ADDRESS = "127.0.0.1:19720";

export interface ConnectionConfig {
  address: string;
  authToken?: string;
}

export interface Credentials extends ConnectionConfig {
  authToken: string;
}

export const LOCAL_CONNECTION: ConnectionConfig = {
  address: DEFAULT_LOCAL_ADDRESS,
};

export interface TrusharClientOptions {
  onState: (state: TrusharState) => void;
  onStatus: (status: ConnectionStatus) => void;
  log?: (message: string) => void;
  timing?: Partial<TrusharTiming>;
}

interface PendingCommand {
  expected: Success["type"];
  resolve: (message: Extract<ServerMessage, { type: "result" }>) => void;
  reject: (error: Error) => void;
  timer: ReturnType<typeof setTimeout>;
}

interface ActivePairing {
  socket: WebSocket;
  cancel: () => void;
}

export class CommandError extends Error {
  constructor(
    public readonly code: string,
    message: string,
  ) {
    super(message);
    this.name = "CommandError";
  }
}

export class TrusharClient {
  #socket: WebSocket | null = null;
  #pairing: ActivePairing | null = null;
  #credentials: ConnectionConfig | null = null;
  #generation = 0;
  #requestSequence = 0;
  #reconnectAttempt = 0;
  #reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  #heartbeatTimer: ReturnType<typeof setTimeout> | null = null;
  #pongTimer: ReturnType<typeof setTimeout> | null = null;
  #pending = new Map<string, PendingCommand>();
  readonly #onState: (state: TrusharState) => void;
  readonly #onStatus: (status: ConnectionStatus) => void;
  readonly #log: (message: string) => void;
  readonly #timing: TrusharTiming;

  constructor(options: TrusharClientOptions) {
    this.#onState = options.onState;
    this.#onStatus = options.onStatus;
    this.#log = options.log ?? (() => undefined);
    this.#timing = { ...DEFAULT_TIMING, ...options.timing };
  }

  configure(credentials: ConnectionConfig | null): void {
    const next = credentials
      ? {
          address: normalizeAddress(credentials.address),
          ...(credentials.authToken
            ? { authToken: credentials.authToken }
            : {}),
        }
      : null;
    const unchanged =
      this.#credentials === null
        ? next === null
        : next !== null &&
          this.#credentials.address === next.address &&
          this.#credentials.authToken === next.authToken;
    if (unchanged) return;

    this.#cancelPairing();
    this.#generation += 1;
    this.#credentials = next;
    this.#cancelReconnect();
    this.#stopHeartbeat();
    this.#closeSocket();
    this.#rejectPending(
      new CommandError("connection_closed", "Stonemite connection changed."),
    );
    this.#reconnectAttempt = 0;
    if (!this.#credentials) {
      this.#onStatus({
        state: "idle",
        title: "Disconnected",
        detail: "The Stonemite connection is stopped.",
      });
      return;
    }
    this.#connect(this.#generation, false);
  }

  reconnect(): void {
    this.#cancelPairing();
    if (!this.#credentials) {
      this.#onStatus({
        state: "idle",
        title: "Disconnected",
        detail: "The Stonemite connection is stopped.",
      });
      return;
    }
    this.#generation += 1;
    this.#cancelReconnect();
    this.#stopHeartbeat();
    this.#closeSocket();
    this.#rejectPending(
      new CommandError(
        "connection_closed",
        "Stonemite reconnect interrupted the command.",
      ),
    );
    this.#reconnectAttempt = 0;
    this.#connect(this.#generation, false);
  }

  disconnect(): void {
    this.#cancelPairing();
    this.configure(null);
  }

  async pair(addressInput: string, code: string): Promise<Credentials> {
    if (!/^\d{6}$/.test(code))
      throw new CommandError(
        "invalid_code",
        "Pairing code must contain exactly six digits.",
      );
    const address = normalizeAddress(addressInput);
    this.#cancelPairing();
    this.#generation += 1;
    const generation = this.#generation;
    this.#cancelReconnect();
    this.#stopHeartbeat();
    this.#closeSocket();
    this.#rejectPending(
      new CommandError("connection_closed", "Pairing started."),
    );
    this.#onStatus({
      state: "pairing",
      title: "Pairing",
      detail: `Contacting ${address}.`,
    });

    const authToken = await new Promise<string>((resolve, reject) => {
      const socket = new WebSocket(endpoint(address, "/trushar/v1/pair"), {
        handshakeTimeout: this.#timing.pairTimeoutMs,
        maxPayload: MAX_MESSAGE_BYTES,
      });
      let settled = false;
      const timer = setTimeout(
        () =>
          finish(
            new CommandError(
              "pairing_timeout",
              "Pairing timed out. Start a new code and try again.",
            ),
          ),
        this.#timing.pairTimeoutMs,
      );
      timer.unref?.();

      const finish = (error?: Error, token?: string): void => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        if (this.#pairing?.socket === socket) this.#pairing = null;
        socket.removeAllListeners();
        socket.on("error", () => undefined);
        if (socket.readyState === WebSocket.OPEN) socket.close();
        else if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
        if (error) reject(error);
        else if (token) resolve(token);
        else reject(new CommandError("pairing_failed", "Pairing failed."));
      };

      this.#pairing = {
        socket,
        cancel: () =>
          finish(
            new CommandError("pairing_cancelled", "Pairing was cancelled."),
          ),
      };

      socket.on("open", () => {
        if (generation !== this.#generation) {
          finish(
            new CommandError("pairing_cancelled", "Pairing was cancelled."),
          );
          return;
        }
        socket.send(
          JSON.stringify({ type: "pair", version: PROTOCOL_VERSION, code }),
        );
      });
      socket.on("message", (data, isBinary) => {
        try {
          const message = decodeFrame(data, isBinary);
          if (message.type === "paired") finish(undefined, message.auth_token);
          else if (message.type === "error")
            finish(new CommandError(message.error.code, message.error.message));
          else
            finish(
              new ProtocolError(
                "Pairing endpoint returned an unexpected message.",
              ),
            );
        } catch (error) {
          finish(asError(error));
        }
      });
      socket.on("unexpected-response", (_request, response) => {
        response.resume();
        finish(
          new CommandError(
            "pairing_unavailable",
            `Pairing was rejected with HTTP ${response.statusCode}.`,
          ),
        );
      });
      socket.on("error", () => {
        finish(
          new CommandError("connection_failed", `Could not reach ${address}.`),
        );
      });
      socket.on("close", () => {
        if (!settled)
          finish(
            new CommandError(
              "pairing_closed",
              "Pairing closed before it completed.",
            ),
          );
      });
    });

    if (generation !== this.#generation)
      throw new CommandError("pairing_cancelled", "Pairing was cancelled.");
    return { address, authToken };
  }

  async activate(
    clientId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type: "activate",
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId("activate"),
        target: { type: "client_id", client_id: clientId },
      },
      "activated",
    );
  }

  async sendEqAction(
    clientId: string,
    action: EqAction,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type: "send_eq_action",
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId("eq-action"),
        client_id: clientId,
        action,
      },
      "eq_action_delivered",
    );
  }

  async swapWindowNumbers(
    clientId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type: "swap_window_numbers",
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId("swap-numbers"),
        target: { type: "client_id", client_id: clientId },
      },
      "window_numbers_swapped",
    );
  }

  async setBroadcast(
    enabled: boolean,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type: "set_broadcast",
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId("broadcast"),
        enabled,
      },
      "broadcast_set",
    );
  }

  async beginMouseClutch(
    holdId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#updateMouseClutch(
      "begin_mouse_clutch",
      "clutch-begin",
      holdId,
    );
  }

  async renewMouseClutch(
    holdId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#updateMouseClutch(
      "renew_mouse_clutch",
      "clutch-renew",
      holdId,
    );
  }

  async endMouseClutch(
    holdId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#updateMouseClutch("end_mouse_clutch", "clutch-end", holdId);
  }

  async listEqKeymapActions(targets: EqActionTargets): Promise<
    Extract<Success, { type: "eq_keymap_actions_listed" }> & {
      mappings: string[];
    }
  > {
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const mappings = new Set<string>();
      const cursors = new Set<string>();
      let after: string | undefined;
      let targetSignature: string | undefined;
      let windowNumbers: number[] = [];
      let rosterChanged = false;
      for (let page = 0; page < 256; page += 1) {
        const message = await this.#request(
          {
            type: "list_eq_keymap_actions",
            version: PROTOCOL_VERSION,
            request_id: this.#nextRequestId("eq-keymaps"),
            targets,
            ...(after ? { after } : {}),
          },
          "eq_keymap_actions_listed",
        );
        const result = message.result;
        if (result.type !== "eq_keymap_actions_listed") {
          throw new ProtocolError(
            "Stonemite returned the wrong keymap list result.",
          );
        }
        const pageSignature = keymapTargetSignature(
          message.state,
          result.window_numbers,
        );
        if (
          targetSignature !== undefined &&
          pageSignature !== targetSignature
        ) {
          rosterChanged = true;
          break;
        }
        targetSignature = pageSignature;
        for (const mapping of result.mappings) mappings.add(mapping);
        windowNumbers = result.window_numbers;
        if (!result.next_after) {
          return {
            type: "eq_keymap_actions_listed",
            mappings: [...mappings],
            window_numbers: windowNumbers,
          };
        }
        if (cursors.has(result.next_after)) {
          throw new ProtocolError(
            "Stonemite repeated an EQ keymap page cursor.",
          );
        }
        cursors.add(result.next_after);
        after = result.next_after;
      }
      if (!rosterChanged) {
        throw new ProtocolError("Stonemite returned too many EQ keymap pages.");
      }
    }
    throw new CommandError(
      "target_changed",
      "The selected EQ roster changed while key mappings were loading. Refresh mappings and try again.",
    );
  }

  async sendEqActionBatch(
    targets: EqActionTargets,
    action: EqAction,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type: "send_eq_action_batch",
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId("eq-action-batch"),
        targets,
        action,
      },
      "eq_action_batch_delivered",
    );
  }

  #updateMouseClutch(
    type: "begin_mouse_clutch" | "renew_mouse_clutch" | "end_mouse_clutch",
    requestKind: string,
    holdId: string,
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    return this.#request(
      {
        type,
        version: PROTOCOL_VERSION,
        request_id: this.#nextRequestId(requestKind),
        hold_id: holdId,
      },
      "mouse_clutch_hold_updated",
    );
  }

  #connect(generation: number, reconnecting: boolean): void {
    const credentials = this.#credentials;
    if (!credentials || generation !== this.#generation) return;
    const local = !credentials.authToken;
    this.#onStatus({
      state: reconnecting ? "reconnecting" : "connecting",
      title: reconnecting ? "Reconnecting" : "Connecting",
      detail: local
        ? "Looking for Stonemite on this PC."
        : `Contacting ${credentials.address}.`,
    });
    const socket = new WebSocket(endpoint(credentials.address, "/trushar/v1"), {
      ...(credentials.authToken
        ? { headers: { Authorization: `Bearer ${credentials.authToken}` } }
        : {}),
      handshakeTimeout: this.#timing.commandTimeoutMs,
      maxPayload: MAX_MESSAGE_BYTES,
    });
    let terminalFailure = false;
    this.#socket = socket;

    socket.on("open", () => {
      if (generation !== this.#generation) return socket.close();
      this.#reconnectAttempt = 0;
      this.#onStatus({
        state: "connected",
        title: "Connected",
        detail: credentials.address,
      });
      this.#log("Connected to Stonemite.");
      this.#scheduleHeartbeat(socket, generation);
    });
    socket.on("pong", () => {
      if (generation !== this.#generation || socket !== this.#socket) return;
      if (this.#pongTimer) clearTimeout(this.#pongTimer);
      this.#pongTimer = null;
      this.#scheduleHeartbeat(socket, generation);
    });
    socket.on("message", (data, isBinary) => {
      if (generation !== this.#generation) return;
      try {
        this.#handleMessage(decodeFrame(data, isBinary));
      } catch (error) {
        terminalFailure = true;
        this.#stopHeartbeat();
        this.#rejectPending(
          new CommandError(
            "protocol_error",
            "Stonemite sent an incompatible response.",
          ),
        );
        this.#onStatus({
          state: "error",
          title: "Protocol error",
          detail:
            "Stonemite sent an incompatible response. Restart or update it.",
        });
        this.#log(
          `Stopped after an invalid Stonemite message: ${asError(error).message}`,
        );
        socket.close(1002, "invalid protocol message");
      }
    });
    socket.on("unexpected-response", (_request, response) => {
      if (generation !== this.#generation) return;
      terminalFailure =
        response.statusCode === 401 || response.statusCode === 403;
      response.resume();
      this.#onStatus({
        state: "error",
        title: "Connection rejected",
        detail: terminalFailure
          ? credentials.authToken
            ? "Pair again to refresh this device credential."
            : "Check the integration access setting in Stonemite."
          : `Stonemite returned HTTP ${response.statusCode}. Retrying.`,
      });
      socket.terminate();
    });
    socket.on("error", () => {
      // Close owns retry and user-visible status; never log credentials or raw frames.
    });
    socket.on("close", () => {
      this.#stopHeartbeat();
      if (this.#socket === socket) this.#socket = null;
      this.#rejectPending(
        new CommandError(
          "connection_closed",
          "Stonemite disconnected before the command completed.",
        ),
      );
      if (
        generation !== this.#generation ||
        !this.#credentials ||
        terminalFailure
      )
        return;
      this.#scheduleReconnect(generation);
    });
  }

  #handleMessage(message: ServerMessage): void {
    if (message.type === "state") {
      this.#onState(message.state);
      return;
    }
    if (message.type === "result") {
      const pending = this.#pending.get(message.request_id);
      if (!pending) return;
      if (message.result.type !== pending.expected) {
        clearTimeout(pending.timer);
        this.#pending.delete(message.request_id);
        pending.reject(
          new CommandError(
            "protocol_error",
            `Expected ${pending.expected} but received ${message.result.type}.`,
          ),
        );
        throw new ProtocolError(
          "Command result type did not match its request.",
        );
      }
      this.#onState(message.state);
      clearTimeout(pending.timer);
      this.#pending.delete(message.request_id);
      pending.resolve(message);
      return;
    }
    if (message.type === "error") {
      if (!message.request_id)
        throw new ProtocolError("Stonemite returned an uncorrelated error.");
      const pending = this.#pending.get(message.request_id);
      if (!pending) return;
      clearTimeout(pending.timer);
      this.#pending.delete(message.request_id);
      pending.reject(
        new CommandError(message.error.code, message.error.message),
      );
      return;
    }
    throw new ProtocolError(
      "Stonemite returned a pairing-only message on the control endpoint.",
    );
  }

  #request(
    message: ClientMessage,
    expected: Success["type"],
  ): Promise<Extract<ServerMessage, { type: "result" }>> {
    const socket = this.#socket;
    if (!socket || socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(
        new CommandError("not_connected", "Stonemite is not connected."),
      );
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.#pending.delete(message.request_id);
        reject(
          new CommandError(
            "command_timeout",
            "Stonemite did not answer before the command timed out.",
          ),
        );
        if (socket === this.#socket) {
          this.#onStatus({
            state: "reconnecting",
            title: "Connection stalled",
            detail: "Stonemite stopped responding. Retrying automatically.",
          });
          socket.terminate();
        }
      }, this.#timing.commandTimeoutMs);
      timer.unref?.();
      this.#pending.set(message.request_id, {
        expected,
        resolve,
        reject,
        timer,
      });
      socket.send(JSON.stringify(message), (error) => {
        if (!error) return;
        const pending = this.#pending.get(message.request_id);
        if (!pending) return;
        clearTimeout(pending.timer);
        this.#pending.delete(message.request_id);
        pending.reject(
          new CommandError("send_failed", "The command could not be sent."),
        );
      });
    });
  }

  #nextRequestId(kind: string): string {
    this.#requestSequence += 1;
    return `streamdeck-${kind}-${Date.now().toString(36)}-${this.#requestSequence.toString(36)}`;
  }

  #scheduleHeartbeat(socket: WebSocket, generation: number): void {
    if (this.#heartbeatTimer) clearTimeout(this.#heartbeatTimer);
    this.#heartbeatTimer = setTimeout(() => {
      this.#heartbeatTimer = null;
      if (
        socket !== this.#socket ||
        generation !== this.#generation ||
        socket.readyState !== WebSocket.OPEN
      )
        return;
      socket.ping();
      this.#pongTimer = setTimeout(() => {
        this.#pongTimer = null;
        if (socket !== this.#socket || generation !== this.#generation) return;
        this.#onStatus({
          state: "reconnecting",
          title: "Connection stalled",
          detail: "The Stonemite connection stopped responding. Retrying.",
        });
        socket.terminate();
      }, this.#timing.heartbeatTimeoutMs);
      this.#pongTimer.unref?.();
    }, this.#timing.heartbeatIntervalMs);
    this.#heartbeatTimer.unref?.();
  }

  #stopHeartbeat(): void {
    if (this.#heartbeatTimer) clearTimeout(this.#heartbeatTimer);
    if (this.#pongTimer) clearTimeout(this.#pongTimer);
    this.#heartbeatTimer = null;
    this.#pongTimer = null;
  }

  #scheduleReconnect(generation: number): void {
    this.#cancelReconnect();
    this.#reconnectAttempt += 1;
    const base = Math.min(
      this.#timing.reconnectMaxMs,
      this.#timing.reconnectBaseMs *
        2 ** Math.min(this.#reconnectAttempt - 1, 5),
    );
    const delay = Math.round(base * (0.85 + Math.random() * 0.3));
    this.#onStatus({
      state: "reconnecting",
      title: "Reconnecting",
      detail: "Stonemite is unavailable. Retrying automatically.",
    });
    this.#reconnectTimer = setTimeout(() => {
      this.#reconnectTimer = null;
      this.#connect(generation, true);
    }, delay);
    this.#reconnectTimer.unref?.();
  }

  #cancelReconnect(): void {
    if (this.#reconnectTimer) clearTimeout(this.#reconnectTimer);
    this.#reconnectTimer = null;
  }

  #cancelPairing(): void {
    const pairing = this.#pairing;
    this.#pairing = null;
    pairing?.cancel();
  }

  #closeSocket(): void {
    const socket = this.#socket;
    this.#socket = null;
    if (socket) {
      socket.removeAllListeners();
      socket.on("error", () => undefined);
      if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
      else if (socket.readyState === WebSocket.OPEN) socket.close();
    }
  }

  #rejectPending(error: Error): void {
    for (const pending of this.#pending.values()) {
      clearTimeout(pending.timer);
      pending.reject(error);
    }
    this.#pending.clear();
  }
}

export function normalizeAddress(input: string): string {
  const trimmed = input.trim();
  if (!trimmed)
    throw new CommandError(
      "invalid_address",
      "Enter the address shown by Stonemite.",
    );
  let url: URL;
  try {
    url = new URL(trimmed.includes("://") ? trimmed : `ws://${trimmed}`);
  } catch {
    throw new CommandError(
      "invalid_address",
      "The Stonemite address is not valid.",
    );
  }
  if (
    url.protocol !== "ws:" ||
    url.username ||
    url.password ||
    url.search ||
    url.hash ||
    (url.pathname !== "/" && url.pathname !== "")
  ) {
    throw new CommandError(
      "invalid_address",
      "Use a host and port only, such as stonemite-pc.local:19720.",
    );
  }
  if (!url.hostname || !url.port)
    throw new CommandError(
      "invalid_address",
      "Include the Stonemite host and port.",
    );
  return url.host.toLowerCase();
}

function keymapTargetSignature(
  state: TrusharState,
  windowNumbers: number[],
): string {
  return JSON.stringify(
    [...windowNumbers]
      .sort((a, b) => a - b)
      .map((windowNumber) => {
        const client = state.clients.find(
          (candidate) => candidate.window_number === windowNumber,
        );
        return [
          windowNumber,
          client?.id ?? null,
          client?.character ?? null,
          client?.server ?? null,
          client?.class_code ?? null,
        ];
      }),
  );
}

function endpoint(address: string, path: string): string {
  return `ws://${address}${path}`;
}

function decodeFrame(data: RawData, isBinary: boolean): ServerMessage {
  if (isBinary) throw new ProtocolError("Binary messages are not supported.");
  const buffer = Buffer.isBuffer(data)
    ? data
    : Array.isArray(data)
      ? Buffer.concat(data)
      : Buffer.from(data);
  if (buffer.byteLength > MAX_MESSAGE_BYTES)
    throw new ProtocolError("Message exceeds the 16384-byte limit.");
  return parseServerMessage(buffer.toString("utf8"));
}

function asError(value: unknown): Error {
  return value instanceof Error
    ? value
    : new Error("Unknown connection error.");
}

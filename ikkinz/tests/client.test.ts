import { once } from "node:events";
import net from "node:net";
import { afterEach, describe, expect, it, vi } from "vitest";
import { WebSocketServer, type WebSocket } from "ws";
import {
  CommandError,
  normalizeAddress,
  TrusharClient,
} from "../src/trushar/client";
import type { ConnectionStatus } from "../src/state/store";
import type { TrusharState } from "../src/types/trushar";
import { stateFixture } from "./fixtures";

const cleanups: Array<() => Promise<void> | void> = [];

afterEach(async () => {
  while (cleanups.length) await cleanups.pop()?.();
});

describe("address validation", () => {
  it.each([
    ["stonemite-pc.local:19720", "stonemite-pc.local:19720"],
    [" WS://STONEMITE-PC.Local:19720 ", "stonemite-pc.local:19720"],
    ["[::1]:19720", "[::1]:19720"],
  ])("normalizes %s", (input, expected) => {
    expect(normalizeAddress(input)).toBe(expected);
  });

  it.each([
    "",
    "https://host:19720",
    "ws://host:19720/path",
    "ws://user:pass@host:19720",
    "host",
  ])("rejects unsafe or incomplete address %s", (input) =>
    expect(() => normalizeAddress(input)).toThrow(CommandError),
  );
});

describe("LAN pairing and commands", () => {
  it("pairs without Origin, reconnects with Authorization, and correlates out-of-order results", async () => {
    const server = await startServer();
    const address = `127.0.0.1:${server.port}`;
    const statuses: ConnectionStatus[] = [];
    const states: TrusharState[] = [];
    const requests: Array<Record<string, unknown>> = [];
    let sawPairOrigin: string | undefined;
    let sawAuthorization: string | undefined;
    let normalSocket: WebSocket | undefined;

    server.wss.on("connection", (socket, request) => {
      if (request.url === "/trushar/v1/pair") {
        sawPairOrigin = request.headers.origin;
        socket.once("message", (raw) => {
          expect(JSON.parse(raw.toString())).toEqual({
            type: "pair",
            version: 1,
            code: "482731",
          });
          socket.send(
            JSON.stringify({
              type: "paired",
              version: 1,
              auth_token: "long-random-token",
            }),
          );
        });
        return;
      }
      normalSocket = socket;
      sawAuthorization = request.headers.authorization;
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
      socket.on("message", (raw) => {
        requests.push(JSON.parse(raw.toString()) as Record<string, unknown>);
        if (requests.length !== 2) return;
        const [first, second] = requests;
        socket.send(
          JSON.stringify({
            type: "state",
            version: 1,
            state: stateFixture({ revision: 5 }),
          }),
        );
        socket.send(
          JSON.stringify({
            type: "result",
            version: 1,
            request_id: second?.request_id,
            result: { type: "broadcast_set", enabled: true },
            state: stateFixture({
              revision: 5,
              broadcast: { available: true, enabled: true },
            }),
          }),
        );
        socket.send(
          JSON.stringify({
            type: "result",
            version: 1,
            request_id: first?.request_id,
            result: {
              type: "activated",
              status: "already_active",
              foreground_confirmed: true,
            },
            state: stateFixture({
              revision: 6,
              broadcast: { available: true, enabled: true },
            }),
          }),
        );
      });
    });

    const client = new TrusharClient({
      onState: (state) => states.push(state),
      onStatus: (status) => statuses.push(status),
    });
    cleanups.push(() => client.disconnect());
    const credentials = await client.pair(address, "482731");
    expect(credentials).toEqual({ address, authToken: "long-random-token" });
    expect(sawPairOrigin).toBeUndefined();

    client.configure(credentials);
    await vi.waitFor(() => expect(states).toHaveLength(1));
    expect(sawAuthorization).toBe("Bearer long-random-token");
    expect(statuses.at(-1)?.state).toBe("connected");

    const activate = client.activate("client-1");
    const broadcast = client.setBroadcast(true);
    const [activationResult, broadcastResult] = await Promise.all([
      activate,
      broadcast,
    ]);
    expect(activationResult.result).toMatchObject({
      type: "activated",
      status: "already_active",
    });
    expect(broadcastResult.result).toMatchObject({
      type: "broadcast_set",
      enabled: true,
    });
    expect(states.map((state) => state.revision)).toEqual([4, 5, 5, 6]);
    expect(states.at(-1)?.revision).toBe(6);
    expect(requests.map((request) => request.type)).toEqual([
      "activate",
      "set_broadcast",
    ]);
    expect((requests[0]?.target as Record<string, unknown>).client_id).toBe(
      "client-1",
    );
    expect(normalSocket?.readyState).toBe(1);
  });

  it("sends exact-client number swaps, text submission, and semantic key chords", async () => {
    const server = await startServer();
    const requests: Array<Record<string, unknown>> = [];
    server.wss.on("connection", (socket) => {
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
      socket.on("message", (raw) => {
        const request = JSON.parse(raw.toString()) as Record<string, unknown>;
        requests.push(request);
        socket.send(
          JSON.stringify({
            type: "result",
            version: 1,
            request_id: request.request_id,
            result:
              request.type === "swap_window_numbers"
                ? {
                    type: "window_numbers_swapped",
                    active_previous_number: 1,
                    selected_previous_number: 2,
                  }
                : {
                    type: "input_delivered",
                    input: request.type === "send_text" ? "text" : "keys",
                    strokes: 1,
                  },
            state: stateFixture(),
          }),
        );
      });
    });
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: () => undefined,
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${server.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(server.wss.clients.size).toBe(1));

    await client.swapWindowNumbers("serein-id");
    await client.sendText("leader-id", "/invite Serein", true);
    await client.sendKeys("serein-id", [
      { keys: ["left_control", "i"], hold_ms: 50, pause_ms: 40 },
    ]);

    expect(requests).toHaveLength(3);
    expect(requests[0]).toMatchObject({
      type: "swap_window_numbers",
      version: 1,
      target: { type: "client_id", client_id: "serein-id" },
    });
    expect(requests[1]).toMatchObject({
      type: "send_text",
      version: 1,
      client_id: "leader-id",
      text: "/invite Serein",
      submit: true,
    });
    expect(requests[2]).toMatchObject({
      type: "send_keys",
      version: 1,
      client_id: "serein-id",
      strokes: [{ keys: ["left_control", "i"], hold_ms: 50, pause_ms: 40 }],
    });
  });

  it("surfaces structured command errors", async () => {
    const server = await startServer();
    server.wss.on("connection", (socket) => {
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
      socket.on("message", (raw) => {
        const request = JSON.parse(raw.toString()) as { request_id: string };
        socket.send(
          JSON.stringify({
            type: "error",
            version: 1,
            request_id: request.request_id,
            error: {
              code: "target_disappeared",
              message: "the target is no longer loaded",
            },
          }),
        );
      });
    });
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: () => undefined,
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${server.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(server.wss.clients.size).toBe(1));
    await expect(client.activate("stale-client")).rejects.toMatchObject({
      code: "target_disappeared",
    });
  });

  it("does not retry a rejected credential forever", async () => {
    let attempts = 0;
    const wss = new WebSocketServer({
      port: 0,
      verifyClient: (_info, done) => {
        attempts += 1;
        done(false, 401, "Unauthorized");
      },
    });
    await once(wss, "listening");
    cleanups.push(async () => closeServer(wss));
    const address = wss.address();
    if (!address || typeof address === "string")
      throw new Error("Test server did not bind to TCP.");
    const statuses: ConnectionStatus[] = [];
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: (status) => statuses.push(status),
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${address.port}`,
      authToken: "stale-token",
    });

    await vi.waitFor(() => expect(statuses.at(-1)?.state).toBe("error"));
    await new Promise((resolve) => setTimeout(resolve, 750));
    expect(attempts).toBe(1);
    expect(statuses.at(-1)?.detail).toContain("Pair again");
  });

  it("rejects a mismatched typed result and stops on protocol incompatibility", async () => {
    const server = await startServer();
    const statuses: ConnectionStatus[] = [];
    server.wss.on("connection", (socket) => {
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
      socket.on("message", (raw) => {
        const request = JSON.parse(raw.toString()) as { request_id: string };
        socket.send(
          JSON.stringify({
            type: "result",
            version: 1,
            request_id: request.request_id,
            result: { type: "broadcast_set", enabled: true },
            state: stateFixture(),
          }),
        );
      });
    });
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: (status) => statuses.push(status),
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${server.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(server.wss.clients.size).toBe(1));
    await expect(client.activate("client-1")).rejects.toMatchObject({
      code: "protocol_error",
    });
    await vi.waitFor(() =>
      expect(statuses.at(-1)?.title).toBe("Protocol error"),
    );
  });

  it("rejects pending commands immediately on a terminal protocol fault", async () => {
    const server = await startServer();
    const statuses: ConnectionStatus[] = [];
    server.wss.on("connection", (socket) => {
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
      socket.on("message", () => socket.send("{"));
    });
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: (status) => statuses.push(status),
      timing: { commandTimeoutMs: 1_000 },
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${server.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(server.wss.clients.size).toBe(1));

    await expect(client.activate("client-1")).rejects.toMatchObject({
      code: "protocol_error",
    });
    expect(statuses.at(-1)?.title).toBe("Protocol error");
  });

  it("rejects pairing-only and uncorrelated error messages on the authenticated endpoint", async () => {
    for (const message of [
      { type: "paired", version: 1, auth_token: "wrong-endpoint-token" },
      {
        type: "error",
        version: 1,
        error: { code: "internal_error", message: "uncorrelated" },
      },
    ]) {
      const server = await startServer();
      const statuses: ConnectionStatus[] = [];
      server.wss.on("connection", (socket) =>
        socket.send(JSON.stringify(message)),
      );
      const client = new TrusharClient({
        onState: () => undefined,
        onStatus: (status) => statuses.push(status),
        timing: { reconnectBaseMs: 20, reconnectMaxMs: 20 },
      });
      cleanups.push(() => client.disconnect());
      client.configure({
        address: `127.0.0.1:${server.port}`,
        authToken: "token",
      });

      await vi.waitFor(() =>
        expect(statuses.at(-1)?.title).toBe("Protocol error"),
      );
      expect(server.wss.clients.size).toBeLessThanOrEqual(1);
      client.disconnect();
    }
  });

  it("rejects pending commands immediately when reconnecting", async () => {
    const server = await startServer();
    let connections = 0;
    server.wss.on("connection", (socket) => {
      connections += 1;
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
    });
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: () => undefined,
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${server.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(connections).toBe(1));
    const pending = client.activate("client-1");
    client.reconnect();
    await expect(pending).rejects.toMatchObject({ code: "connection_closed" });
    await vi.waitFor(() => expect(connections).toBe(2));
  });

  it("retries transient HTTP upgrade failures but keeps auth failures terminal", async () => {
    let attempts = 0;
    const states: TrusharState[] = [];
    const wss = new WebSocketServer({
      port: 0,
      verifyClient: (_info, done) => {
        attempts += 1;
        if (attempts === 1) done(false, 503, "Unavailable");
        else done(true);
      },
    });
    await once(wss, "listening");
    cleanups.push(async () => closeServer(wss));
    wss.on("connection", (socket) =>
      socket.send(
        JSON.stringify({
          type: "state",
          version: 1,
          state: stateFixture({ revision: 12 }),
        }),
      ),
    );
    const address = wss.address();
    if (!address || typeof address === "string") throw new Error("No port.");
    const client = new TrusharClient({
      onState: (state) => states.push(state),
      onStatus: () => undefined,
      timing: { reconnectBaseMs: 20, reconnectMaxMs: 20 },
    });
    cleanups.push(() => client.disconnect());
    client.configure({
      address: `127.0.0.1:${address.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(states.at(-1)?.revision).toBe(12));
    expect(attempts).toBe(2);
  });

  it("cancels a superseded pairing before sending its one-time code", async () => {
    let releaseHandshake: ((allow: boolean) => void) | undefined;
    let pairMessages = 0;
    const wss = new WebSocketServer({
      port: 0,
      verifyClient: (_info, done) => {
        releaseHandshake = (allow) => done(allow, allow ? undefined : 403);
      },
    });
    await once(wss, "listening");
    cleanups.push(async () => closeServer(wss));
    wss.on("connection", (socket) =>
      socket.on("message", () => {
        pairMessages += 1;
      }),
    );
    const address = wss.address();
    if (!address || typeof address === "string") throw new Error("No port.");
    const client = new TrusharClient({
      onState: () => undefined,
      onStatus: () => undefined,
    });
    cleanups.push(() => client.disconnect());
    const pairing = client.pair(`127.0.0.1:${address.port}`, "482731");
    await vi.waitFor(() => expect(releaseHandshake).toBeTypeOf("function"));
    client.disconnect();
    releaseHandshake?.(true);
    await expect(pairing).rejects.toMatchObject({ code: "pairing_cancelled" });
    expect(pairMessages).toBe(0);
  });

  it("uses heartbeat pongs and reconnects a half-open LAN link", async () => {
    const healthy = await startServer();
    let healthyConnections = 0;
    healthy.wss.on("connection", (socket) => {
      healthyConnections += 1;
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
    });
    const healthyClient = new TrusharClient({
      onState: () => undefined,
      onStatus: () => undefined,
      timing: { heartbeatIntervalMs: 20, heartbeatTimeoutMs: 20 },
    });
    cleanups.push(() => healthyClient.disconnect());
    healthyClient.configure({
      address: `127.0.0.1:${healthy.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(healthyConnections).toBe(1));
    await new Promise((resolve) => setTimeout(resolve, 90));
    expect(healthyConnections).toBe(1);

    const silentWss = new WebSocketServer({ port: 0, autoPong: false });
    await once(silentWss, "listening");
    cleanups.push(async () => closeServer(silentWss));
    let silentConnections = 0;
    silentWss.on("connection", (socket) => {
      silentConnections += 1;
      socket.send(
        JSON.stringify({ type: "state", version: 1, state: stateFixture() }),
      );
    });
    const silentAddress = silentWss.address();
    if (!silentAddress || typeof silentAddress === "string")
      throw new Error("No port.");
    const statuses: ConnectionStatus[] = [];
    const silentClient = new TrusharClient({
      onState: () => undefined,
      onStatus: (status) => statuses.push(status),
      timing: {
        heartbeatIntervalMs: 20,
        heartbeatTimeoutMs: 20,
        reconnectBaseMs: 20,
        reconnectMaxMs: 20,
      },
    });
    cleanups.push(() => silentClient.disconnect());
    silentClient.configure({
      address: `127.0.0.1:${silentAddress.port}`,
      authToken: "token",
    });
    await vi.waitFor(() => expect(silentConnections).toBeGreaterThan(1), {
      timeout: 1_000,
    });
    expect(
      statuses.some((status) => status.title === "Connection stalled"),
    ).toBe(true);
  });

  it("shows a terminal sanitized status for malformed or binary frames", async () => {
    for (const binary of [false, true]) {
      const server = await startServer();
      const statuses: ConnectionStatus[] = [];
      let connections = 0;
      server.wss.on("connection", (socket) => {
        connections += 1;
        socket.send(binary ? Buffer.from([1, 2, 3]) : "{", { binary });
      });
      const client = new TrusharClient({
        onState: () => undefined,
        onStatus: (status) => statuses.push(status),
        timing: { reconnectBaseMs: 20, reconnectMaxMs: 20 },
      });
      cleanups.push(() => client.disconnect());
      client.configure({
        address: `127.0.0.1:${server.port}`,
        authToken: "token",
      });
      await vi.waitFor(() =>
        expect(statuses.at(-1)?.title).toBe("Protocol error"),
      );
      await new Promise((resolve) => setTimeout(resolve, 60));
      expect(connections).toBe(1);
      client.disconnect();
    }
  });

  it("recovers when Ikkinz starts before Stonemite", async () => {
    const port = await unusedPort();
    const states: TrusharState[] = [];
    const statuses: ConnectionStatus[] = [];
    const client = new TrusharClient({
      onState: (state) => states.push(state),
      onStatus: (status) => statuses.push(status),
    });
    cleanups.push(() => client.disconnect());
    client.configure({ address: `127.0.0.1:${port}`, authToken: "token" });
    await vi.waitFor(() =>
      expect(statuses.some((status) => status.state === "reconnecting")).toBe(
        true,
      ),
    );

    const wss = new WebSocketServer({ port });
    await once(wss, "listening");
    cleanups.push(async () => closeServer(wss));
    wss.on("connection", (socket, request) => {
      expect(request.headers.authorization).toBe("Bearer token");
      socket.send(
        JSON.stringify({
          type: "state",
          version: 1,
          state: stateFixture({ revision: 9 }),
        }),
      );
    });

    await vi.waitFor(() => expect(states.at(-1)?.revision).toBe(9), {
      timeout: 3_000,
    });
    expect(statuses.at(-1)?.state).toBe("connected");
  });
});

async function startServer(): Promise<{ wss: WebSocketServer; port: number }> {
  const wss = new WebSocketServer({ port: 0 });
  await once(wss, "listening");
  const address = wss.address();
  if (!address || typeof address === "string")
    throw new Error("Test server did not bind to TCP.");
  cleanups.push(async () => closeServer(wss));
  return { wss, port: address.port };
}

async function closeServer(wss: WebSocketServer): Promise<void> {
  for (const client of wss.clients) client.terminate();
  if (wss.address() === null) return;
  await new Promise<void>((resolve) => wss.close(() => resolve()));
}

async function unusedPort(): Promise<number> {
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Could not reserve a port.");
  const { port } = address;
  await new Promise<void>((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
  return port;
}

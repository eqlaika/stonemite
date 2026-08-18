import { once } from "node:events";
import { WebSocketServer } from "ws";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommandError } from "../src/trushar/client";
import { DashboardStore } from "../src/state/store";
import { stateFixture } from "./fixtures";

const sdk = vi.hoisted(() => ({
  getGlobalSettings: vi.fn(),
  setGlobalSettings: vi.fn(),
  sendToPropertyInspector: vi.fn(),
}));

vi.mock("@elgato/streamdeck", () => ({
  default: {
    settings: {
      getGlobalSettings: sdk.getGlobalSettings,
      setGlobalSettings: sdk.setGlobalSettings,
    },
    ui: { sendToPropertyInspector: sdk.sendToPropertyInspector },
  },
}));

import {
  credentialsForReconnect,
  GridKeyController,
  updateVisibleKeyImage,
  type VisibleKey,
} from "../src/actions/grid-key-controller";

beforeEach(() => {
  sdk.getGlobalSettings.mockReset();
  sdk.setGlobalSettings.mockReset().mockResolvedValue(undefined);
  sdk.sendToPropertyInspector.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("GridKeyController", () => {
  it("dispatches exact opaque activation and explicit broadcast once while in flight", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockResolvedValue(broadcastResult(true));
    const action = new GridKeyController(store, client as never);
    const character = fakeKey("character", 0, 0);
    const broadcast = fakeKey("broadcast", 0, 4);

    await action.onWillAppear({ action: character } as never);
    await action.onWillAppear({ action: broadcast } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const firstPress = action.onKeyDown({ action: character } as never);
    const suppressedPress = action.onKeyDown({ action: character } as never);
    expect(client.activate).toHaveBeenCalledTimes(1);
    expect(client.activate).toHaveBeenCalledWith("opaque-client-id");
    await suppressedPress;
    activation.resolve(activatedResult());
    await firstPress;

    await action.onKeyDown({ action: broadcast } as never);
    expect(client.setBroadcast).toHaveBeenCalledWith(true);
  });

  it("invites ready named boxes, waits one second, then sends Ctrl+I", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const firstAcceptance = deferred<ReturnType<typeof inputResult>>();
    const client = fakeClient();
    client.sendText.mockResolvedValue(inputResult("text"));
    client.sendKeys
      .mockReturnValueOnce(firstAcceptance.promise)
      .mockResolvedValue(inputResult("keys"));
    const action = new GridKeyController(store, client as never);
    const group = fakeKey("group", 0, 3);

    await action.onWillAppear({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const press = action.onKeyDown({ action: group } as never);
    await vi.advanceTimersByTimeAsync(0);
    expect(client.sendText.mock.calls).toEqual([
      ["leader-id", "/invite Serein", true],
      ["leader-id", "/invite Rook", true],
    ]);
    expect(client.sendKeys).not.toHaveBeenCalled();
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "pending",
      message: "Waiting 1 sec",
    });

    await action.onKeyDown({ action: group } as never);
    expect(client.sendText).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(999);
    expect(client.sendKeys).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(client.sendKeys).toHaveBeenCalledTimes(1);
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "pending",
      message: "Accepting",
    });
    firstAcceptance.resolve(inputResult("keys"));
    await vi.advanceTimersByTimeAsync(0);
    await press;

    expect(client.sendKeys.mock.calls).toEqual([
      [
        "serein-id",
        [
          {
            keys: ["left_control", "i"],
            hold_ms: 50,
            pause_ms: 40,
          },
        ],
      ],
      [
        "rook-id",
        [
          {
            keys: ["left_control", "i"],
            hold_ms: 50,
            pause_ms: 40,
          },
        ],
      ],
    ]);
    expect(store.view.feedback.get("0,3")).toBeUndefined();
    expect(group.showAlert).not.toHaveBeenCalled();
  });

  it("continues with deliverable invites and reports a partial Group failure", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const client = fakeClient();
    client.sendText
      .mockRejectedValueOnce(new CommandError("send_failed", "missed"))
      .mockResolvedValue(inputResult("text"));
    client.sendKeys.mockResolvedValue(inputResult("keys"));
    const action = new GridKeyController(store, client as never);
    const group = fakeKey("group", 0, 3);

    await action.onWillAppear({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    const press = action.onKeyDown({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_000);
    await press;

    expect(client.sendText).toHaveBeenCalledTimes(2);
    expect(client.sendKeys).toHaveBeenCalledTimes(1);
    expect(client.sendKeys).toHaveBeenCalledWith("rook-id", [
      {
        keys: ["left_control", "i"],
        hold_ms: 50,
        pause_ms: 40,
      },
    ]);
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "error",
      message: "Partial send",
    });
    expect(group.showAlert).toHaveBeenCalledTimes(1);
  });

  it("reveals activation and broadcast state immediately without a done interstitial", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const broadcast = deferred<ReturnType<typeof broadcastResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockReturnValue(broadcast.promise);
    const action = new GridKeyController(store, client as never);
    const character = fakeKey("character", 0, 0);
    const broadcastKey = fakeKey("broadcast", 0, 4);

    await action.onWillAppear({ action: character } as never);
    await action.onWillAppear({ action: broadcastKey } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const activationPress = action.onKeyDown({ action: character } as never);
    expect(store.view.feedback.get("0,0")).toMatchObject({ kind: "pending" });
    const activationSnapshot = store.view.snapshot!;
    store.setSnapshot({
      ...activationSnapshot,
      revision: activationSnapshot.revision + 1,
      active_client_id: "opaque-client-id",
      clients: activationSnapshot.clients.map((candidate) => ({
        ...candidate,
        active: candidate.id === "opaque-client-id",
      })),
    });
    activation.resolve(activatedResult());
    await activationPress;

    expect(store.view.feedback.get("0,0")).toBeUndefined();
    const characterImage = character.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(characterImage)).toContain(">ACTIVE</text>");
    expect(decodeURIComponent(characterImage)).not.toContain("DONE");

    const broadcastPress = action.onKeyDown({ action: broadcastKey } as never);
    expect(store.view.feedback.get("0,4")).toMatchObject({ kind: "pending" });
    const broadcastSnapshot = store.view.snapshot!;
    store.setSnapshot({
      ...broadcastSnapshot,
      revision: broadcastSnapshot.revision + 1,
      broadcast: { available: true, enabled: true },
    });
    broadcast.resolve(broadcastResult(true));
    await broadcastPress;

    expect(store.view.feedback.get("0,4")).toBeUndefined();
    const broadcastImage = broadcastKey.setImage.mock.calls.at(
      -1,
    )?.[0] as string;
    expect(decodeURIComponent(broadcastImage)).toContain("#cc3020");
    expect(decodeURIComponent(broadcastImage)).not.toContain("DONE");
  });

  it("renders a dedicated static image outside the 5 by 3 grid", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = new GridKeyController(store, client as never);
    const unsupported = fakeKey("unsupported", 3, 0);
    await action.onWillAppear({ action: unsupported } as never);
    const image = unsupported.setImage.mock.calls[0]?.[0] as string;
    expect(decodeURIComponent(image)).toContain("LAYOUT REQUIRED");
    expect(decodeURIComponent(image)).not.toContain("REV ");
  });

  it("cannot restore forgotten credentials from a delayed reconnect or pair", async () => {
    const settings = deferred<{ address: string; authToken: string }>();
    sdk.getGlobalSettings.mockReturnValue(settings.promise);
    const store = connectedStore();
    const client = fakeClient();
    const action = new GridKeyController(store, client as never);

    const reconnect = action.onSendToPlugin({
      payload: {
        type: "reconnect",
        address: "server-a.local:19720",
      },
    } as never);
    await Promise.resolve();
    const forget = action.onSendToPlugin({
      payload: { type: "forget" },
    } as never);
    settings.resolve({
      address: "server-a.local:19720",
      authToken: "old-token",
    });
    await Promise.all([reconnect, forget]);
    expect(client.disconnect).toHaveBeenCalledTimes(1);
    expect(client.configure).not.toHaveBeenCalled();
    expect(sdk.setGlobalSettings).toHaveBeenCalledTimes(1);
    expect(sdk.setGlobalSettings).toHaveBeenCalledWith({});

    sdk.setGlobalSettings.mockClear();
    const pairing = deferred<{
      address: string;
      authToken: string;
    }>();
    client.pair.mockReturnValue(pairing.promise);
    const pair = action.onSendToPlugin({
      payload: {
        type: "pair",
        address: "server-a.local:19720",
        code: "482731",
      },
    } as never);
    const forgetPair = action.onSendToPlugin({
      payload: { type: "forget" },
    } as never);
    pairing.resolve({
      address: "server-a.local:19720",
      authToken: "new-token",
    });
    await Promise.all([pair, forgetPair]);
    expect(sdk.setGlobalSettings).toHaveBeenCalledTimes(1);
    expect(sdk.setGlobalSettings).toHaveBeenCalledWith({});
    expect(client.configure).not.toHaveBeenCalled();
  });

  it("never sends a saved token to an edited server address", async () => {
    const serverB = new WebSocketServer({ port: 0 });
    await once(serverB, "listening");
    const bound = serverB.address();
    if (!bound || typeof bound === "string") throw new Error("No test port.");
    let connections = 0;
    serverB.on("connection", () => {
      connections += 1;
    });
    const store = connectedStore();
    const client = fakeClient();
    const action = new GridKeyController(store, client as never);
    sdk.getGlobalSettings.mockResolvedValue({
      address: "server-a.local:19720",
      authToken: "server-a-token",
    });

    await action.onSendToPlugin({
      payload: {
        type: "reconnect",
        address: `127.0.0.1:${bound.port}`,
      },
    } as never);
    expect(client.configure).not.toHaveBeenCalled();
    expect(connections).toBe(0);
    await new Promise<void>((resolve) => serverB.close(() => resolve()));
  });
});

describe("action helpers", () => {
  it("binds a credential to its normalized paired address", () => {
    expect(
      credentialsForReconnect(
        { address: "SERVER-A.local:19720", authToken: "token" },
        "server-a.local:19720",
      ),
    ).toEqual({ address: "server-a.local:19720", authToken: "token" });
    expect(() =>
      credentialsForReconnect(
        { address: "server-a.local:19720", authToken: "token" },
        "server-b.local:19720",
      ),
    ).toThrow("address changed");
  });

  it("only caches an image after setImage succeeds, allowing a later retry", async () => {
    const setImage = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary SDK failure"))
      .mockResolvedValueOnce(undefined);
    const key = {
      action: { setImage },
      row: 0,
      column: 0,
    } as unknown as VisibleKey;
    await expect(updateVisibleKeyImage(key, "image-a")).rejects.toThrow(
      "temporary SDK failure",
    );
    expect(key.lastImage).toBeUndefined();
    await updateVisibleKeyImage(key, "image-a");
    expect(key.lastImage).toBe("image-a");
    expect(setImage).toHaveBeenCalledTimes(2);
  });
});

function connectedStore(): DashboardStore {
  const store = new DashboardStore();
  store.setConnection({
    state: "connected",
    title: "Connected",
    detail: "server-a.local:19720",
  });
  store.setSnapshot(
    stateFixture({
      clients: [
        {
          id: "opaque-client-id",
          character: "Laika",
          server: "Xegony",
          class_code: "WAR",
          window_number: 1,
          active: false,
          activatable: true,
          input_ready: true,
        },
      ],
      broadcast: { available: true, enabled: false },
    }),
  );
  store.setBootStage(3);
  return store;
}

function groupStore(): DashboardStore {
  const store = new DashboardStore();
  store.setConnection({
    state: "connected",
    title: "Connected",
    detail: "server-a.local:19720",
  });
  store.setSnapshot(
    stateFixture({
      active_client_id: "leader-id",
      clients: [
        {
          id: "leader-id",
          character: "Laika",
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: true,
        },
        {
          id: "serein-id",
          character: "Serein",
          window_number: 2,
          active: false,
          activatable: true,
          input_ready: true,
        },
        {
          id: "unknown-id",
          window_number: 3,
          active: false,
          activatable: true,
          input_ready: true,
        },
        {
          id: "mora-id",
          character: "Mora",
          window_number: 4,
          active: false,
          activatable: true,
          input_ready: false,
        },
        {
          id: "rook-id",
          character: "Rook",
          window_number: 5,
          active: false,
          activatable: true,
          input_ready: true,
        },
      ],
    }),
  );
  store.setBootStage(3);
  return store;
}

function fakeClient() {
  return {
    activate: vi.fn(),
    setBroadcast: vi.fn(),
    sendText: vi.fn(),
    sendKeys: vi.fn(),
    pair: vi.fn(),
    configure: vi.fn(),
    disconnect: vi.fn(),
  };
}

function fakeKey(id: string, row: number, column: number) {
  return {
    id,
    coordinates: { row, column },
    isKey: () => true,
    isInMultiAction: () => false,
    setImage: vi.fn().mockResolvedValue(undefined),
    showAlert: vi.fn().mockResolvedValue(undefined),
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function activatedResult() {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: "activate-1",
    result: {
      type: "activated" as const,
      status: "activated" as const,
      foreground_confirmed: true,
    },
    state: stateFixture(),
  };
}

function inputResult(input: "text" | "keys") {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: `${input}-1`,
    result: {
      type: "input_delivered" as const,
      input,
      strokes: 1,
    },
    state: stateFixture(),
  };
}

function broadcastResult(enabled: boolean) {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: "broadcast-1",
    result: { type: "broadcast_set" as const, enabled },
    state: stateFixture({
      broadcast: { available: true, enabled },
    }),
  };
}

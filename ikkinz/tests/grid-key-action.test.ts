import { once } from "node:events";
import { WebSocketServer } from "ws";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

function fakeClient() {
  return {
    activate: vi.fn(),
    setBroadcast: vi.fn(),
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

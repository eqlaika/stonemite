import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { definitionForKey } from "../src/actions/key-definitions";
import { LOCAL_CONNECTION } from "../src/trushar/client";
import type { DashboardKey } from "../src/state/layout";
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
  connectionForSettings,
  DashboardController,
  updateVisibleKeyImage,
  type VisibleKey,
} from "../src/actions/dashboard-controller";

beforeEach(() => {
  sdk.getGlobalSettings.mockReset();
  sdk.setGlobalSettings.mockReset().mockResolvedValue(undefined);
  sdk.sendToPropertyInspector.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("DashboardController", () => {
  it("dispatches exact activation and explicit broadcast once while in flight", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockResolvedValue(broadcastResult(true));
    const controller = new DashboardController(store, client as never);
    const character = fakeKey("character", "character-1");
    const broadcast = fakeKey("broadcast", "broadcast");

    await controller.onWillAppear({ action: character } as never);
    await controller.onWillAppear({ action: broadcast } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const firstPress = controller.onKeyDown({ action: character } as never);
    await controller.onKeyDown({ action: character } as never);
    expect(client.activate).toHaveBeenCalledTimes(1);
    expect(client.activate).toHaveBeenCalledWith("opaque-client-id");
    activation.resolve(activatedResult());
    await firstPress;

    await controller.onKeyDown({ action: broadcast } as never);
    expect(client.setBroadcast).toHaveBeenCalledWith(true);
  });

  it("keeps duplicate character actions synchronized without duplicate commands", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    const controller = new DashboardController(store, client as never);
    const first = fakeKey("character-primary", "character-1");
    const duplicate = fakeKey("character-duplicate", "character-1");

    await controller.onWillAppear({ action: first } as never);
    await controller.onWillAppear({ action: duplicate } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const firstPress = controller.onKeyDown({ action: first } as never);
    await controller.onKeyDown({ action: duplicate } as never);
    expect(client.activate).toHaveBeenCalledTimes(1);

    const snapshot = store.view.snapshot!;
    store.setSnapshot({
      ...snapshot,
      revision: snapshot.revision + 1,
      active_client_id: "opaque-client-id",
      clients: snapshot.clients.map((candidate) => ({
        ...candidate,
        active: candidate.id === "opaque-client-id",
      })),
    });
    activation.resolve(activatedResult());
    await firstPress;
    await vi.advanceTimersByTimeAsync(0);

    for (const key of [first, duplicate]) {
      const image = key.setImage.mock.calls.at(-1)?.[0] as string;
      expect(decodeURIComponent(image)).toContain(">ACTIVE</text>");
    }
  });

  it("arms Swap, then swaps the active and selected character numbers", async () => {
    vi.useFakeTimers();
    const store = multiClientStore();
    const swapRequest = deferred<ReturnType<typeof swapResult>>();
    const client = fakeClient();
    client.swapWindowNumbers.mockReturnValue(swapRequest.promise);
    const controller = new DashboardController(store, client as never);
    const swap = fakeKey("swap", "swap");
    const current = fakeKey("current", "character-1");
    const selected = fakeKey("selected", "character-2");
    const other = fakeKey("other", "character-3");

    for (const key of [swap, current, selected, other]) {
      await controller.onWillAppear({ action: key } as never);
    }
    await vi.advanceTimersByTimeAsync(1_100);

    await controller.onKeyDown({ action: swap } as never);
    const firstArmedFrame = swap.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(firstArmedFrame)).toContain('data-active="true"');
    await vi.advanceTimersByTimeAsync(125);
    expect(swap.setImage.mock.calls.at(-1)?.[0]).not.toBe(firstArmedFrame);
    expect(
      decodeURIComponent(current.setImage.mock.calls.at(-1)?.[0]),
    ).toContain(">CURRENT</text>");
    expect(
      decodeURIComponent(selected.setImage.mock.calls.at(-1)?.[0]),
    ).toContain(">SELECT</text>");

    const press = controller.onKeyDown({ action: selected } as never);
    expect(client.swapWindowNumbers).toHaveBeenCalledWith("serein-id");
    await controller.onKeyDown({ action: other } as never);
    expect(client.activate).not.toHaveBeenCalled();

    swapRequest.resolve(swapResult());
    await press;
    expect(store.view.feedback.get(selected.id)).toBeUndefined();
    expect(decodeURIComponent(swap.setImage.mock.calls.at(-1)?.[0])).toContain(
      'data-active="false"',
    );
  });

  it("reveals activation and broadcast state without a done interstitial", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const broadcast = deferred<ReturnType<typeof broadcastResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockReturnValue(broadcast.promise);
    const controller = new DashboardController(store, client as never);
    const character = fakeKey("character", "character-1");
    const broadcastKey = fakeKey("broadcast", "broadcast");

    await controller.onWillAppear({ action: character } as never);
    await controller.onWillAppear({ action: broadcastKey } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const activationPress = controller.onKeyDown({
      action: character,
    } as never);
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
    await vi.advanceTimersByTimeAsync(0);

    const characterImage = character.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(characterImage)).toContain(">ACTIVE</text>");
    expect(decodeURIComponent(characterImage)).not.toContain("DONE");

    const broadcastPress = controller.onKeyDown({
      action: broadcastKey,
    } as never);
    const broadcastSnapshot = store.view.snapshot!;
    store.setSnapshot({
      ...broadcastSnapshot,
      revision: broadcastSnapshot.revision + 1,
      broadcast: { available: true, enabled: true },
    });
    broadcast.resolve(broadcastResult(true));
    await broadcastPress;
    await vi.advanceTimersByTimeAsync(0);

    const broadcastImage = broadcastKey.setImage.mock.calls.at(
      -1,
    )?.[0] as string;
    expect(decodeURIComponent(broadcastImage)).toContain("#cc3020");
    expect(decodeURIComponent(broadcastImage)).not.toContain("DONE");
  });

  it("keeps Setup inert and ignores unknown actions", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const client = fakeClient();
    const controller = new DashboardController(store, client as never);
    const setup = fakeKey("setup", "logo");
    const unknown = fakeKey("unknown");

    await controller.onWillAppear({ action: setup } as never);
    await controller.onWillAppear({ action: unknown } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    await controller.onKeyDown({ action: setup } as never);
    await controller.onKeyDown({ action: unknown } as never);

    expect(client.activate).not.toHaveBeenCalled();
    expect(client.swapWindowNumbers).not.toHaveBeenCalled();
    expect(client.setBroadcast).not.toHaveBeenCalled();
    expect(store.view.feedback.size).toBe(0);
  });

  it("renders an action by identity at any key position", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const controller = new DashboardController(store, fakeClient() as never);
    const character = fakeKey("custom-character", "character-1");

    await controller.onWillAppear({ action: character } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const image = character.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(image)).toContain("Laika");
    expect(decodeURIComponent(image)).toContain(">1</text>");
  });

  it("returns to loopback and cannot restore a delayed LAN pairing", async () => {
    const pairing = deferred<{ address: string; authToken: string }>();
    const store = connectedStore();
    const client = fakeClient();
    client.pair.mockReturnValue(pairing.promise);
    const controller = new DashboardController(store, client as never);

    const pair = controller.onSendToPlugin({
      payload: {
        type: "pair",
        address: "server-a.local:19720",
        code: "482731",
      },
    } as never);
    const useThisPc = controller.onSendToPlugin({
      payload: { type: "forget" },
    } as never);
    pairing.resolve({
      address: "server-a.local:19720",
      authToken: "new-token",
    });
    await Promise.all([pair, useThisPc]);

    expect(client.configure).toHaveBeenCalledTimes(1);
    expect(client.configure).toHaveBeenCalledWith(LOCAL_CONNECTION);
    expect(sdk.setGlobalSettings).toHaveBeenCalledWith({});
  });

  it("keeps the LAN connection active when clearing its credential fails", async () => {
    sdk.setGlobalSettings.mockRejectedValueOnce(
      new Error("settings unavailable"),
    );
    const store = connectedStore();
    const client = fakeClient();
    const controller = new DashboardController(store, client as never);

    await expect(
      controller.onSendToPlugin({ payload: { type: "forget" } } as never),
    ).rejects.toThrow("settings unavailable");

    expect(client.configure).not.toHaveBeenCalled();
    expect(store.view.snapshot).not.toBeNull();
  });

  it("retries the already configured connection", async () => {
    const client = fakeClient();
    const controller = new DashboardController(
      connectedStore(),
      client as never,
    );

    await controller.onSendToPlugin({
      payload: { type: "reconnect" },
    } as never);

    expect(client.reconnect).toHaveBeenCalledTimes(1);
    expect(client.configure).not.toHaveBeenCalled();
  });
});

describe("action helpers", () => {
  it("uses loopback by default and normalizes complete LAN settings", () => {
    expect(connectionForSettings({})).toEqual(LOCAL_CONNECTION);
    expect(
      connectionForSettings({
        address: "SERVER-A.local:19720",
        authToken: " token ",
      }),
    ).toEqual({ address: "server-a.local:19720", authToken: "token" });
    expect(() =>
      connectionForSettings({ address: "server-a.local:19720" }),
    ).toThrow("incomplete");
    expect(() => connectionForSettings({ authToken: "token" })).toThrow(
      "incomplete",
    );
  });

  it("only caches an image after setImage succeeds", async () => {
    const setImage = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary SDK failure"))
      .mockResolvedValueOnce(undefined);
    const key = {
      action: { setImage },
      key: "character-1",
    } as unknown as VisibleKey;
    await expect(updateVisibleKeyImage(key, "image-a")).rejects.toThrow(
      "temporary SDK failure",
    );
    expect(key.lastImage).toBeUndefined();
    await updateVisibleKeyImage(key, "image-a");
    expect(key.lastImage).toBe("image-a");
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

function multiClientStore(): DashboardStore {
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
          id: "rook-id",
          character: "Rook",
          window_number: 3,
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
    swapWindowNumbers: vi.fn(),
    setBroadcast: vi.fn(),
    pair: vi.fn(),
    configure: vi.fn(),
    reconnect: vi.fn(),
    disconnect: vi.fn(),
  };
}

function fakeKey(label: string, key?: DashboardKey) {
  return {
    id: label,
    label,
    manifestId: key
      ? definitionForKey(key).uuid
      : "co.laikasoft.stonemite.unknown",
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

function swapResult() {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: "swap-numbers-1",
    result: {
      type: "window_numbers_swapped" as const,
      active_previous_number: 1,
      selected_previous_number: 2,
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
    state: stateFixture({ broadcast: { available: true, enabled } }),
  };
}

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CommandError, LOCAL_CONNECTION } from "../src/trushar/client";
import { definitionForKey } from "../src/actions/key-definitions";
import { DEFAULT_LAYOUT, type DashboardKey } from "../src/state/layout";
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
  it("dispatches exact opaque activation and explicit broadcast once while in flight", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockResolvedValue(broadcastResult(true));
    const action = new DashboardController(store, client as never);
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

  it("keeps duplicate action contexts synchronized without duplicate commands", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    const action = new DashboardController(store, client as never);
    const first = fakeKey("character-primary", 0, 0, "character-1");
    const duplicate = fakeKey("character-duplicate", 2, 4, "character-1");

    await action.onWillAppear({ action: first } as never);
    await action.onWillAppear({ action: duplicate } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const firstPress = action.onKeyDown({ action: first } as never);
    await action.onKeyDown({ action: duplicate } as never);
    expect(client.activate).toHaveBeenCalledTimes(1);
    expect(client.activate).toHaveBeenCalledWith("opaque-client-id");

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

  it("arms the Swap key, then swaps the active and selected character numbers", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const swapRequest = deferred<ReturnType<typeof swapResult>>();
    const client = fakeClient();
    client.swapWindowNumbers.mockReturnValue(swapRequest.promise);
    const action = new DashboardController(store, client as never);
    const swap = fakeKey("swap", 2, 4);
    const current = fakeKey("current", 0, 0);
    const selected = fakeKey("selected", 0, 1);
    const other = fakeKey("other", 0, 2);

    for (const key of [swap, current, selected, other]) {
      await action.onWillAppear({ action: key } as never);
    }
    await vi.advanceTimersByTimeAsync(1_100);

    await action.onKeyDown({ action: swap } as never);
    const firstArmedFrame = swap.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(firstArmedFrame)).toContain('data-active="true"');
    expect(decodeURIComponent(firstArmedFrame)).toContain(">Swap</text>");
    expect(decodeURIComponent(firstArmedFrame)).not.toContain("PICK CHARACTER");
    await vi.advanceTimersByTimeAsync(125);
    expect(swap.setImage.mock.calls.at(-1)?.[0]).not.toBe(firstArmedFrame);
    expect(
      decodeURIComponent(current.setImage.mock.calls.at(-1)?.[0]),
    ).toContain(">CURRENT</text>");
    expect(
      decodeURIComponent(selected.setImage.mock.calls.at(-1)?.[0]),
    ).toContain(">SELECT</text>");

    const press = action.onKeyDown({ action: selected } as never);
    expect(client.swapWindowNumbers).toHaveBeenCalledWith("serein-id");
    expect(store.view.feedback.get("0,1")).toMatchObject({
      kind: "pending",
      message: "Swapping",
    });
    await action.onKeyDown({ action: other } as never);
    expect(client.activate).not.toHaveBeenCalled();

    swapRequest.resolve(swapResult());
    await press;
    expect(store.view.feedback.get("0,1")).toBeUndefined();
    expect(decodeURIComponent(swap.setImage.mock.calls.at(-1)?.[0])).toContain(
      'data-active="false"',
    );

    await action.onKeyDown({ action: swap } as never);
    await action.onKeyDown({ action: current } as never);
    expect(client.swapWindowNumbers).toHaveBeenCalledTimes(1);
    const settledFrameCount = swap.setImage.mock.calls.length;
    await vi.advanceTimersByTimeAsync(250);
    expect(swap.setImage).toHaveBeenCalledTimes(settledFrameCount);
  });

  it("invites ready named boxes, waits one second, then sends Invite/Follow", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const firstAcceptance = deferred<ReturnType<typeof eqActionResult>>();
    const client = fakeClient();
    client.sendText.mockResolvedValue(inputResult("text"));
    client.sendEqAction
      .mockReturnValueOnce(firstAcceptance.promise)
      .mockResolvedValue(eqActionResult({ type: "invite_follow" }));
    const action = new DashboardController(store, client as never);
    const group = fakeKey("group", 0, 3);

    await action.onWillAppear({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const press = action.onKeyDown({ action: group } as never);
    await vi.advanceTimersByTimeAsync(0);
    expect(client.sendText.mock.calls).toEqual([
      ["leader-id", "/invite Serein", true],
      ["leader-id", "/invite Rook", true],
    ]);
    expect(client.sendEqAction).not.toHaveBeenCalled();
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "pending",
      message: "Waiting 1 sec",
      motion: "group",
    });

    await action.onKeyDown({ action: group } as never);
    expect(client.sendText).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(999);
    expect(client.sendEqAction).not.toHaveBeenCalled();

    await vi.advanceTimersByTimeAsync(1);
    expect(client.sendEqAction).toHaveBeenCalledTimes(1);
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "pending",
      message: "Accepting",
    });
    firstAcceptance.resolve(eqActionResult({ type: "invite_follow" }));
    await vi.advanceTimersByTimeAsync(0);
    await press;

    expect(client.sendEqAction.mock.calls).toEqual([
      ["serein-id", { type: "invite_follow" }],
      ["rook-id", { type: "invite_follow" }],
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
    client.sendEqAction.mockResolvedValue(
      eqActionResult({ type: "invite_follow" }),
    );
    const action = new DashboardController(store, client as never);
    const group = fakeKey("group", 0, 3);

    await action.onWillAppear({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    const press = action.onKeyDown({ action: group } as never);
    await vi.advanceTimersByTimeAsync(1_000);
    await press;

    expect(client.sendText).toHaveBeenCalledTimes(2);
    expect(client.sendEqAction).toHaveBeenCalledTimes(1);
    expect(client.sendEqAction).toHaveBeenCalledWith("rook-id", {
      type: "invite_follow",
    });
    expect(store.view.feedback.get("0,3")).toMatchObject({
      kind: "error",
      message: "Partial send",
    });
    expect(group.showAlert).toHaveBeenCalledTimes(1);
  });

  it("sends Follow to every ready background box concurrently", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const deliveries = [
      deferred<ReturnType<typeof inputResult>>(),
      deferred<ReturnType<typeof inputResult>>(),
      deferred<ReturnType<typeof inputResult>>(),
    ];
    const client = fakeClient();
    client.sendText
      .mockReturnValueOnce(deliveries[0]!.promise)
      .mockReturnValueOnce(deliveries[1]!.promise)
      .mockReturnValueOnce(deliveries[2]!.promise);
    const action = new DashboardController(store, client as never);
    const follow = fakeKey("follow", 1, 3);

    await action.onWillAppear({ action: follow } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const press = action.onKeyDown({ action: follow } as never);
    expect(client.sendText.mock.calls).toEqual([
      ["serein-id", "/follow Laika", true],
      ["unknown-id", "/follow Laika", true],
      ["rook-id", "/follow Laika", true],
    ]);
    expect(store.view.feedback.get("1,3")).toMatchObject({
      kind: "pending",
      message: "Following",
      motion: "follow",
    });

    await action.onKeyDown({ action: follow } as never);
    expect(client.sendText).toHaveBeenCalledTimes(3);
    for (const delivery of deliveries) delivery.resolve(inputResult("text"));
    await press;

    expect(store.view.feedback.get("1,3")).toBeUndefined();
    expect(follow.showAlert).not.toHaveBeenCalled();
  });

  it("reports partial Follow delivery after attempting every ready box", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const client = fakeClient();
    client.sendText
      .mockResolvedValueOnce(inputResult("text"))
      .mockRejectedValueOnce(new CommandError("send_failed", "missed"))
      .mockResolvedValueOnce(inputResult("text"));
    const action = new DashboardController(store, client as never);
    const follow = fakeKey("follow", 1, 3);

    await action.onWillAppear({ action: follow } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    await action.onKeyDown({ action: follow } as never);

    expect(client.sendText).toHaveBeenCalledTimes(3);
    expect(store.view.feedback.get("1,3")).toMatchObject({
      kind: "error",
      message: "Partial follow",
    });
    expect(follow.showAlert).toHaveBeenCalledTimes(1);
  });

  it("sends Assist to every ready background box concurrently", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const deliveries = [
      deferred<ReturnType<typeof inputResult>>(),
      deferred<ReturnType<typeof inputResult>>(),
      deferred<ReturnType<typeof inputResult>>(),
    ];
    const client = fakeClient();
    client.sendText
      .mockReturnValueOnce(deliveries[0]!.promise)
      .mockReturnValueOnce(deliveries[1]!.promise)
      .mockReturnValueOnce(deliveries[2]!.promise);
    const action = new DashboardController(store, client as never);
    const assist = fakeKey("assist", 2, 3);

    await action.onWillAppear({ action: assist } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const press = action.onKeyDown({ action: assist } as never);
    expect(client.sendText.mock.calls).toEqual([
      ["serein-id", "/assist Laika", true],
      ["unknown-id", "/assist Laika", true],
      ["rook-id", "/assist Laika", true],
    ]);
    expect(store.view.feedback.get("2,3")).toMatchObject({
      kind: "pending",
      message: "Sending",
      motion: "assist",
    });

    await action.onKeyDown({ action: assist } as never);
    expect(client.sendText).toHaveBeenCalledTimes(3);
    for (const delivery of deliveries) delivery.resolve(inputResult("text"));
    await press;

    expect(store.view.feedback.get("2,3")).toBeUndefined();
    expect(assist.showAlert).not.toHaveBeenCalled();
  });

  it("reports partial Assist delivery after attempting every ready box", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const client = fakeClient();
    client.sendText
      .mockResolvedValueOnce(inputResult("text"))
      .mockRejectedValueOnce(new CommandError("send_failed", "missed"))
      .mockResolvedValueOnce(inputResult("text"));
    const action = new DashboardController(store, client as never);
    const assist = fakeKey("assist", 2, 3);

    await action.onWillAppear({ action: assist } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    await action.onKeyDown({ action: assist } as never);

    expect(client.sendText).toHaveBeenCalledTimes(3);
    expect(store.view.feedback.get("2,3")).toMatchObject({
      kind: "error",
      message: "Partial assist",
    });
    expect(assist.showAlert).toHaveBeenCalledTimes(1);
  });

  it("reveals activation and broadcast state immediately without a done interstitial", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const activation = deferred<ReturnType<typeof activatedResult>>();
    const broadcast = deferred<ReturnType<typeof broadcastResult>>();
    const client = fakeClient();
    client.activate.mockReturnValue(activation.promise);
    client.setBroadcast.mockReturnValue(broadcast.promise);
    const action = new DashboardController(store, client as never);
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
    await vi.advanceTimersByTimeAsync(0);

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
    await vi.advanceTimersByTimeAsync(0);

    expect(store.view.feedback.get("0,4")).toBeUndefined();
    const broadcastImage = broadcastKey.setImage.mock.calls.at(
      -1,
    )?.[0] as string;
    expect(decodeURIComponent(broadcastImage)).toContain("#cc3020");
    expect(decodeURIComponent(broadcastImage)).not.toContain("DONE");
  });

  it("sends Use Center Screen to every ready box concurrently", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const deliveries = [
      deferred<ReturnType<typeof eqActionResult>>(),
      deferred<ReturnType<typeof eqActionResult>>(),
      deferred<ReturnType<typeof eqActionResult>>(),
      deferred<ReturnType<typeof eqActionResult>>(),
    ];
    const client = fakeClient();
    client.sendEqAction
      .mockReturnValueOnce(deliveries[0]!.promise)
      .mockReturnValueOnce(deliveries[1]!.promise)
      .mockReturnValueOnce(deliveries[2]!.promise)
      .mockReturnValueOnce(deliveries[3]!.promise);
    const action = new DashboardController(store, client as never);
    const use = fakeKey("use", 1, 4);

    await action.onWillAppear({ action: use } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    const press = action.onKeyDown({ action: use } as never);
    await vi.advanceTimersByTimeAsync(0);

    expect(client.sendEqAction.mock.calls).toEqual([
      ["leader-id", { type: "use_center_screen" }],
      ["serein-id", { type: "use_center_screen" }],
      ["unknown-id", { type: "use_center_screen" }],
      ["rook-id", { type: "use_center_screen" }],
    ]);
    expect(store.view.feedback.get("1,4")).toMatchObject({
      kind: "pending",
      message: "Using",
      motion: "use",
    });
    await action.onKeyDown({ action: use } as never);
    expect(client.sendEqAction).toHaveBeenCalledTimes(4);

    for (const delivery of deliveries)
      delivery.resolve(eqActionResult({ type: "use_center_screen" }));
    await press;
    expect(store.view.feedback.get("1,4")).toBeUndefined();
    expect(use.showAlert).not.toHaveBeenCalled();
  });

  it("reports partial Use Center Screen delivery", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const client = fakeClient();
    client.sendEqAction
      .mockResolvedValueOnce(eqActionResult({ type: "use_center_screen" }))
      .mockRejectedValueOnce(new CommandError("eq_action_unbound", "unbound"))
      .mockResolvedValueOnce(eqActionResult({ type: "use_center_screen" }))
      .mockResolvedValueOnce(eqActionResult({ type: "use_center_screen" }));
    const action = new DashboardController(store, client as never);
    const use = fakeKey("use", 1, 4);

    await action.onWillAppear({ action: use } as never);
    await vi.advanceTimersByTimeAsync(1_100);
    await action.onKeyDown({ action: use } as never);

    expect(store.view.feedback.get("1,4")).toMatchObject({
      kind: "error",
      message: "Partial use",
    });
    expect(use.showAlert).toHaveBeenCalledTimes(1);
  });

  it("keeps the Logo action inert and ignores unknown actions", async () => {
    vi.useFakeTimers();
    const store = groupStore();
    const client = fakeClient();
    const action = new DashboardController(store, client as never);
    const reserved = [
      fakeKey("client-count", 2, 0),
      fakeKey("active", 2, 1),
      fakeKey("server", 2, 2),
    ];

    for (const key of reserved) {
      await action.onWillAppear({ action: key } as never);
    }
    await vi.advanceTimersByTimeAsync(1_100);

    for (const key of reserved) {
      await action.onKeyDown({ action: key } as never);
    }

    expect(client.activate).not.toHaveBeenCalled();
    expect(client.swapWindowNumbers).not.toHaveBeenCalled();
    expect(client.setBroadcast).not.toHaveBeenCalled();
    expect(client.sendText).not.toHaveBeenCalled();
    expect(client.sendKeys).not.toHaveBeenCalled();
    expect(client.sendEqAction).not.toHaveBeenCalled();
    expect(store.view.feedback.size).toBe(0);
    for (const key of reserved) expect(key.showAlert).not.toHaveBeenCalled();
  });

  it("renders an action by identity at any key position", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const client = fakeClient();
    const action = new DashboardController(store, client as never);
    const character = fakeKey("custom-character", 2, 4, "character-1");

    await action.onWillAppear({ action: character } as never);
    await vi.advanceTimersByTimeAsync(1_100);

    const image = character.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(image)).toContain("Laika");
    expect(decodeURIComponent(image)).toContain(">1</text>");
  });

  it("returns to loopback and cannot restore a delayed LAN pairing", async () => {
    const pairing = deferred<{
      address: string;
      authToken: string;
    }>();
    const store = connectedStore();
    const client = fakeClient();
    client.pair.mockReturnValue(pairing.promise);
    const action = new DashboardController(store, client as never);

    const pair = action.onSendToPlugin({
      payload: {
        type: "pair",
        address: "server-a.local:19720",
        code: "482731",
      },
    } as never);
    const useThisPc = action.onSendToPlugin({
      payload: { type: "forget" },
    } as never);
    pairing.resolve({
      address: "server-a.local:19720",
      authToken: "new-token",
    });
    await Promise.all([pair, useThisPc]);

    expect(client.configure).toHaveBeenCalledTimes(1);
    expect(client.configure).toHaveBeenCalledWith(LOCAL_CONNECTION);
    expect(sdk.setGlobalSettings).toHaveBeenCalledTimes(1);
    expect(sdk.setGlobalSettings).toHaveBeenCalledWith({});
  });

  it("keeps the LAN connection active when clearing its credential fails", async () => {
    sdk.setGlobalSettings.mockRejectedValueOnce(
      new Error("settings unavailable"),
    );
    const store = connectedStore();
    const client = fakeClient();
    const action = new DashboardController(store, client as never);

    await expect(
      action.onSendToPlugin({ payload: { type: "forget" } } as never),
    ).rejects.toThrow("settings unavailable");

    expect(client.configure).not.toHaveBeenCalled();
    expect(store.view.snapshot).not.toBeNull();
  });

  it("retries the already configured local or LAN connection", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = new DashboardController(store, client as never);

    await action.onSendToPlugin({ payload: { type: "reconnect" } } as never);

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

  it("only caches an image after setImage succeeds, allowing a later retry", async () => {
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
    swapWindowNumbers: vi.fn(),
    setBroadcast: vi.fn(),
    sendText: vi.fn(),
    sendKeys: vi.fn(),
    sendEqAction: vi.fn(),
    pair: vi.fn(),
    configure: vi.fn(),
    reconnect: vi.fn(),
    disconnect: vi.fn(),
  };
}

function fakeKey(
  label: string,
  row: number,
  column: number,
  keyOverride?: DashboardKey,
) {
  const defaultKey = DEFAULT_LAYOUT[row]?.[column];
  const key =
    keyOverride ?? (defaultKey && defaultKey !== "blank" ? defaultKey : null);
  return {
    id: `${row},${column}`,
    label,
    manifestId: key
      ? definitionForKey(key).uuid
      : "co.laikasoft.ikkinz.unknown",
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

function eqActionResult(action: import("../src/types/trushar").EqAction) {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: "eq-action-1",
    result: {
      type: "eq_action_delivered" as const,
      action,
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

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DashboardStore } from "../src/state/store";
import { XTargetDialAction } from "../src/actions/xtarget-dial";
import { CommandError } from "../src/trushar/client";
import { stateFixture } from "./fixtures";

vi.mock("@elgato/streamdeck", () => ({
  SingletonAction: class {},
}));

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("XTargetDialAction", () => {
  it("applies batched physical ticks without synthetic acceleration and clamps", async () => {
    const store = connectedStore();
    const client = fakeClient(store);
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    dial.onDialRotate(rotateEvent(action, 2));
    await vi.waitFor(() => {
      expect(client.sendEqAction).toHaveBeenCalledWith("client-1", {
        type: "keymap",
        mapping: "TARGET_XTARGET_6",
      });
    });
    const delivered = client.sendEqAction.mock.calls.length;
    dial.onDialRotate(rotateEvent(action, 9));
    await Promise.resolve();
    expect(client.sendEqAction).toHaveBeenCalledTimes(delivered);

    dial.onDialRotate(rotateEvent(action, -1));
    await vi.waitFor(() => {
      expect(client.sendEqAction).toHaveBeenLastCalledWith("client-1", {
        type: "keymap",
        mapping: "TARGET_XTARGET_3",
      });
    });
    dial.onDialRotate(rotateEvent(action, -12));
    await vi.waitFor(() => {
      expect(client.sendEqAction).toHaveBeenLastCalledWith("client-1", {
        type: "keymap",
        mapping: "TARGET_XTARGET_1",
      });
    });
  });

  it("debounces rapid rotation and sends only the final local selection", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const client = fakeClient(store);
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    dial.onDialRotate(rotateEvent(action, 1));
    dial.onDialRotate(rotateEvent(action, 1));
    await vi.advanceTimersByTimeAsync(0);
    expect(client.sendEqAction).not.toHaveBeenCalled();
    expect(action.setFeedback).toHaveBeenLastCalledWith(
      expect.objectContaining({
        activity: expect.objectContaining({ enabled: false }),
        slot: expect.objectContaining({ value: "6" }),
      }),
    );

    await vi.advanceTimersByTimeAsync(299);
    expect(client.sendEqAction).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(client.sendEqAction).toHaveBeenCalledTimes(1);
    expect(client.sendEqAction).toHaveBeenCalledWith("client-1", {
      type: "keymap",
      mapping: "TARGET_XTARGET_6",
    });
  });

  it("shows class identity and concise readiness feedback", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          classIcon: expect.objectContaining({
            enabled: true,
            value: expect.stringMatching(/^data:image\/png;base64,/),
          }),
          character: expect.objectContaining({ value: "Laika" }),
          detail: expect.objectContaining({ enabled: false }),
        }),
      );
    });
  });

  it("queues a rapid press until direct targeting finishes", async () => {
    const store = connectedStore();
    const client = fakeClient();
    let finishSelection!: (value: ReturnType<typeof deliveredResponse>) => void;
    client.sendEqAction.mockImplementationOnce(
      () =>
        new Promise<ReturnType<typeof deliveredResponse>>((resolve) => {
          finishSelection = resolve;
        }),
    );
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    dial.onDialRotate(rotateEvent(action, 1));
    await vi.waitFor(() => {
      expect(client.sendEqAction).toHaveBeenCalledTimes(1);
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          activity: expect.objectContaining({
            enabled: true,
            value: "TARGETING 3",
          }),
        }),
      );
    });
    await dial.onDialDown(downEvent(action));
    expect(client.sendEqAction).toHaveBeenCalledTimes(1);

    const selectionResponse = deliveredResponse(3, store.view.snapshot!);
    store.setSnapshot(selectionResponse.state);
    finishSelection(selectionResponse);
    await vi.waitFor(() => {
      expect(client.sendEqAction).toHaveBeenCalledTimes(2);
      expect(client.sendEqAction).toHaveBeenLastCalledWith("client-1", {
        type: "keymap",
        mapping: "CONSIDER",
      });
    });
  });

  it("shows UNBOUND, clears through the direct attempt, and never substitutes cycling", async () => {
    const store = connectedStore({
      slots: [
        { slot: 1, label: "Auto hater", bound: true },
        { slot: 3, label: "Group tank", bound: false },
      ],
    });
    const client = fakeClient();
    client.sendEqAction.mockRejectedValueOnce(
      new CommandError("eq_action_unbound", "not mapped"),
    );
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    dial.onDialRotate(rotateEvent(action, 1));
    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          role: expect.objectContaining({ value: "UNBOUND" }),
        }),
      );
    });
    await vi.waitFor(() => expect(action.showAlert).toHaveBeenCalledOnce());
    expect(client.sendEqAction).toHaveBeenCalledWith("client-1", {
      type: "keymap",
      mapping: "TARGET_XTARGET_3",
    });
    expect(JSON.stringify(client.sendEqAction.mock.calls)).not.toContain(
      "CYCLE_XTARGET",
    );

    await dial.onDialDown(downEvent(action));
    expect(client.sendEqAction).toHaveBeenCalledTimes(1);
  });

  it("presses Consider and taps to activate the exact configured box", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    await dial.onDialDown(downEvent(action));
    expect(client.sendEqAction).toHaveBeenCalledWith("client-1", {
      type: "keymap",
      mapping: "CONSIDER",
    });
    await dial.onTouchTap(touchEvent(action, false));
    expect(client.activate).toHaveBeenCalledWith("client-1");
    await dial.onTouchTap(touchEvent(action, true));
    expect(client.activate).toHaveBeenCalledTimes(1);
  });

  it("renders authoritative active and short-lived Consider feedback", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    store.setSnapshot(
      stateFixture({
        clients: [
          {
            ...stateFixture().clients[0]!,
            active: true,
            xtarget: {
              ...stateFixture().clients[0]!.xtarget,
              selected_slot: 1,
              consider: {
                type: "target",
                target: "A sarnak knight",
                difficulty: "red",
                level: 35,
              },
            },
          },
        ],
      }),
    );

    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          background: expect.objectContaining({
            value: expect.stringMatching(/^data:image\/svg\+xml;base64,/),
          }),
          character: expect.objectContaining({ color: "#80df89" }),
          role: expect.objectContaining({ value: "A sarnak knight" }),
          detail: expect.objectContaining({ value: "LVL 35 · RED" }),
        }),
      );
    });
  });

  it("shows immediate no-target Consider feedback as a popover", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));
    const state = store.view.snapshot!;

    store.setSnapshot({
      ...state,
      revision: state.revision + 1,
      clients: state.clients.map((client) =>
        client.id === "client-1"
          ? {
              ...client,
              xtarget: {
                ...client.xtarget,
                consider_pending: false,
                consider: { type: "no_target" as const },
              },
            }
          : client,
      ),
    });

    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          activity: expect.objectContaining({
            enabled: true,
            value: "NO TARGET",
          }),
          role: expect.objectContaining({ value: "Auto hater" }),
        }),
      );
    });
  });

  it("distinguishes older Stonemite from an empty XTarget list", async () => {
    const store = connectedStore({ supported: false, slots: [] });
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 1));

    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          role: expect.objectContaining({ value: "UPDATE STONEMITE" }),
        }),
      );
    });
  });

  it("uses the configured box number and installs the custom layout", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const action = fakeDial();
    const dial = new XTargetDialAction(store, client as never);
    await dial.onWillAppear(appearEvent(action, 4));

    expect(action.setFeedbackLayout).toHaveBeenCalledWith(
      "layouts/xtarget-v4.json",
    );
    await vi.waitFor(() => {
      expect(action.setFeedback).toHaveBeenLastCalledWith(
        expect.objectContaining({
          character: expect.objectContaining({ value: "BOX 4" }),
          role: expect.objectContaining({ value: "NOT LOADED" }),
        }),
      );
    });
  });
});

function connectedStore(xtarget: Record<string, unknown> = {}): DashboardStore {
  const store = new DashboardStore();
  store.setConnection({
    state: "connected",
    title: "Connected",
    detail: "local",
  });
  const base = stateFixture();
  store.setSnapshot(
    stateFixture({
      clients: [
        {
          ...base.clients[0]!,
          active: false,
          xtarget: {
            supported: true,
            slots: [
              { slot: 1, label: "Auto hater", bound: true },
              { slot: 3, label: "Group tank", bound: true },
              { slot: 6, label: "Laika", bound: true },
            ],
            selected_slot: 1,
            consider_bound: true,
            consider_pending: false,
            ...xtarget,
          },
        },
      ],
    }),
  );
  return store;
}

function fakeClient(store?: DashboardStore) {
  return {
    sendEqAction: vi
      .fn()
      .mockImplementation((_clientId: string, action: { mapping: string }) => {
        const selected = Number(action.mapping.replace("TARGET_XTARGET_", ""));
        const selectedSlot = Number.isSafeInteger(selected) ? selected : 1;
        const snapshot = store?.view.snapshot;
        const response = snapshot
          ? deliveredResponse(selectedSlot, snapshot)
          : deliveredResponse(selectedSlot);
        store?.setSnapshot(response.state);
        return Promise.resolve(response);
      }),
    activate: vi.fn().mockResolvedValue({ result: { type: "activated" } }),
  };
}

function deliveredResponse(selectedSlot: number, base = stateFixture()) {
  return {
    result: { type: "eq_action_delivered" as const },
    state: stateFixture({
      ...base,
      revision: base.revision + 1,
      clients: base.clients.map((client) =>
        client.id === "client-1"
          ? {
              ...client,
              xtarget: {
                ...client.xtarget,
                selected_slot: selectedSlot,
              },
            }
          : client,
      ),
    }),
  };
}

function fakeDial() {
  return {
    id: "dial-1",
    manifestId: "co.laikasoft.stonemite.xtarget",
    isDial: () => true,
    setFeedbackLayout: vi.fn().mockResolvedValue(undefined),
    setTriggerDescription: vi.fn().mockResolvedValue(undefined),
    setFeedback: vi.fn().mockResolvedValue(undefined),
    showAlert: vi.fn().mockResolvedValue(undefined),
  };
}

function appearEvent(
  action: ReturnType<typeof fakeDial>,
  windowNumber: number,
) {
  return {
    action,
    payload: { settings: { windowNumber } },
  } as never;
}

function rotateEvent(action: ReturnType<typeof fakeDial>, ticks: number) {
  return {
    action,
    payload: { ticks, pressed: false, settings: { windowNumber: 1 } },
  } as never;
}

function downEvent(action: ReturnType<typeof fakeDial>) {
  return { action, payload: { settings: { windowNumber: 1 } } } as never;
}

function touchEvent(action: ReturnType<typeof fakeDial>, hold: boolean) {
  return {
    action,
    payload: { hold, tapPos: [100, 50], settings: { windowNumber: 1 } },
  } as never;
}

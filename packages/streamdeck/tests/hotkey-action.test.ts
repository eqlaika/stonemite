import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { DashboardStore } from "../src/state/store";
import { stateFixture } from "./fixtures";

const sdk = vi.hoisted(() => ({
  sendToPropertyInspector: vi.fn(),
}));

vi.mock("@elgato/streamdeck", () => ({
  default: {
    ui: { sendToPropertyInspector: sdk.sendToPropertyInspector },
  },
  SingletonAction: class {},
}));

import {
  HotkeyAction,
  formatEqMappingLabel,
  normalizeHotkeySettings,
  targetsForSettings,
} from "../src/actions/hotkey";
import { LUCIDE_ANIMATED_ICON_NAMES } from "../src/render/action-icons";
import {
  contrastingHotkeyForeground,
  renderHotkeyTile,
} from "../src/render/key-svg";

beforeEach(() => {
  sdk.sendToPropertyInspector.mockReset().mockResolvedValue(undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("HotkeyAction", () => {
  it("normalizes stable targets, labels, mapping names, and pinned Lucide icons", () => {
    expect(
      normalizeHotkeySettings({
        targetMode: "selected",
        windowNumbers: [6, 2, 2, 9],
        mapping: "sit_stand",
        label: "  SELOS  ",
        icon: "flame",
        color: "#E0B848",
      }),
    ).toEqual({
      targetMode: "selected",
      windowNumbers: [2, 6],
      mapping: "SIT_STAND",
      label: "SELOS",
      icon: "flame",
      color: "#e0b848",
    });
    expect(normalizeHotkeySettings({ targetMode: "active" }).targetMode).toBe(
      "active",
    );
    expect(
      normalizeHotkeySettings({ targetMode: "background" }).targetMode,
    ).toBe("background");
    expect(normalizeHotkeySettings({ targetMode: "unknown" }).targetMode).toBe(
      "all",
    );
    expect(
      targetsForSettings(normalizeHotkeySettings({ targetMode: "active" })),
    ).toEqual({ type: "active" });
    expect(
      targetsForSettings(normalizeHotkeySettings({ targetMode: "background" })),
    ).toEqual({ type: "background_loaded" });
    expect(normalizeHotkeySettings({ color: "invalid" }).color).toBe("#59d8d0");
    expect(formatEqMappingLabel("HOT11_12")).toBe("Hotbar 11 button 12");
    expect(formatEqMappingLabel("CAST14")).toBe("Spell gem 14");
    expect(LUCIDE_ANIMATED_ICON_NAMES).toHaveLength(466);
    expect(LUCIDE_ANIMATED_ICON_NAMES).toContain("keyboard");
    expect(LUCIDE_ANIMATED_ICON_NAMES).toContain("flame");
    expect(LUCIDE_ANIMATED_ICON_NAMES).not.toContain("footprints");
  });

  it("holds a still success fill before returning to idle", async () => {
    vi.useFakeTimers();
    const store = connectedStore();
    const request = deferred<ReturnType<typeof batchResult>>();
    const client = fakeClient();
    client.sendEqActionBatch.mockReturnValue(request.promise);
    const hotkey = new HotkeyAction(store, client as never);
    const action = fakeKey({
      targetMode: "all",
      mapping: "DUCK",
      label: "BURN",
      icon: "flame",
    });
    hotkey.onWillAppear({
      action,
      payload: { settings: action.settings },
    } as never);
    await vi.advanceTimersByTimeAsync(0);

    const first = hotkey.onKeyDown({ action } as never);
    await hotkey.onKeyDown({ action } as never);
    await vi.advanceTimersByTimeAsync(0);
    expect(client.sendEqActionBatch).toHaveBeenCalledTimes(1);
    expect(client.sendEqActionBatch).toHaveBeenCalledWith(
      { type: "all_loaded" },
      { type: "keymap", mapping: "DUCK" },
    );
    const activeImage = action.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(activeImage)).toContain('data-icon="flame"');
    expect(decodeURIComponent(activeImage)).toContain('data-active="true"');

    request.resolve(batchResult());
    await first;
    await vi.advanceTimersByTimeAsync(0);
    expect(action.showOk).not.toHaveBeenCalled();
    const successImage = action.setImage.mock.calls.at(-1)?.[0] as string;
    const successSvg = decodeURIComponent(successImage);
    expect(successSvg).toContain('data-active="false"');
    expect(successSvg).toContain(
      '<rect width="72" height="72" rx="7" fill="#59d8d0"/>',
    );

    await vi.advanceTimersByTimeAsync(699);
    expect(action.setImage.mock.calls.at(-1)?.[0]).toBe(successImage);
    await vi.advanceTimersByTimeAsync(1);
    const idleImage = action.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(idleImage)).not.toContain(
      '<rect width="72" height="72" rx="7" fill="#59d8d0"/>',
    );
  });

  it("resolves background targeting on Stonemite when the key is pressed", async () => {
    const store = connectedStore();
    const snapshot = store.view.snapshot!;
    store.setSnapshot({
      ...snapshot,
      clients: [
        ...snapshot.clients,
        {
          id: "client-2",
          character: "Serein",
          window_number: 2,
          active: false,
          activatable: true,
          input_ready: true,
        },
      ],
    });
    const client = fakeClient();
    const hotkey = new HotkeyAction(store, client as never);
    const action = fakeKey({
      targetMode: "background",
      mapping: "HOT1_1",
      icon: "keyboard",
    });
    hotkey.onWillAppear({
      action,
      payload: { settings: action.settings },
    } as never);

    await hotkey.onKeyDown({ action } as never);

    expect(client.sendEqActionBatch).toHaveBeenCalledWith(
      { type: "background_loaded" },
      { type: "keymap", mapping: "HOT1_1" },
    );
    const image = action.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(image)).toContain(">BG</text>");
  });

  it("requires every selected window to be loaded and ready before sending", async () => {
    const store = connectedStore();
    const client = fakeClient();
    const hotkey = new HotkeyAction(store, client as never);
    const action = fakeKey({
      targetMode: "selected",
      windowNumbers: [1, 2],
      mapping: "SIT_STAND",
      label: "Sit",
      icon: "keyboard",
    });
    hotkey.onWillAppear({
      action,
      payload: { settings: action.settings },
    } as never);

    await hotkey.onKeyDown({ action } as never);
    expect(client.sendEqActionBatch).not.toHaveBeenCalled();
    expect(action.showAlert).toHaveBeenCalledOnce();
    await Promise.resolve();
    const image = action.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(image)).toContain("BOX 2 MISSING");
  });

  it("rejects an active target when no active box is known", async () => {
    const store = connectedStore();
    const snapshot = store.view.snapshot!;
    store.setSnapshot({
      ...snapshot,
      active_client_id: null,
      clients: snapshot.clients.map((client) => ({ ...client, active: false })),
    });
    const client = fakeClient();
    const hotkey = new HotkeyAction(store, client as never);
    const action = fakeKey({
      targetMode: "active",
      mapping: "HOT1_1",
      icon: "keyboard",
    });
    hotkey.onWillAppear({
      action,
      payload: { settings: action.settings },
    } as never);

    await hotkey.onKeyDown({ action } as never);

    expect(client.sendEqActionBatch).not.toHaveBeenCalled();
    expect(action.showAlert).toHaveBeenCalledOnce();
    await Promise.resolve();
    const image = action.setImage.mock.calls.at(-1)?.[0] as string;
    expect(decodeURIComponent(image)).toContain("NO ACTIVE BOX");
  });

  it("lists shared mappings for property-inspector draft targets", async () => {
    const store = connectedStore();
    const client = fakeClient();
    client.listEqKeymapActions.mockResolvedValue({
      type: "eq_keymap_actions_listed",
      mappings: ["DUCK", "HOT1_2"],
      window_numbers: [1],
    });
    const hotkey = new HotkeyAction(store, client as never);
    const action = fakeKey({ mapping: "DUCK", icon: "keyboard" });
    hotkey.onWillAppear({
      action,
      payload: { settings: action.settings },
    } as never);

    await hotkey.onSendToPlugin({
      action,
      payload: {
        type: "list-hotkey-mappings",
        requestId: "draft-2",
        targets: { type: "background_loaded" },
      },
    } as never);

    expect(client.listEqKeymapActions).toHaveBeenCalledWith({
      type: "background_loaded",
    });
    expect(sdk.sendToPropertyInspector).toHaveBeenCalledWith(
      expect.objectContaining({
        type: "hotkey-state",
        requestId: "draft-2",
        mappings: [
          { value: "DUCK", label: "Duck" },
          { value: "HOT1_2", label: "Hotbar 1 button 2" },
        ],
        mappedWindowNumbers: [1],
      }),
    );
  });
});

describe("hotkey rendering", () => {
  it("renders the selected Lucide Animated icon, name, and box summary", () => {
    const image = renderHotkeyTile(
      {
        configured: true,
        label: "BURN",
        icon: "flame",
        color: "#e0b848",
        targets: "ALL",
        available: true,
        active: true,
      },
      3,
    );
    const svg = decodeURIComponent(image);
    expect(svg).toContain('data-icon="flame"');
    expect(svg).toContain('data-frame="3"');
    expect(svg.match(/<svg/gu)).toHaveLength(1);
    expect(svg).toContain('stroke="#000000"');
    expect(svg).toContain('fill="#e0b848"');
    expect(svg).toContain('style="fill:#000000"');
    expect(svg).toContain(">BURN</text>");
    expect(svg).toContain(">ALL</text>");
  });

  it("uses white running content on dark custom colors", () => {
    const image = renderHotkeyTile({
      configured: true,
      label: "MEZ",
      icon: "flame",
      color: "#204080",
      targets: "2·6",
      available: true,
      active: true,
    });
    const svg = decodeURIComponent(image);
    expect(svg).toContain('fill="#204080"');
    expect(svg).toContain('style="fill:#ffffff"');
    expect(contrastingHotkeyForeground("#204080")).toBe("#ffffff");
    expect(contrastingHotkeyForeground("#e0b848")).toBe("#000000");
  });

  it("uses the custom color for an idle configured icon", () => {
    const image = renderHotkeyTile({
      configured: true,
      label: "MEZ",
      icon: "flame",
      color: "#4a86d4",
      targets: "ALL",
      available: true,
      active: false,
    });
    expect(decodeURIComponent(image)).toContain('color="#4a86d4"');
  });

  it("adds an explicit status cue when a configured hotkey is unavailable", () => {
    const image = renderHotkeyTile({
      configured: true,
      label: "BURN",
      icon: "flame",
      targets: "ALL",
      status: "INPUT NOT READY",
      available: false,
      active: false,
    });
    expect(decodeURIComponent(image)).toContain(">INPUT NOT READY</text>");
  });
});

function connectedStore(): DashboardStore {
  const store = new DashboardStore();
  store.setConnection({
    state: "connected",
    title: "Connected",
    detail: "local",
  });
  store.setSnapshot(stateFixture());
  return store;
}

function fakeClient() {
  return {
    sendEqActionBatch: vi.fn().mockResolvedValue(batchResult()),
    listEqKeymapActions: vi.fn().mockResolvedValue({
      type: "eq_keymap_actions_listed",
      mappings: [],
      window_numbers: [],
    }),
  };
}

function fakeKey(settings: Record<string, unknown>) {
  return {
    id: "hotkey-context",
    manifestId: "co.laikasoft.stonemite.hotkey",
    settings,
    isKey: () => true,
    isInMultiAction: () => false,
    getSettings: vi.fn().mockResolvedValue(settings),
    setImage: vi.fn().mockResolvedValue(undefined),
    showAlert: vi.fn().mockResolvedValue(undefined),
    showOk: vi.fn().mockResolvedValue(undefined),
  };
}

function batchResult() {
  return {
    type: "result" as const,
    version: 1 as const,
    request_id: "batch-1",
    result: {
      type: "eq_action_batch_delivered" as const,
      action: { type: "keymap" as const, mapping: "DUCK" },
      window_numbers: [1],
    },
    state: stateFixture(),
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, resolve, reject };
}

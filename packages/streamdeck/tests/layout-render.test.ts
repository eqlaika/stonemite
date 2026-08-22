import { describe, expect, it } from "vitest";
import { renderCell, SWAP_COLOR } from "../src/render/key-svg";
import { buildKey, buildSwapPlan } from "../src/state/layout";
import type { DashboardView } from "../src/state/store";
import { stateFixture } from "./fixtures";

describe("dashboard layout", () => {
  it("renders loaded and empty character slots by stable window number", () => {
    const snapshot = stateFixture({
      clients: [
        {
          id: "two",
          character: "Serein",
          class_code: "BRD",
          window_number: 2,
          active: false,
          activatable: true,
          input_ready: true,
        },
        {
          id: "one",
          character: "Laika",
          class_code: "WAR",
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: true,
        },
      ],
      active_client_id: "one",
    });

    expect(buildKey(view({ snapshot }), "character-1")).toMatchObject({
      type: "character",
      slot: 1,
      client: { id: "one" },
      enabled: true,
    });
    expect(buildKey(view({ snapshot }), "character-3")).toEqual({
      type: "empty",
      slot: 3,
    });

    const svg = decodeSvg(
      renderCell(buildKey(view({ snapshot }), "character-1")),
    );
    expect(svg).toContain("Laika");
    expect(svg).toContain(">ACTIVE</text>");
    expect(svg).toContain(">1</text>");
  });

  it("renders unavailable identity safely and escapes character names", () => {
    const snapshot = stateFixture({
      clients: [
        {
          id: "unknown",
          character: "A&B<One>",
          window_number: 1,
          active: false,
          activatable: false,
          input_ready: false,
        },
      ],
    });
    const cell = buildKey(view({ snapshot }), "character-1");
    expect(cell).toMatchObject({ type: "character", enabled: false });
    const svg = decodeSvg(renderCell(cell));
    expect(svg).toContain("A&amp;B&lt;One&gt;");
    expect(svg).toContain(">?</text>");
    expect(svg).toContain("Input not ready");
  });

  it("shows boot stages before normal controls", () => {
    const first = buildKey(view({ bootStage: 0 }), "character-1");
    const second = buildKey(view({ bootStage: 2 }), "broadcast");
    expect(first).toEqual({ type: "boot", stage: 0 });
    expect(second).toEqual({ type: "boot", stage: 2 });
    expect(decodeSvg(renderCell(second))).toContain("CONNECTING");
  });

  it("derives Swap availability and animates its armed state", () => {
    const snapshot = stateFixture({
      active_client_id: "leader",
      clients: [
        {
          id: "leader",
          character: "Laika",
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: true,
        },
        {
          id: "other",
          character: "Serein",
          window_number: 2,
          active: false,
          activatable: true,
          input_ready: true,
        },
      ],
    });
    expect(buildSwapPlan(view({ snapshot }))).toMatchObject({
      available: true,
      status: "PRESS THEN PICK",
    });

    const idle = buildKey(view({ snapshot }), "swap");
    const armed = buildKey(view({ snapshot }), "swap", "swap", true);
    expect(idle).toMatchObject({ type: "swap", armed: false });
    expect(armed).toMatchObject({ type: "swap", armed: true });
    const idleSvg = decodeSvg(renderCell(idle));
    const firstArmedSvg = decodeSvg(renderCell(armed, 0));
    const secondArmedSvg = decodeSvg(renderCell(armed, 1));
    expect(idleSvg).toContain(`stroke="${SWAP_COLOR}"`);
    expect(firstArmedSvg).toContain(`fill="${SWAP_COLOR}"`);
    expect(firstArmedSvg).not.toBe(secondArmedSvg);

    const current = buildKey(
      view({ snapshot }),
      "character-1",
      "character-1",
      true,
    );
    const target = buildKey(
      view({ snapshot }),
      "character-2",
      "character-2",
      true,
    );
    expect(current).toMatchObject({ interaction: "swap" });
    expect(target).toMatchObject({ interaction: "swap" });
    expect(decodeSvg(renderCell(current))).toContain(">CURRENT</text>");
    expect(decodeSvg(renderCell(target))).toContain(">SELECT</text>");
  });

  it("explains unavailable Swap states", () => {
    const snapshot = stateFixture();
    snapshot.capabilities.swap_window_numbers = false;
    expect(buildSwapPlan(view({ snapshot }))).toMatchObject({
      available: false,
      status: "UPDATE STONEMITE",
    });
    expect(buildSwapPlan(view({ snapshot: null }))).toMatchObject({
      available: false,
      status: "NO STATE",
    });
    expect(
      buildSwapPlan(view({ snapshot, connectionState: "reconnecting" })),
    ).toMatchObject({ available: false, status: "OFFLINE" });
  });

  it("renders authoritative Broadcast state", () => {
    const off = buildKey(
      view({
        snapshot: stateFixture({
          broadcast: { available: true, enabled: false },
        }),
      }),
      "broadcast",
    );
    const on = buildKey(
      view({
        snapshot: stateFixture({
          broadcast: { available: true, enabled: true },
        }),
      }),
      "broadcast",
    );
    const unavailable = buildKey(
      view({
        snapshot: stateFixture({
          broadcast: { available: false, enabled: false },
        }),
      }),
      "broadcast",
    );
    expect(off).toMatchObject({
      type: "broadcast",
      available: true,
      enabled: false,
    });
    expect(on).toMatchObject({
      type: "broadcast",
      available: true,
      enabled: true,
    });
    expect(decodeSvg(renderCell(on))).toContain("#cc3020");
    expect(decodeSvg(renderCell(unavailable))).toContain("UNAVAILABLE");
  });

  it("renders connection state on Setup", () => {
    expect(
      decodeSvg(
        renderCell(buildKey(view({ connectionState: "connecting" }), "logo")),
      ),
    ).toContain("CONNECTING");
    expect(
      decodeSvg(
        renderCell(buildKey(view({ connectionState: "connected" }), "logo")),
      ),
    ).not.toContain("CONNECTING");
    expect(
      decodeSvg(
        renderCell(buildKey(view({ connectionState: "error" }), "logo")),
      ),
    ).toContain("SETUP");
  });

  it("renders bounded pending and error feedback", () => {
    const pending = buildKey(
      view({ feedback: ["broadcast", "pending", "Updating"] }),
      "broadcast",
    );
    const failed = buildKey(
      view({ feedback: ["swap", "error", "Client left"] }),
      "swap",
    );
    expect(decodeSvg(renderCell(pending))).toContain("WORKING");
    expect(decodeSvg(renderCell(failed))).toContain("FAILED");
    expect(decodeSvg(renderCell(failed))).toContain("CLIENT LEFT");
  });
});

function view(
  overrides: {
    snapshot?: DashboardView["snapshot"];
    bootStage?: number;
    connectionState?: DashboardView["connection"]["state"];
    feedback?: [string, "pending" | "error", string];
  } = {},
): DashboardView {
  const feedback = new Map<
    string,
    DashboardView["feedback"] extends ReadonlyMap<string, infer Value>
      ? Value
      : never
  >();
  if (overrides.feedback) {
    const [key, kind, message] = overrides.feedback;
    feedback.set(key, { kind, message, until: Date.now() + 1_000 });
  }
  return {
    connection: {
      state: overrides.connectionState ?? "connected",
      title: "Connected",
      detail: "local",
    },
    snapshot: Object.prototype.hasOwnProperty.call(overrides, "snapshot")
      ? (overrides.snapshot ?? null)
      : stateFixture(),
    feedback,
    bootStage: overrides.bootStage ?? 3,
  };
}

function decodeSvg(dataUrl: string): string {
  return decodeURIComponent(dataUrl.slice(dataUrl.indexOf(",") + 1));
}

import { describe, expect, it } from "vitest";
import {
  BADGE_COLORS,
  buildCell,
  buildGrid,
  buildSwapPlan,
  SLOT_COLORS,
  unsupportedCell,
} from "../src/state/layout";
import type { DashboardView } from "../src/state/store";
import { ACTION_COLORS, escapeXml, renderCell } from "../src/render/key-svg";
import { stateFixture } from "./fixtures";

function view(overrides: Partial<DashboardView> = {}): DashboardView {
  return {
    connection: {
      state: "connected",
      title: "Connected",
      detail: "host:19720",
    },
    snapshot: stateFixture(),
    feedback: new Map(),
    bootStage: 3,
    ...overrides,
  };
}

function sixClients() {
  const names = [
    "Laika",
    "Serein",
    "Mora",
    "Cadence",
    "Ember",
    "Rook",
  ] as const;
  const classes = ["WAR", "CLR", "SHM", "BRD", "MAG", "BER"] as const;
  return names.map((character, index) => ({
    id: `client-${index + 1}`,
    character,
    server: "Xegony",
    class_code: classes[index] ?? "?",
    window_number: index + 1,
    active: index === 0,
    activatable: index !== 5,
    input_ready: index !== 4,
  }));
}

describe("5 by 3 layout", () => {
  it("builds exactly one deterministic cell for every coordinate", () => {
    const grid = buildGrid(view());
    expect(grid).toHaveLength(15);
    expect(new Set(grid.map((cell) => `${cell.row},${cell.column}`)).size).toBe(
      15,
    );
    expect(() => buildCell(view(), 3, 0)).toThrow(RangeError);
  });

  it("maps windows one through six to the two mockup rows", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const grid = buildGrid(view({ snapshot }));
    const characters = grid.filter((cell) => cell.type === "character");
    expect(characters.map((cell) => [cell.row, cell.column])).toEqual([
      [0, 0],
      [0, 1],
      [0, 2],
      [1, 0],
      [1, 1],
      [1, 2],
    ]);
    expect(characters[5]).toMatchObject({
      type: "character",
      enabled: false,
      slot: 6,
    });
  });

  it("handles zero, unknown, and extra clients honestly", () => {
    const empty = buildGrid(
      view({ snapshot: stateFixture({ clients: [], active_client_id: null }) }),
    );
    expect(empty.filter((cell) => cell.type === "empty")).toHaveLength(6);
    expect(
      empty.find((cell) => cell.row === 2 && cell.column === 0),
    ).toMatchObject({ type: "blank" });

    const clients = [
      ...sixClients(),
      {
        id: "client-7",
        server: "Xegony",
        class_code: "WAR",
        window_number: 7,
        active: false,
        activatable: false,
        input_ready: false,
      },
    ];
    const extra = buildGrid(view({ snapshot: stateFixture({ clients }) }));
    expect(extra.filter((cell) => cell.type === "character")).toHaveLength(6);
    expect(
      extra.find((cell) => cell.row === 2 && cell.column === 0),
    ).toMatchObject({ type: "blank" });

    const unknown = buildCell(
      view({
        snapshot: stateFixture({
          clients: [
            {
              id: "unknown",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: false,
            },
          ],
        }),
      }),
      0,
      0,
    );
    expect(unknown).toMatchObject({ type: "character", slot: 1 });
    expect(decodeSvg(renderCell(unknown))).toContain("Client 1");
  });

  it("replaces Link/Live with an actionable Group key for ready named boxes", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const group = buildCell(view({ snapshot }), 0, 3);
    expect(group).toMatchObject({
      type: "group",
      available: true,
      ready: 4,
      status: "4 BOXES READY",
    });
    const svg = decodeSvg(renderCell(group));
    expect(svg).toContain('data-icon="group"');
    expect(svg).toContain('data-icon-set="lucide-animated"');
    expect(svg).toContain(`stroke="${ACTION_COLORS.group}"`);
    expect(svg).toContain('d="M18 21a8 8 0 0 0-16 0"');
    expect(renderCell(group, 0)).toBe(renderCell(group, 6));
    expect(svg).toContain('font-size="15"');
    expect(svg).toContain(">Group</text>");
    expect(svg).not.toContain("BOXES READY");
    expect(svg).not.toContain(">FORM</text>");
    expect(svg).not.toContain(">LIVE</text>");

    const unavailable = buildCell(
      view({
        snapshot: stateFixture({
          clients: [
            {
              id: "active",
              character: "Laika",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: true,
            },
            {
              id: "unknown",
              window_number: 2,
              active: false,
              activatable: true,
              input_ready: true,
            },
            {
              id: "unready",
              character: "Mora",
              window_number: 3,
              active: false,
              activatable: true,
              input_ready: false,
            },
          ],
          active_client_id: "active",
        }),
      }),
      0,
      3,
    );
    expect(unavailable).toMatchObject({
      type: "group",
      available: false,
      ready: 0,
      status: "NO READY BOXES",
    });
    const unavailableSvg = decodeSvg(renderCell(unavailable));
    expect(unavailableSvg).toContain('data-icon="group"');
    expect(unavailableSvg).toContain('stroke="#6d737c"');
    expect(unavailableSvg).toContain(">Group</text>");
    expect(unavailableSvg).not.toContain("NO READY BOXES");
  });

  it("replaces input readiness with Follow for ready background boxes", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const follow = buildCell(view({ snapshot }), 1, 3);
    expect(follow).toMatchObject({
      type: "follow",
      available: true,
      ready: 4,
      status: "4 BOXES READY",
    });
    const svg = decodeSvg(renderCell(follow));
    expect(svg).toContain('data-icon="follow"');
    expect(svg).toContain('data-icon-set="lucide-animated"');
    expect(svg).toContain(`stroke="${ACTION_COLORS.follow}"`);
    expect(svg).toContain(
      'd="M9 19h8.5a3.5 3.5 0 0 0 0-7h-11a3.5 3.5 0 0 1 0-7H15"',
    );
    expect(svg).toContain(">Follow</text>");
    expect(svg).not.toContain("BOXES READY");
    expect(svg).not.toContain(">START</text>");
    expect(svg).not.toContain(">INPUT</text>");

    const leaderUnknown = buildCell(
      view({
        snapshot: stateFixture({
          clients: [
            {
              id: "leader",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: false,
            },
            {
              id: "follower",
              character: "Serein",
              window_number: 2,
              active: false,
              activatable: true,
              input_ready: true,
            },
          ],
          active_client_id: "leader",
        }),
      }),
      1,
      3,
    );
    expect(leaderUnknown).toMatchObject({
      type: "follow",
      available: false,
      ready: 0,
      status: "LEADER UNKNOWN",
    });
  });

  it("replaces the ambient STONE tile with Assist for ready background boxes", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const assist = buildCell(view({ snapshot }), 2, 3);
    expect(assist).toMatchObject({
      type: "assist",
      available: true,
      ready: 4,
      status: "4 BOXES READY",
    });
    const svg = decodeSvg(renderCell(assist));
    expect(svg).toContain('data-icon="assist"');
    expect(svg).toContain('data-icon-set="lucide-animated"');
    expect(svg).toContain(`stroke="${ACTION_COLORS.assist}"`);
    expect(svg).toContain('data-motion-part="outer-target"');
    expect(svg).toContain(">Assist</text>");
    expect(svg).not.toContain("BOXES READY");
    expect(svg).not.toContain(">STONE</text>");

    const mainUnknown = buildCell(
      view({
        snapshot: stateFixture({
          clients: [
            {
              id: "main",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: true,
            },
            {
              id: "assistant",
              character: "Serein",
              window_number: 2,
              active: false,
              activatable: true,
              input_ready: true,
            },
          ],
          active_client_id: "main",
        }),
      }),
      2,
      3,
    );
    expect(mainUnknown).toMatchObject({
      type: "assist",
      available: false,
      ready: 0,
      status: "MAIN UNKNOWN",
    });
  });

  it("animates action icons while Group, Follow, and Assist are in flight", () => {
    const pendingGroup = buildCell(
      view({
        feedback: new Map([
          [
            "0,3",
            {
              kind: "pending",
              message: "Inviting",
              motion: "group",
              until: Date.now() + 1_000,
            },
          ],
        ]),
      }),
      0,
      3,
    );
    const firstFrame = decodeSvg(renderCell(pendingGroup, 0));
    const nextFrame = decodeSvg(renderCell(pendingGroup, 3));
    expect(firstFrame).toContain('data-icon="group"');
    expect(firstFrame).toContain('data-active="true"');
    expect(firstFrame).toContain(`fill="${ACTION_COLORS.group}"`);
    expect(firstFrame).toContain('stroke="#101615"');
    expect(firstFrame).toContain('class="active-text center"');
    expect(firstFrame).toContain(">Group</text>");
    expect(firstFrame).not.toContain("INVITING");
    expect(nextFrame).not.toBe(firstFrame);

    const pendingFollow = buildCell(
      view({
        feedback: new Map([
          [
            "1,3",
            {
              kind: "pending",
              message: "Sending",
              motion: "follow",
              until: Date.now() + 1_000,
            },
          ],
        ]),
      }),
      1,
      3,
    );
    const followFrame = decodeSvg(renderCell(pendingFollow, 3));
    expect(followFrame).toContain('data-icon="follow"');
    expect(followFrame).toContain('data-active="true"');
    expect(followFrame).toContain(`fill="${ACTION_COLORS.follow}"`);
    expect(followFrame).toContain('stroke="#101615"');
    expect(followFrame).toContain(">Follow</text>");

    const pendingAssist = buildCell(
      view({
        feedback: new Map([
          [
            "2,3",
            {
              kind: "pending",
              message: "Sending",
              motion: "assist",
              until: Date.now() + 1_000,
            },
          ],
        ]),
      }),
      2,
      3,
    );
    const assistFrame = decodeSvg(renderCell(pendingAssist, 3));
    expect(assistFrame).toContain('data-icon="assist"');
    expect(assistFrame).toContain('data-active="true"');
    expect(assistFrame).toContain(`fill="${ACTION_COLORS.assist}"`);
    expect(assistFrame).toContain('stroke="#101615"');
    expect(assistFrame).toContain(">Assist</text>");
    expect(assistFrame).not.toContain("SENDING");
  });

  it("turns the MITE key into an explicit two-step window-number swap", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const swap = buildCell(view({ snapshot }), 2, 4);
    expect(buildSwapPlan(view({ snapshot }))).toMatchObject({
      available: true,
      status: "PRESS THEN PICK",
    });
    expect(swap).toMatchObject({
      type: "swap",
      available: true,
      armed: false,
    });
    const idleSwapSvg = decodeSvg(renderCell(swap));
    expect(idleSwapSvg).toContain('data-icon="swap"');
    expect(idleSwapSvg).toContain('data-icon-set="lucide-animated"');
    expect(idleSwapSvg).toContain(`stroke="${ACTION_COLORS.swap}"`);
    expect(idleSwapSvg).toContain('d="M8 3 4 7l4 4"');
    expect(idleSwapSvg).toContain('data-active="false"');
    expect(idleSwapSvg).toContain(">Swap</text>");
    expect(idleSwapSvg).not.toContain("PRESS THEN PICK");

    const armedSwap = buildCell(view({ snapshot }), 2, 4, true);
    expect(armedSwap).toMatchObject({ type: "swap", armed: true });
    const armedFrameZero = decodeSvg(renderCell(armedSwap, 0));
    const armedFrameThree = decodeSvg(renderCell(armedSwap, 3));
    expect(armedFrameZero).toContain('data-active="true"');
    expect(armedFrameZero).toContain(`fill="${ACTION_COLORS.swap}"`);
    expect(armedFrameZero).toContain('stroke="#101615"');
    expect(armedFrameZero).toContain('class="active-text center"');
    expect(armedFrameZero).toContain(">Swap</text>");
    expect(armedFrameZero).not.toContain("PICK CHARACTER");
    expect(armedFrameThree).not.toBe(armedFrameZero);

    const current = buildCell(view({ snapshot }), 0, 0, true);
    const target = buildCell(view({ snapshot }), 0, 1, true);
    expect(current).toMatchObject({
      type: "character",
      interaction: "swap",
    });
    expect(target).toMatchObject({
      type: "character",
      interaction: "swap",
    });
    expect(decodeSvg(renderCell(current))).toContain(">CURRENT</text>");
    expect(decodeSvg(renderCell(target))).toContain(">SELECT</text>");
  });

  it("keeps window-number swap unavailable against older Stonemite versions", () => {
    const snapshot = stateFixture();
    snapshot.capabilities.swap_window_numbers = false;
    const swap = buildCell(view({ snapshot }), 2, 4, true);
    expect(swap).toMatchObject({
      type: "swap",
      available: false,
      armed: false,
      status: "UPDATE STONEMITE",
    });

    const noVisibleTarget = buildCell(
      view({
        snapshot: stateFixture({
          clients: [
            {
              id: "active",
              window_number: 1,
              active: true,
              activatable: true,
              input_ready: true,
            },
            {
              id: "extra",
              window_number: 7,
              active: false,
              activatable: false,
              input_ready: true,
            },
          ],
          active_client_id: "active",
        }),
      }),
      2,
      4,
      true,
    );
    expect(noVisibleTarget).toMatchObject({
      type: "swap",
      available: false,
      armed: false,
      status: "NO VISIBLE TARGET",
    });
  });

  it("reflects broadcast unavailable, off, and on", () => {
    const unavailable = buildCell(
      view({
        snapshot: stateFixture({
          broadcast: { available: false, enabled: false },
        }),
      }),
      0,
      4,
    );
    const off = buildCell(
      view({
        snapshot: stateFixture({
          broadcast: { available: true, enabled: false },
        }),
      }),
      0,
      4,
    );
    const on = buildCell(
      view({
        snapshot: stateFixture({
          broadcast: { available: true, enabled: true },
        }),
      }),
      0,
      4,
    );
    expect(unavailable).toMatchObject({
      type: "broadcast",
      available: false,
      enabled: false,
    });
    expect(decodeSvg(renderCell(unavailable))).toContain("UNAVAILABLE");
    expect(off).toMatchObject({
      type: "broadcast",
      available: true,
      enabled: false,
    });
    expect(decodeSvg(renderCell(off))).toContain(">Bcast</text>");
    const onSvg = decodeSvg(renderCell(on));
    expect(onSvg).toContain("#cc3020");
    expect(onSvg).toContain(">Bcast</text>");
    expect(onSvg).not.toContain("Bcast on");
    expect(onSvg).not.toContain("BROADCAST");
  });

  it("uses all exact Stonemite colors and honest active/readiness states", () => {
    const snapshot = stateFixture({ clients: sixClients() });
    const characterCells = buildGrid(view({ snapshot })).filter(
      (cell) => cell.type === "character",
    );
    characterCells.forEach((cell, index) => {
      const svg = decodeSvg(renderCell(cell));
      expect(svg).toContain(SLOT_COLORS[index]);
      expect(svg).toContain(BADGE_COLORS[index]);
    });
    const active = decodeSvg(renderCell(characterCells[0]!));
    expect(active).toContain('fill="#f5f7fa"');
    expect(active).toContain('class="active-text center"');
    expect(active).toContain(">ACTIVE</text>");
    expect(active).toContain(`stroke="${SLOT_COLORS[0]}" stroke-width="3"`);
    const inputUnready = decodeSvg(renderCell(characterCells[4]!));
    expect(inputUnready).toContain('fill="#ffc75c"');
    const disabled = decodeSvg(renderCell(characterCells[5]!));
    expect(disabled).toContain('fill="#080a0d" opacity=".24"');
    expect(buildCell(view({ snapshot }), 1, 3)).toMatchObject({
      type: "follow",
      available: true,
      ready: 4,
      status: "4 BOXES READY",
    });
  });

  it("renders unsupported coordinates as a static 5 by 3 requirement", () => {
    const svg = decodeSvg(renderCell(unsupportedCell(3, 0)));
    expect(svg).toContain("5 × 3");
    expect(svg).toContain("LAYOUT REQUIRED");
    expect(svg).not.toContain("REV ");
  });
});

describe("SVG rendering", () => {
  it("escapes all dynamic XML characters", () => {
    expect(escapeXml(`<Mora & "Rook" 'x'>`)).toBe(
      "&lt;Mora &amp; &quot;Rook&quot; &apos;x&apos;&gt;",
    );
    const snapshot = stateFixture({
      clients: [
        {
          id: "client-1",
          character: `<Mora & "Rook">`,
          window_number: 1,
          active: true,
          activatable: true,
          input_ready: true,
        },
      ],
    });
    const svg = decodeSvg(renderCell(buildCell(view({ snapshot }), 0, 0)));
    expect(svg).toContain("&lt;Mora &amp; &quot;Rook&quot;&gt;");
    expect(svg).not.toContain(`<Mora & "Rook">`);
  });

  it("keeps the paired, client-count, active, and server cells blank", () => {
    const coordinates = [
      [1, 4],
      [2, 0],
      [2, 1],
      [2, 2],
    ] as const;

    for (const [row, column] of coordinates) {
      const cell = buildCell(view(), row, column);
      expect(cell).toMatchObject({ type: "blank", row, column });
      const svg = decodeSvg(renderCell(cell));
      expect(svg).not.toContain("<text");
      expect(svg).not.toContain("<path");
      expect(svg).not.toContain("<image");
    }
  });

  it("renders no colored top rails", () => {
    for (const cell of buildGrid(view())) {
      expect(decodeSvg(renderCell(cell))).not.toContain(
        '<rect width="72" height="3"',
      );
    }
  });

  it("never renders deck text below the 15 pixel character-name floor", () => {
    const feedback = buildCell(
      view({
        feedback: new Map([
          [
            "0,3",
            {
              kind: "error",
              message: "Partial assist",
              until: Date.now() + 1_000,
            },
          ],
        ]),
      }),
      0,
      3,
    );
    const cells = [
      ...buildGrid(view()),
      unsupportedCell(3, 0),
      buildCell(view({ bootStage: 1 }), 0, 0),
      feedback,
    ];

    for (const cell of cells) {
      const svg = decodeSvg(renderCell(cell));
      const fontSizes = [...svg.matchAll(/font-size(?::|=)"?(\d+)/g)].map(
        (match) => Number(match[1]),
      );
      expect(fontSizes.length).toBeGreaterThan(0);
      expect(Math.min(...fontSizes)).toBeGreaterThanOrEqual(15);
    }
  });

  it("is deterministic and always targets a 72 pixel canvas", () => {
    const cell = buildCell(view(), 0, 3);
    expect(renderCell(cell)).toBe(renderCell(cell));
    expect(decodeSvg(renderCell(cell))).toContain(
      'width="72" height="72" viewBox="0 0 72 72"',
    );
  });
});

function decodeSvg(dataUrl: string): string {
  return decodeURIComponent(dataUrl.slice(dataUrl.indexOf(",") + 1));
}

import { describe, expect, it } from "vitest";
import {
  BADGE_COLORS,
  buildCell,
  buildGrid,
  SLOT_COLORS,
  unsupportedCell,
} from "../src/state/layout";
import type { DashboardView } from "../src/state/store";
import { escapeXml, renderCell } from "../src/render/key-svg";
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
    ).toMatchObject({ main: "0" });

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
    ).toMatchObject({ main: "7" });

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
    expect(decodeSvg(renderCell(on))).toContain("#cc3020");
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
      type: "utility",
      main: "5 / 6",
      bottom: "EXACT CLIENTS",
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

  it("keeps utility headers fully visible from the left edge", () => {
    const coordinates = [
      [1, 3, "INPUT"],
      [1, 4, "STONEMITE"],
      [2, 2, "SERVER"],
    ] as const;

    for (const [row, column, label] of coordinates) {
      const svg = decodeSvg(renderCell(buildCell(view(), row, column)));
      expect(svg).toContain(`<text x="6" y="15" class="utility-label"`);
      expect(svg).toContain(`>${label}</text>`);
      expect(svg).not.toContain('class="center" text-anchor="start"');
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

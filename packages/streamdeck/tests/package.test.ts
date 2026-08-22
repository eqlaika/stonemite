import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vitest";
import {
  DASHBOARD_ACTION_DEFINITIONS,
  HOTKEY_ACTION_DEFINITION,
} from "../src/actions/key-definitions";
import { CLASS_IMAGES } from "../src/render/assets.generated";

const EXPECTED_CLASSES = [
  "BER",
  "BRD",
  "BST",
  "CLR",
  "DRU",
  "ENC",
  "MAG",
  "MNK",
  "NEC",
  "PAL",
  "RNG",
  "ROG",
  "SHK",
  "SHM",
  "WAR",
  "WIZ",
];

async function readPngSize(path: string) {
  const bytes = await readFile(new URL(path, import.meta.url));
  expect(bytes.subarray(1, 4).toString("ascii")).toBe("PNG");
  return {
    height: bytes.readUInt32BE(20),
    width: bytes.readUInt32BE(16),
  };
}

describe("release inputs", () => {
  it("ships every customizable action and keeps Node inspection disabled", async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(
          "../co.laikasoft.stonemite.sdPlugin/manifest.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ) as {
      Actions?: Array<{
        Icon?: string;
        Name?: string;
        PropertyInspectorPath?: string;
        Tooltip?: string;
        UUID?: string;
      }>;
      Author?: string;
      Category?: string;
      Icon?: string;
      Name?: string;
      Nodejs?: { Debug?: string; Version?: string };
      Profiles?: unknown;
      UUID?: string;
      PropertyInspectorPath?: string;
    };
    expect(manifest.Name).toBe("Stonemite · EQ boxing");
    expect(manifest.Author).toBe("Laikasoft");
    expect(manifest.Category).toBe("Stonemite · EQ boxing");
    expect(manifest.Icon).toBe("imgs/plugin/icon");
    expect(manifest.UUID).toBe("co.laikasoft.stonemite");
    expect(manifest.Nodejs?.Version).toBe("24");
    expect(manifest.Nodejs?.Debug).toBeUndefined();
    expect(manifest.Profiles).toBeUndefined();
    expect(manifest.PropertyInspectorPath).toBeUndefined();
    const actualActions =
      manifest.Actions?.map(({ Name, Tooltip, UUID }) => ({
        name: Name,
        tooltip: Tooltip,
        uuid: UUID,
      })).sort((a, b) => String(a.uuid).localeCompare(String(b.uuid))) ?? [];
    const expectedActions = [
      ...DASHBOARD_ACTION_DEFINITIONS.map(({ name, tooltip, uuid }) => ({
        name,
        tooltip,
        uuid,
      })),
      HOTKEY_ACTION_DEFINITION,
    ].sort((a, b) => a.uuid.localeCompare(b.uuid));
    expect(actualActions).toEqual(expectedActions);
    expect(actualActions.map((action) => action.uuid)).not.toEqual(
      expect.arrayContaining([
        "co.laikasoft.stonemite.group",
        "co.laikasoft.stonemite.follow",
        "co.laikasoft.stonemite.assist",
        "co.laikasoft.stonemite.use",
      ]),
    );
    for (const action of manifest.Actions ?? []) {
      expect(action.Icon).toBe("imgs/actions/stonemite/icon");
      expect(action.PropertyInspectorPath).toBe(
        action.UUID === "co.laikasoft.stonemite.logo"
          ? "ui/pairing.html"
          : action.UUID === "co.laikasoft.stonemite.hotkey"
            ? "ui/hotkey.html"
            : undefined,
      );
    }
  });

  it("ships correctly sized Stream Deck and Marketplace icons", async () => {
    const iconCases = [
      ["imgs/plugin/icon.png", 256],
      ["imgs/plugin/icon@2x.png", 512],
      ["imgs/plugin/category-icon.png", 28],
      ["imgs/plugin/category-icon@2x.png", 56],
      ["imgs/actions/stonemite/icon.png", 20],
      ["imgs/actions/stonemite/icon@2x.png", 40],
      ["imgs/actions/stonemite/key.png", 72],
      ["imgs/actions/stonemite/key@2x.png", 144],
    ] as const;
    for (const [path, size] of iconCases) {
      await expect(
        readPngSize(`../co.laikasoft.stonemite.sdPlugin/${path}`),
      ).resolves.toEqual({ height: size, width: size });
    }
    await expect(readPngSize("../marketplace/icon.png")).resolves.toEqual({
      height: 288,
      width: 288,
    });
  });

  it("ships the mapped-hotkey inspector and pinned icon previews", async () => {
    const [html, script, previews] = await Promise.all([
      readFile(
        new URL(
          "../co.laikasoft.stonemite.sdPlugin/ui/hotkey.html",
          import.meta.url,
        ),
        "utf8",
      ),
      readFile(
        new URL(
          "../co.laikasoft.stonemite.sdPlugin/ui/hotkey.js",
          import.meta.url,
        ),
        "utf8",
      ),
      readFile(
        new URL(
          "../co.laikasoft.stonemite.sdPlugin/ui/lucide-animated-icons.generated.js",
          import.meta.url,
        ),
        "utf8",
      ),
    ]);
    expect(html).toContain("Only actions with a key mapping");
    expect(html).toContain('value="active"');
    expect(html).toContain('value="background"');
    expect(html).toContain("character-specific EQ social");
    expect(html).toContain("no key mapping");
    expect(html).toContain("lucide-animated-icons.generated.js");
    expect(html).toContain("No icons match this search.");
    expect(html).toContain("Choose an icon for this tile.");
    expect(html).not.toContain("Lucide Animated");
    expect(html).not.toContain('id="connection"');
    expect(html).toContain('id="tile-color"');
    expect(html).toContain('data-color="#59d8d0"');
    expect(html).toContain("Changes save automatically.");
    expect(html).not.toContain('id="save"');
    expect(html).not.toContain("Save tile");
    expect(script).toContain("await client.getSettings()");
    expect(script).toContain("await client.setSettings(");
    expect(script).toContain("color: draft.color");
    expect(script).not.toContain("showConnection");
    expect(script).toContain("mappingValidated");
    expect(script).toContain('{ type: "active" }');
    expect(script).toContain('{ type: "background_loaded" }');
    expect(script).not.toContain("custom image");
    expect(previews).toContain("Lucide Animated (MIT)");
    expect(previews).toContain('"flame"');
    expect(previews).not.toContain('"footprints"');
  });

  it("embeds every required Stonemite class icon", () => {
    expect(Object.keys(CLASS_IMAGES)).toEqual(
      expect.arrayContaining(EXPECTED_CLASSES),
    );
    for (const code of EXPECTED_CLASSES) {
      expect(CLASS_IMAGES[code]).toMatch(/^data:image\/png;base64,/);
    }
  });
});

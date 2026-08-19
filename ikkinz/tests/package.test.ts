import { readFile } from "node:fs/promises";
import { strFromU8, unzipSync } from "fflate";
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

const EXPECTED_PROFILE_ACTIONS = {
  "0,0": "co.laikasoft.ikkinz.character-slot-1",
  "1,0": "co.laikasoft.ikkinz.character-slot-2",
  "2,0": "co.laikasoft.ikkinz.character-slot-3",
  "3,0": "co.laikasoft.ikkinz.group",
  "4,0": "co.laikasoft.ikkinz.broadcast",
  "0,1": "co.laikasoft.ikkinz.character-slot-4",
  "1,1": "co.laikasoft.ikkinz.character-slot-5",
  "2,1": "co.laikasoft.ikkinz.character-slot-6",
  "3,1": "co.laikasoft.ikkinz.follow",
  "4,1": "co.laikasoft.ikkinz.use",
  "0,2": "co.laikasoft.ikkinz.logo",
  "3,2": "co.laikasoft.ikkinz.assist",
  "4,2": "co.laikasoft.ikkinz.swap",
} as const;

const EXPECTED_POSITIONS = Object.keys(EXPECTED_PROFILE_ACTIONS).sort();

const PROFILE_CASES = [
  {
    deviceModel: "20GBL9901",
    deviceType: 0,
    displayName: "Stonemite · EQ boxing",
    filename: "Ikkinz.streamDeckProfile",
    name: "Ikkinz",
  },
  {
    deviceModel: "VSD2/WiFi",
    deviceType: 3,
    displayName: "Stonemite · EQ boxing",
    filename: "Ikkinz Mobile.streamDeckProfile",
    name: "Ikkinz Mobile",
  },
] as const;

type ProfileAction = {
  ActionID: string;
  Name?: string;
  Plugin?: { Name?: string; UUID?: string };
  UUID?: string;
};

type ProfilePage = {
  Controllers?: Array<{ Actions?: Record<string, ProfileAction> | null }>;
  Name?: string;
};

function parseArchiveJson<T>(
  archive: Record<string, Uint8Array>,
  path: string,
): T {
  const contents = archive[path];
  if (contents === undefined) throw new Error(`Missing ${path}`);
  return JSON.parse(strFromU8(contents)) as T;
}

describe("release inputs", () => {
  it("ships every customizable action and keeps Node inspection disabled", async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/manifest.json",
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
      Category?: string;
      Name?: string;
      Nodejs?: { Debug?: string; Version?: string };
      PropertyInspectorPath?: string;
    };
    expect(manifest.Name).toBe("Stonemite · EQ boxing");
    expect(manifest.Category).toBe("Stonemite · EQ boxing");
    expect(manifest.Nodejs?.Version).toBe("24");
    expect(manifest.Nodejs?.Debug).toBeUndefined();
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
    for (const action of manifest.Actions ?? []) {
      expect(action.Icon).toBe("imgs/actions/stonemite/icon");
      expect(action.PropertyInspectorPath).toBe(
        action.UUID === "co.laikasoft.ikkinz.logo"
          ? "ui/pairing.html"
          : action.UUID === "co.laikasoft.ikkinz.hotkey"
            ? "ui/hotkey.html"
            : undefined,
      );
    }
  });

  it("ships the mapped-hotkey inspector and pinned icon previews", async () => {
    const [html, script, previews] = await Promise.all([
      readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/ui/hotkey.html",
          import.meta.url,
        ),
        "utf8",
      ),
      readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/ui/hotkey.js",
          import.meta.url,
        ),
        "utf8",
      ),
      readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/ui/lucide-animated-icons.generated.js",
          import.meta.url,
        ),
        "utf8",
      ),
    ]);
    expect(html).toContain("Only actions with a key mapping");
    expect(html).toContain("no key mapping");
    expect(html).toContain("lucide-animated-icons.generated.js");
    expect(html).toContain("No icons match this search.");
    expect(html).toContain("Choose an icon for this tile.");
    expect(html).not.toContain("Lucide Animated");
    expect(html).not.toContain('id="connection"');
    expect(html).toContain('id="tile-color"');
    expect(html).toContain('data-color="#59d8d0"');
    expect(script).toContain("await client.getSettings()");
    expect(script).toContain("color: draft.color");
    expect(script).not.toContain("showConnection");
    expect(script).toContain("mappingValidated");
    expect(script).not.toContain("custom image");
    expect(previews).toContain("Lucide Animated (MIT)");
    expect(previews).toContain('"flame"');
    expect(previews).not.toContain('"footprints"');
  });

  it("bundles complete editable 5 by 3 profiles", async () => {
    const pluginManifest = JSON.parse(
      await readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/manifest.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ) as {
      Profiles?: Array<{
        AutoInstall?: boolean;
        DeviceType?: number;
        DontAutoSwitchWhenInstalled?: boolean;
        Name?: string;
        Readonly?: boolean;
      }>;
    };

    expect(pluginManifest.Profiles).toEqual(
      PROFILE_CASES.map((profile) => ({
        Name: `profiles/${profile.name}`,
        DeviceType: profile.deviceType,
        AutoInstall: true,
        DontAutoSwitchWhenInstalled: false,
        Readonly: false,
      })),
    );

    for (const profile of PROFILE_CASES) {
      const bytes = await readFile(
        new URL(
          `../co.laikasoft.ikkinz.sdPlugin/profiles/${profile.filename}`,
          import.meta.url,
        ),
      );
      const archive = unzipSync(new Uint8Array(bytes));
      const packageManifest = parseArchiveJson<{
        DeviceModel?: string;
        RequiredPlugins?: string[];
      }>(archive, "package.json");
      expect(packageManifest.DeviceModel).toBe(profile.deviceModel);
      expect(packageManifest.RequiredPlugins).toEqual(["co.laikasoft.ikkinz"]);

      const rootManifestPath = Object.keys(archive).filter((path) =>
        /^Profiles\/[^/]+\.sdProfile\/manifest\.json$/.test(path),
      );
      expect(rootManifestPath).toHaveLength(1);
      const [rootPath] = rootManifestPath;
      if (rootPath === undefined) throw new Error("Missing profile manifest");
      const rootManifest = parseArchiveJson<{
        Device?: { Model?: string };
        Name?: string;
        Pages?: { Pages?: string[] };
      }>(archive, rootPath);
      expect(rootManifest.Device?.Model).toBe(profile.deviceModel);
      expect(rootManifest.Name).toBe(profile.displayName);
      expect(rootManifest.Pages?.Pages).toHaveLength(1);

      const pageManifestPaths = Object.keys(archive).filter((path) =>
        /^Profiles\/[^/]+\.sdProfile\/Profiles\/[^/]+\/manifest\.json$/.test(
          path,
        ),
      );
      expect(pageManifestPaths).toHaveLength(2);
      const pages = pageManifestPaths.map((path) =>
        parseArchiveJson<ProfilePage>(archive, path),
      );
      const dashboard = pages.find(
        (page) => page.Controllers?.[0]?.Actions != null,
      );
      expect(dashboard?.Name).toBe("Stonemite · EQ boxing");
      const actions = dashboard?.Controllers?.[0]?.Actions;
      expect(Object.keys(actions ?? {}).sort()).toEqual(EXPECTED_POSITIONS);
      expect(
        new Set(Object.values(actions ?? {}).map((action) => action.ActionID))
          .size,
      ).toBe(EXPECTED_POSITIONS.length);
      for (const [position, action] of Object.entries(actions ?? {})) {
        const expectedUuid =
          EXPECTED_PROFILE_ACTIONS[
            position as keyof typeof EXPECTED_PROFILE_ACTIONS
          ];
        const definition = DASHBOARD_ACTION_DEFINITIONS.find(
          (candidate) => candidate.uuid === expectedUuid,
        );
        expect(action.Plugin?.Name).toBe("Stonemite · EQ boxing");
        expect(action.Plugin?.UUID).toBe("co.laikasoft.ikkinz");
        expect(action.UUID).toBe(expectedUuid);
        expect(action.Name).toBe(definition?.name);
      }
    }
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

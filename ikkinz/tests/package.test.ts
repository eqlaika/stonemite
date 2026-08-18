import { readFile } from "node:fs/promises";
import { strFromU8, unzipSync } from "fflate";
import { describe, expect, it } from "vitest";
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

const EXPECTED_POSITIONS = Array.from({ length: 15 }, (_, index) => {
  const column = Math.floor(index / 3);
  const row = index % 3;
  return `${column},${row}`;
});

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
  it("keeps the packed manifest free of the Node inspector", async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(
          "../co.laikasoft.ikkinz.sdPlugin/manifest.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ) as {
      Category?: string;
      Name?: string;
      Nodejs?: { Debug?: string; Version?: string };
    };
    expect(manifest.Name).toBe("Stonemite · EQ boxing");
    expect(manifest.Category).toBe("Stonemite · EQ boxing");
    expect(manifest.Nodejs?.Version).toBe("24");
    expect(manifest.Nodejs?.Debug).toBeUndefined();
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
      ).toBe(15);
      for (const action of Object.values(actions ?? {})) {
        expect(action.Plugin?.Name).toBe("Stonemite · EQ boxing");
        expect(action.Plugin?.UUID).toBe("co.laikasoft.ikkinz");
        expect(action.UUID).toBe("co.laikasoft.ikkinz.grid-key");
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

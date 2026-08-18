import { readFile } from "node:fs/promises";
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

describe("release inputs", () => {
  it("keeps the packed manifest free of the Node inspector", async () => {
    const manifest = JSON.parse(
      await readFile(
        new URL(
          "../com.laikasoft.ikkinz.sdPlugin/manifest.json",
          import.meta.url,
        ),
        "utf8",
      ),
    ) as { Nodejs?: { Debug?: string; Version?: string } };
    expect(manifest.Nodejs?.Version).toBe("24");
    expect(manifest.Nodejs?.Debug).toBeUndefined();
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

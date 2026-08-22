import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { describe, expect, it } from "vitest";

interface TauriConfig {
  build: {
    devUrl?: string;
    frontendDist?: string;
  };
}

const config = JSON.parse(
  readFileSync(resolve("../crates/stonemite/tauri.conf.json"), "utf8"),
) as TauriConfig;

describe("Tauri settings assets", () => {
  it("embeds the built frontend in debug and release binaries", () => {
    expect(config.build.devUrl).toBeUndefined();
    expect(config.build.frontendDist).toBe("../../settings-ui/dist");
  });
});

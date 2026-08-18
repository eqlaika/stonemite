import { cp, mkdir, mkdtemp, rm } from "node:fs/promises";
import { spawn } from "node:child_process";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const command = process.argv[2];
if (command !== "validate" && command !== "pack") {
  throw new Error("Usage: node scripts/plugin-cli.mjs <validate|pack>");
}

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pluginName = "co.laikasoft.ikkinz.sdPlugin";
const source = path.join(root, pluginName);
const cli = path.join(root, "node_modules/@elgato/cli/bin/streamdeck.mjs");
const temporaryRoot = await mkdtemp(path.join(os.tmpdir(), "ikkinz-plugin-"));
const temporaryPlugin = path.join(temporaryRoot, pluginName);

try {
  await cp(source, temporaryPlugin, { recursive: true });

  const args = [cli, command, temporaryPlugin];
  if (command === "pack") {
    const dist = path.join(root, "dist");
    const artifact = path.join(dist, "co.laikasoft.ikkinz.streamDeckPlugin");
    await mkdir(dist, { recursive: true });
    await rm(artifact, { force: true });
    args.push("--output", dist, "--force");
  }

  const exitCode = await new Promise((resolve, reject) => {
    const child = spawn(process.execPath, args, {
      cwd: root,
      stdio: "inherit",
    });
    child.once("error", reject);
    child.once("exit", (code) => resolve(code ?? 1));
  });

  if (exitCode !== 0) process.exitCode = exitCode;
} finally {
  await rm(temporaryRoot, { force: true, recursive: true });
}

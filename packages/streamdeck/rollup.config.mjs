import commonjs from "@rollup/plugin-commonjs";
import nodeResolve from "@rollup/plugin-node-resolve";
import terser from "@rollup/plugin-terser";
import typescript from "@rollup/plugin-typescript";
import path from "node:path";
import url from "node:url";

const isWatching = Boolean(process.env.ROLLUP_WATCH);
const sdPlugin = "co.laikasoft.stonemite.sdPlugin";

/** @type {import("rollup").RollupOptions} */
export default {
  input: "src/plugin.ts",
  output: {
    file: `${sdPlugin}/bin/plugin.js`,
    format: "es",
    sourcemap: isWatching,
    sourcemapPathTransform(relativeSourcePath, sourcemapPath) {
      return url.pathToFileURL(
        path.resolve(path.dirname(sourcemapPath), relativeSourcePath),
      ).href;
    },
  },
  plugins: [
    {
      name: "watch-plugin-files",
      buildStart() {
        this.addWatchFile(`${sdPlugin}/manifest.json`);
        this.addWatchFile(`${sdPlugin}/ui/pairing.html`);
        this.addWatchFile(`${sdPlugin}/ui/pairing.js`);
        this.addWatchFile(`${sdPlugin}/ui/hotkey.html`);
        this.addWatchFile(`${sdPlugin}/ui/hotkey.js`);
        this.addWatchFile(`${sdPlugin}/ui/hotkey.css`);
        this.addWatchFile(`${sdPlugin}/ui/box.html`);
        this.addWatchFile(`${sdPlugin}/ui/box.js`);
        this.addWatchFile(`${sdPlugin}/ui/box.css`);
        this.addWatchFile(`${sdPlugin}/layouts/xtarget-v4.json`);
      },
    },
    typescript({ mapRoot: isWatching ? "./" : undefined }),
    nodeResolve({
      browser: false,
      exportConditions: ["node"],
      preferBuiltins: true,
    }),
    commonjs(),
    !isWatching && terser(),
    {
      name: "emit-module-package-file",
      generateBundle() {
        this.emitFile({
          fileName: "package.json",
          source: '{ "type": "module" }',
          type: "asset",
        });
      },
    },
  ],
};

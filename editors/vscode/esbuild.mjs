// Two bundles:
//   - the extension host (`src/extension.ts` -> `out/extension.js`), CommonJS,
//     with `vscode` left external (the host provides it at runtime);
//   - the webview panel (`src/webview/chat.ts` -> `media/chat.js`), a
//     dependency-free IIFE the panel's HTML loads from `media/`.
// The webview source is separate from its output on purpose: esbuild cannot
// write a bundle over its own entry without nesting the IIFE on a rebuild.
import { build, context } from "esbuild";

const watch = process.argv.includes("--watch");

/** @type {import('esbuild').BuildOptions} */
const hostOptions = {
  entryPoints: ["src/extension.ts"],
  outfile: "out/extension.js",
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node18",
  sourcemap: true,
  external: ["vscode"],
  logLevel: "info",
};

/** @type {import('esbuild').BuildOptions} */
const webviewOptions = {
  entryPoints: ["src/webview/chat.ts"],
  outfile: "media/chat.js",
  bundle: true,
  platform: "browser",
  format: "iife",
  target: "es2020",
  sourcemap: false,
  logLevel: "info",
};

if (watch) {
  const host = await context(hostOptions);
  const webview = await context(webviewOptions);
  await Promise.all([host.watch(), webview.watch()]);
} else {
  await build(hostOptions);
  await build(webviewOptions);
}

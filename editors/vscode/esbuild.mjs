// Bundle the extension host entry point. `vscode` is provided by the host at
// runtime, so it stays external. Output is CommonJS (`main` in package.json is
// loaded with `require` by the extension host). The webview bundle
// (`media/chat.js`) is a P1b concern.
import { build, context } from "esbuild";

const watch = process.argv.includes("--watch");

/** @type {import('esbuild').BuildOptions} */
const options = {
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

if (watch) {
  const ctx = await context(options);
  await ctx.watch();
} else {
  await build(options);
}

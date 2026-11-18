// Test-only module hooks: lets a node test import the real compositions (.tsx, extensionless
// imports) by resolving "./x" to x.ts / x.tsx and compiling .tsx with the esbuild Remotion ships.
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { transform } from "esbuild";

export async function resolve(specifier, context, next) {
  if ((specifier.startsWith("./") || specifier.startsWith("../")) && context.parentURL?.startsWith("file:")) {
    const base = new URL(specifier, context.parentURL);
    const path = fileURLToPath(base);
    if (!existsSync(path) || !/\.[cm]?[jt]sx?$/.test(path)) {
      for (const ext of [".ts", ".tsx", "/index.ts", "/index.tsx"]) {
        if (existsSync(path + ext)) return { url: new URL(base.href + ext).href, shortCircuit: true };
      }
    }
  }
  return next(specifier, context);
}

export async function load(url, context, next) {
  if (url.startsWith("file:") && /\.tsx?$/.test(url) && !url.includes("/node_modules/")) {
    const source = await readFile(fileURLToPath(url), "utf8");
    const out = await transform(source, { loader: url.endsWith(".tsx") ? "tsx" : "ts", format: "esm", jsx: "automatic", sourcefile: url });
    return { format: "module", source: out.code, shortCircuit: true };
  }
  return next(url, context);
}

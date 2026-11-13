import { defineConfig } from "vite";
import { resolve } from "node:path";
import { assistantPlugin } from "./assistant-server";

// The editor lives beside the Remotion project so it can import the same
// `Timeline` composition that the renderer uses. Public assets come from the
// Remotion public dir so `staticFile()` paths resolve identically in preview and render.
// The assistant plugin adds a local-only endpoint that runs `vimmer edit` (dev server only).
export default defineConfig({
  root: import.meta.dirname,
  publicDir: "../public",
  esbuild: { jsx: "automatic" },
  plugins: [assistantPlugin(resolve(import.meta.dirname, "../.."), resolve(import.meta.dirname, "../public"))],
  server: {
    port: 5180,
    host: "localhost",
    fs: { allow: [".."] },
  },
});

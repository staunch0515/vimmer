import { defineConfig } from "vite";

// The editor lives beside the Remotion project so it can import the same
// `Timeline` composition that the renderer uses. Public assets come from the
// Remotion public dir so `staticFile()` paths resolve identically in preview and render.
export default defineConfig({
  root: import.meta.dirname,
  publicDir: "../public",
  esbuild: { jsx: "automatic" },
  server: {
    port: 5180,
    fs: { allow: [".."] },
  },
});

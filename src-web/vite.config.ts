import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

// Dev server settings are fixed for the Tauri shell (devUrl in
// src-tauri/tauri.conf.json points at this port).
export default defineConfig({
  root: "src",
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "../../dist",
    emptyOutDir: true,
  },
});

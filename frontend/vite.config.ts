import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import path from "path";

const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig({
  plugins: [svelte()],

  resolve: {
    alias: {
      "$lib": path.resolve("./src/lib"),
    },
  },

  // Tauri expects a fixed port
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      ignored: ["**/src-tauri/**", "**/src/tauri_commands/**"],
    },
  },

  // Tauri uses the `dist` directory
  build: {
    outDir: "./dist",
    emptyOutDir: true,
  },

  // Prevent vite from clearing the terminal
  clearScreen: false,

  // Define env variables
  envPrefix: ["VITE_", "TAURI_"],
});

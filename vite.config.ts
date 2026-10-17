/// <reference types="vitest/config" />
import { fileURLToPath, URL } from "node:url";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and leaves the screen clear for its own output.
const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "jsdom",
    globals: true,
    passWithNoTests: true,
    // 5s (Vitest's default) flakes under parallel load (CH12): a jsdom render
    // plus a React Query tick can exceed it on a busy machine.
    testTimeout: 15_000,
    // Never scan tooling/agent dirs or the Rust tree.
    exclude: ["**/node_modules/**", "**/dist/**", "**/src-tauri/**", "**/.hermad/**"],
  },
});

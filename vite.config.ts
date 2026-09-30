/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri's dev window loads this fixed port (tauri.conf.json devUrl), so fail
// rather than silently pick another one.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Never inline assets as data: URLs: the strict CSP (img-src, font-src
  // 'self') blocks them, so they'd silently fail to load.
  build: {
    assetsInlineLimit: 0,
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.test.{ts,tsx}"],
    css: true,
    restoreMocks: true,
    // CI shares one laptop with the lanes' builds (1aC-11), where a cold
    // render can take seconds. Long enough for that, short enough to still
    // catch a hang. Tests never assert elapsed time (see src/test/setup.ts).
    testTimeout: 20_000,
  },
});

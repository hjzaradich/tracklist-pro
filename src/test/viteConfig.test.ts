import { loadConfigFromFile } from "vite";
import { describe, expect, it } from "vitest";

describe("vite build config", () => {
  // The strict CSP (0C-10) allows images and fonts from 'self' only, so an
  // asset inlined as a data: URL would be blocked and silently not load.
  it("never inlines assets as data: URLs", async () => {
    const loaded = await loadConfigFromFile(
      { command: "build", mode: "production" },
      "vite.config.ts",
    );
    expect(loaded?.config.build?.assetsInlineLimit).toBe(0);
  });
});

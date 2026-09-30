import { describe, expect, it } from "vitest";
import { namespaceFromPath } from "./index";
import generated from "./resources.gen.ts?raw";
import { RESOURCE_TYPES_COMMAND, renderResourceTypes } from "./resourceTypes";

// The same files the app loads at runtime (src/i18n/index.ts).
const namespaceFiles = Object.keys(import.meta.glob("../locales/en/*.json"));

describe("generated i18n namespace types", () => {
  it(`resources.gen.ts is up to date with src/locales/en (else run ${RESOURCE_TYPES_COMMAND})`, () => {
    const expected = renderResourceTypes(namespaceFiles.map(namespaceFromPath));
    expect(generated.replace(/\r\n/g, "\n")).toBe(expected);
  });

  it("lists every namespace once, sorted, one line each", () => {
    expect(renderResourceTypes(["theme", "home", "common", "home"])).toContain(
      [
        'export interface EnglishResources {',
        '  "common": typeof import("../locales/en/common.json");',
        '  "home": typeof import("../locales/en/home.json");',
        '  "theme": typeof import("../locales/en/theme.json");',
        "}",
      ].join("\n"),
    );
  });

  it("a new namespace file shows up as a new line (so a stale file is caught)", () => {
    const before = renderResourceTypes(["common", "home"]);
    const after = renderResourceTypes(["common", "crates", "home"]);
    expect(after).not.toBe(before);
    expect(after).toContain('"crates": typeof import("../locales/en/crates.json");');
  });

  it("rejects namespace file names that would clash with i18next's : and . separators", () => {
    expect(() => renderResourceTypes(["crates:list"])).toThrow(/crates:list/);
    expect(() => renderResourceTypes(["crates.list"])).toThrow(/crates\.list/);
  });
});

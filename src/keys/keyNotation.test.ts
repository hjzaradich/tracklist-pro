import { describe, expect, it } from "vitest";
import { KEY_NAMES } from "../bindings";
import { DEFAULT_KEY_NOTATION, formatKey, KEY_NOTATIONS, type KeyNotation } from "./keyNotation";

describe("key notation", () => {
  it("defaults to Camelot", () => {
    expect(DEFAULT_KEY_NOTATION).toBe("camelot");
  });

  it("offers the five notations the roadmap lists, Camelot first", () => {
    expect(KEY_NOTATIONS).toEqual([
      "camelot",
      "musical_standard",
      "musical_rekordbox",
      "musical_sharps",
      "musical_flats",
    ]);
  });

  it("formats a stored Camelot key in each notation", () => {
    expect(formatKey("8A", "camelot")).toBe("8A");
    expect(formatKey("8A", "musical_standard")).toBe("Am");
    expect(formatKey("12A", "musical_standard")).toBe("C#m");
    expect(formatKey("12A", "musical_flats")).toBe("Dbm");
    expect(formatKey("12A", "musical_rekordbox")).toBe("Dbm");
    expect(formatKey("1A", "musical_rekordbox")).toBe("Abm");
    expect(formatKey("5B", "musical_sharps")).toBe("D#");
    expect(formatKey("5B", "musical_flats")).toBe("Eb");
    expect(formatKey("2B", "musical_flats")).toBe("Gb");
    expect(formatKey("12B", "musical_standard")).toBe("E");
  });

  it("uses the backend's name tables for every key, so both sides spell keys the same", () => {
    for (const { notation, names } of KEY_NAMES) {
      expect(names).toHaveLength(24);
      for (let n = 1; n <= 12; n++) {
        expect(formatKey(`${n}A`, notation)).toBe(names[n - 1]);
        expect(formatKey(`${n}B`, notation)).toBe(names[n + 11]);
      }
    }
  });

  it("accepts a leading zero, lowercase and stray spaces in the stored key", () => {
    expect(formatKey("08A", "musical_standard")).toBe("Am");
    expect(formatKey("8a", "camelot")).toBe("8A");
    expect(formatKey(" 11b ", "musical_standard")).toBe("A");
  });

  it("shows nothing for no key or for something that isn't a Camelot key", () => {
    for (const notation of KEY_NOTATIONS) {
      expect(formatKey(null, notation)).toBeNull();
      expect(formatKey(undefined, notation)).toBeNull();
      for (const junk of ["", "0A", "13B", "8C", "Am", "8", "A", "8AB"]) {
        expect(formatKey(junk, notation), junk).toBeNull();
      }
    }
  });

  it("shows Camelot for a notation this build doesn't know", () => {
    expect(formatKey("8A", "open_key" as KeyNotation)).toBe("8A");
  });
});

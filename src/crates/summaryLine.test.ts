import { describe, expect, it } from "vitest";
import type { CrateSummary } from "../bindings";
import i18n from "../i18n";
import { tx } from "../test/tx";
import { MOST_KEYS, summaryParts } from "./summaryLine";

const t = i18n.getFixedT("en", "crates");

const FULL: CrateSummary = {
  notes: null,
  tracks: 12,
  bpm: { min: 122, max: 128.5 },
  energy: { min: 4, max: 8 },
  keys: ["8A", "9A"],
  genres: ["House", "Techno", "Disco"],
};

describe("the parts of a crate's summary line", () => {
  it("gives BPM range, energy range, keys and top genres, in that order", () => {
    expect(summaryParts(FULL, "camelot", t)).toEqual([
      tx("crates:summary.bpm", { min: "122", max: "128.5" }),
      tx("crates:summary.energy", { min: 4, max: 8 }),
      tx("crates:summary.keys", { keys: "8A, 9A", count: 2 }),
      "House, Techno, Disco",
    ]);
  });

  it("leaves out a part with no data and keeps the others", () => {
    const parts = (change: Partial<CrateSummary>) => summaryParts({ ...FULL, ...change }, "camelot", t);
    expect(parts({ bpm: null })).toHaveLength(3);
    expect(parts({ bpm: null })[0]).toBe(tx("crates:summary.energy", { min: 4, max: 8 }));
    expect(parts({ energy: null })).toHaveLength(3);
    expect(parts({ keys: [] })).toHaveLength(3);
    expect(parts({ genres: [] })).toHaveLength(3);
    expect(parts({ bpm: null, energy: null, genres: [] })).toEqual([
      tx("crates:summary.keys", { keys: "8A, 9A", count: 2 }),
    ]);
  });

  it("says plainly when the crate is empty, and when its tracks have none of the four", () => {
    expect(summaryParts({ ...FULL, tracks: 0, bpm: null, energy: null, keys: [], genres: [] }, "camelot", t)).toEqual([
      tx("crates:summary.empty"),
    ]);
    expect(summaryParts({ ...FULL, bpm: null, energy: null, keys: [], genres: [] }, "camelot", t)).toEqual([
      tx("crates:summary.nothing"),
    ]);
  });

  it("writes one value as one value, not a range", () => {
    const parts = summaryParts({ ...FULL, bpm: { min: 124, max: 124 }, energy: { min: 6, max: 6 } }, "camelot", t);
    expect(parts.slice(0, 2)).toEqual([
      tx("crates:summary.bpmOne", { bpm: "124" }),
      tx("crates:summary.energyOne", { energy: 6 }),
    ]);
    expect(summaryParts({ ...FULL, keys: ["8A"] }, "camelot", t)[2]).toBe(
      tx("crates:summary.keys", { keys: "8A", count: 1 }),
    );
  });

  it("names the first five keys and then how many more, in the order given", () => {
    const keys = ["1A", "1B", "2A", "3B", "8A", "9A", "12B"];
    expect(MOST_KEYS).toBe(5);
    expect(summaryParts({ ...FULL, keys }, "camelot", t)[2]).toBe(
      tx("crates:summary.keysMore", { keys: "1A, 1B, 2A, 3B, 8A", more: 2 }),
    );
    // Exactly five need no "more".
    expect(summaryParts({ ...FULL, keys: keys.slice(0, 5) }, "camelot", t)[2]).toBe(
      tx("crates:summary.keys", { keys: "1A, 1B, 2A, 3B, 8A", count: 5 }),
    );
  });

  it("writes the keys in the user's notation but keeps Camelot order", () => {
    expect(summaryParts({ ...FULL, keys: ["8A", "9A", "8B"] }, "musical_standard", t)[2]).toBe(
      tx("crates:summary.keys", { keys: "Am, Em, C", count: 3 }),
    );
  });
});

import { describe, expect, it } from "vitest";
import trackTable from "../locales/en/trackTable.json";
import type { TableFilter } from "../bindings";
import { addFilter, NAMED_FORMATS, removeFilter } from "./filters";

describe("the file formats the filter names", () => {
  it("are exactly the formats the locale has a name for", () => {
    expect([...NAMED_FORMATS].sort()).toEqual(Object.keys(trackTable.format).sort());
  });
});

describe("filters that say the opposite of each other", () => {
  it("replace each other: analyzed by rekordbox and not analyzed by rekordbox, as in a crate and in no crate", () => {
    const analyzed: TableFilter = { kind: "analyzedByRekordbox" };
    const not: TableFilter = { kind: "notAnalyzedByRekordbox" };
    expect(addFilter([analyzed], not)).toEqual([not]);
    expect(addFilter([not], analyzed)).toEqual([analyzed]);
    expect(addFilter([{ kind: "inCrate" }, analyzed], { kind: "inNoCrate" })).toEqual([analyzed, { kind: "inNoCrate" }]);
  });

  it("leave the filters that aren't their opposites alone", () => {
    const low: TableFilter = { kind: "lowQuality" };
    expect(addFilter([low], { kind: "notAnalyzedByRekordbox" })).toEqual([low, { kind: "notAnalyzedByRekordbox" }]);
  });

  it("add a filter once, and take it off whole", () => {
    const low: TableFilter = { kind: "lowQuality" };
    expect(addFilter([low], low)).toEqual([low]);
    expect(removeFilter([low, { kind: "inNoCrate" }], low)).toEqual([{ kind: "inNoCrate" }]);
  });
});

describe("the energy filters", () => {
  it("are one at a time: another level, or no energy, takes the one before off", () => {
    const seven: TableFilter = { kind: "energy", level: 7 };
    const eight: TableFilter = { kind: "energy", level: 8 };
    const none: TableFilter = { kind: "noEnergy" };
    const low: TableFilter = { kind: "lowQuality" };
    expect(addFilter([low, seven], eight)).toEqual([low, eight]);
    expect(addFilter([seven], none)).toEqual([none]);
    expect(addFilter([none, low], seven)).toEqual([low, seven]);
  });

  it("are the same filter only at the same level", () => {
    const seven: TableFilter = { kind: "energy", level: 7 };
    expect(addFilter([seven], { kind: "energy", level: 7 })).toEqual([seven]);
    expect(removeFilter([seven], { kind: "energy", level: 8 })).toEqual([seven]);
    expect(removeFilter([seven], { kind: "energy", level: 7 })).toEqual([]);
  });
});

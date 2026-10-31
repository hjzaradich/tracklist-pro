import { describe, expect, it } from "vitest";
import type { TableFilter } from "../bindings";
import { doorWord, filterOfDoor, validateLibrarySearch } from "./doors";

describe("the words a door is written as in the Library's address", () => {
  const FILTERS: TableFilter[] = [
    { kind: "lowQuality" },
    { kind: "analyzedByRekordbox" },
    { kind: "notAnalyzedByRekordbox" },
    { kind: "inCrate" },
    { kind: "inNoCrate" },
    { kind: "noEnergy" },
    { kind: "energy", level: 1 },
    { kind: "energy", level: 10 },
  ];

  it.each(FILTERS)("stand for the filter they were made from: %j", (filter) => {
    const word = doorWord(filter);
    expect(word).not.toBeNull();
    expect(filterOfDoor(word)).toEqual(filter);
    expect(validateLibrarySearch({ filter: word })).toEqual({ filter: word });
  });

  it("have none for a filter no Overview number opens", () => {
    expect(doorWord({ kind: "missing" })).toBeNull();
    expect(doorWord({ kind: "format", format: "flac" })).toBeNull();
    expect(doorWord({ kind: "qualityProblem" })).toBeNull();
  });

  it.each(["", "nonsense", "energy-0", "energy-11", "energy-1.5", "energy-", "energy-x", "missing", 7, null, undefined])(
    "mean no filter, and no error, when the word is %j",
    (word) => {
      expect(filterOfDoor(word)).toBeNull();
      expect(validateLibrarySearch({ filter: word })).toEqual({});
    },
  );
});

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
    { kind: "missingTags" },
    { kind: "missingArt" },
    { kind: "energy", level: 1 },
    { kind: "energy", level: 10 },
  ];

  it.each(FILTERS)("stand for the filter they were made from: %j", (filter) => {
    const word = doorWord(filter);
    expect(word).not.toBeNull();
    expect(filterOfDoor(word)).toEqual(filter);
    expect(validateLibrarySearch({ filter: word })).toEqual({ filter: word });
  });

  it.each([
    "House",
    "Drum & Bass",
    "R&B / Soul",
    "100% Pure",
    "a+b",
    "#1 Hits",
    "Rock?filter=lowQuality",
    "genre-nested",
    "%",
    "%41",
    "Caf\u00e9",
    "\u30c6\u30af\u30ce",
    "\u{1f3b5} Mix",
    "  spaced  ",
    "x'; DROP TABLE track_row; --",
  ])("carry a genre's name as it is, whatever it holds: %j", (name) => {
    const filter: TableFilter = { kind: "genre", name };
    const word = doorWord(filter);
    expect(word).not.toBeNull();
    // Only characters that mean nothing in an address.
    expect(word).toMatch(/^genre-[A-Za-z0-9\-_.!~*'()%]+$/);
    expect(filterOfDoor(word)).toEqual(filter);
    expect(validateLibrarySearch({ filter: word })).toEqual({ filter: word });
  });

  it("have none for a filter no Overview number opens", () => {
    expect(doorWord({ kind: "missing" })).toBeNull();
    expect(doorWord({ kind: "format", format: "flac" })).toBeNull();
    expect(doorWord({ kind: "qualityProblem" })).toBeNull();
    expect(doorWord({ kind: "genre", name: "" })).toBeNull();
    expect(doorWord({ kind: "genre", name: "   " })).toBeNull();
  });

  it.each([
    "",
    "nonsense",
    "energy-0",
    "energy-11",
    "energy-1.5",
    "energy-",
    "energy-x",
    "missing",
    "genre-",
    "genre-%20%20",
    "genre-%E0%A4%A",
    "genre",
    "Genre-House",
    7,
    null,
    undefined,
  ])(
    "mean no filter, and no error, when the word is %j",
    (word) => {
      expect(filterOfDoor(word)).toBeNull();
      expect(validateLibrarySearch({ filter: word })).toEqual({});
    },
  );
});

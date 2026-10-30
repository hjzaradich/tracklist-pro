import { describe, expect, it } from "vitest";
import type { Crate } from "../bindings";
import { crateChoices, folderChoices, pathFinder, pathOf } from "./tree";

function entry(id: number, name: string, parentId: number | null, kind: Crate["kind"]): Crate {
  return { id, name, parentId, kind, trackCount: 0 };
}

/** Friday residency > (Warmup, Late > Peak time); Loose at the top. */
const LIST = [
  entry(1, "Friday residency", null, "folder"),
  entry(2, "Warmup", 1, "static"),
  entry(3, "Late", 1, "folder"),
  entry(4, "Peak time", 3, "static"),
  entry(5, "Loose", null, "static"),
];

describe("crate paths", () => {
  it("names a crate by the folders above it, from the top down", () => {
    expect(pathOf(LIST, 4)).toEqual(["Friday residency", "Late", "Peak time"]);
    expect(pathOf(LIST, 5)).toEqual(["Loose"]);
  });

  it("gives no names for a crate that isn't in the list", () => {
    expect(pathOf(LIST, 99)).toEqual([]);
  });

  it("stops at a loop of folders instead of walking it forever", () => {
    const loop = [entry(1, "A", 2, "folder"), entry(2, "B", 1, "folder")];
    expect(pathOf(loop, 1)).toEqual(["B", "A"]);
  });

  it("finds every path from one lookup", () => {
    const pathTo = pathFinder(LIST);
    expect(pathTo(2)).toEqual(["Friday residency", "Warmup"]);
    expect(pathTo(4)).toEqual(["Friday residency", "Late", "Peak time"]);
  });
});

describe("the crates to choose from", () => {
  it("lists hand-made crates in tree order, each named by where it is", () => {
    expect(crateChoices(LIST).map(({ crate, label }) => [crate.id, label])).toEqual([
      [2, "Friday residency › Warmup"],
      [4, "Friday residency › Late › Peak time"],
      [5, "Loose"],
    ]);
  });

  it("lists folders to move into, leaving out the ones named", () => {
    expect(folderChoices(LIST, new Set([3])).map(({ id, label }) => [id, label])).toEqual([
      [1, "Friday residency"],
    ]);
  });
});

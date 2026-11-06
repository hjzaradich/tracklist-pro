import { describe, expect, it } from "vitest";
import type { GenreNode } from "../bindings";
import { rows, withDescendants } from "./tree";

const node = (
  id: number,
  parentId: number | null,
  name: string,
): GenreNode => ({
  id,
  parentId,
  name,
  tracks: 0,
});

const NODES = [
  node(1, null, "A"),
  node(2, null, "B"),
  node(3, 1, "A1"),
  node(4, 3, "A1a"),
];

describe("rows", () => {
  it("lists a branch, then what is below it, with depths", () => {
    expect(rows(NODES).map((r) => [r.node.name, r.depth])).toEqual([
      ["A", 0],
      ["A1", 1],
      ["A1a", 2],
      ["B", 0],
    ]);
  });

  it("shows a node whose parent is missing at the top", () => {
    expect(rows([node(5, 99, "Lost")]).map((r) => r.depth)).toEqual([0]);
  });
});

describe("withDescendants", () => {
  it("holds the node and all below it", () => {
    expect([...withDescendants(NODES, 1)].sort()).toEqual([1, 3, 4]);
    expect([...withDescendants(NODES, 2)]).toEqual([2]);
  });
});

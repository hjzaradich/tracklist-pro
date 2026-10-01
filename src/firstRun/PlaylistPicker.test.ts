import { describe, expect, it } from "vitest";
import type { PlaylistChoice } from "../bindings";
import { treeRows } from "./PlaylistPicker";

function playlist(...path: string[]): PlaylistChoice {
  return { path, tracks: 1 };
}

/** Each row as indentation plus its name; folders end in a slash. */
function drawn(playlists: PlaylistChoice[]): string[] {
  return treeRows(playlists).map((row) => {
    const indent = "  ".repeat(row.depth);
    return row.kind === "folder" ? `${indent}${row.name}/` : `${indent}${row.playlist.path.at(-1)}`;
  });
}

describe("the playlist tree's rows", () => {
  it("name each folder once, above the playlists it holds", () => {
    expect(
      drawn([
        playlist("Loose"),
        playlist("Sets", "2025", "Closing"),
        playlist("Sets", "2025", "Warmup"),
        playlist("Sets", "2026", "Warmup"),
        playlist("Sets", "Favourites"),
        playlist("Zed"),
      ]),
    ).toEqual([
      "Loose",
      "Sets/",
      "  2025/",
      "    Closing",
      "    Warmup",
      "  2026/",
      "    Warmup",
      "  Favourites",
      "Zed",
    ]);
  });

  it("give two playlists with one name in different folders their own rows", () => {
    const rows = treeRows([playlist("A", "Warmup"), playlist("B", "Warmup")]);
    expect(new Set(rows.map((row) => row.key)).size).toBe(rows.length);
  });

  it("are empty for no playlists", () => {
    expect(treeRows([])).toEqual([]);
  });
});

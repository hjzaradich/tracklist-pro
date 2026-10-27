import { describe, expect, it } from "vitest";
import {
  ALL_COLUMNS,
  hiddenColumns,
  hideColumn,
  isDefault,
  moveColumn,
  resolveColumns,
  showColumn,
  storedFrom,
} from "./columnChoice";
import { COLUMNS, DEFAULT_COLUMNS, type ColumnId } from "./columns";

const LIBRARY: ColumnId[] = ["title", "artist", "bpm", "key", "time", "genre", "rating", "dateAdded"];

describe("the columns a screen shows", () => {
  it("lists every column there is, once", () => {
    expect([...ALL_COLUMNS].sort()).toEqual(Object.keys(COLUMNS).sort());
  });

  it("are the screen's own defaults until the user chooses (the owner's lists)", () => {
    for (const none of [null, undefined]) {
      expect(resolveColumns("library", none)).toEqual(LIBRARY);
      expect(resolveColumns("crate", none)).toEqual(LIBRARY);
      expect(resolveColumns("allMusic", none)).toEqual([...LIBRARY, "file", "folder"]);
    }
    expect(DEFAULT_COLUMNS.crate).toEqual(DEFAULT_COLUMNS.library);
  });

  it("are the user's choice, in the order chosen", () => {
    const chosen = storedFrom(["title", "bpm", "artist"]);
    expect(resolveColumns("library", chosen)).toEqual(["title", "bpm", "artist"]);
  });

  it("ignore an id that is not a column, instead of failing", () => {
    const stored = {
      shown: ["title", "tempo_curve", "bpm", "toString", "__proto__"],
      known: [...ALL_COLUMNS, "tempo_curve"],
    };
    expect(resolveColumns("library", stored)).toEqual(["title", "bpm"]);
  });

  it("show a column once however often it was stored", () => {
    const stored = { shown: ["title", "bpm", "title", "bpm"], known: [...ALL_COLUMNS] };
    expect(resolveColumns("library", stored)).toEqual(["title", "bpm"]);
  });

  it("keep a column the user hid hidden, even when it is a default column", () => {
    // The choice was saved by a version that knew every column.
    const stored = storedFrom(["title", "artist"]);
    expect(resolveColumns("library", stored)).toEqual(["title", "artist"]);
  });

  it("show a default column added later, next to the default column before it", () => {
    // Saved before "genre" and "rating" existed: neither is known to the choice.
    const known = ALL_COLUMNS.filter((id) => id !== "genre" && id !== "rating");
    const stored = { shown: ["title", "artist", "time", "dateAdded"], known };
    expect(resolveColumns("library", stored)).toEqual(["title", "artist", "time", "genre", "rating", "dateAdded"]);
  });

  it("show a default column added later before the next default column shown when none before it is, else at the end", () => {
    const stored = { shown: ["time"], known: ALL_COLUMNS.filter((id) => id !== "title" && id !== "artist") };
    expect(resolveColumns("library", stored)).toEqual(["title", "artist", "time"]);
  });

  it("hide a column added later that is not a default column of the screen", () => {
    // The Library never shows file or folder by default; a version that adds them leaves them hidden.
    const known = ALL_COLUMNS.filter((id) => id !== "file" && id !== "folder");
    const stored = { shown: [...known], known };
    expect(resolveColumns("library", stored)).toEqual(LIBRARY);
    // On All music they are default columns, so they appear.
    expect(resolveColumns("allMusic", stored)).toEqual([...LIBRARY, "file", "folder"]);
  });

  it("always include the title, first when it was dropped", () => {
    expect(resolveColumns("library", storedFrom(["bpm"]))).toEqual(["title", "bpm"]);
    expect(resolveColumns("library", { shown: [], known: [...ALL_COLUMNS] })).toEqual(["title"]);
  });
});

describe("changing the columns", () => {
  it("moves a column one place and stops at either end", () => {
    expect(moveColumn(["title", "artist", "bpm"], "bpm", -1)).toEqual(["title", "bpm", "artist"]);
    expect(moveColumn(["title", "artist", "bpm"], "title", 1)).toEqual(["artist", "title", "bpm"]);
    expect(moveColumn(["title", "artist"], "title", -1)).toEqual(["title", "artist"]);
    expect(moveColumn(["title", "artist"], "artist", 1)).toEqual(["title", "artist"]);
    expect(moveColumn(["title"], "bpm", 1)).toEqual(["title"]);
  });

  it("hides any column but the title", () => {
    expect(hideColumn(["title", "artist", "bpm"], "artist")).toEqual(["title", "bpm"]);
    expect(hideColumn(["title", "artist"], "title")).toEqual(["title", "artist"]);
  });

  it("shows a hidden column at the end, once", () => {
    expect(showColumn(["title"], "file")).toEqual(["title", "file"]);
    expect(showColumn(["title", "file"], "file")).toEqual(["title", "file"]);
  });

  it("lists the hidden columns in the editor's order", () => {
    expect(hiddenColumns(LIBRARY)).toEqual(["file", "folder"]);
    expect(hiddenColumns(["title"])).toEqual(ALL_COLUMNS.slice(1));
  });

  it("knows whether the columns are the screen's default", () => {
    expect(isDefault("library", resolveColumns("library", null))).toBe(true);
    expect(isDefault("library", [...DEFAULT_COLUMNS.library].reverse())).toBe(false);
    expect(isDefault("library", ["title"])).toBe(false);
    expect(isDefault("allMusic", DEFAULT_COLUMNS.library)).toBe(false);
  });

  it("stores the columns shown and every column there is, so a later column can be told from a hidden one", () => {
    expect(storedFrom(["title", "bpm"])).toEqual({ shown: ["title", "bpm"], known: [...ALL_COLUMNS] });
  });
});

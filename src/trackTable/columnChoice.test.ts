import { describe, expect, it } from "vitest";
import {
  ALL_COLUMNS,
  WIDEST,
  clampWidth,
  hiddenColumns,
  hideColumn,
  isDefault,
  moveColumn,
  moveColumnTo,
  resolveColumns,
  resolveWidths,
  showColumn,
  storedFrom,
  tableLayout,
} from "./columnChoice";
import { COLUMNS, DEFAULT_COLUMNS, type ColumnId } from "./columns";

// What every screen shows at first (owner decision, 2026-10-30), and the eight a user had before it.
const FIRST: ColumnId[] = ["title", "artist", "bpm", "key", "time", "genre"];
const EIGHT: ColumnId[] = [...FIRST, "rating", "dateAdded"];

describe("the columns a screen shows", () => {
  it("lists every column there is, once", () => {
    // The Fit column is not one to choose: it shows while a crate is sorted by fit.
    expect([...ALL_COLUMNS].sort()).toEqual(Object.keys(COLUMNS).filter((id) => id !== "fit").sort());
  });

  it("are the screen's own defaults until the user chooses (the owner's lists)", () => {
    for (const none of [null, undefined]) {
      expect(resolveColumns("library", none)).toEqual(FIRST);
      expect(resolveColumns("crate", none)).toEqual(FIRST);
      expect(resolveColumns("allMusic", none)).toEqual(FIRST);
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
    // Saved before "genre" existed: it is not known to the choice.
    const known = ALL_COLUMNS.filter((id) => id !== "genre");
    const stored = { shown: ["title", "artist", "time", "dateAdded"], known };
    expect(resolveColumns("library", stored)).toEqual(["title", "artist", "time", "genre", "dateAdded"]);
  });

  it("show a default column added later before the next default column shown when none before it is, else at the end", () => {
    const stored = { shown: ["time"], known: ALL_COLUMNS.filter((id) => id !== "title" && id !== "artist") };
    expect(resolveColumns("library", stored)).toEqual(["title", "artist", "time"]);
  });

  it("hide a column added later that is not a default column of the screen", () => {
    // No screen shows file, folder or energy by default; a version that adds them leaves them hidden.
    const known = ALL_COLUMNS.filter((id) => id !== "file" && id !== "folder" && id !== "energy");
    const stored = { shown: [...known], known };
    expect(resolveColumns("library", stored)).toEqual(EIGHT);
    expect(resolveColumns("allMusic", stored)).toEqual(EIGHT);
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
    expect(hiddenColumns(EIGHT)).toEqual(["energy", "file", "folder"]);
    expect(hiddenColumns(["title"])).toEqual(ALL_COLUMNS.slice(1));
  });

  it("knows whether the columns are the screen's default", () => {
    expect(isDefault("library", resolveColumns("library", null))).toBe(true);
    expect(isDefault("library", [...DEFAULT_COLUMNS.library].reverse())).toBe(false);
    expect(isDefault("library", ["title"])).toBe(false);
    expect(isDefault("allMusic", ["title", "artist"])).toBe(false);
  });

  it("stores the columns shown and every column there is, so a later column can be told from a hidden one", () => {
    expect(storedFrom(["title", "bpm"])).toEqual({ shown: ["title", "bpm"], known: [...ALL_COLUMNS], widths: {} });
  });
});

describe("moving a column to a place", () => {
  const shown: ColumnId[] = ["title", "artist", "bpm", "key"];

  it("puts it at that place among the others", () => {
    expect(moveColumnTo(shown, "bpm", 1)).toEqual(["title", "bpm", "artist", "key"]);
    expect(moveColumnTo(shown, "title", 3)).toEqual(["artist", "bpm", "key", "title"]);
    expect(moveColumnTo(shown, "key", 0)).toEqual(["key", "title", "artist", "bpm"]);
  });

  it("leaves the order alone when it is dropped where it is, or is not shown", () => {
    expect(moveColumnTo(shown, "artist", 1)).toEqual(shown);
    expect(moveColumnTo(shown, "genre", 0)).toEqual(shown);
  });

  it("keeps a place that is out of range inside the list", () => {
    expect(moveColumnTo(shown, "title", 99)).toEqual(["artist", "bpm", "key", "title"]);
    expect(moveColumnTo(shown, "key", -4)).toEqual(["key", "title", "artist", "bpm"]);
  });
});

describe("the widths of the columns", () => {
  it("are kept with the columns when a choice is stored", () => {
    expect(storedFrom(["title", "bpm"], { bpm: 120 })).toEqual({
      shown: ["title", "bpm"],
      known: [...ALL_COLUMNS],
      widths: { bpm: 120 },
    });
  });

  it("are none for a choice saved before widths existed, or no choice", () => {
    expect(resolveWidths({ shown: ["title"], known: [...ALL_COLUMNS] })).toEqual({});
    expect(resolveWidths(null)).toEqual({});
    expect(resolveWidths(undefined)).toEqual({});
  });

  it("never go below a column's minimum or above the widest", () => {
    for (const id of ALL_COLUMNS) {
      expect(clampWidth(id, 1)).toBe(COLUMNS[id].min);
      expect(clampWidth(id, 1_000_000)).toBe(WIDEST);
      expect(COLUMNS[id].min).toBeLessThanOrEqual(COLUMNS[id].width);
    }
    expect(clampWidth("title", 200.4)).toBe(200);
  });

  it("drop an id that is not a column and a width that is not a number", () => {
    const stored = {
      shown: ["title"],
      known: [...ALL_COLUMNS],
      widths: { title: 300, tempo_curve: 80, toString: 90, bpm: Number.NaN },
    };
    expect(resolveWidths(stored)).toEqual({ title: 300 });
  });

  it("make the grid: a dragged column is exactly that wide, the others start where they did", () => {
    expect(tableLayout(["title", "artist", "bpm"], { artist: 200 })).toEqual({
      template: "minmax(120px, 3fr) 200px 48px",
      minWidth: 120 + 200 + 48,
    });
    expect(tableLayout(["bpm", "genre"], {}, 100)).toEqual({
      template: "48px minmax(52px, 1fr) 100px",
      minWidth: 48 + 52 + 100,
    });
  });

  it("leave the default layouts as the Designer fixed them", () => {
    const total = (screen: keyof typeof DEFAULT_COLUMNS, actions: number) =>
      tableLayout(DEFAULT_COLUMNS[screen], {}, actions).minWidth;
    expect(total("library", 232)).toBe(620);
    expect(total("allMusic", 220)).toBe(608);
    expect(total("crate", 96)).toBe(484);
  });
});

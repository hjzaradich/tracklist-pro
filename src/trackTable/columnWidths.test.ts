import { describe, expect, it } from "vitest";
import { COLUMNS, DEFAULT_COLUMNS } from "./columns";

// The room a table has at a 1270px window with the sidebar open, less the
// stage's gutters and a vertical scrollbar (a crate's table also has the
// crate list, 14rem, beside it). The actions widths are what each
// screen passes to its table (LibraryScreen, AllMusicScreen, CratesScreen).
const ROOM = { library: 1000, allMusic: 1000, crate: 760 };
const ACTIONS = { library: 232, allMusic: 220, crate: 120 };

describe("the default columns", () => {
  for (const screen of ["library", "allMusic", "crate"] as const) {
    it(`fit the ${screen} table at a 1270px window without scrolling sideways`, () => {
      const columns = DEFAULT_COLUMNS[screen].reduce((sum, id) => sum + COLUMNS[id].width, 0);
      expect(columns + ACTIONS[screen]).toBeLessThanOrEqual(ROOM[screen]);
    });
  }

  it("give Title and Artist the most room, and the numbers the least", () => {
    const widest = Math.max(...Object.values(COLUMNS).map((c) => c.width));
    expect(COLUMNS.title.width).toBe(widest);
    expect(COLUMNS.artist.width).toBeGreaterThan(COLUMNS.genre.width);
    for (const id of ["bpm", "key", "time"] as const) {
      expect(COLUMNS[id].width).toBeLessThan(COLUMNS.artist.width);
    }
  });

  it("let Title and Artist take the room that is left", () => {
    expect(COLUMNS.title.grow).toBeGreaterThan(COLUMNS.artist.grow ?? 0);
    expect(COLUMNS.artist.grow).toBeGreaterThan(COLUMNS.genre.grow ?? 0);
  });
});

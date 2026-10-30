import { describe, expect, it } from "vitest";
import { COLUMNS, DEFAULT_COLUMNS } from "./columns";

// The room a table has at a 1270px window. Each part is a fixed size of the
// shell or the screen (AppShell.module.css, StageScreen.module.css,
// TrackTable.module.css, CratesScreen.module.css), so a change to one of them
// shows up here.
const WINDOW = 1270;
const SIDEBAR = 216;
const DETAILS_PANEL = 288;
const STAGE_GUTTERS = 2 * 16;
// The table's border (2px) and a vertical scrollbar (15px).
const TABLE_FRAME = 2 + 15;
// A crate's table also has the crate list beside it (13rem) and a gap (16px).
const CRATE_LIST = 13 * 16 + 16;

const TABLE = WINDOW - SIDEBAR - DETAILS_PANEL - STAGE_GUTTERS - TABLE_FRAME;
const ROOM = { library: TABLE, allMusic: TABLE, crate: TABLE - CRATE_LIST };
// What each screen passes to its table as `actionsWidth` (LibraryScreen,
// AllMusicScreen, CratesScreen).
const ACTIONS = { library: 232, allMusic: 220, crate: 96 };

describe("the default columns", () => {
  it("are the same six on every screen", () => {
    const six = ["title", "artist", "bpm", "key", "time", "genre"];
    for (const screen of ["library", "allMusic", "crate"] as const) {
      expect(DEFAULT_COLUMNS[screen]).toEqual(six);
    }
  });

  for (const screen of ["library", "allMusic", "crate"] as const) {
    it(`fit the ${screen} table beside the Details panel at a 1270px window without scrolling sideways`, () => {
      const columns = DEFAULT_COLUMNS[screen].reduce((sum, id) => sum + COLUMNS[id].width, 0);
      expect(columns + ACTIONS[screen]).toBeLessThanOrEqual(ROOM[screen]);
    });
  }

  it("give Title and Artist the most room, and the numbers the least", () => {
    const widest = Math.max(
      ...DEFAULT_COLUMNS.library.map((id) => COLUMNS[id].width),
    );
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

  it("leave the columns that are left out as wide as before, for a user who shows them", () => {
    expect(COLUMNS.rating.width).toBe(76);
    expect(COLUMNS.dateAdded.width).toBe(90);
  });
});

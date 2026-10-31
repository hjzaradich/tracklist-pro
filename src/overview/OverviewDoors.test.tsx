import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { Overview, TableFilter, TableQuery } from "../bindings";
import { fakeBackend } from "../test/overviewBackend";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";

// "Every number is a door" (1cC-4): a number on the Overview opens the Library
// with a chip that says why, and the Library lists as many tracks as the
// number says. (That the backend's rows really are the number's is proven in
// Rust, on a Library of edge cases: track_table/tests/doors.rs.)

afterEach(() => {
  cleanup();
  clearMocks();
});

/** Matches text that contains `text`, whatever characters it holds. */
function contains(text: string): RegExp {
  return new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, String.raw`\$&`));
}

const LIBRARY_TRACKS = 40;

/**
 * A Library whose table answers with `rows` tracks while `chip` is on and with
 * every track when no filter is: what the real table does for a number that
 * counts exactly those tracks. The queries it was asked are kept.
 */
function libraryWith(overview: Partial<Overview>, chip: TableFilter | null, rows: number) {
  const asked: TableQuery[] = [];
  const backend = fakeBackend({
    library: LIBRARY_TRACKS,
    overview,
    tablePage: (query) => {
      asked.push(query);
      const on = query.filters.length === 1 && chip !== null && query.filters[0]?.kind === chip.kind;
      const count = query.filters.length === 0 ? LIBRARY_TRACKS : on ? rows : 0;
      return {
        total: query.withTotal ? count : null,
        offset: query.offset,
        rows: Array.from({ length: Math.max(0, Math.min(count - query.offset, query.limit, 5)) }, (_, i) =>
          tableRow(i + 1),
        ),
        behind: false,
      };
    },
  });
  return { asked, ...backend };
}

const chipsList = () => screen.findByRole("list", { name: tx("trackTable:filters.label") });
const lastAsked = (asked: TableQuery[]) => asked[asked.length - 1];

interface Door {
  name: string;
  overview: Partial<Overview>;
  /** How to find the door on the Overview. */
  open: () => Promise<HTMLElement>;
  chip: TableFilter;
  label: string;
  /** The number the door was opened from. */
  number: number;
  word: string;
}

const todoLine = (key: string, count: number) => () =>
  screen.findByRole("link", { name: contains(tx(`overview:todo.${key}` as "overview:todo.lowQuality", { count })) });

const DOORS: Door[] = [
  {
    name: "lowQuality todo line",
    overview: { lowQuality: 6 },
    open: todoLine("lowQuality", 6),
    chip: { kind: "lowQuality" },
    label: tx("trackTable:filters.lowQuality"),
    number: 6,
    word: "lowQuality",
  },
  {
    name: "missingTags todo line",
    overview: { missingTags: 7 },
    open: todoLine("missingTags", 7),
    chip: { kind: "missingTags" },
    label: tx("trackTable:filters.missingTags"),
    number: 7,
    word: "missingTags",
  },
  {
    name: "missingArt todo line",
    overview: { missingArt: 8 },
    open: todoLine("missingArt", 8),
    chip: { kind: "missingArt" },
    label: tx("trackTable:filters.missingArt"),
    number: 8,
    word: "missingArt",
  },
  {
    name: "notAnalyzed todo line",
    overview: { notAnalyzed: 9 },
    open: todoLine("notAnalyzed", 9),
    chip: { kind: "notAnalyzedByRekordbox" },
    label: tx("trackTable:filters.notAnalyzedByRekordbox"),
    number: 9,
    word: "notAnalyzedByRekordbox",
  },
  {
    name: "inNoCrate todo line",
    overview: { inNoCrate: 1 },
    open: todoLine("inNoCrate", 1),
    chip: { kind: "inNoCrate" },
    label: tx("trackTable:filters.inNoCrate"),
    number: 1,
    word: "inNoCrate",
  },
  {
    name: "inCrates number",
    overview: { inCrates: 3 },
    open: async () => screen.findByRole("link", { name: contains(tx("overview:numbers.inCrates")) }),
    chip: { kind: "inCrate" },
    label: tx("trackTable:filters.inCrate"),
    number: 3,
    word: "inCrate",
  },
  {
    name: "analyzed number",
    overview: { analyzedByRekordbox: 4 },
    open: async () => screen.findByRole("link", { name: contains(tx("overview:numbers.analyzed")) }),
    chip: { kind: "analyzedByRekordbox" },
    label: tx("trackTable:filters.analyzedByRekordbox"),
    number: 4,
    word: "analyzedByRekordbox",
  },
  {
    name: "energy 7 bar",
    overview: { energy: [0, 0, 0, 0, 0, 0, 5, 0, 0, 0], noEnergy: 2 },
    open: async () => {
      return screen.findByRole("link", { name: tx("overview:charts.energy.door", { level: 7, count: 5 }) });
    },
    chip: { kind: "energy", level: 7 },
    label: tx("trackTable:filters.energy", { level: 7 }),
    number: 5,
    word: "energy-7",
  },
  {
    name: "genre bar",
    overview: { genres: [{ name: "House", tracks: 5 }], noGenre: 2 },
    open: async () =>
      screen.findByRole("link", { name: tx("overview:charts.genres.door", { genre: "House", count: 5 }) }),
    chip: { kind: "genre", name: "House" },
    label: tx("trackTable:filters.genre", { genre: "House" }),
    number: 5,
    word: "genre-House",
  },
  {
    name: "no energy bar",
    overview: { energy: [0, 0, 0, 0, 0, 0, 5, 0, 0, 0], noEnergy: 2 },
    open: async () => screen.findByRole("link", { name: tx("overview:charts.energy.noneDoor", { count: 2 }) }),
    chip: { kind: "noEnergy" },
    label: tx("trackTable:filters.noEnergy"),
    number: 2,
    word: "noEnergy",
  },
];

describe("a number that is a door", () => {
  it.each(DOORS)(
    "opens the Library with a chip, and as many tracks as the number says: $name",
    async (door) => {
      const { asked } = libraryWith(door.overview, door.chip, door.number);
      const { router } = renderApp("/overview");
      await userEvent.click(await door.open());

      await waitFor(() => expect(router.state.location.pathname).toBe("/library"));
      expect(router.state.location.search).toEqual({ filter: door.word });
      // The chip says why the Library shows fewer tracks than it holds.
      const chips = await chipsList();
      expect(within(chips).getByText(door.label)).toBeInTheDocument();
      expect(within(chips).getAllByRole("listitem")).toHaveLength(1);
      // The tracks listed are the number's: the very first window is asked with the chip on.
      expect(await screen.findByText(tx("library:trackCount", { count: door.number }))).toBeInTheDocument();
      const first = asked.find((query) => query.filters.length > 0);
      expect(first?.filters).toEqual([door.chip]);
      expect(first?.source).toEqual({ kind: "library" });
    },
  );

  it.each(DOORS)("lets the chip go with its ×, and the Library has every track again: $name", async (door) => {
    const { asked } = libraryWith(door.overview, door.chip, door.number);
    const { router } = renderApp("/overview");
    await userEvent.click(await door.open());
    await chipsList();
    await userEvent.click(
      await screen.findByRole("button", { name: tx("trackTable:filters.remove", { filter: door.label }) }),
    );
    await waitFor(() => expect(lastAsked(asked)?.filters).toEqual([]));
    expect(await screen.findByText(tx("library:trackCount", { count: LIBRARY_TRACKS }))).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
    expect(router.state.location.pathname).toBe("/library");
  });

  it("opens the whole Library, with no chip, from the number of tracks", async () => {
    const { asked } = libraryWith({}, null, 0);
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: contains(tx("overview:numbers.tracks", { count: 3 })) }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/library"));
    expect(router.state.location.search).toEqual({});
    expect(await screen.findByText(tx("library:trackCount", { count: LIBRARY_TRACKS }))).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
    expect(lastAsked(asked)?.filters).toEqual([]);
  });

  it("opens the chip's rows from its address, so back and forward come back to it", async () => {
    const { asked } = libraryWith({}, { kind: "lowQuality" }, 6);
    renderApp("/library?filter=lowQuality");
    const chips = await chipsList();
    expect(within(chips).getByText(tx("trackTable:filters.lowQuality"))).toBeInTheDocument();
    expect(await screen.findByText(tx("library:trackCount", { count: 6 }))).toBeInTheDocument();
    expect(asked[0]?.filters).toEqual([{ kind: "lowQuality" }]);
  });

  it("opens the Library with no chip for a word that stands for no filter", async () => {
    const { asked } = libraryWith({}, null, 0);
    renderApp("/library?filter=nonsense");
    expect(await screen.findByText(tx("library:trackCount", { count: LIBRARY_TRACKS }))).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
    expect(asked[0]?.filters).toEqual([]);
  });

  it("shows another door's chip when the Library is opened through another door", async () => {
    const { asked } = libraryWith({}, { kind: "lowQuality" }, 6);
    const { router } = renderApp("/library?filter=lowQuality");
    await chipsList();
    await router.navigate({ to: "/library", search: { filter: "inNoCrate" } });
    await waitFor(() =>
      expect(within(screen.getByRole("list", { name: tx("trackTable:filters.label") })).getByText(tx("trackTable:filters.inNoCrate"))).toBeInTheDocument(),
    );
    expect(lastAsked(asked)?.filters).toEqual([{ kind: "inNoCrate" }]);
  });

  it("leaves a plain visit to the Library without a chip, though a door opened it before", async () => {
    libraryWith({}, { kind: "lowQuality" }, 6);
    const { router } = renderApp("/library?filter=lowQuality");
    await chipsList();
    await router.navigate({ to: "/library" });
    await waitFor(() => expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull());
  });
});

describe("the energy bars as doors", () => {
  it("name what they open, and an empty bar is no door", async () => {
    fakeBackend({ overview: { energy: [0, 0, 0, 0, 0, 0, 1, 0, 0, 0], noEnergy: 0 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
    expect(
      within(region)
        .getAllByRole("link")
        .map((link) => link.getAttribute("aria-label")),
    ).toEqual([tx("overview:charts.energy.door", { level: 7, count: 1 })]);
    expect(within(region).getAllByRole("listitem")).toHaveLength(10);
  });
});

describe("the genre bars as doors", () => {
  /** Names a DJ's tags really hold: free text, so the address has to carry any of them. */
  const AWKWARD = [
    "Drum & Bass",
    "R&B / Soul",
    "100% Pure",
    "50/50",
    "a+b",
    "#1 Hits",
    "Rock?filter=lowQuality",
    "Caf\u00e9",
    "\u30c6\u30af\u30ce",
    "\u{1f3b5} Mix",
    "Hip  Hop",
    "%20",
    "x'; DROP TABLE track_row; --",
  ];

  it.each(AWKWARD)("carry the name %j through the address to the chip and the query, as it is", async (name) => {
    const { asked } = libraryWith({ genres: [{ name, tracks: 3 }], noGenre: 0 }, { kind: "genre", name }, 3);
    const { router } = renderApp("/overview");
    await userEvent.click(
      await screen.findByRole("link", { name: tx("overview:charts.genres.door", { genre: name, count: 3 }) }),
    );
    await waitFor(() => expect(router.state.location.pathname).toBe("/library"));
    // The address names the genre, and only the genre.
    expect(Object.keys(router.state.location.search)).toEqual(["filter"]);
    const chips = await chipsList();
    expect(within(chips).getByText(tx("trackTable:filters.genre", { genre: name }))).toBeInTheDocument();
    expect(within(chips).getAllByRole("listitem")).toHaveLength(1);
    expect(await screen.findByText(tx("library:trackCount", { count: 3 }))).toBeInTheDocument();
    expect(asked.find((query) => query.filters.length > 0)?.filters).toEqual([{ kind: "genre", name }]);
  });

  it("open the Library from an address typed or shared, whatever the name holds", async () => {
    const name = "R&B / Soul 100%";
    const { asked } = libraryWith({}, { kind: "genre", name }, 4);
    renderApp(`/library?filter=${encodeURIComponent(`genre-${encodeURIComponent(name)}`)}`);
    const chips = await chipsList();
    expect(within(chips).getByText(tx("trackTable:filters.genre", { genre: name }))).toBeInTheDocument();
    expect(asked[0]?.filters).toEqual([{ kind: "genre", name }]);
  });

  it.each(["genre-", "genre-%20", "genre-%E0%A4%A", "genre"])(
    "open the Library as it is for the word %j, which names no genre",
    async (word) => {
      const { asked } = libraryWith({}, null, 0);
      renderApp(`/library?filter=${encodeURIComponent(word)}`);
      expect(await screen.findByText(tx("library:trackCount", { count: LIBRARY_TRACKS }))).toBeInTheDocument();
      expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
      expect(asked[0]?.filters).toEqual([]);
    },
  );

  it("leave Other and No genre as plain bars: they are not one genre", async () => {
    const many = Array.from({ length: 10 }, (_, i) => ({ name: `Genre ${i + 1}`, tracks: 20 - i }));
    fakeBackend({ overview: { genres: many, noGenre: 4 } });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.genres.title") });
    const rows = within(region).getAllByRole("listitem");
    // Eight genres, Other and No genre.
    expect(rows).toHaveLength(10);
    expect(rows.filter((row) => within(row).queryByRole("link") !== null)).toHaveLength(8);
    for (const row of rows.slice(8)) expect(within(row).queryByRole("link")).toBeNull();
  });
});

describe("the numbers that open nothing in the Library", () => {
  it("are the length and the last send", async () => {
    fakeBackend({
      overview: { genres: [{ name: "House", tracks: 2 }], noGenre: 1 },
    });
    renderApp("/overview");
    await screen.findByRole("region", { name: tx("overview:charts.genres.title") });
    const list = screen.getByText(tx("overview:numbers.inCrates")).closest("ul") as HTMLElement;
    // The four numbers: tracks, in crates and analyzed are doors; the last send is not.
    expect(within(list).getAllByRole("link")).toHaveLength(3);
  });

  it("include the analyzed number before rekordbox has been read: there is nothing behind it", async () => {
    fakeBackend({ overview: { rekordboxRead: false, analyzedByRekordbox: 0 } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:numbers.analyzedUnread"))).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: contains(tx("overview:numbers.analyzedUnread")) })).toBeNull();
  });
});

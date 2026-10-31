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
    name: "the tracks made from low-quality files",
    overview: { lowQuality: 6 },
    open: todoLine("lowQuality", 6),
    chip: { kind: "lowQuality" },
    label: tx("trackTable:filters.lowQuality"),
    number: 6,
    word: "lowQuality",
  },
  {
    name: "the tracks rekordbox hasn't analyzed",
    overview: { notAnalyzed: 9 },
    open: todoLine("notAnalyzed", 9),
    chip: { kind: "notAnalyzedByRekordbox" },
    label: tx("trackTable:filters.notAnalyzedByRekordbox"),
    number: 9,
    word: "notAnalyzedByRekordbox",
  },
  {
    name: "the tracks in no crate",
    overview: { inNoCrate: 1 },
    open: todoLine("inNoCrate", 1),
    chip: { kind: "inNoCrate" },
    label: tx("trackTable:filters.inNoCrate"),
    number: 1,
    word: "inNoCrate",
  },
  {
    name: "the number of tracks in crates",
    overview: { inCrates: 3 },
    open: async () => screen.findByRole("link", { name: contains(tx("overview:numbers.inCrates")) }),
    chip: { kind: "inCrate" },
    label: tx("trackTable:filters.inCrate"),
    number: 3,
    word: "inCrate",
  },
  {
    name: "the number of tracks analyzed by rekordbox",
    overview: { analyzedByRekordbox: 4 },
    open: async () => screen.findByRole("link", { name: contains(tx("overview:numbers.analyzed")) }),
    chip: { kind: "analyzedByRekordbox" },
    label: tx("trackTable:filters.analyzedByRekordbox"),
    number: 4,
    word: "analyzedByRekordbox",
  },
  {
    name: "the energy 7 bar",
    overview: { energy: [0, 0, 0, 0, 0, 0, 5, 0, 0, 0], noEnergy: 2 },
    open: async () => {
      const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
      return within(region).getAllByRole("link")[6] as HTMLElement;
    },
    chip: { kind: "energy", level: 7 },
    label: tx("trackTable:filters.energy", { level: 7 }),
    number: 5,
    word: "energy-7",
  },
  {
    name: "the No energy bar",
    overview: { energy: [0, 0, 0, 0, 0, 0, 5, 0, 0, 0], noEnergy: 2 },
    open: async () => {
      const region = await screen.findByRole("region", { name: tx("overview:charts.energy.title") });
      const links = within(region).getAllByRole("link");
      return links[links.length - 1] as HTMLElement;
    },
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

describe("the numbers that open nothing in the Library", () => {
  it("are the genre bars, the length and the last send", async () => {
    fakeBackend({
      overview: { genres: [{ name: "House", tracks: 2 }], noGenre: 1 },
    });
    renderApp("/overview");
    const region = await screen.findByRole("region", { name: tx("overview:charts.genres.title") });
    expect(within(region).queryAllByRole("link")).toEqual([]);
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

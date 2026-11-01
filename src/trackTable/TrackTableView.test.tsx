import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableColumns, TablePositionQuery, TableQuery, TableRow } from "../bindings";
import "../i18n";
import { pageOf, queryOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import type { StoredChoice } from "./columnChoice";
import type { TableScreen } from "./columns";
import { useSelectedTrack } from "../shell/selectedTrack";
import { TrackTableView } from "./TrackTableView";
import { WINDOW } from "./useTrackRows";

const ROW = 28;

/** The queries the backend was asked, in order. */
let asked: TableQuery[] = [];
let formats: string[] = [];
let formatsFail = false;
let behind = false;
/** The column choices stored, by screen (none stored is no entry), and what was saved. */
let storedColumns: Record<string, StoredChoice | null> = {};
let saved: { screen: string; columns: TableColumns | null }[] = [];
let columnsFail = false;
/** The positions asked for, and the answer: where a track is in a query's rows (null: not in them). */
let positionsAsked: TablePositionQuery[] = [];
let positionIn: (query: TablePositionQuery) => number | null = () => null;

/** Answers with `total` made-up rows (made on demand: a list of 100,000 costs nothing). */
function serve(
  total: number,
  rowAt: (index: number, query: TableQuery) => TableRow = (i) => tableRow(i, { title: `Tune ${i}` }),
) {
  mockIPC((cmd, args) => {
    if (cmd === "key_notation") return "camelot";
    if (cmd === "table_columns") {
      if (columnsFail) throw { kind: "database", params: {} };
      return storedColumns[(args as { screen: string }).screen] ?? null;
    }
    if (cmd === "set_table_columns") {
      const { screen, columns } = args as { screen: string; columns: TableColumns | null };
      saved.push({ screen, columns });
      storedColumns[screen] = columns;
      return null;
    }
    if (cmd === "track_table_position") {
      const query = (args as { query: TablePositionQuery }).query;
      positionsAsked.push(query);
      return positionIn(query);
    }
    if (cmd === "track_table_formats") {
      if (formatsFail) throw { kind: "database", params: {} };
      return formats;
    }
    if (cmd !== "track_table_page") throw new Error(`unexpected command ${cmd}`);
    const query = queryOf(args);
    asked.push(query);
    const rows = Array.from({ length: Math.max(0, Math.min(query.limit, total - query.offset)) }, (_, i) =>
      rowAt(query.offset + i, query),
    );
    return { ...pageOf([], query), total: query.withTotal ? total : null, rows, behind };
  });
}

beforeEach(() => {
  asked = [];
  formats = [];
  formatsFail = false;
  behind = false;
  storedColumns = {};
  saved = [];
  columnsFail = false;
  positionsAsked = [];
  positionIn = () => null;
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

function renderView(
  screenName: TableScreen = "library",
  props: Partial<React.ComponentProps<typeof TrackTableView>> = {},
) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <TrackTableView
        screen={screenName}
        source={screenName === "crate" ? { kind: "crate", id: 7 } : screenName === "allMusic" ? { kind: "allMusic" } : { kind: "library" }}
        keyPrefix={["test", screenName]}
        count={(total) => `rows: ${total}`}
        empty={<p>Nothing at all</p>}
        {...props}
      />
    </QueryClientProvider>,
  );
}

const grid = () => screen.findByRole("grid");
const drawn = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const indexOf = (row: HTMLElement) => Number(row.getAttribute("aria-rowindex")) - 2;
const lastQuery = () => asked[asked.length - 1];

async function heading(column: string) {
  return screen.findByRole("button", { name: tx(`trackTable:columns.${column}`) });
}

describe("asking for rows", () => {
  it("asks for the first window, with the count, and nothing else", async () => {
    serve(50);
    renderView("library");
    await grid();
    expect(asked).toHaveLength(1);
    expect(asked[0]).toEqual({
      source: { kind: "library" },
      search: null,
      filters: [],
      sort: null,
      offset: 0,
      limit: WINDOW,
      withTotal: true,
    });
    expect(screen.getByText("rows: 50")).toBeInTheDocument();
    expect(drawn()[0]).toHaveTextContent("Tune 0");
  });

  it("draws a few rows of 100,000 and fetches only the windows around them", async () => {
    serve(100_000);
    renderView("allMusic");
    await grid();
    expect(drawn().length).toBeLessThan(40);
    const table = screen.getByRole("grid");
    table.scrollTop = 50_000 * ROW;
    fireEvent.scroll(table);
    expect(await screen.findByText("Tune 50000")).toBeInTheDocument();
    // Never the whole list: every request is one window, only the first
    // counts, and a long scroll costs a handful of requests.
    expect(asked.every((q) => q.limit <= WINDOW)).toBe(true);
    expect(asked.filter((q) => q.withTotal)).toHaveLength(1);
    expect(asked.length).toBeLessThanOrEqual(5);
    expect(asked.some((q) => q.offset === 50_000)).toBe(true);
    expect(drawn().length).toBeLessThan(40);
    expect(indexOf(drawn()[0])).toBeGreaterThan(49_900);
  });

  it("passes the search, and a blank search is none", async () => {
    serve(3);
    const view = renderView("allMusic", { search: "  " });
    await grid();
    expect(lastQuery().search).toBeNull();
    view.rerender(
      <QueryClientProvider client={createQueryClient()}>
        <TrackTableView
          screen="allMusic"
          source={{ kind: "allMusic" }}
          keyPrefix={["test", "allMusic"]}
          search="alpha wave"
          count={(total) => `rows: ${total}`}
          empty={<p>Nothing at all</p>}
        />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(lastQuery().search).toBe("alpha wave"));
  });

  it("says how many tracks there are, and that it is catching up when the backend says so", async () => {
    behind = true;
    serve(4);
    renderView();
    expect(await screen.findByText(tx("trackTable:updating"))).toBeInTheDocument();
  });

  it("shows an error, not an empty list, when the rows can't be read", async () => {
    mockIPC((cmd) => {
      if (cmd === "key_notation") return "camelot";
      throw { kind: "database", params: {} };
    });
    renderView();
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:database"));
    expect(screen.queryByText("Nothing at all")).toBeNull();
  });
});

describe("when there is nothing to list", () => {
  it("shows what the screen says for no tracks at all, and no table", async () => {
    serve(0);
    renderView();
    expect(await screen.findByText("Nothing at all")).toBeInTheDocument();
    expect(screen.queryByRole("grid")).toBeNull();
    expect(screen.queryByRole("button", { name: tx("trackTable:filters.add") })).toBeNull();
  });

  it("says no track matches when a search or a filter leaves none, and keeps the filters to take off", async () => {
    serve(0);
    renderView("library", { search: "zzz" });
    expect(await screen.findByText(tx("trackTable:noMatch"))).toBeInTheDocument();
    expect(screen.queryByText("Nothing at all")).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:filters.add") })).toBeInTheDocument();
  });

  it("keeps the chips when the filters leave no track, so they can be taken off", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.missing"));
    serve(0);
    await userEvent.click(await screen.findByRole("button", { name: tx("trackTable:filters.add") }));
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.qualityProblem") }));
    expect(await screen.findByText(tx("trackTable:noMatch"))).toBeInTheDocument();
    expect(screen.getAllByRole("listitem")).toHaveLength(2);
  });
});

describe("while the index is still being written", () => {
  it("says the list is being updated, not that there are no tracks, when it has no rows yet", async () => {
    behind = true;
    serve(0);
    renderView();
    expect(await screen.findByText(tx("trackTable:updating"))).toBeInTheDocument();
    expect(screen.queryByText("Nothing at all")).toBeNull();
    expect(screen.queryByText(tx("trackTable:noMatch"))).toBeNull();
    expect(screen.queryByRole("grid")).toBeNull();
  });

  it("does not say no track matches a search while the index is behind", async () => {
    behind = true;
    serve(0);
    renderView("allMusic", { search: "zzz" });
    expect(await screen.findByText(tx("trackTable:updating"))).toBeInTheDocument();
    expect(screen.queryByText(tx("trackTable:noMatch"))).toBeNull();
  });
});

describe("sorting", () => {
  it("shows the Library's own order, title then artist, as an ascending title", async () => {
    serve(5);
    renderView("library");
    await grid();
    expect(screen.getByRole("columnheader", { name: tx("trackTable:columns.title") })).toHaveAttribute("aria-sort", "ascending");
  });

  it("sorts by a heading and reverses it when it is clicked again", async () => {
    serve(5);
    renderView("library");
    await grid();
    // Already ascending by title, so a click reverses it.
    await userEvent.click(await heading("title"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "title", descending: true }));
    await userEvent.click(await heading("title"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "title", descending: false }));
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "bpm", descending: false }));
    expect(screen.getByRole("columnheader", { name: tx("trackTable:columns.bpm") })).toHaveAttribute("aria-sort", "ascending");
    expect(screen.getByRole("columnheader", { name: tx("trackTable:columns.title") })).not.toHaveAttribute("aria-sort");
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "bpm", descending: true }));
    expect(screen.getByRole("columnheader", { name: tx("trackTable:columns.bpm") })).toHaveAttribute("aria-sort", "descending");
  });

  it("asks for the first window again, and goes back to the top, when the sort changes", async () => {
    serve(2000);
    renderView("allMusic");
    const table = await grid();
    table.scrollTop = 1000 * ROW;
    fireEvent.scroll(table);
    expect(await screen.findByText("Tune 1000")).toBeInTheDocument();
    await userEvent.click(await heading("artist"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "artist", descending: false }));
    expect(lastQuery()).toMatchObject({ offset: expect.any(Number), limit: WINDOW });
    await waitFor(() => expect(table.scrollTop).toBe(0));
    expect(await screen.findByText("Tune 0")).toBeInTheDocument();
  });

  it("keeps the heading's focus after sorting by it", async () => {
    serve(5);
    renderView("library");
    await grid();
    const title = await heading("artist");
    await userEvent.click(title);
    await waitFor(() => expect(lastQuery().sort?.column).toBe("artist"));
    expect(await heading("artist")).toHaveFocus();
  });

  it("leaves a crate in the order its tracks were added until a heading is clicked", async () => {
    serve(5);
    renderView("crate");
    await grid();
    expect(lastQuery().sort).toBeNull();
    expect(screen.getAllByRole("columnheader").some((h) => h.hasAttribute("aria-sort"))).toBe(false);
    await userEvent.click(await heading("title"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "title", descending: false }));
  });
});

/** Opens the Add filter list and chooses `name`. */
async function addFilter(name: string) {
  await userEvent.click(await screen.findByRole("button", { name: tx("trackTable:filters.add") }));
  await userEvent.click(await screen.findByRole("button", { name }));
}

const chips = () => within(screen.getByRole("list", { name: tx("trackTable:filters.label") })).getAllByRole("listitem");

describe("filter chips", () => {
  it("shows no chips to begin with", async () => {
    serve(5);
    renderView();
    await grid();
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
  });

  it("adds a chip and asks for the rows again with the filter", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.missing"));
    expect(chips().map((c) => c.textContent)).toEqual([tx("trackTable:filters.missing")]);
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "missing" }]));
    expect(lastQuery().offset).toBe(0);
    // The list closes once a filter is chosen, and the focus goes back to its button.
    expect(screen.queryByRole("group", { name: tx("trackTable:filters.menu") })).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:filters.add") })).toHaveFocus();
  });

  it("takes a chip off with its ×, which names the filter", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.missing"));
    await addFilter(tx("trackTable:filters.qualityProblem"));
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "missing" }, { kind: "qualityProblem" }]));
    await userEvent.click(
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: tx("trackTable:filters.missing") }) }),
    );
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "qualityProblem" }]));
    expect(chips().map((c) => c.textContent)).toEqual([tx("trackTable:filters.qualityProblem")]);
    await userEvent.click(
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: tx("trackTable:filters.qualityProblem") }) }),
    );
    await waitFor(() => expect(lastQuery().filters).toEqual([]));
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
  });

  it("takes the opposite filter off when one is added, and offers a filter only once", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.inRekordbox"));
    await addFilter(tx("trackTable:filters.notInRekordbox"));
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "notInRekordbox" }]));
    expect(chips().map((c) => c.textContent)).toEqual([tx("trackTable:filters.notInRekordbox")]);
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = screen.getByRole("group", { name: tx("trackTable:filters.menu") });
    expect(within(menu).queryByRole("button", { name: tx("trackTable:filters.notInRekordbox") })).toBeNull();
  });

  it("offers a format once the formats on the screen are known, and the chip names it", async () => {
    formats = ["flac", "mp3", "someday"];
    serve(5);
    renderView();
    await grid();
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = await screen.findByRole("group", { name: tx("trackTable:filters.menu") });
    expect(await within(menu).findByText(tx("trackTable:filters.formats"))).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole("button", { name: tx("trackTable:format.flac") }));
    expect(chips().map((c) => c.textContent)).toEqual([
      tx("trackTable:filters.format", { format: tx("trackTable:format.flac") }),
    ]);
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "format", format: "flac" }]));
    // A second format is another chip, and a format this build doesn't name still shows.
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    await userEvent.click(await screen.findByRole("button", { name: "SOMEDAY" }));
    await waitFor(() =>
      expect(lastQuery().filters).toEqual([
        { kind: "format", format: "flac" },
        { kind: "format", format: "someday" },
      ]),
    );
    expect(chips()).toHaveLength(2);
  });

  it.each([
    [
      "library",
      [
        "missing",
        "inRekordbox",
        "notInRekordbox",
        "qualityProblem",
        "inCrate",
        "inNoCrate",
        "lowQuality",
        "analyzedByRekordbox",
        "notAnalyzedByRekordbox",
        "missingTags",
        "missingArt",
      ],
    ],
    ["allMusic", ["missing", "inRekordbox", "notInRekordbox", "qualityProblem", "inLibrary", "notInLibrary"]],
    ["crate", ["missing", "inRekordbox", "notInRekordbox", "qualityProblem"]],
  ] as const)("offers the filters that make sense on the %s screen", async (screenName, kinds) => {
    serve(5);
    renderView(screenName);
    await grid();
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = screen.getByRole("group", { name: tx("trackTable:filters.menu") });
    expect(within(menu).getAllByRole("button").map((b) => b.textContent)).toEqual(
      kinds.map((kind) => tx(`trackTable:filters.${kind}`)),
    );
  });

  it("closes the list on Escape, back on its button, and on a click elsewhere", async () => {
    serve(5);
    renderView();
    await grid();
    const add = screen.getByRole("button", { name: tx("trackTable:filters.add") });
    await userEvent.click(add);
    expect(add).toHaveAttribute("aria-expanded", "true");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("group", { name: tx("trackTable:filters.menu") })).toBeNull();
    expect(add).toHaveFocus();
    expect(add).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(add);
    expect(screen.getByRole("group", { name: tx("trackTable:filters.menu") })).toBeInTheDocument();
    fireEvent.mouseDown(document.body);
    expect(screen.queryByRole("group", { name: tx("trackTable:filters.menu") })).toBeNull();
  });

  it("tells the screen when the tracks listed change, but not for a sort", async () => {
    serve(5);
    const onNarrowed = vi.fn();
    renderView("library", { onNarrowed });
    await grid();
    expect(onNarrowed).not.toHaveBeenCalled();
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort?.column).toBe("bpm"));
    expect(onNarrowed).not.toHaveBeenCalled();
    await addFilter(tx("trackTable:filters.missing"));
    await waitFor(() => expect(onNarrowed).toHaveBeenCalledTimes(1));
  });

  it("is all keyboard: Tab reaches Add filter, Enter opens it, Enter on a filter adds it", async () => {
    serve(5);
    renderView();
    await grid();
    const add = screen.getByRole("button", { name: tx("trackTable:filters.add") });
    add.focus();
    await userEvent.keyboard("{Enter}");
    await userEvent.tab();
    expect(screen.getByRole("button", { name: tx("trackTable:filters.missing") })).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "missing" }]));
  });

  it("moves the focus to the next chip's ×, else the previous one's, else Add filter, when a chip is taken off", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.missing"));
    await addFilter(tx("trackTable:filters.qualityProblem"));
    await addFilter(tx("trackTable:filters.inRekordbox"));
    const remove = (filter: string) =>
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: tx(`trackTable:filters.${filter}`) }) });
    // The middle one goes: the focus lands on the one after it.
    await userEvent.click(remove("qualityProblem"));
    await waitFor(() => expect(remove("inRekordbox")).toHaveFocus());
    // The last one goes: the focus lands on the one before it.
    await userEvent.click(remove("inRekordbox"));
    await waitFor(() => expect(remove("missing")).toHaveFocus());
    // The only one goes: the focus lands on Add filter.
    await userEvent.click(remove("missing"));
    await waitFor(() => expect(screen.getByRole("button", { name: tx("trackTable:filters.add") })).toHaveFocus());
  });

  it("moves the focus to the next chip's × when the first of several is taken off by the keyboard", async () => {
    serve(5);
    renderView();
    await grid();
    await addFilter(tx("trackTable:filters.missing"));
    await addFilter(tx("trackTable:filters.qualityProblem"));
    const first = screen.getByRole("button", {
      name: tx("trackTable:filters.remove", { filter: tx("trackTable:filters.missing") }),
    });
    first.focus();
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: tx("trackTable:filters.remove", { filter: tx("trackTable:filters.qualityProblem") }),
        }),
      ).toHaveFocus(),
    );
  });

  it("closes the list when the focus moves out of it, by Tab", async () => {
    serve(5);
    renderView();
    await grid();
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = screen.getByRole("group", { name: tx("trackTable:filters.menu") });
    const choices = within(menu).getAllByRole("button").length;
    // Through the choices the list stays open...
    for (let i = 0; i < choices; i += 1) await userEvent.tab();
    expect(screen.getByRole("group", { name: tx("trackTable:filters.menu") })).toBeInTheDocument();
    // ...and one more leaves it.
    await userEvent.tab();
    expect(screen.queryByRole("group", { name: tx("trackTable:filters.menu") })).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:filters.add") })).toHaveAttribute("aria-expanded", "false");
  });

  it("says the formats could not be read, and does not say there are no more filters, when that request fails", async () => {
    formatsFail = true;
    serve(5);
    renderView();
    await grid();
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = screen.getByRole("group", { name: tx("trackTable:filters.menu") });
    expect(await within(menu).findByRole("alert")).toHaveTextContent(tx("errors:database"));
    expect(within(menu).queryByText(tx("trackTable:filters.none"))).toBeNull();
    // The other filters can still be chosen.
    expect(within(menu).getByRole("button", { name: tx("trackTable:filters.missing") })).toBeInTheDocument();
  });
});

describe("sorting keeps the selected track", () => {
  const TOTAL = 2000;
  const desc = (query: TableQuery | TablePositionQuery) => query.sort?.descending === true;
  /** The list is in id order, and turned upside down when the sort is descending. */
  const upsideDown = (index: number, query: TableQuery) =>
    tableRow(desc(query) ? TOTAL - 1 - index : index, { title: `Tune ${desc(query) ? TOTAL - 1 - index : index}` });
  const where = (query: TablePositionQuery) => {
    const n = query.recordingId - 100;
    return desc(query) ? TOTAL - 1 - n : n;
  };
  const selected = () =>
    within(screen.getByRole("grid"))
      .getAllByRole("row")
      .filter((r) => r.getAttribute("aria-selected") === "true");

  it("keeps the track selected, and scrolls to it, when the list is sorted the other way", async () => {
    serve(TOTAL, upsideDown);
    positionIn = where;
    renderView("allMusic");
    const table = await grid();
    await userEvent.click(drawn()[5]);
    await userEvent.click(await heading("title"));
    // The selected row is the same track (its window arrives a moment after the scroll).
    expect(await screen.findByText("Tune 5")).toBeInTheDocument();
    expect(selected()).toHaveLength(1);
    expect(selected()[0]).toHaveTextContent("Tune 5");
    expect(indexOf(selected()[0])).toBe(TOTAL - 1 - 5);
    expect(table.scrollTop).toBeGreaterThan(1000 * ROW);
    // It asked for that track's place in the same list the rows come from.
    expect(positionsAsked).toHaveLength(1);
    expect(positionsAsked[0]).toEqual({
      source: { kind: "allMusic" },
      search: null,
      filters: [],
      sort: { column: "title", descending: true },
      recordingId: 105,
    });
    // And back again.
    await userEvent.click(await heading("title"));
    await waitFor(() => expect(indexOf(selected()[0])).toBe(5));
    expect(table.scrollTop).toBeLessThan(ROW * 20);
  });

  it("starts at the top, and asks for no position, when nothing is selected", async () => {
    serve(TOTAL, upsideDown);
    positionIn = where;
    renderView("allMusic");
    const table = await grid();
    table.scrollTop = 1000 * ROW;
    fireEvent.scroll(table);
    expect(await screen.findByText("Tune 1000")).toBeInTheDocument();
    await userEvent.click(await heading("artist"));
    await waitFor(() => expect(table.scrollTop).toBe(0));
    expect(selected()).toHaveLength(0);
    expect(positionsAsked).toHaveLength(0);
  });

  it("keeps a track selected through a filter that still lists it, and clears the selection when it does not", async () => {
    serve(TOTAL, upsideDown);
    positionIn = (query) => (query.filters.length === 1 ? 7 : query.filters.length === 2 ? null : where(query));
    renderView("allMusic");
    await grid();
    await userEvent.click(drawn()[2]);
    await addFilter(tx("trackTable:filters.inLibrary"));
    await waitFor(() => expect(selected()).toHaveLength(1));
    expect(indexOf(selected()[0])).toBe(7);
    await addFilter(tx("trackTable:filters.missing"));
    await waitFor(() => expect(positionsAsked).toHaveLength(2));
    await waitFor(() => expect(lastQuery().filters).toHaveLength(2));
    expect(selected()).toHaveLength(0);
  });

  it("clears the selection when the position cannot be read", async () => {
    serve(TOTAL, upsideDown);
    positionIn = () => {
      throw { kind: "database", params: {} };
    };
    renderView("allMusic");
    await grid();
    await userEvent.click(drawn()[2]);
    await userEvent.click(await heading("artist"));
    await waitFor(() => expect(positionsAsked).toHaveLength(1));
    await waitFor(() => expect(lastQuery().sort?.column).toBe("artist"));
    expect(selected()).toHaveLength(0);
  });
});

describe("the Details panel's track", () => {
  it("is the selected row, which goes when the table does", async () => {
    serve(50);
    const { unmount } = renderView("library");
    await grid();
    expect(useSelectedTrack.getState().tables).toBe(1);
    expect(useSelectedTrack.getState().row).toBeNull();
    await userEvent.click(drawn()[2]);
    await waitFor(() => expect(useSelectedTrack.getState().row?.title).toBe("Tune 2"));
    unmount();
    expect(useSelectedTrack.getState()).toMatchObject({ row: null, tables: 0 });
  });
});

describe("the column editor", () => {
  const headings = () => screen.getAllByRole("columnheader").map((h) => h.textContent);
  const names = (ids: string[]) => ids.map((id) => tx(`trackTable:columns.${id}`));
  const editor = async () => {
    await userEvent.click(await screen.findByRole("button", { name: tx("trackTable:editor.open") }));
    return screen.getByRole("dialog", { name: tx("trackTable:editor.title") });
  };
  const everyColumn = ["title", "artist", "bpm", "key", "time", "genre", "energy", "rating", "dateAdded", "file", "folder"];
  const named = (key: string, column: string) => tx(`trackTable:editor.${key}`, { column: tx(`trackTable:columns.${column}`) });

  it("shows each screen's default columns until a choice is made", async () => {
    serve(5);
    const library = renderView("library");
    await grid();
    // The same six everywhere (owner decision, 2026-10-30); the other four are in the editor.
    expect(headings()).toEqual(names(["title", "artist", "bpm", "key", "time", "genre"]));
    library.unmount();
    renderView("allMusic");
    await grid();
    expect(headings()).toEqual(names(["title", "artist", "bpm", "key", "time", "genre"]));
  });

  it("draws the columns that were chosen for the screen, in that order, and ignores an id it does not know", async () => {
    storedColumns.library = { shown: ["bpm", "tempo_curve", "title", "artist"], known: [...everyColumn, "tempo_curve"] };
    serve(5);
    renderView("library");
    await grid();
    expect(headings()).toEqual(names(["bpm", "title", "artist"]));
  });

  it("falls back to the default columns when the choice cannot be read", async () => {
    columnsFail = true;
    serve(5);
    renderView("library");
    await grid();
    expect(headings()).toHaveLength(6);
  });

  it("hides a column at once and keeps the choice for the screen", async () => {
    serve(5);
    renderView("library");
    await grid();
    const dialog = await editor();
    await userEvent.click(within(dialog).getByRole("button", { name: named("hide", "genre") }));
    expect(headings()).toEqual(names(["title", "artist", "bpm", "key", "time"]));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({
      screen: "library",
      columns: { shown: ["title", "artist", "bpm", "key", "time"], known: everyColumn, widths: {} },
    });
  });

  it("remembers the choice the next time the screen is shown, and not for another screen", async () => {
    serve(5);
    const first = renderView("library");
    await grid();
    const dialog = await editor();
    await userEvent.click(within(dialog).getByRole("button", { name: named("moveUp", "bpm") }));
    await waitFor(() => expect(saved).toHaveLength(1));
    first.unmount();
    const again = renderView("library");
    await grid();
    expect(headings().slice(0, 3)).toEqual(names(["title", "bpm", "artist"]));
    again.unmount();
    renderView("allMusic");
    await grid();
    expect(headings().slice(0, 3)).toEqual(names(["title", "artist", "bpm"]));
  });

  it("is one choice for every crate", async () => {
    serve(5);
    const first = renderView("crate");
    await grid();
    const dialog = await editor();
    await userEvent.click(within(dialog).getByRole("button", { name: named("hide", "key") }));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].screen).toBe("crate");
    first.unmount();
    // Another crate (the screen gives each its own view).
    renderView("crate", { source: { kind: "crate", id: 8 } });
    await grid();
    expect(headings()).not.toContain(tx("trackTable:columns.key"));
    expect(headings()).toHaveLength(5);
  });

  it("goes back to the default and forgets the choice", async () => {
    storedColumns.library = { shown: ["title", "bpm"], known: everyColumn };
    serve(5);
    renderView("library");
    await grid();
    expect(headings()).toHaveLength(2);
    const dialog = await editor();
    await userEvent.click(within(dialog).getByRole("button", { name: tx("trackTable:editor.reset") }));
    await waitFor(() => expect(headings()).toHaveLength(6));
    await waitFor(() => expect(saved).toEqual([{ screen: "library", columns: null }]));
  });

  it("keeps the selection and the scroll when a column is hidden", async () => {
    serve(2000);
    renderView("library");
    const table = await grid();
    table.scrollTop = 100 * ROW;
    fireEvent.scroll(table);
    await userEvent.click((await screen.findAllByRole("row")).find((r) => indexOf(r) === 105)!);
    const dialog = await editor();
    await userEvent.click(within(dialog).getByRole("button", { name: named("hide", "genre") }));
    expect(table.scrollTop).toBe(100 * ROW);
    expect(screen.getAllByRole("row").filter((r) => r.getAttribute("aria-selected") === "true")).toHaveLength(1);
  });
});

describe("dragging the headings (1cB-21)", () => {
  const WIDE = 100;
  /** Puts the headings side by side, as a browser would (jsdom has no layout). */
  const place = () =>
    screen.getAllByRole("columnheader").forEach((cell, at) => {
      cell.getBoundingClientRect = () =>
        ({
          left: at * WIDE,
          right: (at + 1) * WIDE,
          width: WIDE,
          top: 0,
          bottom: 28,
          height: 28,
          x: at * WIDE,
          y: 0,
          toJSON: () => ({}),
        }) as DOMRect;
    });
  const drag = (element: HTMLElement, from: number, to: number) => {
    fireEvent.pointerDown(element, { clientX: from, button: 0, pointerId: 1 });
    fireEvent.pointerMove(element, { clientX: to, pointerId: 1 });
    fireEvent.pointerUp(element, { clientX: to, pointerId: 1 });
  };
  const line = (column: string) =>
    screen.getByRole("separator", {
      name: tx("trackTable:resize", { column: tx(`trackTable:columns.${column}`) }),
    });
  const template = () => screen.getByRole("grid").parentElement!.style.getPropertyValue("--tt-columns");
  const headings = () => screen.getAllByRole("columnheader").map((h) => h.textContent);
  const names = (ids: string[]) => ids.map((id) => tx(`trackTable:columns.${id}`));
  // The six every screen shows at first, and the eight a user who also showed Rating and Date added has.
  const six = ["title", "artist", "bpm", "key", "time", "genre"];
  const library = [...six, "rating", "dateAdded"];
  const everyColumn = [...six, "energy", "rating", "dateAdded", "file", "folder"];
  const defaultTemplate = "minmax(120px, 3fr) minmax(80px, 2fr) 48px 42px 46px minmax(52px, 1fr)";
  const openEditor = async () => {
    await userEvent.click(await screen.findByRole("button", { name: tx("trackTable:editor.open") }));
    return screen.getByRole("dialog", { name: tx("trackTable:editor.title") });
  };

  it("reorders the columns when a heading is dragged, and saves the order for the screen", async () => {
    serve(5);
    renderView("library");
    await grid();
    place();
    // BPM (200..300) dropped before the middle of Artist (100..200).
    drag(await heading("bpm"), 250, 120);
    await waitFor(() => expect(headings().slice(0, 3)).toEqual(names(["title", "bpm", "artist"])));
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({
      screen: "library",
      columns: { shown: ["title", "bpm", "artist", ...six.slice(3)], known: everyColumn, widths: {} },
    });
  });

  it("is the order the Columns panel edits: one order for both", async () => {
    serve(5);
    renderView("library");
    await grid();
    place();
    drag(await heading("bpm"), 250, 120);
    await waitFor(() => expect(saved).toHaveLength(1));
    const dialog = await openEditor();
    const shown = within(within(dialog).getByRole("list", { name: tx("trackTable:editor.shown") })).getAllByRole(
      "listitem",
    );
    expect(shown.slice(0, 3).map((item) => item.getAttribute("data-column"))).toEqual(["title", "bpm", "artist"]);
  });

  it("sorts by a click on a heading, and a drag of a heading does not sort", async () => {
    serve(5);
    renderView("library");
    await grid();
    place();
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "bpm", descending: false }));
    const before = asked.length;
    const key = await heading("key");
    drag(key, 350, 120);
    fireEvent.click(key);
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(asked).toHaveLength(before);
    expect(lastQuery().sort).toEqual({ column: "bpm", descending: false });
  });

  it("resizes a column when the line after its heading is dragged, and saves the width with the columns", async () => {
    serve(5);
    renderView("library");
    await grid();
    drag(line("bpm"), 300, 340);
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({
      screen: "library",
      columns: { shown: six, known: everyColumn, widths: { bpm: 88 } },
    });
    expect(template()).toContain("minmax(80px, 2fr) 88px 42px");
  });

  it("remembers the widths the next time the screen is shown, and not for another screen", async () => {
    serve(5);
    const first = renderView("library");
    await grid();
    drag(line("artist"), 200, 260);
    await waitFor(() => expect(saved).toHaveLength(1));
    first.unmount();
    const again = renderView("library");
    await grid();
    expect(template()).toContain(" 140px ");
    again.unmount();
    renderView("allMusic");
    await grid();
    expect(template()).not.toContain("140px");
  });

  it("keeps the widths when a column is hidden, and the columns when a width changes", async () => {
    storedColumns.library = { shown: library, known: everyColumn, widths: { genre: 140 } };
    serve(5);
    renderView("library");
    await grid();
    const dialog = await openEditor();
    await userEvent.click(
      within(dialog).getByRole("button", {
        name: tx("trackTable:editor.hide", { column: tx("trackTable:columns.key") }),
      }),
    );
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0].columns?.widths).toEqual({ genre: 140 });
    expect(saved[0].columns?.shown).not.toContain("key");
    drag(line("genre"), 500, 510);
    await waitFor(() => expect(saved).toHaveLength(2));
    expect(saved[1].columns).toEqual({
      shown: library.filter((id) => id !== "key"),
      known: everyColumn,
      widths: { genre: 150 },
    });
  });

  it("goes back to the default order and widths with Reset to default", async () => {
    storedColumns.library = { shown: ["title", "bpm", "artist"], known: everyColumn, widths: { bpm: 200 } };
    serve(5);
    renderView("library");
    await grid();
    expect(template()).toContain("200px");
    const dialog = await openEditor();
    await userEvent.click(within(dialog).getByRole("button", { name: tx("trackTable:editor.reset") }));
    await waitFor(() => expect(saved).toEqual([{ screen: "library", columns: null }]));
    await waitFor(() => expect(headings()).toEqual(names(six)));
    expect(template()).toContain(defaultTemplate);
  });

  it("has something to reset when only a width was changed", async () => {
    storedColumns.library = { shown: library, known: everyColumn, widths: { bpm: 200 } };
    serve(5);
    renderView("library");
    await grid();
    const dialog = await openEditor();
    expect(within(dialog).getByRole("button", { name: tx("trackTable:editor.reset") })).toBeEnabled();
  });

  it("still loads a choice saved before widths existed, with every column at its own width", async () => {
    storedColumns.library = { shown: ["title", "bpm", "artist"], known: everyColumn };
    serve(5);
    renderView("library");
    await grid();
    expect(headings()).toEqual(names(["title", "bpm", "artist"]));
    expect(template()).toContain("minmax(120px, 3fr) 48px minmax(80px, 2fr)");
  });

  it("brings a stored width back inside what its column allows, and ignores one for an id it does not know", async () => {
    storedColumns.library = {
      shown: library,
      known: everyColumn,
      widths: { bpm: 3, artist: 99_999, tempo_curve: 80 },
    };
    serve(5);
    renderView("library");
    await grid();
    expect(template()).toContain(" 1200px 44px ");
    expect(template()).not.toContain("80px");
  });
});

describe("a filter chip the table starts with (a door from the Overview, 1cC-4)", () => {
  const low = { kind: "lowQuality" } as const;
  const lowLabel = () => tx("trackTable:filters.lowQuality");

  it("shows the chip and asks for its very first window with the filter on", async () => {
    serve(6);
    renderView("library", { initialFilters: [low] });
    await grid();
    expect(chips().map((c) => c.textContent)).toEqual([lowLabel()]);
    expect(asked).toHaveLength(1);
    expect(asked[0]?.filters).toEqual([low]);
    expect(asked[0]?.withTotal).toBe(true);
  });

  it("says how many rows the chip leaves, which is the number the door was opened from", async () => {
    serve(6);
    renderView("library", { initialFilters: [low] });
    await grid();
    expect(screen.getByText("rows: 6")).toBeInTheDocument();
  });

  it("takes the chip off with its ×, like any other, and asks for every row again", async () => {
    serve(6);
    renderView("library", { initialFilters: [low] });
    await grid();
    await userEvent.click(
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: lowLabel() }) }),
    );
    await waitFor(() => expect(lastQuery().filters).toEqual([]));
    expect(screen.queryByRole("list", { name: tx("trackTable:filters.label") })).toBeNull();
    expect(screen.getByRole("button", { name: tx("trackTable:filters.add") })).toHaveFocus();
  });

  it("works with a search: both narrow the same rows", async () => {
    serve(2);
    renderView("allMusic", { initialFilters: [low], search: "wave" });
    await grid();
    expect(asked[0]?.search).toBe("wave");
    expect(asked[0]?.filters).toEqual([low]);
  });

  it("works with the other filters: a chip added beside it", async () => {
    serve(6);
    renderView("library", { initialFilters: [low] });
    await grid();
    await addFilter(tx("trackTable:filters.inNoCrate"));
    await waitFor(() => expect(lastQuery().filters).toEqual([low, { kind: "inNoCrate" }]));
    expect(chips().map((c) => c.textContent)).toEqual([lowLabel(), tx("trackTable:filters.inNoCrate")]);
  });

  it("survives a sort", async () => {
    serve(6);
    renderView("library", { initialFilters: [low] });
    await grid();
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort).toEqual({ column: "bpm", descending: false }));
    expect(lastQuery().filters).toEqual([low]);
    expect(chips().map((c) => c.textContent)).toEqual([lowLabel()]);
  });

  it("is replaced by its opposite: not analyzed by rekordbox takes analyzed off", async () => {
    serve(6);
    renderView("library", { initialFilters: [{ kind: "analyzedByRekordbox" }] });
    await grid();
    await addFilter(tx("trackTable:filters.notAnalyzedByRekordbox"));
    await waitFor(() => expect(lastQuery().filters).toEqual([{ kind: "notAnalyzedByRekordbox" }]));
    expect(chips().map((c) => c.textContent)).toEqual([tx("trackTable:filters.notAnalyzedByRekordbox")]);
  });

  it("names an energy level and no energy, which the Add filter list doesn't offer", async () => {
    serve(6);
    renderView("library", { initialFilters: [{ kind: "energy", level: 7 }] });
    await grid();
    expect(chips().map((c) => c.textContent)).toEqual([tx("trackTable:filters.energy", { level: 7 })]);
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    const menu = screen.getByRole("group", { name: tx("trackTable:filters.menu") });
    for (const kind of ["energy", "noEnergy"]) {
      expect(within(menu).queryByRole("button", { name: tx(`trackTable:filters.${kind}`, { level: 7 }) })).toBeNull();
    }
  });

  it("is not remembered: a table made afresh starts with the chips it is given", async () => {
    serve(6);
    const first = renderView("library", { initialFilters: [low] });
    await grid();
    await userEvent.click(
      screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: lowLabel() }) }),
    );
    await waitFor(() => expect(lastQuery().filters).toEqual([]));
    first.unmount();
    asked = [];
    renderView("library");
    await grid();
    expect(asked[0]?.filters).toEqual([]);
  });
});

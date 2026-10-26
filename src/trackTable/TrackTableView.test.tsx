import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableQuery, TableRow } from "../bindings";
import "../i18n";
import { pageOf, queryOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import type { TableScreen } from "./columns";
import { TrackTableView } from "./TrackTableView";
import { WINDOW } from "./useTrackRows";

const ROW = 28;

/** The queries the backend was asked, in order. */
let asked: TableQuery[] = [];
let formats: string[] = [];
let formatsFail = false;
let behind = false;

/** Answers with `total` made-up rows (made on demand: a list of 100,000 costs nothing). */
function serve(total: number, rowAt: (index: number) => TableRow = (i) => tableRow(i, { title: `Tune ${i}` })) {
  mockIPC((cmd, args) => {
    if (cmd === "key_notation") return "camelot";
    if (cmd === "track_table_formats") {
      if (formatsFail) throw { kind: "database", params: {} };
      return formats;
    }
    if (cmd !== "track_table_page") throw new Error(`unexpected command ${cmd}`);
    const query = queryOf(args);
    asked.push(query);
    const rows = Array.from({ length: Math.max(0, Math.min(query.limit, total - query.offset)) }, (_, i) =>
      rowAt(query.offset + i),
    );
    return { ...pageOf([], query), total: query.withTotal ? total : null, rows, behind };
  });
}

beforeEach(() => {
  asked = [];
  formats = [];
  formatsFail = false;
  behind = false;
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
    ["library", ["missing", "inRekordbox", "notInRekordbox", "qualityProblem", "inCrate", "inNoCrate"]],
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

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
let behind = false;

/** Answers with `total` made-up rows (made on demand: a list of 100,000 costs nothing). */
function serve(total: number, rowAt: (index: number) => TableRow = (i) => tableRow(i, { title: `Tune ${i}` })) {
  mockIPC((cmd, args) => {
    if (cmd === "key_notation") return "camelot";
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
  });

  it("says no track matches when a search leaves none", async () => {
    serve(0);
    renderView("library", { search: "zzz" });
    expect(await screen.findByText(tx("trackTable:noMatch"))).toBeInTheDocument();
    expect(screen.queryByText("Nothing at all")).toBeNull();
    expect(screen.queryByRole("grid")).toBeNull();
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

describe("telling the screen the tracks listed changed", () => {
  it("does so for another search, but not for a sort", async () => {
    serve(5);
    const onNarrowed = vi.fn();
    const view = renderView("allMusic", { onNarrowed });
    await grid();
    await userEvent.click(await heading("bpm"));
    await waitFor(() => expect(lastQuery().sort?.column).toBe("bpm"));
    expect(onNarrowed).not.toHaveBeenCalled();
    view.rerender(
      <QueryClientProvider client={createQueryClient()}>
        <TrackTableView
          screen="allMusic"
          source={{ kind: "allMusic" }}
          keyPrefix={["test", "allMusic"]}
          search="alpha"
          count={(total) => `rows: ${total}`}
          empty={<p>Nothing at all</p>}
          onNarrowed={onNarrowed}
        />
      </QueryClientProvider>,
    );
    await waitFor(() => expect(onNarrowed).toHaveBeenCalledTimes(1));
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableRow, TableSort } from "../bindings";
import "../i18n";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import type { ColumnId } from "./columns";
import { formatDate, formatLength, folderOf } from "./columns";
import { TrackTable } from "./TrackTable";

const ROW = 28;
/** A viewport this tall shows the header and ten rows. */
const VIEWPORT = ROW * 11;

let keyNotation = "camelot";

beforeEach(() => {
  keyNotation = "camelot";
  mockIPC((cmd) => {
    if (cmd === "key_notation") return keyNotation;
    throw new Error(`unexpected command ${cmd}`);
  });
  // jsdom has no layout: say how tall the scroll area is.
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(VIEWPORT);
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

const ALL: ColumnId[] = ["title", "artist", "bpm", "key", "time", "genre", "rating", "dateAdded", "file", "folder"];

/** A made-up row for list position `index`. */
const rowAt = (index: number): TableRow => tableRow(index, { title: `Tune ${index}` });

function renderTable(
  props: Partial<React.ComponentProps<typeof TrackTable>> & { rows?: (index: number) => TableRow | undefined } = {},
) {
  const { rows = rowAt, ...rest } = props;
  const onRangeChange = vi.fn();
  const onSort = vi.fn();
  const view = render(
    <QueryClientProvider client={createQueryClient()}>
      <TrackTable
        label="Under test"
        columns={ALL}
        total={100}
        getRow={rows}
        onRangeChange={onRangeChange}
        sort={null}
        onSort={onSort}
        resetKey="a"
        {...rest}
      />
    </QueryClientProvider>,
  );
  return { ...view, onRangeChange, onSort };
}

/** The body rows that are drawn (not the header). */
function drawn() {
  return within(screen.getByRole("grid")).getAllByRole("row").slice(1);
}

const indexOf = (row: HTMLElement) => Number(row.getAttribute("aria-rowindex")) - 2;

describe("the track table draws only the rows on screen", () => {
  it("puts a few rows in the page for a list of 100,000", () => {
    renderTable({ total: 100_000 });
    const rows = drawn();
    // Ten on screen and a few either side, nowhere near 100,000.
    expect(rows.length).toBeGreaterThanOrEqual(10);
    expect(rows.length).toBeLessThan(40);
    expect(indexOf(rows[0])).toBe(0);
    // The table is as long as the list, so the scrollbar is the right size.
    expect(screen.getByRole("grid")).toHaveAttribute("aria-rowcount", "100001");
  });

  it("draws the rows around where it has been scrolled to, and no others", () => {
    const { onRangeChange } = renderTable({ total: 100_000 });
    const grid = screen.getByRole("grid");
    grid.scrollTop = 50_000 * ROW;
    fireEvent.scroll(grid);
    const rows = drawn();
    expect(rows.length).toBeLessThan(40);
    const indexes = rows.map(indexOf);
    expect(Math.min(...indexes)).toBeGreaterThan(49_980);
    expect(Math.max(...indexes)).toBeLessThan(50_030);
    expect(indexes).toContain(50_000);
    expect(screen.getByText("Tune 50000")).toBeInTheDocument();
    expect(screen.queryByText("Tune 0")).toBeNull();
    // The range it asked its data for is the rows it draws.
    const [from, to] = onRangeChange.mock.calls.at(-1) as [number, number];
    expect(from).toBeLessThanOrEqual(50_000);
    expect(to).toBeGreaterThanOrEqual(50_009);
    expect(to - from).toBeLessThan(40);
  });

  it("draws every row that any part of the viewport shows, wherever it is scrolled to", () => {
    const total = 1_000;
    renderTable({ total });
    const grid = screen.getByRole("grid");
    // The header takes one row of the viewport; the rest is room for rows.
    const room = VIEWPORT - ROW;
    for (const scrollTop of [0, 13, 500 * ROW, 500 * ROW + 1, 500 * ROW + 27, (total - 10) * ROW]) {
      grid.scrollTop = scrollTop;
      fireEvent.scroll(grid);
      const indexes = new Set(drawn().map(indexOf));
      for (let i = 0; i < total; i += 1) {
        const shown = i * ROW < scrollTop + room && (i + 1) * ROW > scrollTop;
        if (shown) expect(indexes, `row ${i} at scrollTop ${scrollTop}`).toContain(i);
      }
    }
  });

  it("puts each row where it belongs in the list", () => {
    renderTable({ total: 100_000 });
    for (const row of drawn()) {
      expect(row.style.transform).toBe(`translateY(${indexOf(row) * ROW}px)`);
    }
  });

  it("draws a row whose window hasn't arrived as an empty one, and fills it in when it does", () => {
    const { rerender } = renderTable({ rows: (index) => (index < 3 ? rowAt(index) : undefined) });
    const rows = drawn();
    expect(rows[2]).toHaveTextContent("Tune 2");
    expect(rows[3]).toHaveAttribute("aria-busy", "true");
    expect(rows[3]).toHaveTextContent("");
    rerender(
      <QueryClientProvider client={createQueryClient()}>
        <TrackTable
          label="Under test"
          columns={ALL}
          total={100}
          getRow={rowAt}
          onRangeChange={() => {}}
          sort={null}
          onSort={() => {}}
          resetKey="a"
        />
      </QueryClientProvider>,
    );
    expect(drawn()[3]).toHaveTextContent("Tune 3");
    expect(drawn()[3]).not.toHaveAttribute("aria-busy");
  });

  it("goes back to the top when the list becomes another one", () => {
    const { rerender } = renderTable({ total: 1000 });
    const grid = screen.getByRole("grid");
    grid.scrollTop = 500 * ROW;
    fireEvent.scroll(grid);
    expect(screen.queryByText("Tune 0")).toBeNull();
    rerender(
      <QueryClientProvider client={createQueryClient()}>
        <TrackTable
          label="Under test"
          columns={ALL}
          total={1000}
          getRow={rowAt}
          onRangeChange={() => {}}
          sort={null}
          onSort={() => {}}
          resetKey="b"
        />
      </QueryClientProvider>,
    );
    expect(grid.scrollTop).toBe(0);
    expect(screen.getByText("Tune 0")).toBeInTheDocument();
  });
});

describe("the columns are data", () => {
  it("draws exactly the columns it is given, in the order given", () => {
    renderTable({ columns: ["bpm", "title"] });
    expect(screen.getAllByRole("columnheader").map((h) => h.textContent)).toEqual([
      tx("trackTable:columns.bpm"),
      tx("trackTable:columns.title"),
    ]);
    expect(within(drawn()[0]).getAllByRole("gridcell")).toHaveLength(2);
    expect(screen.getByRole("grid")).toHaveAttribute("aria-colcount", "2");
  });

  it("adds a last column for the screen's own buttons, named for screen readers", () => {
    renderTable({
      columns: ["title"],
      actions: (row) => <button type="button">Act on {row.title}</button>,
    });
    const headers = screen.getAllByRole("columnheader");
    expect(headers.at(-1)).toHaveTextContent(tx("trackTable:columns.actions"));
    expect(within(drawn()[1]).getByRole("button", { name: "Act on Tune 1" })).toBeInTheDocument();
  });
});

describe("what the cells show", () => {
  const row = tableRow(1, {
    title: "Synthetic Tune",
    artist: "Made Up Artist",
    bpm: 124,
    key: "8A",
    timeMs: 215_400,
    genre: "House",
    rating: 3,
    addedAt: "2026-09-30T10:00:00.000Z",
  });

  it("shows each column's value, with the key in the user's notation", async () => {
    keyNotation = "musical_standard";
    renderTable({ total: 1, rows: () => row });
    const cells = within(drawn()[0])
      .getAllByRole("gridcell")
      .map((cell) => cell.textContent);
    expect(cells.slice(0, 3)).toEqual(["Synthetic Tune", "Made Up Artist", "124.0"]);
    // Time as m:ss; the date is the user's own format of the day.
    expect(cells[4]).toBe("3:35");
    expect(cells[5]).toBe("House");
    expect(cells[7]).toBe(formatDate(row.addedAt, "en"));
    expect(cells[8]).toBe("tune 1.mp3");
    expect(cells[9]).toBe(String.raw`E:\Music`);
    // The key follows the setting once it has loaded.
    expect(await screen.findByText("Am")).toBeInTheDocument();
  });

  it("shows a rating as stars that screen readers read as a number", () => {
    renderTable({ total: 1, rows: () => row });
    expect(within(drawn()[0]).getByRole("img", { name: tx("trackTable:rating", { count: 3 }) })).toHaveTextContent(
      "\u2605\u2605\u2605\u2606\u2606",
    );
  });

  it("leaves a value the track doesn't have blank", () => {
    renderTable({
      total: 1,
      rows: () => tableRow(2, { bpm: null, key: null, timeMs: null, genre: null, rating: null, addedAt: null, artist: null }),
    });
    const cells = within(drawn()[0]).getAllByRole("gridcell");
    expect(cells.slice(1, 8).map((c) => c.textContent)).toEqual(["", "", "", "", "", "", ""]);
  });

  it("shows a track with no title by its file's name", () => {
    renderTable({ total: 1, rows: () => tableRow(3, { title: null }) });
    expect(within(drawn()[0]).getAllByRole("gridcell")[0]).toHaveTextContent("tune 3.mp3");
  });

  it("sets the numbers in tabular figures, to the right", () => {
    renderTable({ total: 1, rows: () => row });
    const [, , bpm, key, time] = within(drawn()[0]).getAllByRole("gridcell");
    for (const cell of [bpm, key, time]) expect(cell.className).toMatch(/numeric/);
  });

  it("cuts a long text off with an ellipsis and carries the whole text as its hover text", () => {
    const long = "A very long title that would not fit in any column at all";
    renderTable({ total: 1, rows: () => tableRow(4, { title: long, artist: long }) });
    const [title, artist] = within(drawn()[0]).getAllByRole("gridcell");
    expect(within(title).getByText(long)).toHaveAttribute("title", long);
    expect(artist).toHaveAttribute("title", long);
  });

  it("says a file is missing, in words, on its row", () => {
    renderTable({
      total: 1,
      rows: () => tableRow(5, { sourceMissing: true, file: { id: 5, path: String.raw`E:\a.mp3`, name: "a.mp3", present: false, driveConnected: true } }),
    });
    expect(drawn()[0]).toHaveTextContent(tx("library:fileMissing"));
  });

  it("gives a track with a file a play button named for the track", () => {
    renderTable({ total: 1, rows: () => tableRow(6, { title: "Synthetic Tune 6" }) });
    expect(within(drawn()[0]).getByRole("button", { name: tx("player:playNamed", { name: "Synthetic Tune 6" }) })).toBeInTheDocument();
  });

  it("formats lengths, folders and dates", () => {
    expect(formatLength(null)).toBe("");
    expect(formatLength(59_400)).toBe("0:59");
    expect(formatLength(3_725_000)).toBe("1:02:05");
    expect(folderOf(String.raw`E:\Music\a.mp3`, "a.mp3")).toBe(String.raw`E:\Music`);
    expect(folderOf("a.mp3", "a.mp3")).toBe("");
    expect(formatDate(null, "en")).toBe("");
    expect(formatDate("not a date", "en")).toBe("");
  });
});

describe("sorting", () => {
  it("sorts by a column when its heading is clicked, and says so on hover", async () => {
    const { onSort } = renderTable();
    expect(screen.getByRole("button", { name: tx("trackTable:columns.bpm") })).toHaveAccessibleDescription(
      tx("trackTable:sortBy", { column: tx("trackTable:columns.bpm") }),
    );
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:columns.bpm") }));
    expect(onSort).toHaveBeenCalledWith("bpm");
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:columns.dateAdded") }));
    expect(onSort).toHaveBeenLastCalledWith("dateAdded");
  });

  it("marks the sorted column, and which way, and no other", () => {
    const sort: TableSort = { column: "artist", descending: true };
    renderTable({ sort });
    const headers = screen.getAllByRole("columnheader");
    expect(headers.map((h) => h.getAttribute("aria-sort"))).toEqual([
      null,
      "descending",
      ...Array<null>(8).fill(null),
    ]);
  });

  it("reads an ascending column as ascending", () => {
    renderTable({ sort: { column: "title", descending: false } });
    expect(screen.getAllByRole("columnheader")[0]).toHaveAttribute("aria-sort", "ascending");
  });

  it("can be sorted from the keyboard: the headings are buttons", async () => {
    const { onSort } = renderTable();
    const heading = screen.getByRole("button", { name: tx("trackTable:columns.genre") });
    heading.focus();
    await userEvent.keyboard("{Enter}");
    expect(onSort).toHaveBeenCalledWith("genre");
  });
});

describe("selecting a row", () => {
  it("selects the row that is clicked, and only that one", async () => {
    const onSelectionChange = vi.fn();
    renderTable({ onSelectionChange });
    await userEvent.click(drawn()[2]);
    expect(drawn().map((r) => r.getAttribute("aria-selected"))).toSatisfy((states: (string | null)[]) =>
      states.every((state, i) => (i === 2 ? state === "true" : state === "false")),
    );
    expect(onSelectionChange).toHaveBeenLastCalledWith(expect.objectContaining({ title: "Tune 2" }));
    await userEvent.click(drawn()[4]);
    expect(drawn()[2]).toHaveAttribute("aria-selected", "false");
    expect(drawn()[4]).toHaveAttribute("aria-selected", "true");
    expect(onSelectionChange).toHaveBeenLastCalledWith(expect.objectContaining({ title: "Tune 4" }));
  });

  it("does not select a row because a track is playing", () => {
    renderTable();
    expect(drawn().some((r) => r.getAttribute("aria-selected") === "true")).toBe(false);
  });
});

describe("the keyboard", () => {
  const focused = () => {
    const grid = screen.getByRole("grid");
    grid.focus();
    return grid;
  };
  const selected = () => drawn().filter((r) => r.getAttribute("aria-selected") === "true").map(indexOf);

  it("is one tab stop that takes the focus", async () => {
    renderTable();
    await userEvent.tab();
    expect(screen.getByRole("grid")).toHaveFocus();
  });

  it("selects the first row on Down, then moves a row at a time", async () => {
    renderTable();
    const grid = focused();
    await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([0]);
    await userEvent.keyboard("{ArrowDown}{ArrowDown}");
    expect(selected()).toEqual([2]);
    await userEvent.keyboard("{ArrowUp}");
    expect(selected()).toEqual([1]);
    expect(grid).toHaveAttribute("aria-activedescendant", drawn()[1].id);
  });

  it("stops at the first and the last row", async () => {
    renderTable({ total: 5 });
    focused();
    await userEvent.keyboard("{ArrowDown}{ArrowUp}{ArrowUp}");
    expect(selected()).toEqual([0]);
    await userEvent.keyboard("{End}");
    expect(selected()).toEqual([4]);
    await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([4]);
    await userEvent.keyboard("{Home}");
    expect(selected()).toEqual([0]);
  });

  it("jumps a page at a time, and to the very end of a long list", async () => {
    renderTable({ total: 100_000 });
    focused();
    await userEvent.keyboard("{ArrowDown}{PageDown}");
    const afterPage = selected();
    expect(afterPage).toHaveLength(1);
    expect(afterPage[0]).toBeGreaterThanOrEqual(9);
    expect(afterPage[0]).toBeLessThanOrEqual(11);
    await userEvent.keyboard("{End}");
    expect(selected()).toEqual([99_999]);
    // The row is brought into view, and the rows drawn stay few.
    expect(drawn().length).toBeLessThan(40);
    expect(screen.getByText("Tune 99999")).toBeInTheDocument();
    await userEvent.keyboard("{PageUp}");
    expect(selected()[0]).toBeLessThan(99_999);
  });

  it("scrolls to keep the selected row on screen", async () => {
    renderTable({ total: 1000 });
    const grid = focused();
    for (let i = 0; i < 25; i++) await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([24]);
    // 25 rows have gone by, so the list has scrolled.
    expect(grid.scrollTop).toBeGreaterThan(0);
    expect(screen.getByText("Tune 24")).toBeInTheDocument();
  });

  it("leaves the keys of a button inside a row to the button", async () => {
    renderTable({ actions: () => <button type="button">Act</button> });
    focused();
    await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([0]);
    within(drawn()[3]).getByRole("button", { name: "Act" }).focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([0]);
  });

  it("does nothing with a modifier held, and nothing for other keys", async () => {
    renderTable();
    focused();
    await userEvent.keyboard("{Control>}{ArrowDown}{/Control}");
    await userEvent.keyboard("a");
    expect(selected()).toEqual([]);
  });
});

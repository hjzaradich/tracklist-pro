import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import "../i18n";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { WIDEST } from "./columnChoice";
import type { ColumnId } from "./columns";
import { TrackTable } from "./TrackTable";

// Dragging the headings of the track table (1cB-21): reorder, and resize.
// Pointer events by hand, so the tests fire them by hand. jsdom has no
// layout: a test that needs to know where the headings are says so.

const COLUMNS: ColumnId[] = ["title", "artist", "bpm"];
/** How wide `place` makes each heading, in pixels. */
const WIDE = 100;

beforeEach(() => {
  mockIPC((cmd) => {
    if (cmd === "key_notation") return "camelot";
    throw new Error(`unexpected command ${cmd}`);
  });
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(28 * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

function renderTable(props: Partial<React.ComponentProps<typeof TrackTable>> = {}) {
  const onSort = vi.fn();
  const onReorder = vi.fn();
  const onResize = vi.fn();
  render(
    <QueryClientProvider client={createQueryClient()}>
      <TrackTable
        label="Under test"
        columns={COLUMNS}
        total={5}
        getRow={(i) => tableRow(i, { title: `Tune ${i}` })}
        onRangeChange={() => {}}
        sort={null}
        onSort={onSort}
        onReorder={onReorder}
        onResize={onResize}
        resetKey="a"
        {...props}
      />
    </QueryClientProvider>,
  );
  return { onSort, onReorder, onResize };
}

/** Puts the headings side by side, `WIDE` pixels each, as a browser would. */
function place() {
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
}

const heading = (column: ColumnId) =>
  screen.getByRole("button", { name: tx(`trackTable:columns.${column}`) });
const line = (column: ColumnId) =>
  screen.getByRole("separator", { name: tx("trackTable:resize", { column: tx(`trackTable:columns.${column}`) }) });
const template = () => screen.getByRole("grid").parentElement!.style.getPropertyValue("--tt-columns");

/** A press, a move and a release: a drag from `from` to `to` (x, in pixels). */
function drag(element: HTMLElement, from: number, to: number) {
  fireEvent.pointerDown(element, { clientX: from, button: 0, pointerId: 1 });
  fireEvent.pointerMove(element, { clientX: to, pointerId: 1 });
  fireEvent.pointerUp(element, { clientX: to, pointerId: 1 });
}

describe("dragging a heading", () => {
  it("moves the column to where it is dropped, and hands over the columns in the new order", () => {
    const { onReorder } = renderTable();
    place();
    // Artist (100..200) dropped past the middle of BPM (200..300).
    drag(heading("artist"), 150, 270);
    expect(onReorder).toHaveBeenCalledExactlyOnceWith(["title", "bpm", "artist"]);
  });

  it("moves a column to the front when it is dropped before the middle of the first", () => {
    const { onReorder } = renderTable();
    place();
    drag(heading("bpm"), 250, 20);
    expect(onReorder).toHaveBeenCalledExactlyOnceWith(["bpm", "title", "artist"]);
  });

  it("changes nothing when the heading is dropped where it was", () => {
    const { onReorder, onSort } = renderTable();
    place();
    drag(heading("artist"), 150, 190);
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSort).not.toHaveBeenCalled();
  });

  it("shows where the heading would go while it is dragged, and takes it away at the end", async () => {
    renderTable();
    place();
    const artist = heading("artist");
    fireEvent.pointerDown(artist, { clientX: 150, button: 0, pointerId: 1 });
    fireEvent.pointerMove(artist, { clientX: 290, pointerId: 1 });
    const cell = screen.getAllByRole("columnheader")[1];
    await waitFor(() => expect(cell.style.transform).toBe("translateX(140px)"));
    fireEvent.pointerUp(artist, { clientX: 290, pointerId: 1 });
    expect(cell.style.transform).toBe("");
  });

  it("is given up with Escape: nothing moves, and the click that ends it does not sort", () => {
    const { onReorder, onSort } = renderTable();
    place();
    const artist = heading("artist");
    fireEvent.pointerDown(artist, { clientX: 150, button: 0, pointerId: 1 });
    fireEvent.pointerMove(artist, { clientX: 290, pointerId: 1 });
    fireEvent.keyDown(artist, { key: "Escape" });
    fireEvent.pointerUp(artist, { clientX: 290, pointerId: 1 });
    fireEvent.click(artist);
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSort).not.toHaveBeenCalled();
    expect(screen.getAllByRole("columnheader")[1].style.transform).toBe("");
  });

  it("is given up with Escape even if the button is let go much later", async () => {
    const { onSort } = renderTable();
    place();
    const artist = heading("artist");
    fireEvent.pointerDown(artist, { clientX: 150, button: 0, pointerId: 1 });
    fireEvent.pointerMove(artist, { clientX: 290, pointerId: 1 });
    fireEvent.keyDown(artist, { key: "Escape" });
    await new Promise((done) => setTimeout(done, 5));
    fireEvent.pointerUp(artist, { clientX: 290, pointerId: 1 });
    fireEvent.click(artist);
    expect(onSort).not.toHaveBeenCalled();
    // The next click is a click again.
    await new Promise((done) => setTimeout(done, 5));
    await userEvent.click(artist);
    expect(onSort).toHaveBeenCalledExactlyOnceWith("artist");
  });

  it("is given up with Escape wherever the focus is", () => {
    const { onReorder, onSort } = renderTable();
    place();
    const artist = heading("artist");
    fireEvent.pointerDown(artist, { clientX: 150, button: 0, pointerId: 1 });
    fireEvent.pointerMove(artist, { clientX: 290, pointerId: 1 });
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.pointerUp(artist, { clientX: 290, pointerId: 1 });
    fireEvent.click(artist);
    expect(onReorder).not.toHaveBeenCalled();
    expect(onSort).not.toHaveBeenCalled();
  });

  it("cannot be dragged when the table was not given a way to save the order", () => {
    const { onSort } = renderTable({ onReorder: undefined });
    place();
    drag(heading("artist"), 150, 270);
    fireEvent.click(heading("artist"));
    expect(onSort).toHaveBeenCalledWith("artist");
  });
});

describe("a click on a heading and a drag", () => {
  it("a click still sorts", async () => {
    const { onSort, onReorder } = renderTable();
    await userEvent.click(heading("bpm"));
    expect(onSort).toHaveBeenCalledExactlyOnceWith("bpm");
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("a press that hardly moves is still a click", () => {
    const { onSort, onReorder } = renderTable();
    place();
    const bpm = heading("bpm");
    drag(bpm, 250, 252);
    fireEvent.click(bpm);
    expect(onSort).toHaveBeenCalledExactlyOnceWith("bpm");
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("a drag does not sort when it ends", () => {
    const { onSort, onReorder } = renderTable();
    place();
    const artist = heading("artist");
    drag(artist, 150, 270);
    // The browser sends the click that ends a drag.
    fireEvent.click(artist);
    expect(onReorder).toHaveBeenCalledOnce();
    expect(onSort).not.toHaveBeenCalled();
  });

  it("sorts again with the next click after a drag", async () => {
    const { onSort } = renderTable();
    place();
    const artist = heading("artist");
    drag(artist, 150, 270);
    fireEvent.click(artist);
    // The next click comes later than the end of the drag.
    await new Promise((done) => setTimeout(done, 5));
    await userEvent.click(artist);
    expect(onSort).toHaveBeenCalledExactlyOnceWith("artist");
  });
});

describe("dragging the line after a heading", () => {
  it("changes the column's width by how far the line was dragged, and hands over all the widths", () => {
    const { onResize } = renderTable({ widths: { title: 200 } });
    // BPM starts at its own width, 54 pixels.
    drag(line("bpm"), 300, 340);
    expect(onResize).toHaveBeenCalledExactlyOnceWith({ title: 200, bpm: 94 });
  });

  it("starts from the width the column has on screen", () => {
    const { onResize } = renderTable();
    place();
    drag(line("artist"), 200, 230);
    expect(onResize).toHaveBeenCalledExactlyOnceWith({ artist: WIDE + 30 });
  });

  it("never makes a column narrower than its minimum, however far the line is dragged", () => {
    const { onResize } = renderTable();
    drag(line("bpm"), 300, -900);
    expect(onResize).toHaveBeenCalledExactlyOnceWith({ bpm: 44 });
  });

  it("never makes a column wider than the widest there is", () => {
    const { onResize } = renderTable();
    drag(line("bpm"), 300, 90_000);
    expect(onResize).toHaveBeenCalledExactlyOnceWith({ bpm: WIDEST });
  });

  it("shows the width as the line moves, and saves it only at the end", async () => {
    const { onResize } = renderTable();
    const before = template();
    const bpm = line("bpm");
    fireEvent.pointerDown(bpm, { clientX: 300, button: 0, pointerId: 1 });
    fireEvent.pointerMove(bpm, { clientX: 330, pointerId: 1 });
    await waitFor(() => expect(template()).toBe(before.replace(/54px$/, "84px")));
    expect(onResize).not.toHaveBeenCalled();
    fireEvent.pointerUp(bpm, { clientX: 330, pointerId: 1 });
    expect(onResize).toHaveBeenCalledExactlyOnceWith({ bpm: 84 });
    expect(template()).toBe(before.replace(/54px$/, "84px"));
  });

  it("puts the width back when the drag is cancelled", async () => {
    const { onResize } = renderTable();
    const before = template();
    const bpm = line("bpm");
    fireEvent.pointerDown(bpm, { clientX: 300, button: 0, pointerId: 1 });
    fireEvent.pointerMove(bpm, { clientX: 330, pointerId: 1 });
    await waitFor(() => expect(template()).not.toBe(before));
    fireEvent.pointerCancel(bpm, { pointerId: 1 });
    expect(template()).toBe(before);
    expect(onResize).not.toHaveBeenCalled();
  });

  it("is given up with Escape: the width goes back and nothing is saved", async () => {
    const { onResize } = renderTable();
    const before = template();
    const bpm = line("bpm");
    fireEvent.pointerDown(bpm, { clientX: 300, button: 0, pointerId: 1 });
    fireEvent.pointerMove(bpm, { clientX: 330, pointerId: 1 });
    await waitFor(() => expect(template()).not.toBe(before));
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.pointerUp(bpm, { clientX: 330, pointerId: 1 });
    expect(template()).toBe(before);
    expect(onResize).not.toHaveBeenCalled();
  });

  it("does not sort and does not move the column", () => {
    const { onSort, onReorder } = renderTable();
    drag(line("artist"), 200, 260);
    expect(onSort).not.toHaveBeenCalled();
    expect(onReorder).not.toHaveBeenCalled();
  });

  it("draws a column at the width it was given, and the others as they were", () => {
    renderTable({ widths: { artist: 321 } });
    expect(template()).toBe("minmax(132px, 3fr) 321px 54px");
  });

  it("cannot be done when the table was not given a way to save the widths", () => {
    renderTable({ onResize: undefined });
    expect(screen.queryByRole("separator")).not.toBeInTheDocument();
  });
});

describe("the keyboard and the line after a heading", () => {
  it("gives each line a name, and a place in the Tab order", () => {
    renderTable();
    expect(screen.getAllByRole("separator")).toHaveLength(COLUMNS.length);
    expect(line("artist")).toHaveAttribute("tabindex", "0");
    expect(line("bpm")).toHaveAttribute("aria-valuenow", "54");
    expect(line("bpm")).toHaveAttribute("aria-valuemin", "44");
  });

  it("makes the column wider with the Right arrow key and narrower with the Left", () => {
    const { onResize } = renderTable({ widths: { bpm: 100 } });
    line("bpm").focus();
    fireEvent.keyDown(line("bpm"), { key: "ArrowRight" });
    expect(onResize).toHaveBeenLastCalledWith({ bpm: 110 });
    fireEvent.keyDown(line("bpm"), { key: "ArrowLeft" });
    expect(onResize).toHaveBeenLastCalledWith({ bpm: 90 });
  });

  it("stops at the minimum", () => {
    const { onResize } = renderTable({ widths: { bpm: 44 } });
    fireEvent.keyDown(line("bpm"), { key: "ArrowLeft" });
    expect(onResize).not.toHaveBeenCalled();
  });

  it("leaves the other keys, and the table's own arrow keys, alone", async () => {
    const { onResize } = renderTable();
    line("bpm").focus();
    await userEvent.keyboard("{ArrowDown}{Enter}x");
    expect(onResize).not.toHaveBeenCalled();
    expect(screen.queryAllByRole("row", { selected: true })).toHaveLength(0);
  });
});

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import type { LibraryTrack, OperationInfo, TablePage, TableQuery } from "../bindings";
import { usePlayer } from "../player/playerStore";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";

const ROW = 28;
/** More than two windows of rows (200 each), so some are never held. */
const TOTAL = 450;
/** How many rows the list has; a test may make it longer. */
let total = TOTAL;

/** The window of the list that `query` asks for. */
function windowOf(query: TableQuery): TablePage {
  const count = Math.max(0, Math.min(query.limit, total - query.offset));
  return {
    total: query.withTotal ? total : null,
    offset: query.offset,
    rows: Array.from({ length: count }, (_, k) => tableRow(query.offset + k, { title: `Tune ${query.offset + k}` })),
    behind: false,
  };
}
/** The track id of the row at list position `index`. */
const idAt = (index: number) => index + 100;

function libraryTrack(id: number): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: null,
    file: { id: 1, path: String.raw`E:\Music\tune.mp3`, name: "tune.mp3", present: true, driveConnected: true },
    fragile: null,
    sourceMissing: false,
    openConflicts: 0,
    addedAt: "2026-10-01T10:00:00.000Z",
    betterFile: null,
  };
}

/** Stands in for the Rust side: the track table, the energy each track was given, and Undo. */
function fakeBackend() {
  const backend = {
    /** The ids track_table_ids was asked for, as [first, last]. */
    idsAsked: [] as [number, number][],
    /** What set_energy was asked, in order. */
    energyAsked: [] as { recordings: number[]; energy: number }[],
    energy: new Map<number, number>(),
    undone: 0,
    history: [] as { before: Map<number, number>; tracks: number }[],
  };
  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "library_tracks":
          return Array.from({ length: 3 }, (_, n) => libraryTrack(n + 1));
        case "track_table_page":
          return windowOf(args.query as TableQuery);
        case "track_table_ids": {
          const query = args.query as TableQuery;
          backend.idsAsked.push([query.offset, query.offset + query.limit - 1]);
          return windowOf({ ...query, limit: query.limit }).rows.map((row) => row.recordingId);
        }
        case "set_energy": {
          const { recordings, energy } = args as { recordings: number[]; energy: number };
          backend.energyAsked.push({ recordings, energy });
          // As the backend does: a track already at that energy is left alone,
          // and when none changes there is no operation.
          const changing = recordings.filter((id) => backend.energy.get(id) !== energy);
          if (changing.length > 0) backend.history.push({ before: new Map(backend.energy), tracks: changing.length });
          for (const id of changing) backend.energy.set(id, energy);
          return {
            changed: changing.length,
            skipped: recordings.length - changing.length,
            operationId: changing.length > 0 ? backend.history.length : null,
          };
        }
        case "next_undo_operation": {
          const last = backend.history.at(-1);
          const operation: OperationInfo | null = last
            ? { id: backend.history.length, kind: "set_energy", details: { name: null, from: null, tracks: last.tracks } }
            : null;
          return { operation, refusal: null };
        }
        case "undo_last_operation": {
          const last = backend.history.pop();
          if (last) backend.energy = last.before;
          backend.undone += 1;
          return {
            status: "undone",
            operation: { id: 1, kind: "set_energy", details: { name: null, from: null, tracks: last?.tracks ?? 0 } },
          };
        }
        case "send_state":
          return {
            revision: 0,
            filePath: String.raw`C:\data\send.xml`,
            exportPath: null,
            preflight: null,
            step: null,
            failure: null,
            sent: null,
          };
        case "rekordbox_offer":
          return { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
        case "rekordbox_xml_source":
          return { path: null, watch: false, lastRead: null, lastFailure: null, newerExport: null, exportFolder: null };
        case "list_crates":
        case "music_folders":
          return [];
        case "read_online_only_files":
          return false;
        case "key_notation":
          return "camelot";
        case "table_columns":
          return null;
        case "track_table_position":
          return null;
        default:
          throw { kind: "internal", params: {} };
      }
    },
    { shouldMockEvents: true },
  );
  return backend;
}

beforeEach(() => {
  total = TOTAL;
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  usePlayer.setState({ file: null });
  document.body.innerHTML = "";
});

const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const indexOf = (row: HTMLElement) => Number(row.getAttribute("aria-rowindex")) - 2;
const rowAt = (index: number) => {
  const found = rows().find((row) => indexOf(row) === index);
  if (!found) throw new Error(`row ${index} is not drawn`);
  return found;
};
const selected = () =>
  rows()
    .filter((row) => row.getAttribute("aria-selected") === "true")
    .map(indexOf)
    .sort((a, b) => a - b);

async function library() {
  const backend = fakeBackend();
  renderApp("/library");
  await screen.findByRole("grid");
  await waitFor(() => expect(rows().length).toBeGreaterThan(5));
  return backend;
}

const clickRow = (index: number, modifiers: { shiftKey?: boolean; ctrlKey?: boolean } = {}) =>
  fireEvent.click(rowAt(index), modifiers);

const optionNames = () =>
  within(screen.getByRole("listbox", { name: tx("palette:list.label") }))
    .queryAllByRole("option")
    .map((option) => option.textContent);

async function openPalette() {
  await userEvent.keyboard("{Control>}k{/Control}");
  return screen.findByRole("combobox", { name: tx("palette:input.label") });
}

const selectedText = (count: number) => tx("trackTable:selected", { selected: count });

describe("selecting many tracks", () => {
  it("a click selects one row, and Shift+click selects the rows from the last click to this one", async () => {
    await library();
    clickRow(2);
    expect(selected()).toEqual([2]);
    clickRow(6, { shiftKey: true });
    expect(selected()).toEqual([2, 3, 4, 5, 6]);
    // Backwards from the same anchor.
    clickRow(0, { shiftKey: true });
    expect(selected()).toEqual([0, 1, 2]);
    // A click without Shift starts again.
    clickRow(4);
    expect(selected()).toEqual([4]);
  });

  it("Ctrl+click adds a row to the selection and takes a selected row out of it", async () => {
    await library();
    clickRow(1);
    clickRow(3, { ctrlKey: true });
    clickRow(5, { ctrlKey: true });
    expect(selected()).toEqual([1, 3, 5]);
    clickRow(3, { ctrlKey: true });
    expect(selected()).toEqual([1, 5]);
    // A row taken out is not tinted even though it has the focus.
    expect(rowAt(3)).toHaveAttribute("aria-selected", "false");
  });

  it("Ctrl+A selects every track in the list, those not on screen too, and Escape clears the selection", async () => {
    const backend = await library();
    expect(rows().length).toBeLessThan(40);
    await userEvent.keyboard("{Control>}a{/Control}");
    expect(await screen.findByText(selectedText(TOTAL))).toBeInTheDocument();
    expect(backend.idsAsked).toEqual([[0, TOTAL - 1]]);
    expect(selected().length).toBe(rows().length);

    await userEvent.keyboard("{Escape}");
    expect(selected()).toEqual([]);
    expect(screen.queryByText(selectedText(TOTAL))).toBeNull();
  });

  it("a cleared selection leaves no row tinted, and J carries on from the row that had the focus", async () => {
    await library();
    clickRow(2);
    clickRow(4, { shiftKey: true });
    await userEvent.keyboard("{Escape}");
    expect(selected()).toEqual([]);
    await userEvent.keyboard("j");
    expect(selected()).toEqual([5]);
  });

  it("says how many tracks are selected when there are several", async () => {
    await library();
    clickRow(1);
    expect(screen.queryByText(selectedText(1))).toBeNull();
    clickRow(4, { shiftKey: true });
    expect(screen.getByText(selectedText(4))).toBeInTheDocument();
  });
});

describe("J and K", () => {
  it("move the focus down and up, selecting that row alone", async () => {
    await library();
    clickRow(3);
    await userEvent.keyboard("j");
    expect(selected()).toEqual([4]);
    await userEvent.keyboard("jj");
    expect(selected()).toEqual([6]);
    await userEvent.keyboard("k");
    expect(selected()).toEqual([5]);
  });

  it("Escape starts a Shift range again from the focused row", async () => {
    await library();
    clickRow(2);
    clickRow(10, { shiftKey: true });
    await userEvent.keyboard("{Escape}");
    fireEvent.keyDown(document.body, { key: "J", shiftKey: true });
    expect(selected()).toEqual([10, 11]);
  });

  it("start at the top of the screen when nothing has the focus, and stop at the ends", async () => {
    await library();
    await userEvent.keyboard("j");
    expect(selected()).toEqual([0]);
    await userEvent.keyboard("k");
    expect(selected()).toEqual([0]);
  });

  it("with Shift extend the selection from where it started, down and back up", async () => {
    await library();
    clickRow(2);
    fireEvent.keyDown(document.body, { key: "J", shiftKey: true });
    fireEvent.keyDown(document.body, { key: "J", shiftKey: true });
    expect(selected()).toEqual([2, 3, 4]);
    fireEvent.keyDown(document.body, { key: "K", shiftKey: true });
    expect(selected()).toEqual([2, 3]);
    // Past the anchor the other way round.
    fireEvent.keyDown(document.body, { key: "K", shiftKey: true });
    fireEvent.keyDown(document.body, { key: "K", shiftKey: true });
    expect(selected()).toEqual([1, 2]);
  });
});

describe("Escape meant for something else", () => {
  it("closing the Activity popover with Escape leaves the selection alone", async () => {
    await library();
    clickRow(3);
    const status = screen.getByRole("status", { name: tx("activity:label") });
    await userEvent.click(status);
    expect(status.closest("button")).toHaveAttribute("aria-expanded", "true");
    await userEvent.keyboard("{Escape}");
    expect(status.closest("button")).toHaveAttribute("aria-expanded", "false");
    expect(selected()).toEqual([3]);
    // With nothing else to close, Escape clears it as before.
    await userEvent.keyboard("{Escape}");
    expect(selected()).toEqual([]);
  });
});

describe("the energy keys", () => {
  it("1 to 0 give every selected track the energy as one operation, and one Undo restores all of them", async () => {
    const backend = await library();
    backend.energy.set(idAt(2), 4);
    const before = new Map(backend.energy);
    clickRow(2);
    clickRow(5, { shiftKey: true });
    expect(selected()).toEqual([2, 3, 4, 5]);

    await userEvent.keyboard("7");
    expect(backend.energyAsked).toEqual([{ recordings: [2, 3, 4, 5].map(idAt), energy: 7 }]);
    expect([...backend.energy.entries()].sort()).toEqual([2, 3, 4, 5].map((i) => [idAt(i), 7]));
    expect(await screen.findByText(tx("trackTable:energySet", { energy: 7, count: 4 }))).toBeInTheDocument();

    // The top bar's Undo names the step, and is the only Undo.
    const undo = await screen.findByRole("button", { name: tx("shell:undo.action.setEnergy", { count: 4 }) });
    await userEvent.click(undo);
    await waitFor(() => expect(backend.undone).toBe(1));
    expect(backend.energy).toEqual(before);
    expect(backend.energyAsked).toHaveLength(1);
  });

  it("says how many tracks changed, not how many were asked, and says nothing when none did", async () => {
    const backend = await library();
    backend.energy.set(idAt(3), 7);
    backend.energy.set(idAt(4), 7);
    clickRow(2);
    clickRow(5, { shiftKey: true });
    await userEvent.keyboard("7");
    expect(await screen.findByText(tx("trackTable:energySet", { energy: 7, count: 2 }))).toBeInTheDocument();
    expect(screen.queryByText(tx("trackTable:energySet", { energy: 7, count: 4 }))).toBeNull();

    // Every one of them is at 7 now: nothing changes, and the line goes.
    await userEvent.keyboard("7");
    await waitFor(() => expect(backend.energyAsked).toHaveLength(2));
    for (const count of [1, 2, 3, 4]) {
      expect(screen.queryByText(tx("trackTable:energySet", { energy: 7, count }))).toBeNull();
    }
    expect(backend.history).toHaveLength(1);
  });

  it("0 gives energy 10, and 1 to 9 give themselves", async () => {
    const backend = await library();
    clickRow(0);
    for (const key of ["1", "9", "0"]) await userEvent.keyboard(key);
    expect(backend.energyAsked.map((asked) => asked.energy)).toEqual([1, 9, 10]);
  });

  it("do nothing while no track is selected", async () => {
    const backend = await library();
    await userEvent.keyboard("5");
    clickRow(1);
    await userEvent.keyboard("{Escape}5");
    expect(backend.energyAsked).toEqual([]);
  });

  it("apply to a selection of the whole list", async () => {
    const backend = await library();
    await userEvent.keyboard("{Control>}a{/Control}");
    await screen.findByText(selectedText(TOTAL));
    await userEvent.keyboard("3");
    await waitFor(() => expect(backend.energyAsked).toHaveLength(1));
    expect(backend.energyAsked[0].recordings).toHaveLength(TOTAL);
    expect(backend.energyAsked[0].recordings.slice(0, 3)).toEqual([idAt(0), idAt(1), idAt(2)]);
  });
});

describe("when the keys stand aside", () => {
  it("ignores every key typed in a text field", async () => {
    const backend = await library();
    clickRow(1);
    const box = document.createElement("input");
    box.type = "text";
    document.body.append(box);
    box.focus();
    await userEvent.keyboard("j7t {Control>}a{/Control}{Escape}");
    expect(backend.energyAsked).toEqual([]);
    expect(backend.idsAsked).toEqual([]);
    expect(selected()).toEqual([1]);
    expect(box).toHaveValue("j7t ");
  });

  it("ignores every key while a dialog is open", async () => {
    const backend = await library();
    clickRow(1);
    const dialog = document.createElement("div");
    dialog.setAttribute("role", "alertdialog");
    document.body.append(dialog);
    await userEvent.keyboard("j7{Control>}a{/Control}{Escape}");
    expect(backend.energyAsked).toEqual([]);
    expect(selected()).toEqual([1]);
    dialog.remove();
    await userEvent.keyboard("j");
    expect(selected()).toEqual([2]);
  });

  it("ignores every key while the command palette is open", async () => {
    const backend = await library();
    clickRow(1);
    await openPalette();
    fireEvent.keyDown(document.body, { key: "j" });
    fireEvent.keyDown(document.body, { key: "7" });
    fireEvent.keyDown(document.body, { key: "a", ctrlKey: true });
    expect(backend.energyAsked).toEqual([]);
    expect(selected()).toEqual([1]);
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull());
    await userEvent.keyboard("j");
    expect(selected()).toEqual([2]);
  });
});

describe("the command palette lists the keys", () => {
  it("shows the movement and selection actions with their shortcuts", async () => {
    await library();
    clickRow(1);
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.next"));
    expect(optionNames()).toContain(tx("trackTable:keys.next") + tx("trackTable:keys.shortcut.next"));
    await userEvent.clear(screen.getByRole("combobox", { name: tx("palette:input.label") }));
    await userEvent.keyboard(tx("trackTable:keys.selectAll"));
    expect(optionNames()).toEqual([tx("trackTable:keys.selectAll") + tx("trackTable:keys.shortcut.selectAll")]);
    await userEvent.clear(screen.getByRole("combobox", { name: tx("palette:input.label") }));
    await userEvent.keyboard(tx("trackTable:keys.extendPrevious"));
    expect(optionNames()).toEqual([
      tx("trackTable:keys.extendPrevious") + tx("trackTable:keys.shortcut.extendPrevious"),
    ]);
  });

  it("lists the energy actions, with the digits as shortcuts, only while a track is selected, and runs one", async () => {
    const backend = await library();
    await openPalette();
    await userEvent.keyboard("energy");
    expect(optionNames()).toEqual([]);
    await userEvent.keyboard("{Escape}");

    clickRow(3);
    clickRow(4, { shiftKey: true });
    await openPalette();
    await userEvent.keyboard("energy");
    expect(optionNames()).toEqual(
      [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((energy) => tx("trackTable:keys.energy", { energy }) + String(energy % 10)),
    );
    await userEvent.clear(screen.getByRole("combobox", { name: tx("palette:input.label") }));
    await userEvent.keyboard(tx("trackTable:keys.energy", { energy: 6 }) + "{Enter}");
    await waitFor(() => expect(backend.energyAsked).toEqual([{ recordings: [3, 4].map(idAt), energy: 6 }]));
  });

  it("lists Clear selection only while a track is selected, and runs it", async () => {
    await library();
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.clearSelection"));
    expect(optionNames()).toEqual([]);
    await userEvent.keyboard("{Escape}");

    clickRow(2);
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.clearSelection"));
    expect(optionNames()).toEqual([
      tx("trackTable:keys.clearSelection") + tx("trackTable:keys.shortcut.clearSelection"),
    ]);
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(selected()).toEqual([]));
  });
});

describe("Space and T", () => {
  it("Space is hidden in the palette and does nothing while no file is loaded in the player", async () => {
    await library();
    const toggle = vi.fn();
    usePlayer.setState({ file: null, toggle });
    clickRow(1);
    await userEvent.keyboard(" ");
    expect(toggle).not.toHaveBeenCalled();
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.playPause"));
    expect(optionNames()).toEqual([]);
  });

  it("Space plays and pauses once a file is loaded, and is listed with its shortcut", async () => {
    await library();
    const toggle = vi.fn();
    usePlayer.setState({ file: { id: 1, name: "tune.mp3" }, toggle });
    await userEvent.keyboard(" ");
    expect(toggle).toHaveBeenCalledTimes(1);
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.playPause"));
    expect(optionNames()).toEqual([tx("trackTable:keys.playPause") + tx("trackTable:keys.shortcut.playPause")]);
  });

  it("Space leaves a button that has the focus to press it", async () => {
    await library();
    const toggle = vi.fn();
    usePlayer.setState({ file: { id: 1, name: "tune.mp3" }, toggle });
    const button = document.createElement("button");
    document.body.append(button);
    button.focus();
    await userEvent.keyboard(" ");
    expect(toggle).not.toHaveBeenCalled();
  });

  it("T is not listed and does nothing until there are tags to apply", async () => {
    const backend = await library();
    clickRow(1);
    await userEvent.keyboard("t");
    expect(backend.energyAsked).toEqual([]);
    expect(selected()).toEqual([1]);
    await openPalette();
    await userEvent.keyboard(tx("trackTable:keys.tag"));
    expect(optionNames()).toEqual([]);
  });
});

// The rows drawn are a few of the list (a render of 450 rows would say nothing more).
describe("the table of a list longer than a window", () => {
  it("selects a range across rows it has not been given with the ids from the backend", async () => {
    total = 1_000;
    const backend = await library();
    clickRow(0);
    // Scroll far down, to rows whose window hasn't arrived, and Shift+click there.
    const grid = screen.getByRole("grid");
    grid.scrollTop = 700 * ROW;
    fireEvent.scroll(grid);
    await waitFor(() => expect(rows().some((row) => indexOf(row) === 700 && !row.hasAttribute("aria-busy"))).toBe(true));
    clickRow(700, { shiftKey: true });
    await screen.findByText(selectedText(701));
    expect(backend.idsAsked.at(-1)).toEqual([0, 700]);
  });
});


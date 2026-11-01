import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import {
  cleanup,
  fireEvent,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import type { EnergySource, TablePage, TableQuery } from "../bindings";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { ALL_COLUMNS } from "./columnChoice";

const escapeRegExp = (text: string) =>
  text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const ROW = 28;
const TOTAL = 60;
const idAt = (index: number) => index + 100;

/** Stands in for the Rust side: a table whose tracks have the energy in `energy`, and set_energy. */
let energy: Map<number, { energy: number; source: EnergySource }>;
let asked: { recordings: number[]; energy: number }[];
/** The columns the user chose: Title and Energy, or nothing chosen. */
let chosen: string[] | null;

function windowOf(query: TableQuery): TablePage {
  const count = Math.max(0, Math.min(query.limit, TOTAL - query.offset));
  return {
    total: query.withTotal ? TOTAL : null,
    offset: query.offset,
    rows: Array.from({ length: count }, (_, k) => {
      const index = query.offset + k;
      const held = energy.get(idAt(index));
      return tableRow(index, {
        title: `Tune ${index}`,
        energy: held?.energy ?? null,
        energySource: held?.source ?? null,
      });
    }),
    behind: false,
  };
}

beforeEach(() => {
  energy = new Map([
    [idAt(1), { energy: 7, source: "user" }],
    [idAt(2), { energy: 5, source: "mik" }],
  ]);
  asked = [];
  chosen = ["title", "energy"];
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(
    ROW * 11,
  );
  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      switch (cmd) {
        case "library_summary":
          return { tracks: 3, changeStamp: 3 };
        case "track_table_page":
          return windowOf(args.query as TableQuery);
        case "track_table_ids": {
          const query = args.query as TableQuery;
          return windowOf(query).rows.map((row) => row.recordingId);
        }
        case "set_energy": {
          const { recordings, energy: given } = args as {
            recordings: number[];
            energy: number;
          };
          asked.push({ recordings, energy: given });
          for (const id of recordings)
            energy.set(id, { energy: given, source: "user" });
          return {
            changed: recordings.length,
            skipped: 0,
            operationId: asked.length,
          };
        }
        case "table_columns":
          return chosen === null
            ? null
            : { shown: chosen, known: [...ALL_COLUMNS], widths: {} };
        case "next_undo_operation":
          return { operation: null, refusal: null };
        case "key_notation":
          return "camelot";
        case "list_crates":
        case "music_folders":
        case "crates_of_track":
          return [];
        case "track_tag_sources":
          return { inRekordbox: false, fields: [], confirmedEntry: null };
        case "track_table_position":
          return null;
        case "read_online_only_files":
          return false;
        case "send_state":
          return {
            revision: 0,
            filePath: String.raw`C:\data\send.xml`,
            exportPath: null,
            preflight: null,
            step: null,
            workingOn: null,
            failure: null,
            sent: null,
          };
        case "rekordbox_offer":
          return {
            toAdd: 0,
            alreadyInLibrary: 0,
            waitingInMissing: 0,
            waitingForConfirmation: 0,
          };
        case "rekordbox_xml_source":
          return {
            path: null,
            watch: false,
            lastRead: null,
            lastFailure: null,
            newerExport: null,
            exportFolder: null,
          };
        default:
          throw { kind: "internal", params: {} };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  cleanup();
  clearMocks();
  document.body.innerHTML = "";
});

const rows = () =>
  within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const indexOf = (row: HTMLElement) =>
  Number(row.getAttribute("aria-rowindex")) - 2;
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
const cellAt = (index: number) =>
  rowAt(index).querySelector<HTMLElement>("[data-energy-cell]")!;
const popover = () =>
  screen.queryByRole("dialog", { name: tx("trackTable:energyPopover.label") });
const buttonOf = (energyPicked: number) =>
  within(popover()!).getByRole("button", {
    name: tx("shell:details.energy.set", { energy: energyPicked }),
  });

async function table() {
  renderApp("/library");
  await screen.findByRole("grid");
  await waitFor(() => expect(rows().length).toBeGreaterThan(5));
}

describe("the Energy column", () => {
  it("is not among the columns shown at first, and shows when the user chose it", async () => {
    chosen = null;
    await table();
    expect(
      screen.queryByRole("columnheader", {
        name: tx("trackTable:columns.energy"),
      }),
    ).toBeNull();
    cleanup();
    chosen = ["title", "energy"];
    await table();
    expect(
      screen.getByRole("columnheader", {
        name: new RegExp(escapeRegExp(tx("trackTable:columns.energy"))),
      }),
    ).toBeInTheDocument();
  });

  it("shows each track's energy in a button that names it, and a dash name for a track with none", async () => {
    await table();
    expect(cellAt(1)).toHaveAccessibleName(
      tx("trackTable:energyCell.change", { energy: 7 }),
    );
    expect(cellAt(1)).toHaveTextContent("7");
    expect(cellAt(0)).toHaveAccessibleName(tx("trackTable:energyCell.set"));
    expect(cellAt(0)).toHaveAttribute("aria-haspopup", "dialog");
    expect(cellAt(0)).toHaveAttribute("aria-expanded", "false");
  });

  it("marks an energy that is not the user's own, so it shows muted", async () => {
    await table();
    expect(cellAt(1)).toHaveAttribute("data-source", "user");
    expect(cellAt(2)).toHaveAttribute("data-source", "mik");
    expect(cellAt(2)).toHaveTextContent("5");
  });
});

describe("the Energy column's popover", () => {
  it("opens under a click on the cell, selects that row and puts the focus on the track's current energy", async () => {
    await table();
    await userEvent.click(cellAt(1));
    expect(popover()).toBeInTheDocument();
    expect(selected()).toEqual([1]);
    expect(cellAt(1)).toHaveAttribute("aria-expanded", "true");
    expect(buttonOf(7)).toHaveFocus();
    expect(buttonOf(7)).toHaveAttribute("aria-pressed", "true");
    expect(buttonOf(3)).toHaveAttribute("aria-pressed", "false");
  });

  it("puts the focus on 1 for a track with no energy, and presses none for another source's energy", async () => {
    await table();
    await userEvent.click(cellAt(0));
    expect(buttonOf(1)).toHaveFocus();
    await userEvent.keyboard("{Escape}");
    await userEvent.click(cellAt(2));
    expect(buttonOf(5)).toHaveFocus();
    expect(buttonOf(5)).toHaveAttribute("aria-pressed", "false");
  });

  it("gives the row's track the energy picked, closes, returns the focus to the cell and shows the new energy", async () => {
    await table();
    await userEvent.click(cellAt(0));
    await userEvent.click(buttonOf(9));
    expect(asked).toEqual([{ recordings: [idAt(0)], energy: 9 }]);
    expect(popover()).toBeNull();
    await waitFor(() =>
      expect(cellAt(0)).toHaveAccessibleName(
        tx("trackTable:energyCell.change", { energy: 9 }),
      ),
    );
    expect(cellAt(0)).toHaveAttribute("data-source", "user");
    expect(cellAt(0)).toHaveFocus();
  });

  it("picks with the digit keys inside it, 0 for 10, and the table's own keys do not pick a second time", async () => {
    await table();
    await userEvent.click(cellAt(0));
    await userEvent.keyboard("4");
    expect(asked).toEqual([{ recordings: [idAt(0)], energy: 4 }]);
    expect(popover()).toBeNull();
    await userEvent.click(cellAt(3));
    await userEvent.keyboard("0");
    expect(asked.at(-1)).toEqual({ recordings: [idAt(3)], energy: 10 });
    expect(asked).toHaveLength(2);
  });

  it("moves between the buttons with the arrow keys and picks with Enter", async () => {
    await table();
    await userEvent.click(cellAt(1));
    await userEvent.keyboard("{ArrowRight}");
    expect(buttonOf(8)).toHaveFocus();
    await userEvent.keyboard("{ArrowUp}");
    expect(buttonOf(3)).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    expect(asked).toEqual([{ recordings: [idAt(1)], energy: 3 }]);
  });

  it("closes on Escape without picking, returns the focus to the cell and keeps the selection", async () => {
    await table();
    await userEvent.click(cellAt(1));
    await userEvent.keyboard("{Escape}");
    expect(popover()).toBeNull();
    expect(asked).toEqual([]);
    expect(cellAt(1)).toHaveFocus();
    expect(selected()).toEqual([1]);
  });

  it("closes on a click outside without picking", async () => {
    await table();
    await userEvent.click(cellAt(1));
    await userEvent.click(screen.getByRole("grid"));
    await waitFor(() => expect(popover()).toBeNull());
    expect(asked).toEqual([]);
  });

  it("keeps the focus where the user put it when they click another control", async () => {
    await table();
    await userEvent.click(cellAt(1));
    const other = screen.getByRole("button", {
      name: tx("trackTable:editor.open"),
    });
    const toCell = vi.spyOn(cellAt(1), "focus");
    await userEvent.click(other);
    expect(popover()).toBeNull();
    expect(toCell).not.toHaveBeenCalled();
    expect(asked).toEqual([]);
  });

  it("closes when the focus tabs out, without picking, and the cell has the focus", async () => {
    await table();
    await userEvent.click(cellAt(1));
    await userEvent.tab();
    expect(popover()).toBeNull();
    expect(asked).toEqual([]);
    expect(cellAt(1)).toHaveFocus();
  });

  it("closes when the table is scrolled", async () => {
    await table();
    await userEvent.click(cellAt(1));
    fireEvent.scroll(screen.getByRole("grid"), { target: { scrollTop: 40 } });
    expect(popover()).toBeNull();
  });

  it("is open for one cell at a time, and a second click on the cell closes it (a double click leaves it closed)", async () => {
    await table();
    await userEvent.click(cellAt(1));
    await userEvent.click(cellAt(2));
    expect(
      screen.getAllByRole("dialog", {
        name: tx("trackTable:energyPopover.label"),
      }),
    ).toHaveLength(1);
    expect(cellAt(2)).toHaveAttribute("aria-expanded", "true");
    expect(cellAt(1)).toHaveAttribute("aria-expanded", "false");
    await userEvent.dblClick(cellAt(1));
    expect(popover()).toBeNull();
  });

  it("is a dialog, so the table's keys and the palette stand aside while it is open", async () => {
    await table();
    await userEvent.click(cellAt(1));
    // J would move the selection to the next row if the table heard it.
    fireEvent.keyDown(buttonOf(7), { key: "j" });
    expect(selected()).toEqual([1]);
  });
});

describe("the Energy column with several tracks selected", () => {
  const clickRow = (
    index: number,
    modifiers: { shiftKey?: boolean; ctrlKey?: boolean } = {},
  ) => fireEvent.click(rowAt(index), modifiers);

  it("keeps the selection on a plain click on the cell of a selected row, and a pick gives them all the energy", async () => {
    await table();
    clickRow(1);
    clickRow(3, { shiftKey: true });
    expect(selected()).toEqual([1, 2, 3]);
    await userEvent.click(cellAt(2));
    expect(selected()).toEqual([1, 2, 3]);
    expect(
      within(popover()!).getByText(
        tx("shell:details.energy.many", { count: 3 }),
      ),
    ).toBeInTheDocument();
    // None is pressed: the tracks have no one energy.
    expect(
      within(popover()!).queryAllByRole("button", { pressed: true }),
    ).toHaveLength(0);
    await userEvent.click(buttonOf(6));
    expect(asked).toEqual([
      { recordings: [idAt(1), idAt(2), idAt(3)], energy: 6 },
    ]);
  });

  it("gives only the row's track the energy when the cell is on a row that is not selected", async () => {
    await table();
    clickRow(1);
    clickRow(3, { shiftKey: true });
    await userEvent.click(cellAt(6));
    expect(selected()).toEqual([6]);
    expect(
      within(popover()!).queryByText(
        tx("shell:details.energy.many", { count: 1 }),
      ),
    ).toBeNull();
    await userEvent.click(buttonOf(2));
    expect(asked).toEqual([{ recordings: [idAt(6)], energy: 2 }]);
  });

  it("reaches the Details panel, whose buttons then act on every selected track", async () => {
    await table();
    clickRow(1);
    clickRow(3, { shiftKey: true });
    const details = await screen.findByRole("complementary", {
      name: tx("shell:details.label"),
    });
    expect(
      await within(details).findByText(
        tx("shell:details.energy.many", { count: 3 }),
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      within(details).getByRole("button", {
        name: tx("shell:details.energy.set", { energy: 8 }),
      }),
    );
    await waitFor(() =>
      expect(asked).toEqual([
        { recordings: [idAt(1), idAt(2), idAt(3)], energy: 8 },
      ]),
    );
  });

  it("offers the Details buttons nothing once Ctrl+click takes the focused row out of the selection", async () => {
    await table();
    clickRow(1);
    clickRow(2, { ctrlKey: true });
    clickRow(2, { ctrlKey: true });
    expect(selected()).toEqual([1]);
    const details = await screen.findByRole("complementary", {
      name: tx("shell:details.label"),
    });
    expect(
      within(details).queryByRole("group", {
        name: tx("shell:details.energy.title"),
      }),
    ).toBeNull();
    expect(asked).toEqual([]);
  });

  it("shows the Details panel the energy of the one selected track, and its new energy after a pick", async () => {
    await table();
    await userEvent.click(cellAt(2));
    const details = await screen.findByRole("complementary", {
      name: tx("shell:details.label"),
    });
    const source = tx("shell:details.energy.source.mik");
    expect(
      await within(details).findByText(
        tx("shell:details.energy.from", { energy: 5, source }),
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      within(popover()!).getByRole("button", {
        name: tx("shell:details.energy.set", { energy: 9 }),
      }),
    );
    const group = within(details).getByRole("group", {
      name: tx("shell:details.energy.title"),
    });
    await waitFor(() =>
      expect(
        within(group).getByRole("button", {
          name: tx("shell:details.energy.set", { energy: 9 }),
        }),
      ).toHaveAttribute("aria-pressed", "true"),
    );
    expect(
      within(details).queryByText(
        tx("shell:details.energy.from", { energy: 5, source }),
      ),
    ).toBeNull();
  });

  it("opens nothing on a Shift or Ctrl click on the cell, which select rows as anywhere else", async () => {
    await table();
    clickRow(1);
    fireEvent.click(cellAt(4), { shiftKey: true });
    expect(popover()).toBeNull();
    expect(selected()).toEqual([1, 2, 3, 4]);
    fireEvent.click(cellAt(8), { ctrlKey: true });
    expect(popover()).toBeNull();
    expect(selected()).toEqual([1, 2, 3, 4, 8]);
  });
});

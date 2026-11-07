import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { TableRow } from "../bindings";
import "../i18n";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { TrackTable } from "./TrackTable";

// An estimate of the app's own in the table (3C-E1c; ROADMAP 3.7): muted, with a hover text.

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

/** The cells (Title, BPM, Key, Energy) of one row with `fields`. */
function renderRow(fields: Partial<TableRow>) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <TrackTable
        label="Under test"
        columns={["title", "bpm", "key", "energy"]}
        total={1}
        getRow={() => tableRow(0, { title: "Tune", ...fields })}
        onRangeChange={() => {}}
        sort={null}
        onSort={() => {}}
        resetKey="a"
      />
    </QueryClientProvider>,
  );
  const row = within(screen.getByRole("grid")).getAllByRole("row")[1];
  return within(row).getAllByRole("gridcell");
}

describe("an estimated BPM and key in the track table", () => {
  it("shows a local BPM muted, with the hover text that rekordbox's value replaces it", () => {
    const cells = renderRow({ bpm: 128, bpmEstimated: true });
    expect(cells[1]).toHaveTextContent("128.0");
    expect(cells[1]).toHaveAttribute("title", expect.stringContaining(tx("estimate:estimated.bpm")));
    expect(cells[1].className).toMatch(/estimate/);
  });

  it("shows rekordbox's BPM as it is: not muted, and no hover text about estimates", () => {
    const cells = renderRow({ bpm: 128, bpmEstimated: false });
    expect(cells[1]).toHaveAttribute("title", "128.0");
    expect(cells[1].className).not.toMatch(/estimate/);
  });

  it("marks a local key the same way, and leaves a key from elsewhere alone", () => {
    const local = renderRow({ key: "8A", keyEstimated: true });
    expect(local[2]).toHaveAttribute("title", expect.stringContaining(tx("estimate:estimated.key")));
    expect(local[2].className).toMatch(/estimate/);
    cleanup();
    const other = renderRow({ key: "8A", keyEstimated: false });
    expect(other[2].className).not.toMatch(/estimate/);
  });

  it("marks nothing in a cell with no value", () => {
    const cells = renderRow({ bpm: null, bpmEstimated: true });
    expect(cells[1].className).not.toMatch(/estimate/);
    expect(cells[1]).not.toHaveAttribute("title");
  });

  it("keeps the Energy column unknown for a track whose only energy is a suggestion", () => {
    // The row carries no energy: the table never asks for a suggestion, so it can't reach the column.
    const cells = renderRow({ bpm: 128, bpmEstimated: true, energy: null, energySource: null });
    expect(cells[3]).toBeEmptyDOMElement();
  });
});

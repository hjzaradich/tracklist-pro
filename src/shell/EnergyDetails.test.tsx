import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { EnergySource } from "../bindings";
import "../i18n";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { DetailsPanel } from "./DetailsPanel";
import { useSelectedTrack } from "./selectedTrack";

let asked: { recordings: number[]; energy: number }[];
let refuse: boolean;

beforeEach(() => {
  asked = [];
  refuse = false;
  mockIPC((cmd, args) => {
    if (cmd === "track_tag_sources") return { inRekordbox: false, fields: [], confirmedEntry: null };
    if (cmd === "list_crates" || cmd === "crates_of_track") return [];
    if (cmd === "set_energy") {
      const { recordings, energy } = args as { recordings: number[]; energy: number };
      asked.push({ recordings, energy });
      if (refuse) throw { kind: "database", params: {} };
      return { changed: recordings.length, skipped: 0, operationId: 1 };
    }
    return null;
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  useSelectedTrack.setState({ row: null, ids: [], tables: 0 });
});

function renderPanel(energy: number | null, energySource: EnergySource | null, ids?: number[]) {
  const row = tableRow(1, { energy, energySource });
  useSelectedTrack.setState({ row, ids: ids ?? [row.recordingId] });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <DetailsPanel />
    </QueryClientProvider>,
  );
  return row;
}

const group = () => screen.getByRole("group", { name: tx("shell:details.energy.title") });
const button = (energy: number) =>
  within(group()).getByRole("button", { name: tx("shell:details.energy.set", { energy }) });
const pressed = () => within(group()).queryAllByRole("button", { pressed: true });

describe("the Details panel's Energy section", () => {
  it("has ten buttons, 1 to 10, and says what the keys do", () => {
    renderPanel(null, null);
    expect(within(group()).getAllByRole("button")).toHaveLength(10);
    expect(screen.getByText(tx("shell:details.energy.keys"))).toBeInTheDocument();
  });

  it("presses the button of the energy the user set and says nothing more", () => {
    renderPanel(7, "user");
    expect(pressed()).toEqual([button(7)]);
    expect(screen.queryByText(tx("shell:details.energy.none"))).toBeNull();
  });

  it.each([
    ["mik", "shell:details.energy.source.mik"],
    ["local", "shell:details.energy.source.local"],
    ["ml", "shell:details.energy.source.ml"],
  ] as const)("presses no button for an energy from %s and says whose it is", (source, name) => {
    renderPanel(6, source);
    expect(pressed()).toEqual([]);
    expect(screen.getByText(tx("shell:details.energy.from", { energy: 6, source: tx(name) }))).toBeInTheDocument();
  });

  it("presses no button and says so for a track with no energy", () => {
    renderPanel(null, null);
    expect(pressed()).toEqual([]);
    expect(screen.getByText(tx("shell:details.energy.none"))).toBeInTheDocument();
  });

  it("gives the track the energy picked, also over another source's energy", async () => {
    const row = renderPanel(6, "mik");
    await userEvent.click(button(9));
    await waitFor(() => expect(asked).toEqual([{ recordings: [row.recordingId], energy: 9 }]));
  });

  it("acts on every selected track when several are selected, and presses none", async () => {
    renderPanel(7, "user", [501, 502, 503]);
    expect(screen.getByText(tx("shell:details.energy.many", { count: 3 }))).toBeInTheDocument();
    expect(pressed()).toEqual([]);
    await userEvent.click(button(4));
    await waitFor(() => expect(asked).toEqual([{ recordings: [501, 502, 503], energy: 4 }]));
  });

  it("offers nothing for a focused track that is not selected, or when nothing is selected", () => {
    renderPanel(7, "user", [999]);
    expect(screen.queryByRole("group", { name: tx("shell:details.energy.title") })).toBeNull();
    cleanup();
    renderPanel(7, "user", []);
    expect(screen.queryByRole("group", { name: tx("shell:details.energy.title") })).toBeNull();
  });

  it("says when the energy could not be set", async () => {
    refuse = true;
    renderPanel(null, null);
    await userEvent.click(button(2));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
  });

  it("is one tab stop that starts on the pressed button, and the arrow keys move between the buttons", async () => {
    renderPanel(7, "user");
    await userEvent.tab();
    expect(button(7)).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(button(6)).toHaveFocus();
    await userEvent.keyboard("{ArrowUp}");
    expect(button(1)).toHaveFocus();
    // Up from the top row, and Left from 1, stay where they are.
    await userEvent.keyboard("{ArrowUp}{ArrowLeft}");
    expect(button(1)).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}");
    expect(button(6)).toHaveFocus();
    await userEvent.keyboard("{End}");
    expect(button(10)).toHaveFocus();
    await userEvent.keyboard("{Home}");
    expect(button(1)).toHaveFocus();
  });

  it("starts the tab stop on 1 when no button is pressed, and Enter and Space pick", async () => {
    const row = renderPanel(null, null);
    await userEvent.tab();
    expect(button(1)).toHaveFocus();
    await userEvent.keyboard("{Enter}");
    await userEvent.keyboard("{ArrowRight} ");
    await waitFor(() =>
      expect(asked).toEqual([
        { recordings: [row.recordingId], energy: 1 },
        { recordings: [row.recordingId], energy: 2 },
      ]),
    );
  });
});

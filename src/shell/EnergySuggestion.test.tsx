import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { EnergySuggestion, JobUpdate, OperationInfo } from "../bindings";
import "../i18n";
import { quietAnswer } from "../test/quietCommands";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { DetailsPanel } from "./DetailsPanel";
import { useSelectedTrack } from "./selectedTrack";
import { UndoButton } from "./UndoButton";
import { useReloadListsWhenJobsEnd } from "./useReloadListsWhenJobsEnd";

// The app's energy suggestion beside the Energy buttons (3C-E1c; ROADMAP 3.7).

const SUGGESTION: EnergySuggestion = {
  value: 7,
  basis: [
    { code: "loudness", percentile: 80 },
    { code: "onsets", percentile: 64.6 },
    { code: "flux", percentile: 51 },
    { code: "tempo", percentile: 40 },
  ],
};

let suggestion: EnergySuggestion | null;
let asked: { recordings: number[]; energy: number }[];
let undo: OperationInfo | null;
let suggestionAsked: number;

beforeEach(() => {
  suggestion = SUGGESTION;
  asked = [];
  undo = null;
  suggestionAsked = 0;
  mockIPC((cmd, args) => {
    if (cmd === "energy_suggestion") {
      suggestionAsked += 1;
      return suggestion;
    }
    if (cmd === "track_tag_sources") return { inRekordbox: false, fields: [], confirmedEntry: null };
    if (cmd === "list_crates" || cmd === "crates_of_track" || cmd === "list_tags" || cmd === "tags_of_track") return [];
    if (["key_notation", "next_for", "next_settings", "identify_suggestions", "identify_lookup_status", "online_services_status"].includes(cmd)) return quietAnswer(cmd);
    if (cmd === "next_undo_operation") return { operation: undo, refusal: null };
    if (cmd === "set_energy") {
      const { recordings, energy } = args as { recordings: number[]; energy: number };
      asked.push({ recordings, energy });
      undo = {
        id: 5,
        kind: "set_energy",
        details: { name: null, from: null, tracks: recordings.length, newToLibrary: null, newCrate: null },
      };
      return { changed: recordings.length, skipped: 0, operationId: 5 };
    }
    if (cmd === "user_energy_count") return 0;
    return null;
  }, { shouldMockEvents: true });
});

afterEach(() => {
  cleanup();
  clearMocks();
  useSelectedTrack.setState({ row: null, ids: [], tables: 0 });
});

/** The Details panel inside what the shell does for every screen. */
function InTheShell() {
  useReloadListsWhenJobsEnd();
  return <DetailsPanel />;
}

function renderPanel(fields: Parameters<typeof tableRow>[1] = {}, ids?: number[]) {
  const row = tableRow(1, fields);
  useSelectedTrack.setState({ row, ids: ids ?? [row.recordingId] });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <UndoButton />
      <InTheShell />
    </QueryClientProvider>,
  );
  return row;
}

const group = () => screen.getByRole("group", { name: tx("shell:details.energy.title") });
const useButton = (energy: number) => screen.findByRole("button", { name: tx("estimate:suggestion.use", { energy }) });

describe("the energy suggestion in the Details panel", () => {
  it("shows the suggested value with its basis, in plain words", async () => {
    renderPanel();
    expect(await screen.findByText(tx("estimate:suggestion.label", { energy: 7 }))).toBeInTheDocument();
    const percents = { loudness: 80, onsets: 65, flux: 51, tempo: 40 };
    const reasons = Object.entries(percents).map(([code, percent]) => tx(`estimate:suggestion.reason.${code}`, { percent }));
    expect(screen.getByText(tx("estimate:suggestion.basis", { reasons: reasons.join(", ") }))).toBeInTheDocument();
  });

  it("gives the track the suggested energy as an ordinary edit when Use is pressed, and Undo names it", async () => {
    const row = renderPanel();
    await userEvent.click(await useButton(7));
    await waitFor(() => expect(asked).toEqual([{ recordings: [row.recordingId], energy: 7 }]));
    expect(await screen.findByRole("button", { name: tx("shell:undo.action.setEnergy", { count: 1 }) })).toBeInTheDocument();
  });

  it("is not the track's energy: no energy button is pressed, nothing is set, and the track still has none", async () => {
    renderPanel();
    await useButton(7);
    expect(within(group()).queryAllByRole("button", { pressed: true })).toEqual([]);
    expect(screen.getByText(tx("shell:details.energy.none"))).toBeInTheDocument();
    expect(asked).toEqual([]);
  });

  it("leaves a user's own energy alone and offers the suggestion beside it", async () => {
    renderPanel({ energy: 4, energySource: "user" });
    await useButton(7);
    expect(
      within(group()).getByRole("button", { name: tx("shell:details.energy.set", { energy: 4 }), pressed: true }),
    ).toBeInTheDocument();
    expect(asked).toEqual([]);
  });

  it("shows nothing when there is no suggestion", async () => {
    suggestion = null;
    renderPanel();
    await screen.findByText(tx("shell:details.energy.none"));
    await waitFor(() => expect(suggestionAsked).toBeGreaterThan(0));
    expect(screen.queryByTestId("energy-suggestion")).toBeNull();
  });

  it("shows nothing when several tracks are selected, since a suggestion is for one track", async () => {
    renderPanel({}, [501, 502]);
    await screen.findByText(tx("shell:details.energy.many", { count: 2 }));
    expect(screen.queryByTestId("energy-suggestion")).toBeNull();
    expect(suggestionAsked).toBe(0);
  });

  it("leaves out the basis line when the suggestion has no basis", async () => {
    suggestion = { value: 3, basis: [] };
    renderPanel();
    await screen.findByText(tx("estimate:suggestion.label", { energy: 3 }));
    expect(within(screen.getByTestId("energy-suggestion")).getAllByRole("paragraph")).toHaveLength(1);
  });

  it("asks again for the suggestion when a background job ends, so a finished Analyze run is not missed", async () => {
    suggestion = null;
    renderPanel();
    await screen.findByText(tx("shell:details.energy.none"));
    await waitFor(() => expect(suggestionAsked).toBe(1));
    expect(screen.queryByTestId("energy-suggestion")).toBeNull();

    suggestion = SUGGESTION;
    const ended: JobUpdate = { seq: 9, id: 9, kind: "analyze", status: "done", progress: null, priority: 0 };
    await act(() => emit("job-updates", [ended]));
    expect(await screen.findByText(tx("estimate:suggestion.label", { energy: 7 }))).toBeInTheDocument();
    expect(suggestionAsked).toBe(2);
  });
});

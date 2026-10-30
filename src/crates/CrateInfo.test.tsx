import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { Crate, CrateSummary, TableRow } from "../bindings";
import "../i18n";
import { expectOnlyTheTopBarUndo, TopBarUndo } from "../test/topBarUndo";
import { pageOf, queryOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { CratesScreen } from "./CratesScreen";
import { MAX_NOTES_CHARS, NOTES_ROWS } from "./CrateInfo";

const ROW = 28;
const CRATE: Crate = { id: 1, parentId: null, kind: "static", name: "Warmup", trackCount: 2 };
const FOLDER: Crate = { id: 2, parentId: null, kind: "folder", name: "Friday", trackCount: 0 };

const ROWS: TableRow[] = [0, 1].map((n) =>
  tableRow(n, { recordingId: 100 + n, libraryTrackId: n + 1, title: `Tune ${n}` }),
);

const SUMMARY: CrateSummary = {
  notes: null,
  tracks: 2,
  bpm: { min: 122, max: 128 },
  energy: { min: 4, max: 8 },
  keys: ["8A", "9A"],
  genres: ["House", "Techno"],
};

/** The backend (faked): the crate's summary, which a test can change, and the notes edits. */
let summary: CrateSummary;
let calls: { cmd: string; args: Record<string, unknown> }[];
/** Notes edits, most recent last, with the note each replaced. */
let history: { from: string | null; name: string }[];
let fail: string[];

function serve() {
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    if (fail.includes(cmd)) throw { kind: "crateNotFound", params: {} };
    switch (cmd) {
      case "list_crates":
        return [CRATE, FOLDER];
      case "key_notation":
        return "camelot";
      case "table_columns":
        return null;
      case "track_table_formats":
        return [];
      case "track_table_page":
        return pageOf(ROWS, queryOf(args));
      case "crate_summary":
        return summary;
      case "set_crate_notes": {
        const notes = String(args.notes).trim();
        const next = notes === "" ? null : notes;
        if (next === summary.notes) return null;
        history.push({ from: summary.notes, name: CRATE.name });
        summary = { ...summary, notes: next };
        return history.length;
      }
      case "next_undo_operation": {
        const last = history.at(-1);
        return {
          operation:
            last === undefined
              ? null
              : { id: history.length, kind: "set_crate_notes", details: { name: last.name, from: null, tracks: null } },
          refusal: null,
        };
      }
      case "undo_last_operation": {
        const last = history.pop();
        if (last === undefined) return { status: "nothingToUndo" };
        summary = { ...summary, notes: last.from };
        return {
          status: "undone",
          operation: { id: history.length + 1, kind: "set_crate_notes", details: { name: last.name, from: null, tracks: null } },
        };
      }
      case "remove_tracks_from_crate":
        return { changed: 1, skipped: 0, operationId: 9 };
      case "set_energy":
        return { changed: 1, skipped: 0, operationId: 10 };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
}

beforeEach(() => {
  summary = { ...SUMMARY };
  calls = [];
  history = [];
  fail = [];
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

function Page({ chosen }: { chosen: number }) {
  const [id, setId] = useState<number | null>(chosen);
  return <CratesScreen chosen={id} onChoose={setId} />;
}

function renderCrate(chosen = CRATE.id) {
  serve();
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <TopBarUndo />
      <Page chosen={chosen} />
    </QueryClientProvider>,
  );
}

const summaryLine = () => screen.findByText((text) => text.includes(tx("crates:summary.separator")));
const noteButton = (name: string) => screen.findByRole("button", { name });
const noteBox = () => screen.findByRole("textbox", { name: tx("crates:notes.label") });
const saves = () => calls.filter((call) => call.cmd === "set_crate_notes");

describe("a crate's note", () => {
  it("stands under the crate's name, with a place to write one when there is none", async () => {
    renderCrate();
    expect(await noteButton(tx("crates:notes.add"))).toBeInTheDocument();
    summary = { ...summary, notes: "Opens slow, builds to 124" };
    cleanup();
    renderCrate();
    expect(await noteButton("Opens slow, builds to 124")).toBeInTheDocument();
    expect(screen.queryByText(tx("crates:notes.add"))).toBeNull();
  });

  it("is saved with Enter, shown at once, and said so", async () => {
    renderCrate();
    await userEvent.click(await noteButton(tx("crates:notes.add")));
    await userEvent.type(await noteBox(), "Opens slow{Enter}");
    expect(await noteButton("Opens slow")).toBeInTheDocument();
    expect(saves().map((call) => call.args)).toEqual([{ id: CRATE.id, notes: "Opens slow" }]);
    expect(await screen.findByText(tx("crates:done.notesSaved"))).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).toBeNull();
  });

  it("is written in a box of at least three rows that holds the longest note whole", async () => {
    const longest = "x".repeat(MAX_NOTES_CHARS);
    summary = { ...summary, notes: longest };
    renderCrate();
    await userEvent.click(await noteButton(longest));
    const box = await noteBox();
    expect(box.tagName).toBe("TEXTAREA");
    expect(box).toHaveAttribute("rows", String(NOTES_ROWS));
    expect(box).toHaveValue(longest);
    expect(box).toHaveAttribute("maxlength", String(MAX_NOTES_CHARS));
  });

  it("has no line breaks: Enter saves instead of adding one, and a pasted one is a space", async () => {
    renderCrate();
    await userEvent.click(await noteButton(tx("crates:notes.add")));
    const box = await noteBox();
    await userEvent.click(box);
    await userEvent.paste("Opens slow\r\nbuilds up");
    expect(box).toHaveValue("Opens slow builds up");
    await userEvent.keyboard("{Enter}");
    expect(saves().map((call) => call.args)).toEqual([{ id: CRATE.id, notes: "Opens slow builds up" }]);
  });

  it("is not saved by the Enter that settles an input method's candidate, but by the Enter after it", async () => {
    renderCrate();
    await userEvent.click(await noteButton(tx("crates:notes.add")));
    const box = await noteBox();
    await userEvent.type(box, "Opens slow");

    // While composing, and (in some browsers) right after the composition ended.
    fireEvent.keyDown(box, { key: "Enter", isComposing: true });
    fireEvent.keyDown(box, { key: "Enter", keyCode: 229 });
    expect(saves()).toHaveLength(0);
    expect(box).toBeInTheDocument();
    expect(box).toHaveValue("Opens slow");

    fireEvent.keyDown(box, { key: "Enter" });
    expect(await noteButton("Opens slow")).toBeInTheDocument();
    expect(saves().map((call) => call.args)).toEqual([{ id: CRATE.id, notes: "Opens slow" }]);
  });

  it("keeps the note being written when Escape settles an input method's candidate", async () => {
    renderCrate();
    await userEvent.click(await noteButton(tx("crates:notes.add")));
    const box = await noteBox();
    await userEvent.type(box, "Opens slow");

    fireEvent.keyDown(box, { key: "Escape", isComposing: true });
    expect(box).toBeInTheDocument();
    expect(box).toHaveValue("Opens slow");
  });

  it("is saved when the box is left", async () => {
    renderCrate();
    await userEvent.click(await noteButton(tx("crates:notes.add")));
    await userEvent.type(await noteBox(), "Builds");
    await userEvent.tab();
    expect(await noteButton("Builds")).toBeInTheDocument();
    expect(saves()).toHaveLength(1);
  });

  it("goes back to the note it was with Escape, and saves nothing", async () => {
    summary = { ...summary, notes: "Opens slow" };
    renderCrate();
    await userEvent.click(await noteButton("Opens slow"));
    const box = await noteBox();
    expect(box).toHaveValue("Opens slow");
    await userEvent.type(box, " and long{Escape}");
    expect(await noteButton("Opens slow")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(saves()).toEqual([]);
  });

  it("is cleared by saving an empty note", async () => {
    summary = { ...summary, notes: "Opens slow" };
    renderCrate();
    await userEvent.click(await noteButton("Opens slow"));
    await userEvent.clear(await noteBox());
    await userEvent.keyboard("{Enter}");
    expect(await noteButton(tx("crates:notes.add"))).toBeInTheDocument();
    expect(saves().map((call) => call.args)).toEqual([{ id: CRATE.id, notes: "" }]);
    expect(await screen.findByText(tx("crates:done.notesCleared"))).toBeInTheDocument();
  });

  it("is not saved, and nothing is said, when it is left as it was", async () => {
    summary = { ...summary, notes: "Opens slow" };
    renderCrate();
    await userEvent.click(await noteButton("Opens slow"));
    await noteBox();
    await userEvent.keyboard("{Enter}");
    await noteButton("Opens slow");
    // Spaces round it are not a change either.
    await userEvent.click(await noteButton("Opens slow"));
    await userEvent.type(await noteBox(), "  {Enter}");
    expect(saves()).toEqual([]);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("is one operation naming the crate in the top bar's Undo, which puts the old note back", async () => {
    summary = { ...summary, notes: "Opens slow" };
    renderCrate();
    await userEvent.click(await noteButton("Opens slow"));
    const box = await noteBox();
    await userEvent.clear(box);
    await userEvent.type(box, "Builds to 124{Enter}");
    await noteButton("Builds to 124");

    const undo = await expectOnlyTheTopBarUndo();
    await waitFor(() => expect(undo).toHaveAccessibleName(tx("shell:undo.action.setCrateNotes", { name: CRATE.name })));
    await userEvent.click(undo);
    expect(await noteButton("Opens slow")).toBeInTheDocument();
    // The line about the save described a change that no longer stands.
    expect(screen.queryByText(tx("crates:done.notesSaved"))).toBeNull();
  });

  it("says why a save was refused, and keeps the note it had", async () => {
    summary = { ...summary, notes: "Opens slow" };
    renderCrate();
    fail = ["set_crate_notes"];
    await userEvent.click(await noteButton("Opens slow"));
    const box = await noteBox();
    await userEvent.clear(box);
    await userEvent.type(box, "Changed{Enter}");
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(await noteButton("Opens slow")).toBeInTheDocument();
  });

  it("is not offered on a folder", async () => {
    renderCrate(FOLDER.id);
    await screen.findByRole("heading", { level: 2, name: FOLDER.name });
    expect(screen.queryByRole("button", { name: tx("crates:notes.add") })).toBeNull();
    expect(calls.filter((call) => call.cmd === "crate_summary")).toEqual([]);
  });
});

describe("a crate's summary line", () => {
  it("gives BPM range, energy range, keys and top genres", async () => {
    renderCrate();
    const line = await summaryLine();
    expect(line).toHaveTextContent(
      [
        tx("crates:summary.bpm", { min: "122", max: "128" }),
        tx("crates:summary.energy", { min: 4, max: 8 }),
        tx("crates:summary.keys", { keys: "8A, 9A", count: 2 }),
        "House, Techno",
      ].join(tx("crates:summary.separator")),
    );
  });

  it("leaves out the parts the crate has no data for", async () => {
    summary = { ...summary, bpm: null, energy: null, genres: [] };
    renderCrate();
    const line = await screen.findByText(tx("crates:summary.keys", { keys: "8A, 9A", count: 2 }));
    expect(line.textContent).toBe(tx("crates:summary.keys", { keys: "8A, 9A", count: 2 }));
  });

  it("says plainly when the crate is empty", async () => {
    summary = { ...SUMMARY, tracks: 0, bpm: null, energy: null, keys: [], genres: [] };
    renderCrate();
    expect(await screen.findByText(tx("crates:summary.empty"))).toBeInTheDocument();
  });

  it("is asked for again, and shows the new numbers, after a track leaves the crate", async () => {
    renderCrate();
    await summaryLine();
    const asked = () => calls.filter((call) => call.cmd === "crate_summary").length;
    const before = asked();
    // The backend now has one track fewer: its summary changes with it.
    summary = { ...summary, tracks: 1, bpm: { min: 128, max: 128 }, genres: ["Techno"] };
    await userEvent.click(await screen.findByRole("button", { name: tx("crates:removeTrack.buttonFor", { title: "Tune 0" }) }));
    await waitFor(() => expect(asked()).toBeGreaterThan(before));
    expect(await screen.findByText((text) => text.includes(tx("crates:summary.bpmOne", { bpm: "128" })))).toBeInTheDocument();
  });

  it("is asked for again after an energy is given to a track", async () => {
    renderCrate();
    await summaryLine();
    const asked = () => calls.filter((call) => call.cmd === "crate_summary").length;
    const before = asked();
    summary = { ...summary, energy: { min: 5, max: 8 } };
    await userEvent.click(await screen.findByText("Tune 0"));
    await userEvent.keyboard("5");
    await waitFor(() => expect(asked()).toBeGreaterThan(before));
    expect(await screen.findByText((text) => text.includes(tx("crates:summary.energy", { min: 5, max: 8 })))).toBeInTheDocument();
  });
});

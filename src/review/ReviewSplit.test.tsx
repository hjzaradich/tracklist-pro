import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type {
  NextUndo,
  OperationInfo,
  ReviewComparison,
  ReviewFileRow,
  ReviewGroup,
  ReviewItem,
} from "../bindings";
import "../i18n";
import { UndoButton } from "../shell/UndoButton";
import { tx } from "../test/tx";
import { ReviewScreen } from "./ReviewScreen";

// "Same track", "Not the same" and Split off (1bE-2), on a possible version
// mix-up and on "Unclear duplicates", with the Rust side stood in for: a queue
// an action takes an item out of (or, after a split that leaves its question
// open, keeps) and an undo puts back.

function item(kind: ReviewItem["kind"], a: number, title: string): ReviewItem {
  return {
    id: { kind, a, b: kind === "versionDoubt" ? a + 1 : null },
    kind,
    tracks: [
      { recordingId: a, title, artist: "Made Up Artist" },
      { recordingId: a + 1, title: `${title} (other take)`, artist: "Made Up Artist" },
    ],
    moreTracks: 0,
  };
}

function row(fileId: number, recordingId: number, name: string, linked = false): ReviewFileRow {
  return {
    fileId,
    source: "file",
    recordingId,
    name,
    path: "E:\\Music\\" + name,
    onDisk: true,
    driveConnected: true,
    linked,
    format: "mp3",
    verdict: null,
    cutoffHz: null,
    durationMs: 215000,
    tagsFilled: 3,
    tagsTotal: 6,
    hasArt: false,
    rekordbox: "unknown",
    bitrateKbps: 320,
    sampleRateHz: 44100,
    addedAt: "2026-09-30T10:00:00.000Z",
  };
}

const DOUBT = item("versionDoubt", 70, "Doubtful Tune");
const HELD = item("heldSet", 80, "Unclear Tune");
const NEXT = item("versionDoubt", 90, "Next Tune");
const EMPTY = { needsCloserLook: [], clearBestFile: [], qualityProblems: [] };

interface World {
  lists: Record<ReviewGroup, ReviewItem[]>;
  /** How many files each item has: two or three. */
  files: number;
  /** Whether a split leaves the item in the queue. */
  stays: boolean;
  refuse: Record<string, { kind: string; params: object }>;
  /** The item's reasons say the names give different versions (B-4). */
  namesSay: boolean;
  /** After a refusal, every read of the lists hangs: it never answers. */
  slowReload: boolean;
  calls: { cmd: string; args: Record<string, unknown> }[];
  undo: { operation: OperationInfo; restore: () => void } | null;
}

function comparisonOf(it: ReviewItem, files: number, namesSay = false): ReviewComparison {
  const a = it.id.a;
  const rows = [row(a * 10 + 1, a, "linked.flac", true), row(a * 10 + 2, a + 1, "take.mp3")];
  if (files > 2) rows.splice(1, 0, row(a * 10 + 3, a, "linked copy.mp3"));
  return {
    id: it.id,
    kind: it.kind,
    group: "needsCloserLook",
    tracks: it.tracks,
    doubt: [],
    namesSayVersions: namesSay && it.kind === "versionDoubt",
    suggestion: null,
    rows,
    moreFiles: 0,
    switch: null,
    copies: [],
    relink: null,
    sameRecording: null,
  };
}

function backend(start: Partial<World> = {}): World {
  const world: World = {
    lists: { ...EMPTY },
    files: 2,
    stays: false,
    refuse: {},
    namesSay: false,
    slowReload: false,
    calls: [],
    undo: null,
    ...start,
  };
  let nextOperation = 100;
  let reloadHangs = false;
  const find = (id: ReviewItem["id"]) =>
    world.lists.needsCloserLook.find((i) => i.id.kind === id.kind && i.id.a === id.a);
  /** Takes the item out of the queue (unless it stays), as one undoable step. */
  const act = (id: ReviewItem["id"], kind: string, stays: boolean) => {
    const before = world.lists.needsCloserLook;
    if (!stays) world.lists.needsCloserLook = before.filter((i) => i.id.a !== id.a);
    const operationId = nextOperation++;
    world.undo = {
      operation: { id: operationId, kind, details: { name: null, from: null, tracks: 2 } },
      restore: () => (world.lists.needsCloserLook = before),
    };
    return operationId;
  };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    world.calls.push({ cmd, args });
    const refused = world.refuse[cmd];
    if (refused) {
      if (world.slowReload) reloadHangs = true;
      throw refused;
    }
    // A reload that never ends: a refusal must not wait for it.
    if (reloadHangs && cmd !== "next_undo_operation") return new Promise(() => {});
    switch (cmd) {
      case "missing_tracks":
        return { total: 0, groups: [] };
      case "review_counts": {
        const a = world.lists.needsCloserLook.length;
        return { needsCloserLook: a, clearBestFile: 0, qualityProblems: 0, total: a };
      }
      case "review_items": {
        const group = args.group as ReviewGroup;
        return { group, total: world.lists[group].length, items: world.lists[group] };
      }
      case "review_item": {
        const found = find(args.id as ReviewItem["id"]);
        return found ? comparisonOf(found, world.files, world.namesSay) : null;
      }
      case "review_same_track":
        return { operationId: act(args.id as ReviewItem["id"], "same_track", false) };
      case "review_not_same": {
        const split = world.files > 2;
        const stays = split && world.stays;
        const kind = split ? "split_off_file" : "not_same_track";
        return { operationId: act(args.id as ReviewItem["id"], kind, stays), itemStays: stays };
      }
      case "next_undo_operation":
        return { operation: world.undo?.operation ?? null, refusal: null } satisfies NextUndo;
      case "undo_last_operation": {
        const undone = world.undo;
        if (!undone) return { status: "nothingToUndo" };
        undone.restore();
        world.undo = null;
        return { status: "undone", operation: undone.operation };
      }
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  }, { shouldMockEvents: true });
  return world;
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <UndoButton />
      <ReviewScreen />
    </QueryClientProvider>,
  );
}

function callsOf(world: World, cmd: string) {
  return world.calls.filter((c) => c.cmd === cmd);
}

function writesOf(world: World) {
  const reads = ["review_counts", "review_items", "review_item", "review_accept_all_items"];
  return world.calls.filter((c) => c.cmd.startsWith("review_") && !reads.includes(c.cmd));
}

async function actions() {
  return within(await screen.findByRole("group", { name: tx("review:action.label") }));
}

function press(key: string, init: KeyboardEventInit = {}, target: Element = document.body) {
  fireEvent.keyDown(target, { key, ...init });
}

async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 30));
}

function selectedTitle() {
  return screen.getByRole("option", { selected: true }).textContent ?? "";
}

/** The file rows of the comparison table, the header row left out. */
async function fileRows() {
  const table = await screen.findByRole("table", { name: tx("review:compare.label") });
  return within(table).getAllByRole("row").slice(1);
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe.each([
  ["versionDoubt", DOUBT],
  ["heldSet", HELD],
])("a %s item", (_name, it_) => {
  it("is made one track with Enter, with the tracks the screen showed, and Undo brings it back", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [it_] } });
    renderScreen();
    await actions();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_same_track")).toHaveLength(1));
    expect(callsOf(world, "review_same_track")[0]?.args).toEqual({
      id: it_.id,
      recordingIds: [it_.id.a, it_.id.a + 1],
    });
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
    await userEvent.click(
      await screen.findByRole("button", { name: new RegExp(escape(tx("shell:undo.action.sameTrack"))) }),
    );
    expect(await screen.findByRole("option", { selected: true })).toBeInTheDocument();
    expect(world.lists.needsCloserLook).toEqual([it_]);
  });

  it("offers Same track as the one filled button, and Not the same with two files", async () => {
    backend({ lists: { ...EMPTY, needsCloserLook: [it_] } });
    renderScreen();
    const buttons = await actions();
    const same = buttons.getByRole("button", { name: tx("review:action.sameTrack") });
    const notSame = buttons.getByRole("button", { name: tx("review:action.notSame") });
    expect(same.className).toMatch(/primary/);
    expect(notSame.className).not.toMatch(/primary/);
    expect(buttons.getByText(tx("review:keys.enter", { action: tx("review:action.sameTrack") })))
      .toBeInTheDocument();
    expect(buttons.getByText(tx("review:keys.s", { action: tx("review:action.notSame") })))
      .toBeInTheDocument();
  });

  it("is not the same on S with two files, and the next item loads", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [it_, NEXT] } });
    renderScreen();
    await actions();
    press("s");
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")[0]?.args).toEqual({
      id: it_.id,
      recordingIds: [it_.id.a, it_.id.a + 1],
      // The file that isn't the Library file: the one split off if the two
      // shared a track.
      fileId: it_.id.a * 10 + 2,
    });
    await waitFor(() => expect(selectedTitle()).toContain("Next Tune"));
  });
});

describe("S with two files", () => {
  it("sends the selected row, the one split off if the two share a track", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] } });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("take.mp3"));
    expect(rows[1]).toHaveAttribute("aria-selected", "true");
    press("s");
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")[0]?.args).toMatchObject({ fileId: 702 });
  });
});

describe("with three files or more", () => {
  it("S does nothing until a row is selected, and the keys line says so", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] }, files: 3 });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getByRole("button", { name: tx("review:action.splitOff") })).toBeDisabled();
    expect(buttons.getByText(tx("review:keys.selectRow"))).toBeInTheDocument();
    for (const r of await fileRows()) expect(r).toHaveAttribute("aria-selected", "false");
    press("s");
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("splits off the selected row with S, and stays on the item while it is still queued", async () => {
    const world = backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] },
      files: 3,
      stays: true,
    });
    renderScreen();
    await actions();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    expect(rows[1]).toHaveAttribute("aria-selected", "true");
    expect(rows[0]).toHaveAttribute("aria-selected", "false");
    const buttons = await actions();
    expect(buttons.getByText(tx("review:keys.s", { action: tx("review:action.splitOff") })))
      .toBeInTheDocument();
    expect(buttons.getByRole("button", { name: tx("review:action.splitOff") })).toBeEnabled();
    press("s");
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")[0]?.args).toEqual({
      id: DOUBT.id,
      recordingIds: [70, 71],
      fileId: 703,
    });
    await settle();
    expect(selectedTitle()).toContain("Doubtful Tune");
    expect(selectedTitle()).not.toContain("Next Tune");
  });

  it("goes on to the next item when the split answered the item", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] }, files: 3 });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[0]!).getByText("linked.flac"));
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.splitOff") }),
    );
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")[0]?.args).toMatchObject({ fileId: 701 });
    await waitFor(() => expect(selectedTitle()).toContain("Next Tune"));
  });

  it("selects a row with Space while it has the focus", async () => {
    backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] }, files: 3 });
    renderScreen();
    const rows = await fileRows();
    rows[2]!.focus();
    press(" ", {}, rows[2]!);
    await waitFor(() => expect(rows[2]).toHaveAttribute("aria-selected", "true"));
  });

  it("says why a split is refused, and stays", async () => {
    backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] },
      files: 3,
      refuse: { review_not_same: { kind: "reviewSameAudio", params: {} } },
    });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    press("s");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.sameAudio"));
    expect(selectedTitle()).toContain("Doubtful Tune");
  });
});

describe("a refusal (DOGFOOD B-5)", () => {
  // Every list is read again after an answer, and here that read never
  // ends: the reason must show without waiting for it.
  it("of Same track shows its reason at once, and the screen stays", async () => {
    backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] },
      refuse: { review_same_track: { kind: "reviewMergeLinkedVersions", params: {} } },
      slowReload: true,
    });
    renderScreen();
    await actions();
    press("Enter");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.mergeLinkedVersions"));
    expect(selectedTitle()).toContain("Doubtful Tune");
  });

  it("of Not the same shows its reason at once, and the screen stays", async () => {
    backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] },
      refuse: { review_not_same: { kind: "reviewSameAudio", params: {} } },
      slowReload: true,
    });
    renderScreen();
    await userEvent.click((await actions()).getByRole("button", { name: tx("review:action.notSame") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.sameAudio"));
    expect(selectedTitle()).toContain("Doubtful Tune");
  });

  it("of Split off for copies in two Library tracks says so", async () => {
    backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT] },
      files: 3,
      refuse: { review_not_same: { kind: "reviewSplitTwoLibraryTracks", params: {} } },
    });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    press("s");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.splitTwoLibraryTracks"));
  });

  it("of Split off shows its reason at once, and the button works again", async () => {
    const world = backend({
      lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] },
      files: 3,
      refuse: { review_not_same: { kind: "reviewItemGone", params: {} } },
      slowReload: true,
    });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    const split = (await actions()).getByRole("button", { name: tx("review:action.splitOff") });
    await userEvent.click(split);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.itemGone"));
    expect(selectedTitle()).toContain("Doubtful Tune");
    // Nothing is under way any more: a second press asks again.
    expect(split).toBeEnabled();
    await userEvent.click(split);
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(2));
  });
});

describe("the filled button follows the evidence (DOGFOOD B-4)", () => {
  it("is Not the same, and Enter does it, on a mix-up of two files the names say are versions", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT, NEXT] }, namesSay: true });
    renderScreen();
    const buttons = await actions();
    const same = buttons.getByRole("button", { name: tx("review:action.sameTrack") });
    const notSame = buttons.getByRole("button", { name: tx("review:action.notSame") });
    expect(notSame.className).toMatch(/primary/);
    expect(same.className).not.toMatch(/primary/);
    expect(buttons.getByText(tx("review:keys.enter", { action: tx("review:action.notSame") })))
      .toBeInTheDocument();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_same_track")).toEqual([]);
    expect(callsOf(world, "review_not_same")[0]?.args).toMatchObject({ id: DOUBT.id, recordingIds: [70, 71] });
    await waitFor(() => expect(selectedTitle()).toContain("Next Tune"));
  });

  it("is none with three files until a row is selected, then Split off, and Enter does it", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] }, files: 3, namesSay: true });
    renderScreen();
    let buttons = await actions();
    for (const name of [tx("review:action.sameTrack"), tx("review:action.splitOff")]) {
      expect(buttons.getByRole("button", { name }).className).not.toMatch(/primary/);
    }
    expect(buttons.getByText(tx("review:keys.selectRow"))).toBeInTheDocument();
    expect(buttons.queryByText(tx("review:keys.enter", { action: tx("review:action.sameTrack") })))
      .not.toBeInTheDocument();
    press("Enter");
    await settle();
    expect(writesOf(world)).toEqual([]);

    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    buttons = await actions();
    expect(buttons.getByRole("button", { name: tx("review:action.splitOff") }).className).toMatch(/primary/);
    expect(buttons.getByRole("button", { name: tx("review:action.sameTrack") }).className).not.toMatch(
      /primary/,
    );
    expect(buttons.getByText(tx("review:keys.enter", { action: tx("review:action.splitOff") })))
      .toBeInTheDocument();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_not_same")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")[0]?.args).toMatchObject({ fileId: 703 });
    expect(callsOf(world, "review_same_track")).toEqual([]);
  });

  it("stays Same track on every other version mix-up, with three files and a row selected too", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] }, files: 3 });
    renderScreen();
    const rows = await fileRows();
    await userEvent.click(within(rows[1]!).getByText("linked copy.mp3"));
    const buttons = await actions();
    expect(buttons.getByRole("button", { name: tx("review:action.sameTrack") }).className).toMatch(/primary/);
    expect(buttons.getByRole("button", { name: tx("review:action.splitOff") }).className).not.toMatch(
      /primary/,
    );
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_same_track")).toHaveLength(1));
    expect(callsOf(world, "review_not_same")).toEqual([]);
  });

  it("stays Same track on Unclear duplicates whatever the names say", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [HELD] }, namesSay: true });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getByRole("button", { name: tx("review:action.sameTrack") }).className).toMatch(/primary/);
    expect(buttons.getByRole("button", { name: tx("review:action.notSame") }).className).not.toMatch(
      /primary/,
    );
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_same_track")).toHaveLength(1));
  });
});

describe("a key press", () => {
  it("is left to a focused play button: neither Enter nor S does anything", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] } });
    renderScreen();
    await actions();
    const play = document.querySelector<HTMLElement>('[data-play-for="701"]')!;
    expect(play).not.toBeNull();
    play.focus();
    press("Enter", {}, play);
    press("s", {}, play);
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("S with a modifier does nothing", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [DOUBT] } });
    renderScreen();
    await actions();
    press("s", { ctrlKey: true });
    press("S", { shiftKey: true });
    press("s", { repeat: true });
    await settle();
    expect(writesOf(world)).toEqual([]);
  });
});

/** `text` as a regular expression that matches it literally. */
function escape(text: string) {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

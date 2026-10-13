import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
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
  SwitchOffer,
} from "../bindings";
import "../i18n";
import { UndoButton } from "../shell/UndoButton";
import { tx } from "../test/tx";
import { ReviewScreen } from "./ReviewScreen";

// The Review actions (1bE-5, 1bE-7), with the Rust side stood in for: a
// queue that an action takes an item out of and an undo puts it back into.

function item(kind: ReviewItem["kind"], a: number, title: string): ReviewItem {
  return {
    id: { kind, a, b: null },
    kind,
    tracks: [{ recordingId: a, title, artist: "Made Up Artist" }],
    moreTracks: 0,
  };
}

function row(fileId: number, name: string, fields: Partial<ReviewFileRow> = {}): ReviewFileRow {
  return {
    fileId,
    source: "file",
    recordingId: 1,
    name,
    path: "E:\\Music\\" + name,
    onDisk: true,
    driveConnected: true,
    linked: false,
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
    ...fields,
  };
}

const BETTER = item("betterFile", 10, "Better Tune");
const QUALITY = item("qualityProblem", 20, "Flagged Tune");
const GONE = item("clearBestFile", 30, "Gone Tune");
const GONE_TOO = item("clearBestFile", 31, "Other Gone Tune");

function comparisonOf(it: ReviewItem, switchOffer: SwitchOffer | null): ReviewComparison {
  const base = { id: it.id, kind: it.kind, tracks: it.tracks, doubt: [], moreFiles: 0 };
  if (it.kind === "betterFile") {
    return {
      ...base,
      group: "qualityProblems",
      suggestion: {
        what: 12,
        reasons: [
          { key: "review:reason.betterFormat", params: { best: "FLAC", other: "MP3" }, source: "fact" },
        ],
      },
      rows: [row(12, "better.flac", { format: "flac" }), row(11, "linked.mp3", { linked: true })],
      switch: null,
    };
  }
  if (it.kind === "qualityProblem") {
    return {
      ...base,
      group: "qualityProblems",
      suggestion: null,
      rows: [row(21, "flagged.mp3", { linked: true, verdict: "lowBitrate" })],
      switch: null,
    };
  }
  return {
    ...base,
    group: "clearBestFile",
    suggestion: { what: 32, reasons: [{ key: "review:reason.notOnDisk", params: {}, source: "fact" }] },
    rows: [row(32, "copy.flac"), row(31, "gone.mp3", { linked: true, onDisk: false })],
    switch: switchOffer,
  };
}

interface World {
  lists: Record<ReviewGroup, ReviewItem[]>;
  /** The switch the "Linked file gone" items offer. */
  offer: SwitchOffer;
  /** How many of the "Linked file gone" items "Accept all" would switch (the first ones). */
  clear: number;
  /** What a command answers instead of working, by command. */
  refuse: Record<string, { kind: string; params: object }>;
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The operation Undo would take back, and how to take it back. */
  undo: { operation: OperationInfo; restore: () => void } | null;
}

function operation(id: number, kind: string, tracks: number | null = null): OperationInfo {
  return { id, kind, details: { name: null, from: null, tracks } };
}

function backend(start: Partial<World> = {}): World {
  const world: World = {
    lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [] },
    offer: { fileId: 32, effect: "notInRekordbox", refusal: null },
    clear: 0,
    refuse: {},
    calls: [],
    undo: null,
    ...start,
  };
  let nextOperation = 100;
  /** Takes the item of `recordingId` out of `group`, as one undoable step. */
  const act = (group: ReviewGroup, recordingId: number, kind: string) => {
    const before = world.lists[group];
    world.lists[group] = before.filter((i) => i.id.a !== recordingId);
    const id = nextOperation++;
    world.undo = { operation: operation(id, kind), restore: () => (world.lists[group] = before) };
    return id;
  };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    world.calls.push({ cmd, args });
    const refused = world.refuse[cmd];
    if (refused) throw refused;
    switch (cmd) {
      case "missing_tracks":
        return { total: 0, groups: [] };
      case "review_counts": {
        const [a, b, c] = [
          world.lists.needsCloserLook.length,
          world.lists.clearBestFile.length,
          world.lists.qualityProblems.length,
        ];
        return { needsCloserLook: a, clearBestFile: b, qualityProblems: c, total: a + b + c };
      }
      case "review_items": {
        const group = args.group as ReviewGroup;
        return { group, total: world.lists[group].length, items: world.lists[group] };
      }
      case "review_item": {
        const id = args.id as ReviewItem["id"];
        const found = Object.values(world.lists)
          .flat()
          .find((i) => i.id.kind === id.kind && i.id.a === id.a);
        return found ? comparisonOf(found, world.offer) : null;
      }
      case "review_accept_all_items":
        return world.lists.clearBestFile
          .slice(0, world.clear)
          .map((i) => ({ recordingId: i.id.a, fileId: 32 }));
      case "review_dismiss_better_file":
        return { operationId: act("qualityProblems", args.recordingId as number, "dismiss_better_file") };
      case "review_keep_anyway":
        return { operationId: act("qualityProblems", args.recordingId as number, "keep_anyway") };
      case "review_want_better_file":
        return { operationId: act("qualityProblems", args.recordingId as number, "want_better_file") };
      case "review_switch_linked_file": {
        const recordingId = args.recordingId as number;
        const operationId = act("clearBestFile", recordingId, "switch_linked_file");
        return {
          operationId,
          track: {
            recordingId,
            libraryTrackId: 1,
            fromFileId: 31,
            toFileId: args.fileId,
            effect: args.effect,
          },
        };
      }
      case "review_accept_clear_best_files": {
        // Exactly the items it was sent, and nothing else.
        const sent = (args.items as { recordingId: number }[]).map((i) => i.recordingId);
        const before = world.lists.clearBestFile;
        const switched = before.filter((i) => sent.includes(i.id.a));
        world.lists.clearBestFile = before.filter((i) => !sent.includes(i.id.a));
        world.clear = 0;
        const id = nextOperation++;
        world.undo = {
          operation: operation(id, "accept_clear_best_files", switched.length),
          restore: () => (world.lists.clearBestFile = before),
        };
        return {
          operationId: id,
          switched: switched.map((i) => ({
            recordingId: i.id.a,
            libraryTrackId: 1,
            fromFileId: 31,
            toFileId: 32,
            effect: "notInRekordbox",
          })),
          skipped: [],
        };
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

/** Review with the top bar's Undo, as the shell shows them together. */
function ReviewWithUndo() {
  return (
    <>
      <UndoButton />
      <ReviewScreen />
    </>
  );
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ReviewWithUndo />
    </QueryClientProvider>,
  );
}

function callsOf(world: World, cmd: string) {
  return world.calls.filter((c) => c.cmd === cmd);
}

/** The buttons under the comparison table. */
async function actions() {
  return within(await screen.findByRole("group", { name: tx("review:action.label") }));
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("a Better file available item", () => {
  it("is dismissed with one button, leaves the queue, and Undo brings it back", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [BETTER, QUALITY] },
    });
    renderScreen();
    const quality = await screen.findByRole("region", { name: tx("review:group.qualityProblems") });
    expect(await within(quality).findAllByRole("option")).toHaveLength(2);
    const buttons = await actions();
    // Only a dismissal is offered: no switch while the linked file is there.
    expect(buttons.getAllByRole("button")).toHaveLength(1);

    const countsBefore = callsOf(world, "review_counts").length;
    const itemsBefore = callsOf(world, "review_items").length;
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.dismiss") }));
    expect(callsOf(world, "review_dismiss_better_file")[0]?.args).toEqual({
      recordingId: 10,
      fileId: 12,
    });
    // The queue is asked for again after the action, and the item is gone.
    await waitFor(() => expect(within(quality).getAllByRole("option")).toHaveLength(1));
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(countsBefore);
    expect(callsOf(world, "review_items").length).toBeGreaterThan(itemsBefore);
    expect(within(quality).getByText(tx("review:queue.left", { count: 1 }))).toBeInTheDocument();

    // The Undo button names the action; undoing asks for the queue again.
    const undo = await screen.findByRole("button", {
      name: tx("shell:undo.action.dismissBetterFile"),
    });
    const beforeUndo = callsOf(world, "review_counts").length;
    await userEvent.click(undo);
    await waitFor(() => expect(within(quality).getAllByRole("option")).toHaveLength(2));
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(beforeUndo);
  });
});

describe("a Quality problem item", () => {
  it("offers Keep anyway and Find a better file, and nothing else", async () => {
    backend({ lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [QUALITY] } });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.keepAnyway"),
      tx("review:action.findBetter"),
    ]);
  });

  it("leaves the queue on Keep anyway, and Undo brings it back", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [QUALITY] },
    });
    renderScreen();
    const buttons = await actions();
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.keepAnyway") }));
    expect(callsOf(world, "review_keep_anyway")[0]?.args).toEqual({ recordingId: 20 });
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();

    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.keepAnyway") }),
    );
    expect(
      await screen.findByRole("region", { name: tx("review:group.qualityProblems") }),
    ).toBeInTheDocument();
  });

  it("leaves the queue on Find a better file, and Undo brings it back", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [QUALITY] },
    });
    renderScreen();
    const buttons = await actions();
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.findBetter") }));
    expect(callsOf(world, "review_want_better_file")[0]?.args).toEqual({ recordingId: 20 });
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();

    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.wantBetterFile") }),
    );
    expect(
      await screen.findByRole("region", { name: tx("review:group.qualityProblems") }),
    ).toBeInTheDocument();
  });

  it("says why when the action is refused, and asks for the queue again", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [QUALITY] },
      refuse: { review_keep_anyway: { kind: "reviewItemGone", params: {} } },
    });
    renderScreen();
    const buttons = await actions();
    const before = callsOf(world, "review_counts").length;
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.keepAnyway") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.itemGone"));
    await waitFor(() => expect(callsOf(world, "review_counts").length).toBeGreaterThan(before));
  });
});

describe("a Linked file gone, copy found item", () => {
  const texts = [
    tx("review:switch.keepsEntry"),
    tx("review:switch.newTrack"),
    tx("review:switch.otherEntry"),
  ];

  it("switches a track rekordbox doesn't know with no warning about rekordbox", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
    });
    renderScreen();
    const buttons = await actions();
    for (const text of texts) expect(screen.queryByText(text)).not.toBeInTheDocument();
    // A switch, and no dismissal.
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.switch"),
    ]);

    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.switch") }));
    expect(callsOf(world, "review_switch_linked_file")[0]?.args).toEqual({
      recordingId: 30,
      fileId: 32,
      effect: "notInRekordbox",
    });
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();

    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.switchLinkedFile") }),
    );
    expect(
      await screen.findByRole("region", { name: tx("review:group.clearBestFile") }),
    ).toBeInTheDocument();
  });

  it("says rekordbox will see a new track without the cues, for a track rekordbox knows", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      offer: { fileId: 32, effect: "newTrack", refusal: null },
    });
    renderScreen();
    const buttons = await actions();
    expect(screen.getByText(tx("review:switch.newTrack"))).toBeInTheDocument();
    expect(screen.queryByText(tx("review:switch.keepsEntry"))).not.toBeInTheDocument();
    // The switch is sent with the effect that was shown.
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.switch") }));
    expect(callsOf(world, "review_switch_linked_file")[0]?.args).toMatchObject({
      effect: "newTrack",
    });
  });

  it("says rekordbox keeps the track and its cues where its entry stays the track's", async () => {
    backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      offer: { fileId: 32, effect: "keepsEntry", refusal: null },
    });
    renderScreen();
    await actions();
    expect(screen.getByText(tx("review:switch.keepsEntry"))).toBeInTheDocument();
    expect(screen.queryByText(tx("review:switch.newTrack"))).not.toBeInTheDocument();
  });

  it("says the track becomes rekordbox's other entry where rekordbox holds the copy too", async () => {
    backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      offer: { fileId: 32, effect: "otherEntry", refusal: null },
    });
    renderScreen();
    await actions();
    expect(screen.getByText(tx("review:switch.otherEntry"))).toBeInTheDocument();
  });

  it("greys the switch out with its reason when it can't be made", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      offer: { fileId: 32, effect: null, refusal: "targetLinked" },
    });
    renderScreen();
    const buttons = await actions();
    const button = buttons.getByRole("button", { name: tx("review:action.switch") });
    expect(button).toBeDisabled();
    expect(screen.getByText(tx("review:error.switchTargetLinked"))).toBeInTheDocument();
    await userEvent.click(button);
    expect(callsOf(world, "review_switch_linked_file")).toHaveLength(0);
  });

  it("shows the item again when the switch no longer means what was shown", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      offer: { fileId: 32, effect: "keepsEntry", refusal: null },
      refuse: { review_switch_linked_file: { kind: "reviewSwitchChanged", params: {} } },
    });
    renderScreen();
    const buttons = await actions();
    // By the time of the click, the switch means something else.
    world.offer = { fileId: 32, effect: "newTrack", refusal: null };
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.switch") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.switchChanged"));
    expect(await screen.findByText(tx("review:switch.newTrack"))).toBeInTheDocument();
    expect(screen.queryByText(tx("review:switch.keepsEntry"))).not.toBeInTheDocument();
  });
});

describe("Accept all", () => {
  it("is offered at the Clearly best file heading with how many it will switch", async () => {
    const world = backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE, GONE_TOO], qualityProblems: [QUALITY] },
      clear: 1,
    });
    renderScreen();
    const clear = await screen.findByRole("region", { name: tx("review:group.clearBestFile") });
    const accept = await within(clear).findByRole("button", {
      name: tx("review:queue.acceptAll", { count: 1 }),
    });
    // No other group has one.
    const quality = screen.getByRole("region", { name: tx("review:group.qualityProblems") });
    expect(
      within(quality).queryByRole("button", { name: tx("review:queue.acceptAll", { count: 1 }) }),
    ).not.toBeInTheDocument();

    const before = callsOf(world, "review_counts").length;
    await userEvent.click(accept);
    // The click sends the items the button counted: one item, N of 1.
    expect(callsOf(world, "review_accept_clear_best_files").map((c) => c.args)).toEqual([
      { items: [{ recordingId: 30, fileId: 32 }] },
    ]);
    // The item that isn't a clear case stays; the queue was asked for again.
    await waitFor(() => expect(within(clear).getAllByRole("option")).toHaveLength(1));
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(before);
    expect(within(quality).getAllByRole("option")).toHaveLength(1);
    // Nothing clear is left, so the button is gone.
    await waitFor(() => expect(within(clear).queryByRole("button")).not.toBeInTheDocument());

    // One Undo takes back the whole step and brings the item back.
    await userEvent.click(
      await screen.findByRole("button", {
        name: tx("shell:undo.action.acceptClearBestFiles", { count: 1 }),
      }),
    );
    await waitFor(() => expect(within(clear).getAllByRole("option")).toHaveLength(2));
  });

  it("is not offered when no item is a clear case", async () => {
    backend({
      lists: { needsCloserLook: [], clearBestFile: [GONE], qualityProblems: [] },
      clear: 0,
    });
    renderScreen();
    const clear = await screen.findByRole("region", { name: tx("review:group.clearBestFile") });
    await within(clear).findAllByRole("option");
    await actions();
    expect(within(clear).queryByRole("button")).not.toBeInTheDocument();
  });
});

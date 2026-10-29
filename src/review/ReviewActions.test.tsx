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
  SwitchOffer,
} from "../bindings";
import "../i18n";
import { UndoButton } from "../shell/UndoButton";
import { tx } from "../test/tx";
import { useAcceptedStore } from "./acceptedStore";
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
/** A probable relink match: the item names rekordbox's entry (row 50). */
const RELINK = item("relink", 50, "Old Tune");

function comparisonOf(
  it: ReviewItem,
  switchOffer: SwitchOffer | null,
  confirmedBefore: boolean,
  copies: SwitchOffer[],
): ReviewComparison {
  const base = { id: it.id, kind: it.kind, tracks: it.tracks, doubt: [], namesSayVersions: false, moreFiles: 0 };
  if (it.kind === "relink") {
    return {
      ...base,
      group: "needsCloserLook",
      suggestion: {
        what: 8,
        reasons: [
          {
            key: confirmedBefore ? "review:relink.user" : "review:relink.filenameOnly",
            params: {},
            source: "fact",
          },
        ],
      },
      rows: [
        row(8, "new tune.mp3"),
        row(0, "old tune.mp3", { fileId: null, source: "rekordboxEntry", recordingId: null, onDisk: false }),
      ],
      switch: null,
      copies: [],
      relink: { fileId: 8, confirmedBefore },
      sameRecording: null,
    };
  }
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
      copies: [],
      relink: null,
      sameRecording: null,
    };
  }
  if (it.kind === "qualityProblem") {
    return {
      ...base,
      group: "qualityProblems",
      suggestion: null,
      rows: [row(21, "flagged.mp3", { linked: true, verdict: "lowBitrate" })],
      switch: null,
      copies: [],
      relink: null,
      sameRecording: null,
    };
  }
  if (it.kind === "versionDoubt" || it.kind === "heldSet" || it.kind === "sameRecording") {
    return {
      ...base,
      group: "needsCloserLook",
      suggestion: {
        what: 62,
        reasons: [{ key: "review:reason.betterFormat", params: { best: "FLAC", other: "MP3" }, source: "fact" }],
      },
      rows: [row(62, "one.flac", { format: "flac" }), row(61, "two.mp3")],
      switch: null,
      copies: [],
      relink: null,
      sameRecording: null,
    };
  }
  return {
    ...base,
    group: "clearBestFile",
    suggestion: { what: 32, reasons: [{ key: "review:reason.notOnDisk", params: {}, source: "fact" }] },
    // The suggested copy, the gone linked file, then the other copies.
    rows: [
      row(32, "copy.flac"),
      row(31, "gone.mp3", { linked: true, onDisk: false }),
      ...copies.filter((c) => c.fileId !== 32).map((c) => row(c.fileId, `copy ${c.fileId}.mp3`)),
    ],
    switch: switchOffer,
    copies,
    relink: null,
    sameRecording: null,
  };
}

interface World {
  lists: Record<ReviewGroup, ReviewItem[]>;
  /** The switch the "Linked file gone" items offer. */
  offer: SwitchOffer;
  /** The switch to each copy on disk of a "Linked file gone" item (the number keys). */
  copies: SwitchOffer[];
  /** Whether the relink items were confirmed before (their audio changed since). */
  confirmedBefore: boolean;
  /** How many of the "Linked file gone" items "Accept all" would switch (the first ones). */
  clear: number;
  /** What a command answers instead of working, by command. */
  refuse: Record<string, { kind: string; params: object }>;
  calls: { cmd: string; args: Record<string, unknown> }[];
  /** The operation Undo would take back, and how to take it back. */
  undo: { operation: OperationInfo; restore: () => void } | null;
}

function operation(id: number, kind: string, tracks: number | null = null): OperationInfo {
  return { id, kind, details: { name: null, from: null, tracks, newToLibrary: null, newCrate: null } };
}

function backend(start: Partial<World> = {}): World {
  const world: World = {
    lists: { needsCloserLook: [], clearBestFile: [], qualityProblems: [] },
    offer: { fileId: 32, effect: "notInRekordbox", refusal: null },
    copies: [],
    confirmedBefore: false,
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
      case "review_conflict_fields":
        return [];
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
        return found
          ? comparisonOf(found, world.offer, world.confirmedBefore, world.copies)
          : null;
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
      case "review_confirm_relink":
        return { operationId: act("needsCloserLook", args.entryId as number, "confirm_relink") };
      case "review_reject_relink":
        return { operationId: act("needsCloserLook", args.entryId as number, "reject_relink") };
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
      case "review_switched_view":
        return (args.switched as { recordingId: number; fromFileId: number; toFileId: number }[]).map(
          (s) => ({
            recordingId: s.recordingId,
            track: { recordingId: s.recordingId, title: `Track ${s.recordingId}`, artist: "Made Up Artist" },
            fromName: `old ${s.fromFileId}.mp3`,
            toName: `new ${s.toFileId}.flac`,
          }),
        );
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
  // The line after "Accept all" outlives a screen: start every test without one.
  useAcceptedStore.setState({ accepted: null });
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

describe("a Confirm relink item", () => {
  // A new object each time: the stand-in backend takes items out of it.
  const relinkOnly = () => ({ needsCloserLook: [RELINK], clearBestFile: [], qualityProblems: [] });

  it("offers Confirm and Not the same, and no way to withdraw", async () => {
    backend({ lists: relinkOnly() });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.confirm"),
      tx("review:action.rejectMatch"),
    ]);
  });

  it("leaves the queue when it is confirmed, and Undo brings it back", async () => {
    const world = backend({ lists: relinkOnly() });
    renderScreen();
    const buttons = await actions();
    const before = callsOf(world, "review_counts").length;
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.confirm") }));
    // The entry and the file the item showed.
    expect(callsOf(world, "review_confirm_relink")[0]?.args).toEqual({ entryId: 50, fileId: 8 });
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(before);

    const beforeUndo = callsOf(world, "review_counts").length;
    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.confirmRelink") }),
    );
    expect(
      await screen.findByRole("region", { name: tx("review:group.needsCloserLook") }),
    ).toBeInTheDocument();
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(beforeUndo);
  });

  it("leaves the queue on Not the same, and Undo brings it back", async () => {
    const world = backend({ lists: relinkOnly() });
    renderScreen();
    const buttons = await actions();
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.rejectMatch") }));
    expect(callsOf(world, "review_reject_relink")[0]?.args).toEqual({ entryId: 50, fileId: 8 });
    expect(callsOf(world, "review_confirm_relink")).toHaveLength(0);
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();

    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.rejectRelink") }),
    );
    expect(
      await screen.findByRole("region", { name: tx("review:group.needsCloserLook") }),
    ).toBeInTheDocument();
  });

  it("asks to confirm again when the match was confirmed before and the audio changed", async () => {
    const world = backend({ lists: relinkOnly(), confirmedBefore: true });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.confirmAgain"),
      tx("review:action.rejectMatch"),
    ]);
    // Why it is asked again is the suggestion's reason.
    expect(screen.getByText(tx("review:relink.user"), { exact: false })).toBeInTheDocument();
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.confirmAgain") }));
    expect(callsOf(world, "review_confirm_relink")[0]?.args).toEqual({ entryId: 50, fileId: 8 });
  });

  it("says so when the match was answered already, and asks for the queue again", async () => {
    const world = backend({
      lists: relinkOnly(),
      refuse: { review_confirm_relink: { kind: "reviewItemGone", params: {} } },
    });
    renderScreen();
    const buttons = await actions();
    const before = callsOf(world, "review_counts").length;
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.confirm") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.itemGone"));
    await waitFor(() => expect(callsOf(world, "review_counts").length).toBeGreaterThan(before));
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

// ---- the keyboard (1bE-2) -----------------------------------------------------

const RELINK_B = item("relink", 51, "Second Old Tune");
const DOUBT_ITEM = item("versionDoubt", 70, "Doubtful Tune");
const HELD_ITEM = item("heldSet", 71, "Unclear Tune");
const EMPTY = { needsCloserLook: [], clearBestFile: [], qualityProblems: [] };

/** The commands that change something: any `review_` call that is not a read. */
function writesOf(world: World) {
  const reads = [
    "review_counts",
    "review_items",
    "review_item",
    "review_accept_all_items",
    "review_conflict_fields",
  ];
  return world.calls.filter((c) => c.cmd.startsWith("review_") && !reads.includes(c.cmd));
}

/** Presses a key as the browser sends it: on `target`, reaching the window. */
function press(key: string, init: KeyboardEventInit = {}, target: Element = document.body) {
  fireEvent.keyDown(target, { key, ...init });
}

/** The text of the selected item in the queue. */
function selectedTitle() {
  return screen.getByRole("option", { selected: true }).textContent ?? "";
}

/** A moment for anything a key press might have started: the tests that expect nothing wait this long. */
async function settle() {
  await new Promise((resolve) => setTimeout(resolve, 30));
}

describe("Enter", () => {
  it("confirms a relink match, with the entry and the file the item showed", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    await actions();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_confirm_relink")).toHaveLength(1));
    expect(callsOf(world, "review_confirm_relink")[0]?.args).toEqual({ entryId: 50, fileId: 8 });
    expect(callsOf(world, "review_reject_relink")).toHaveLength(0);
  });

  it("confirms again where the match was confirmed before", async () => {
    const world = backend({
      lists: { ...EMPTY, needsCloserLook: [RELINK] },
      confirmedBefore: true,
    });
    renderScreen();
    await actions();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_confirm_relink")).toHaveLength(1));
  });

  it("switches to the suggested copy on a File missing, copy found item", async () => {
    const world = backend({
      lists: { ...EMPTY, clearBestFile: [GONE] },
      offer: { fileId: 32, effect: "keepsEntry", refusal: null },
    });
    renderScreen();
    await actions();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_switch_linked_file")).toHaveLength(1));
    // With the effect the item showed, as the button sends it.
    expect(callsOf(world, "review_switch_linked_file")[0]?.args).toEqual({
      recordingId: 30,
      fileId: 32,
      effect: "keepsEntry",
    });
  });

  it("does nothing where the switch is refused beforehand", async () => {
    const world = backend({
      lists: { ...EMPTY, clearBestFile: [GONE] },
      offer: { fileId: 32, effect: null, refusal: "targetLinked" },
    });
    renderScreen();
    await actions();
    press("Enter");
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  // On a version mix-up and on "Unclear duplicates" it is "Same track"
  // (ReviewSplit.test.tsx).
  it.each([
    ["qualityProblem", { ...EMPTY, qualityProblems: [QUALITY] }],
    ["betterFile", { ...EMPTY, qualityProblems: [BETTER] }],
  ])("does nothing on a %s item", async (_name, lists) => {
    const world = backend({ lists });
    renderScreen();
    await screen.findByRole("table", { name: tx("review:compare.label") });
    press("Enter");
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("acts on the item that is selected, not on the first one", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK, RELINK_B] } });
    renderScreen();
    await userEvent.click(await screen.findByRole("option", { name: /Second Old Tune/ }));
    await waitFor(() => expect(selectedTitle()).toContain("Second Old Tune"));
    await actions();
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_confirm_relink")).toHaveLength(1));
    expect(callsOf(world, "review_confirm_relink")[0]?.args).toMatchObject({ entryId: 51 });
  });

  it("works with the focus in the queue", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    await actions();
    const option = screen.getByRole("option", { selected: true });
    option.focus();
    press("Enter", {}, option);
    await waitFor(() => expect(callsOf(world, "review_confirm_relink")).toHaveLength(1));
  });
});

describe("the number keys", () => {
  const two: SwitchOffer[] = [
    { fileId: 32, effect: "notInRekordbox", refusal: null },
    { fileId: 33, effect: "keepsEntry", refusal: null },
  ];

  it("switch to the copy in that row where there are two copies on disk", async () => {
    // Rows: 1 the suggested copy, 2 the gone linked file, 3 the other copy.
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, copies: two });
    renderScreen();
    await actions();
    press("3");
    await waitFor(() => expect(callsOf(world, "review_switch_linked_file")).toHaveLength(1));
    // The switch to that copy, with the effect that copy showed.
    expect(callsOf(world, "review_switch_linked_file")[0]?.args).toEqual({
      recordingId: 30,
      fileId: 33,
      effect: "keepsEntry",
    });
  });

  it("switch to the first row's copy on 1", async () => {
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, copies: two });
    renderScreen();
    await actions();
    press("1");
    await waitFor(() => expect(callsOf(world, "review_switch_linked_file")).toHaveLength(1));
    expect(callsOf(world, "review_switch_linked_file")[0]?.args).toMatchObject({ fileId: 32 });
  });

  it("do nothing on the linked file's row, on a row that isn't there, and on 0", async () => {
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, copies: two });
    renderScreen();
    await actions();
    for (const key of ["2", "4", "9", "0"]) press(key);
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("do nothing on a copy whose switch is refused beforehand", async () => {
    const world = backend({
      lists: { ...EMPTY, clearBestFile: [GONE] },
      copies: [two[0]!, { fileId: 33, effect: null, refusal: "targetLinked" }],
    });
    renderScreen();
    await actions();
    press("3");
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("do nothing where there is only one copy", async () => {
    const world = backend({
      lists: { ...EMPTY, clearBestFile: [GONE] },
      copies: [two[0]!],
    });
    renderScreen();
    await actions();
    press("1");
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it.each([
    ["relink", { ...EMPTY, needsCloserLook: [RELINK] }],
    ["qualityProblem", { ...EMPTY, qualityProblems: [QUALITY] }],
    ["betterFile", { ...EMPTY, qualityProblems: [BETTER] }],
    ["versionDoubt", { ...EMPTY, needsCloserLook: [DOUBT_ITEM] }],
    ["heldSet", { ...EMPTY, needsCloserLook: [HELD_ITEM] }],
  ])("do nothing on a %s item", async (_name, lists) => {
    const world = backend({ lists });
    renderScreen();
    await screen.findByRole("table", { name: tx("review:compare.label") });
    for (const key of ["1", "2", "3"]) press(key);
    await settle();
    expect(writesOf(world)).toEqual([]);
  });
});

describe("a key press", () => {
  it("is left to a focused button: the play buttons sit in every file row", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    const buttons = await actions();
    const play = document.querySelector<HTMLElement>('[data-play-for="8"]')!;
    expect(play).not.toBeNull();
    play.focus();
    press("Enter", {}, play);
    press("1", {}, play);
    // Any button: the action's own too.
    press("Enter", {}, buttons.getByRole("button", { name: tx("review:action.rejectMatch") }));
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("is left to a text field", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    await actions();
    const field = document.body.appendChild(document.createElement("input"));
    field.focus();
    press("Enter", {}, field);
    field.remove();
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("is left to a dialog that is waiting for an answer", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    await actions();
    const dialog = document.body.appendChild(document.createElement("div"));
    dialog.setAttribute("role", "alertdialog");
    press("Enter");
    dialog.remove();
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("is ignored with a modifier key, and when the key is held down", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] } });
    renderScreen();
    await actions();
    press("Enter", { ctrlKey: true });
    press("Enter", { altKey: true });
    press("Enter", { metaKey: true });
    press("Enter", { shiftKey: true });
    press("Enter", { repeat: true });
    await settle();
    expect(writesOf(world)).toEqual([]);
  });

  it("makes the action once, however often the key goes down", async () => {
    const world = backend({ lists: { ...EMPTY, needsCloserLook: [RELINK, RELINK_B] } });
    renderScreen();
    await actions();
    press("Enter");
    press("Enter");
    press("Enter");
    await waitFor(() => expect(callsOf(world, "review_confirm_relink")).toHaveLength(1));
    await settle();
    expect(callsOf(world, "review_confirm_relink")).toHaveLength(1);
    expect(callsOf(world, "review_confirm_relink")[0]?.args).toMatchObject({ entryId: 50 });
  });

  it("is not taken (not prevented) when it does nothing here", async () => {
    backend({ lists: { ...EMPTY, qualityProblems: [QUALITY] } });
    renderScreen();
    await screen.findByRole("table", { name: tx("review:compare.label") });
    expect(fireEvent.keyDown(document.body, { key: "Enter", cancelable: true })).toBe(true);
  });
});

describe("the keys line", () => {
  const two: SwitchOffer[] = [
    { fileId: 32, effect: "notInRekordbox", refusal: null },
    { fileId: 33, effect: "notInRekordbox", refusal: null },
  ];
  const enterLine = (action: string) => tx("review:keys.enter", { action });

  it("says Enter switches, on File missing, copy found with one copy", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, copies: [two[0]!] });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getByText(enterLine(tx("review:action.switch")))).toBeInTheDocument();
    expect(buttons.queryByText(tx("review:keys.rows"))).not.toBeInTheDocument();
  });

  it("adds the number keys where there are two copies", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, copies: two });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getByText(enterLine(tx("review:action.switch")))).toBeInTheDocument();
    expect(buttons.getByText(tx("review:keys.rows"))).toBeInTheDocument();
  });

  it("says Enter confirms on a relink match, or confirms again", async () => {
    backend({ lists: { ...EMPTY, needsCloserLook: [RELINK] }, confirmedBefore: true });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getByText(enterLine(tx("review:action.confirmAgain")))).toBeInTheDocument();
  });

  it("is not there where no key works, and the row still has its buttons", async () => {
    backend({ lists: { ...EMPTY, qualityProblems: [QUALITY] } });
    renderScreen();
    const buttons = await actions();
    expect(buttons.queryByText(tx("review:keys.rows"))).not.toBeInTheDocument();
    expect(buttons.queryByText(/Enter/)).not.toBeInTheDocument();
    expect(buttons.getAllByRole("button")).toHaveLength(2);
  });

  it("is not there where the switch is refused beforehand", async () => {
    backend({
      lists: { ...EMPTY, clearBestFile: [GONE] },
      offer: { fileId: 32, effect: null, refusal: "targetGone" },
    });
    renderScreen();
    const buttons = await actions();
    expect(buttons.queryByText(enterLine(tx("review:action.switch")))).not.toBeInTheDocument();
  });
});

describe("after a decision", () => {
  it("goes to the next item in the group, then the first item of the next group, then the empty state", async () => {
    const world = backend({
      lists: {
        needsCloserLook: [RELINK, RELINK_B],
        clearBestFile: [GONE],
        qualityProblems: [QUALITY],
      },
    });
    renderScreen();
    await actions();
    expect(selectedTitle()).toContain("Old Tune");

    // The next item in the same group.
    press("Enter");
    await waitFor(() => expect(selectedTitle()).toContain("Second Old Tune"));
    await actions();
    // The last one of its group: the first item of the next group.
    press("Enter");
    await waitFor(() => expect(selectedTitle()).toContain("Gone Tune"));
    await actions();
    press("Enter");
    await waitFor(() => expect(selectedTitle()).toContain("Flagged Tune"));
    expect(
      callsOf(world, "review_confirm_relink").map((c) => (c.args as { entryId: number }).entryId),
    ).toEqual([50, 51]);
    expect(callsOf(world, "review_switch_linked_file")).toHaveLength(1);

    // Enter does nothing on a quality problem: the screen stays. Its button decides.
    press("Enter");
    await settle();
    expect(selectedTitle()).toContain("Flagged Tune");
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.keepAnyway") }),
    );
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
  });

  it("goes on from the item that was decided, whatever the first item is", async () => {
    backend({ lists: { ...EMPTY, needsCloserLook: [RELINK, RELINK_B, DOUBT_ITEM] } });
    renderScreen();
    await userEvent.click(await screen.findByRole("option", { name: /Second Old Tune/ }));
    await actions();
    press("Enter");
    await waitFor(() => expect(selectedTitle()).toContain("Doubtful Tune"));
  });

  it("goes on after a click on a button too", async () => {
    backend({ lists: { ...EMPTY, qualityProblems: [BETTER, QUALITY] } });
    renderScreen();
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.dismiss") }),
    );
    await waitFor(() => expect(selectedTitle()).toContain("Flagged Tune"));
  });

  it("stays where it is after an action the app refuses", async () => {
    const world = backend({
      lists: { ...EMPTY, needsCloserLook: [RELINK, RELINK_B] },
      refuse: { review_confirm_relink: { kind: "reviewItemGone", params: {} } },
    });
    renderScreen();
    await actions();
    const before = callsOf(world, "review_counts").length;
    press("Enter");
    // The error shows once the action has settled, which includes asking for
    // the queue again: nothing is pending after this, so the check is direct.
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.itemGone"));
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(before);
    expect(selectedTitle()).toContain("Old Tune");
    expect(selectedTitle()).not.toContain("Second");
  });
});

// ---- Accept all: the line at the top (1bE-3) ------------------------------------

describe("the line after Accept all", () => {
  const accepted = (count: number) => tx("review:accepted.line", { count });
  const acceptAll = async (count: number) =>
    userEvent.click(
      await screen.findByRole("button", { name: tx("review:queue.acceptAll", { count }) }),
    );

  it("says how many tracks were switched, with View and Dismiss, and no Undo of its own", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE, GONE_TOO] }, clear: 2 });
    renderScreen();
    expect(screen.queryByRole("region", { name: accepted(2) })).not.toBeInTheDocument();
    await acceptAll(2);
    const line = within(await screen.findByRole("region", { name: accepted(2) }));
    expect(line.getByRole("button", { name: tx("review:accepted.view") })).toBeInTheDocument();
    expect(line.getByRole("button", { name: tx("review:accepted.dismiss") })).toBeInTheDocument();
    // Undo is in the top bar only (owner decision, 2026-10-27): the line has
    // View and Dismiss and nothing else, while the top bar names the step.
    expect(line.getAllByRole("button")).toHaveLength(2);
    expect(
      await screen.findByRole("button", {
        name: tx("shell:undo.action.acceptClearBestFiles", { count: 2 }),
      }),
    ).toBeInTheDocument();
    // The queue is empty now; the line is still there.
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
  });

  it("says one track in the singular", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, clear: 1 });
    renderScreen();
    await acceptAll(1);
    expect(await screen.findByRole("region", { name: accepted(1) })).toBeInTheDocument();
  });

  it("shows each track with its old and new file on View, from what the command returned", async () => {
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE, GONE_TOO] }, clear: 2 });
    renderScreen();
    await acceptAll(2);
    const line = within(await screen.findByRole("region", { name: accepted(2) }));
    await userEvent.click(line.getByRole("button", { name: tx("review:accepted.view") }));
    const table = within(await line.findByRole("table", { name: tx("review:accepted.list") }));
    const rows = table.getAllByRole("row").slice(1);
    expect(rows).toHaveLength(2);
    const track = (id: number) =>
      tx("review:item.trackName", { artist: "Made Up Artist", title: `Track ${id}` });
    expect(
      within(rows[0]!)
        .getAllByRole("cell")
        .map((c) => c.textContent),
    ).toEqual([track(30), "old 31.mp3", "new 32.flac"]);
    expect(within(rows[1]!).getAllByRole("cell")[0]).toHaveTextContent(track(31));
    // The names were asked for with the ids the switch returned.
    expect(callsOf(world, "review_switched_view")[0]?.args).toEqual({
      switched: [
        { recordingId: 30, fromFileId: 31, toFileId: 32 },
        { recordingId: 31, fromFileId: 31, toFileId: 32 },
      ],
    });
    // Hide closes it.
    await userEvent.click(line.getByRole("button", { name: tx("review:accepted.hide") }));
    expect(line.queryByRole("table")).not.toBeInTheDocument();
  });

  it("stays until it is dismissed, also after leaving Review and coming back", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE] }, clear: 1 });
    const first = renderScreen();
    await acceptAll(1);
    await screen.findByRole("region", { name: accepted(1) });
    first.unmount();
    renderScreen();
    const line = within(await screen.findByRole("region", { name: accepted(1) }));
    await userEvent.click(line.getByRole("button", { name: tx("review:accepted.dismiss") }));
    expect(screen.queryByRole("region", { name: accepted(1) })).not.toBeInTheDocument();
  });

  it("is replaced by the next Accept all, and left alone by one that switched nothing", () => {
    const switched = (recordingId: number) => ({
      recordingId,
      libraryTrackId: 1,
      fromFileId: 31,
      toFileId: 32,
      effect: "notInRekordbox" as const,
    });
    const { record } = useAcceptedStore.getState();
    record({ operationId: 5, switched: [switched(1), switched(2)], skipped: [] });
    expect(useAcceptedStore.getState().accepted).toMatchObject({ operationId: 5 });
    record({ operationId: null, switched: [], skipped: [] });
    expect(useAcceptedStore.getState().accepted).toMatchObject({ operationId: 5 });
    record({ operationId: 6, switched: [switched(3)], skipped: [] });
    expect(useAcceptedStore.getState().accepted).toMatchObject({ operationId: 6 });
    expect(useAcceptedStore.getState().accepted?.switched).toHaveLength(1);
  });

  it("goes when the top bar's Undo takes the Accept all back", async () => {
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE, GONE_TOO] }, clear: 2 });
    renderScreen();
    await acceptAll(2);
    await screen.findByRole("region", { name: accepted(2) });
    // The top bar names the step; it is the Accept all.
    await userEvent.click(
      await screen.findByRole("button", {
        name: tx("shell:undo.action.acceptClearBestFiles", { count: 2 }),
      }),
    );
    await waitFor(() => expect(callsOf(world, "undo_last_operation")).toHaveLength(1));
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: accepted(2) })).not.toBeInTheDocument(),
    );
  });

  it("goes when Ctrl+Z takes the Accept all back", async () => {
    const world = backend({ lists: { ...EMPTY, clearBestFile: [GONE, GONE_TOO] }, clear: 2 });
    renderScreen();
    await acceptAll(2);
    await screen.findByRole("region", { name: accepted(2) });
    await screen.findByRole("button", {
      name: tx("shell:undo.action.acceptClearBestFiles", { count: 2 }),
    });
    await userEvent.keyboard("{Control>}z{/Control}");
    await waitFor(() => expect(callsOf(world, "undo_last_operation")).toHaveLength(1));
    await waitFor(() =>
      expect(screen.queryByRole("region", { name: accepted(2) })).not.toBeInTheDocument(),
    );
  });

  it("stays when an Undo takes back some other step", async () => {
    backend({ lists: { ...EMPTY, clearBestFile: [GONE], qualityProblems: [QUALITY] }, clear: 1 });
    renderScreen();
    await acceptAll(1);
    await screen.findByRole("region", { name: accepted(1) });
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.keepAnyway") }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.keepAnyway") }),
    );
    await screen.findByRole("region", { name: tx("review:group.qualityProblems") });
    expect(screen.getByRole("region", { name: accepted(1) })).toBeInTheDocument();
  });
});

// ---- Undo refreshes Review (1bE-2) ----------------------------------------------

describe("Undo", () => {
  it("brings the item back in the queue and in the comparison, from the top bar and with Ctrl+Z", async () => {
    const world = backend({ lists: { ...EMPTY, qualityProblems: [QUALITY] } });
    renderScreen();
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.keepAnyway") }),
    );
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();

    const counts = callsOf(world, "review_counts").length;
    const items = callsOf(world, "review_items").length;
    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.keepAnyway") }),
    );
    const quality = await screen.findByRole("region", { name: tx("review:group.qualityProblems") });
    expect(await within(quality).findAllByRole("option")).toHaveLength(1);
    expect(callsOf(world, "review_counts").length).toBeGreaterThan(counts);
    expect(callsOf(world, "review_items").length).toBeGreaterThan(items);
    // The comparison of the item that came back is on screen.
    expect(
      await screen.findByRole("table", { name: tx("review:compare.label") }),
    ).toBeInTheDocument();

    // And the same with the keyboard.
    await userEvent.click(
      (await actions()).getByRole("button", { name: tx("review:action.findBetter") }),
    );
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
    await screen.findByRole("button", { name: tx("shell:undo.action.wantBetterFile") });
    await userEvent.keyboard("{Control>}z{/Control}");
    expect(
      await screen.findByRole("region", { name: tx("review:group.qualityProblems") }),
    ).toBeInTheDocument();
  });
});

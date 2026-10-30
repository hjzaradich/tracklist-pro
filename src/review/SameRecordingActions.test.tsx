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
  MergeField,
  ReviewItem,
  SameRecordingOffer,
} from "../bindings";
import "../i18n";
import { UndoButton } from "../shell/UndoButton";
import { tx } from "../test/tx";
import { ReviewScreen } from "./ReviewScreen";

// "Same track twice in your Library" (1bE-9): Keep both, or Merge, which
// opens the choice of the track that stays and its confirmation. The Rust
// side is stood in for: a queue an action takes the item out of and an undo
// puts it back into.

const ITEM: ReviewItem = {
  id: { kind: "sameRecording", a: 4, b: null },
  kind: "sameRecording",
  tracks: [
    { recordingId: 4, title: "Made Up Tune", artist: "Made Up Artist" },
    { recordingId: 9, title: "Made Up Tune (copy)", artist: "Made Up Artist" },
  ],
  moreTracks: 0,
};

function row(fileId: number, recordingId: number, name: string): ReviewFileRow {
  return {
    fileId,
    source: "file",
    recordingId,
    name,
    path: "E:\\Music\\" + name,
    onDisk: true,
    driveConnected: true,
    linked: true,
    format: "mp3",
    verdict: null,
    cutoffHz: null,
    durationMs: 215000,
    tagsFilled: 3,
    tagsTotal: 6,
    hasArt: false,
    rekordbox: "trusted",
    bitrateKbps: 320,
    sampleRateHz: 44100,
    addedAt: "2026-09-30T10:00:00.000Z",
  };
}

const OFFER: SameRecordingOffer = {
  tracks: [
    {
      recordingId: 4,
      libraryTrackId: 1,
      linkedFileId: 40,
      rekordbox: { cues: 2, stars: 4, plays: 40 },
      crates: 0,
    },
    {
      recordingId: 9,
      libraryTrackId: 2,
      linkedFileId: 90,
      rekordbox: null,
      crates: 3,
    },
  ],
  keep: 4,
  linkedVersions: false,
};

function comparison(offer: SameRecordingOffer): ReviewComparison {
  return {
    id: ITEM.id,
    kind: ITEM.kind,
    group: "needsCloserLook",
    tracks: ITEM.tracks,
    doubt: [],
    namesSayVersions: false,
    suggestion: {
      what: 40,
      reasons: [{ key: "review:keep.moreCues", params: { count: 2, other: 0 }, source: "fact" }],
    },
    rows: [row(40, 4, "tune.mp3"), row(90, 9, "tune copy.mp3")],
    moreFiles: 0,
    switch: null,
    copies: [],
    relink: null,
    sameRecording: offer,
  };
}

interface World {
  queued: boolean;
  offer: SameRecordingOffer;
  /** What a merge asks, by the track that stays (1cB-20). */
  questions: (keep: number) => MergeField[];
  refuse: Record<string, { kind: string; params: object }>;
  calls: { cmd: string; args: Record<string, unknown> }[];
  undo: OperationInfo | null;
}

function backend(start: Partial<World> = {}): World {
  const world: World = {
    queued: true,
    offer: OFFER,
    questions: () => [],
    refuse: {},
    calls: [],
    undo: null,
    ...start,
  };
  const act = (kind: string) => {
    world.queued = false;
    world.undo = { id: 100, kind, details: { name: null, from: null, tracks: 1, newToLibrary: null, newCrate: null } };
    return 100;
  };
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    world.calls.push({ cmd, args });
    const refused = world.refuse[cmd];
    if (refused) throw refused;
    const items = world.queued ? [ITEM] : [];
    switch (cmd) {
      case "review_conflict_fields":
        return [];
      case "missing_tracks":
        return { total: 0, groups: [] };
      case "review_counts":
        return { needsCloserLook: items.length, clearBestFile: 0, qualityProblems: 0, total: items.length };
      case "review_items": {
        const group = args.group as string;
        const listed = group === "needsCloserLook" ? items : [];
        return { group, total: listed.length, items: listed };
      }
      case "review_item":
        return world.queued ? comparison(world.offer) : null;
      case "review_accept_all_items":
        return [];
      case "review_keep_both":
        return { operationId: act("keep_both_library_tracks") };
      case "review_merge_questions":
        return world.questions(args.keep as number);
      case "review_merge":
        return { operationId: act("merge_library_tracks"), kept: args.keep, merged: [9] };
      case "next_undo_operation":
        return { operation: world.undo, refusal: null } satisfies NextUndo;
      case "undo_last_operation": {
        const undone = world.undo;
        if (!undone) return { status: "nothingToUndo" };
        world.queued = true;
        world.undo = null;
        return { status: "undone", operation: undone };
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

async function actions() {
  return within(await screen.findByRole("group", { name: tx("review:action.label") }));
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("a Same track twice item", () => {
  it("shows each Library track's cues, rating, plays and crates, the suggested one marked", async () => {
    backend();
    renderScreen();
    const table = await screen.findByRole("table", { name: tx("review:same.label") });
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows).toHaveLength(2);
    const first = within(rows[0]!);
    expect(first.getByText(tx("review:mark.suggested"))).toBeInTheDocument();
    expect(first.getByText(tx("review:same.stars", { count: 4 }))).toBeInTheDocument();
    expect(first.getByText("40")).toBeInTheDocument();
    const second = within(rows[1]!);
    expect(second.getByText(tx("review:same.notInRekordbox"))).toBeInTheDocument();
    expect(second.getByText("3")).toBeInTheDocument();
    expect(second.queryByText(tx("review:mark.suggested"))).toBeNull();
  });

  it("offers Merge as the one filled button and Keep both", async () => {
    backend();
    renderScreen();
    const buttons = await actions();
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.merge"),
      tx("review:action.keepBoth"),
    ]);
  });

  it("keeps both with the tracks it showed, leaves the queue, and Undo brings it back", async () => {
    const world = backend();
    renderScreen();
    const buttons = await actions();
    await userEvent.click(buttons.getByRole("button", { name: tx("review:action.keepBoth") }));
    expect(callsOf(world, "review_keep_both")[0]?.args).toEqual({ anchor: 4, recordingIds: [4, 9] });
    await waitFor(() => expect(screen.queryByRole("table", { name: tx("review:same.label") })).toBeNull());
    await userEvent.click(await screen.findByRole("button", { name: tx("shell:undo.action.keepBoth") }));
    expect(await screen.findByRole("table", { name: tx("review:same.label") })).toBeInTheDocument();
  });

  it("merges only on the confirmation, into the chosen track, and Go back changes nothing", async () => {
    const world = backend();
    renderScreen();
    await userEvent.click((await actions()).getByRole("button", { name: tx("review:action.merge") }));
    // The choice and the confirmation: nothing merged yet, "Go back" focused.
    const dialog = await screen.findByRole("alertdialog", { name: tx("review:same.choose") });
    expect(callsOf(world, "review_merge")).toHaveLength(0);
    expect(within(dialog).getByRole("button", { name: tx("review:action.goBack") })).toHaveFocus();
    expect(within(dialog).getByText(tx("review:same.confirmValues", { count: 1 }))).toBeInTheDocument();
    // The suggested track is chosen.
    const radios = within(dialog).getAllByRole("radio");
    expect(radios.map((r) => (r as HTMLInputElement).checked)).toEqual([true, false]);

    await userEvent.click(within(dialog).getByRole("button", { name: tx("review:action.goBack") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(callsOf(world, "review_merge")).toHaveLength(0);

    await userEvent.click((await actions()).getByRole("button", { name: tx("review:action.merge") }));
    const again = await screen.findByRole("alertdialog", { name: tx("review:same.choose") });
    await userEvent.click(within(again).getAllByRole("radio")[1]!);
    await userEvent.click(within(again).getByRole("button", { name: tx("review:action.mergeConfirm") }));
    expect(callsOf(world, "review_merge")[0]?.args).toEqual({
      anchor: 4,
      recordingIds: [4, 9],
      keep: 9,
      choices: [],
    });
    await waitFor(() => expect(screen.queryByRole("table", { name: tx("review:same.label") })).toBeNull());
    // Undo names the merge and brings the item back.
    await userEvent.click(await screen.findByRole("button", { name: tx("shell:undo.action.mergeTracks") }));
    expect(await screen.findByRole("table", { name: tx("review:same.label") })).toBeInTheDocument();
  });

  it("says why a merge was refused", async () => {
    backend({ refuse: { review_merge: { kind: "reviewMergeLinkedVersions", params: {} } } });
    renderScreen();
    await userEvent.click((await actions()).getByRole("button", { name: tx("review:action.merge") }));
    const dialog = await screen.findByRole("alertdialog");
    await userEvent.click(within(dialog).getByRole("button", { name: tx("review:action.mergeConfirm") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("review:error.mergeLinkedVersions"));
  });

  it("offers only Keep both for tracks linked as versions, and says why", async () => {
    backend({ offer: { ...OFFER, keep: null, linkedVersions: true } });
    renderScreen();
    const buttons = await actions();
    expect(buttons.getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("review:action.keepBoth"),
    ]);
    expect(screen.getByText(tx("review:error.mergeLinkedVersions"))).toBeInTheDocument();
  });

  it("opens the confirmation with the track of a number key's row chosen; Enter does nothing", async () => {
    const world = backend();
    renderScreen();
    await actions();
    await userEvent.keyboard("{Enter}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(world.calls.filter((c) => c.cmd.startsWith("review_merge") || c.cmd === "review_keep_both")).toHaveLength(0);
    await userEvent.keyboard("2");
    const dialog = await screen.findByRole("alertdialog", { name: tx("review:same.choose") });
    const radios = within(dialog).getAllByRole("radio");
    expect(radios.map((r) => (r as HTMLInputElement).checked)).toEqual([false, true]);
    expect(callsOf(world, "review_merge")).toHaveLength(0);
  });

  describe("asks field by field where the two tracks differ (1cB-20)", () => {
    const STAYS = tx("review:item.trackName", { artist: "Made Up Artist", title: "Made Up Tune" });
    const LEAVES = tx("review:item.trackName", { artist: "Made Up Artist", title: "Made Up Tune (copy)" });

    /** The rating differs (4 stars against 2) and so does the genre; the stayer comes first. */
    const questions = (keep: number): MergeField[] => {
      const other = keep === 4 ? 9 : 4;
      const rating = (id: number) => (id === 4 ? "204" : "102");
      const genre = (id: number) => (id === 4 ? "Techno" : "House");
      return [
        {
          field: "Genre",
          kind: "text",
          values: [keep, other].map((recordingId) => ({ recordingId, value: genre(recordingId) })),
          chosen: keep,
        },
        {
          field: "Rating",
          kind: "rating",
          values: [keep, other].map((recordingId) => ({ recordingId, value: rating(recordingId) })),
          chosen: keep,
        },
      ];
    };

    async function openChoice() {
      renderScreen();
      await userEvent.click((await actions()).getByRole("button", { name: tx("review:action.merge") }));
      return screen.findByRole("alertdialog", { name: tx("review:same.choose") });
    }

    /** The radio that keeps `field` from the track `name`. */
    const radio = (dialog: HTMLElement, field: string, name: string) =>
      within(dialog).getByRole("radio", {
        name: tx("review:same.values.use", { name, field: tx(`conflicts:field.${field}`) }),
      }) as HTMLInputElement;

    it("shows a row per field with each track's value, the staying track's picked, and says which stays", async () => {
      backend({ questions });
      const dialog = await openChoice();
      await within(dialog).findByText(tx("review:same.values.heading"));
      const heads = within(dialog).getAllByRole("columnheader");
      expect(heads[0]).toHaveTextContent(tx("review:same.values.field"));
      expect(heads[1]).toHaveTextContent(tx("review:same.values.stays"));
      expect(heads[2]).not.toHaveTextContent(tx("review:same.values.stays"));
      for (const field of ["Genre", "Rating"]) {
        expect(radio(dialog, field, STAYS).checked).toBe(true);
        expect(radio(dialog, field, LEAVES).checked).toBe(false);
      }
      // The values are shown by their kind: stars for the rating.
      expect(within(dialog).getByRole("img", { name: tx("conflicts:stars", { count: 4 }) })).toBeVisible();
      expect(within(dialog).getByRole("img", { name: tx("conflicts:stars", { count: 2 }) })).toBeVisible();
    });

    it("sends what was picked with Merge, the default for the rows left alone, and Undo takes it all back", async () => {
      const world = backend({ questions });
      const dialog = await openChoice();
      await userEvent.click(await waitFor(() => radio(dialog, "Rating", LEAVES)));
      expect(radio(dialog, "Rating", LEAVES).checked).toBe(true);
      expect(radio(dialog, "Genre", STAYS).checked).toBe(true);
      await userEvent.click(within(dialog).getByRole("button", { name: tx("review:action.mergeConfirm") }));
      expect(callsOf(world, "review_merge")[0]?.args).toEqual({
        anchor: 4,
        recordingIds: [4, 9],
        keep: 4,
        choices: [
          { field: "Genre", from: 4, value: "Techno" },
          { field: "Rating", from: 9, value: "102" },
        ],
      });
      await waitFor(() => expect(screen.queryByRole("table", { name: tx("review:same.label") })).toBeNull());
      await userEvent.click(await screen.findByRole("button", { name: tx("shell:undo.action.mergeTracks") }));
      expect(await screen.findByRole("table", { name: tx("review:same.label") })).toBeInTheDocument();
    });

    it("asks again, with the picks starting over, when another track is chosen to stay", async () => {
      const world = backend({ questions });
      const dialog = await openChoice();
      await userEvent.click(await waitFor(() => radio(dialog, "Rating", LEAVES)));
      await userEvent.click(within(dialog).getByRole("radio", { name: LEAVES }));
      await waitFor(() =>
        expect(callsOf(world, "review_merge_questions").map((c) => c.args.keep)).toEqual([4, 9]),
      );
      // The other track stays now: its values are the ones picked.
      await waitFor(() => expect(radio(dialog, "Rating", LEAVES).checked).toBe(true));
      await userEvent.click(within(dialog).getByRole("button", { name: tx("review:action.mergeConfirm") }));
      const sent = callsOf(world, "review_merge")[0]?.args as { keep: number; choices: { from: number }[] };
      expect(sent.keep).toBe(9);
      expect(sent.choices.map((c) => c.from)).toEqual([9, 9]);
    });

    it("shows no rows, and merges with no choices, where nothing differs", async () => {
      const world = backend();
      const dialog = await openChoice();
      await waitFor(() => expect(callsOf(world, "review_merge_questions")).toHaveLength(1));
      expect(within(dialog).queryByText(tx("review:same.values.heading"))).toBeNull();
      await userEvent.click(within(dialog).getByRole("button", { name: tx("review:action.mergeConfirm") }));
      expect(callsOf(world, "review_merge")[0]?.args).toMatchObject({ choices: [] });
    });
  });
});

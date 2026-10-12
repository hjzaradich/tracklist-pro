import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type {
  ReviewComparison,
  ReviewCounts,
  ReviewFileRow,
  ReviewGroup,
  ReviewItem,
  ReviewItemId,
} from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { ReviewScreen } from "./ReviewScreen";

function item(kind: ReviewItem["kind"], a: number, titles: string[]): ReviewItem {
  return {
    id: { kind, a, b: titles.length > 1 ? a + 1 : null },
    kind,
    tracks: titles.map((title, n) => ({ recordingId: a + n, title, artist: "Made Up Artist" })),
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

const COUNTS: ReviewCounts = {
  needsCloserLook: 2,
  clearBestFile: 1,
  qualityProblems: 0,
  total: 3,
};

const DOUBT = item("versionDoubt", 10, ["Synthetic Tune", "Synthetic Tune Dirty"]);
const HELD = item("heldSet", 20, ["Another Tune", "Another Tune b", "Another Tune c"]);
const CLEAR = item("clearBestFile", 30, ["Gone Tune"]);

/** What selecting `DOUBT` shows: a FLAC suggested over the linked MP3. */
const DOUBT_COMPARISON: ReviewComparison = {
  id: DOUBT.id,
  kind: "versionDoubt",
  group: "needsCloserLook",
  tracks: DOUBT.tracks,
  doubt: [{ key: "review:doubt.sameAudioNamesDisagree", params: {}, source: "fact" }],
  suggestion: {
    what: 2,
    reasons: [
      {
        key: "review:reason.betterFormat",
        params: { best: "FLAC", other: "MP3" },
        source: "fact",
      },
    ],
  },
  rows: [
    row(2, "tune dirty.flac", { recordingId: 11, format: "flac" }),
    row(1, "tune.mp3", { recordingId: 10, linked: true, verdict: "lowBitrate", cutoffHz: 16000 }),
  ],
  moreFiles: 0,
};

const CLEAR_COMPARISON: ReviewComparison = {
  id: CLEAR.id,
  kind: "clearBestFile",
  group: "clearBestFile",
  tracks: CLEAR.tracks,
  doubt: [],
  suggestion: {
    what: 4,
    reasons: [{ key: "review:reason.notOnDisk", params: {}, source: "fact" }],
  },
  rows: [row(4, "gone copy.mp3"), row(3, "gone.mp3", { linked: true, onDisk: false })],
  moreFiles: 0,
};

const QUALITY = item("qualityProblem", 40, ["Flagged Tune"]);

/** A flagged file with no better copy: nothing to suggest. */
const QUALITY_COMPARISON: ReviewComparison = {
  id: QUALITY.id,
  kind: "qualityProblem",
  group: "qualityProblems",
  tracks: QUALITY.tracks,
  doubt: [],
  suggestion: null,
  rows: [row(5, "flagged.mp3", { linked: true, verdict: "truncated" })],
  moreFiles: 0,
};

const COMPARISONS = [DOUBT_COMPARISON, CLEAR_COMPARISON, QUALITY_COMPARISON];

interface Backend {
  counts?: ReviewCounts | "broken";
  pages?: Partial<Record<ReviewGroup, ReviewItem[]>>;
  /** How many items a group says it has, when more than it lists. */
  totals?: Partial<Record<ReviewGroup, number>>;
}

/** Stands in for the Rust side. Returns the calls it got, for the test to read. */
function backend({ counts = COUNTS, pages, totals = {} }: Backend = {}) {
  const lists: Record<ReviewGroup, ReviewItem[]> = {
    needsCloserLook: [DOUBT, HELD],
    clearBestFile: [CLEAR],
    qualityProblems: [],
    ...pages,
  };
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    if (cmd === "missing_tracks") return { total: 0, groups: [] };
    if (cmd === "review_counts") {
      if (counts === "broken") throw { kind: "database", params: {} };
      return counts;
    }
    if (cmd === "review_items") {
      const { group, offset, limit } = args as { group: ReviewGroup; offset: number; limit: number };
      return {
        group,
        total: totals[group] ?? lists[group].length,
        items: lists[group].slice(offset, offset + limit),
      };
    }
    if (cmd === "review_item") {
      const { id } = args as { id: ReviewItemId };
      return COMPARISONS.find((c) => c.id.kind === id.kind && c.id.a === id.a) ?? null;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return calls;
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ReviewScreen />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  clearMocks();
  cleanup();
});

describe("the Review queue", () => {
  it("groups the items in the roadmap's order, with how many are left in each group", async () => {
    backend();
    renderScreen();
    const needs = await screen.findByRole("region", { name: tx("review:group.needsCloserLook") });
    expect(await within(needs).findAllByRole("option")).toHaveLength(2);
    expect(within(needs).getByText(tx("review:queue.left", { count: 2 }))).toBeInTheDocument();
    const clear = await screen.findByRole("region", { name: tx("review:group.clearBestFile") });
    const headings = screen.getAllByRole("heading", { level: 2 });
    expect(headings.slice(0, 2).map((h) => h.textContent)).toEqual([
      tx("review:group.needsCloserLook"),
      tx("review:group.clearBestFile"),
    ]);
    expect(within(clear).getByText(tx("review:queue.left", { count: 1 }))).toBeInTheDocument();
  });

  it("leaves out a group with nothing in it", async () => {
    backend();
    renderScreen();
    await screen.findByRole("region", { name: tx("review:group.needsCloserLook") });
    expect(screen.queryByRole("region", { name: tx("review:group.qualityProblems") })).toBeNull();
  });

  it("names an item's tracks and what kind of decision it is", async () => {
    backend();
    renderScreen();
    const needs = await screen.findByRole("region", { name: tx("review:group.needsCloserLook") });
    const options = await within(needs).findAllByRole("option");
    const first = options[0] as HTMLElement;
    expect(first).toHaveTextContent(
      tx("review:item.trackName", { artist: "Made Up Artist", title: "Synthetic Tune" }),
    );
    expect(first).toHaveTextContent(
      tx("review:item.trackName", { artist: "Made Up Artist", title: "Synthetic Tune Dirty" }),
    );
    expect(first).toHaveTextContent(tx("review:kind.versionDoubt"));
    // A third track is counted, not listed.
    expect(options[1]).toHaveTextContent(
      tx("review:item.moreTracks", { count: 1 }),
    );
  });

  it("selects the first item and shows its files, the suggestion first, with its reason", async () => {
    backend();
    renderScreen();
    const table = await screen.findByRole("table", { name: tx("review:compare.label") });
    const rows = within(table).getAllByRole("row");
    // The header, then the suggested file, then the linked one.
    expect(rows).toHaveLength(3);
    expect(rows[1]).toHaveTextContent("tune dirty.flac");
    expect(rows[1]).toHaveTextContent(tx("review:mark.suggested"));
    expect(rows[2]).toHaveTextContent("tune.mp3");
    expect(rows[2]).toHaveTextContent(tx("review:mark.linked"));
    expect(rows[2]).toHaveTextContent(tx("review:verdict.lowBitrate"));
    expect(rows[2]).toHaveTextContent(tx("review:cell.cutoff", { khz: 16 }));
    // Why: the suggestion's reason, and why the pair is in the queue.
    expect(
      screen.getByText(tx("review:reason.betterFormat", { best: "FLAC", other: "MP3" })),
    ).toBeInTheDocument();
    expect(
      screen.getByText(tx("review:doubt.sameAudioNamesDisagree"), { exact: false }),
    ).toBeInTheDocument();
  });

  it("shows another item's files when it is selected", async () => {
    backend();
    renderScreen();
    const clear = await screen.findByRole("region", { name: tx("review:group.clearBestFile") });
    await userEvent.click(await within(clear).findByRole("option"));
    const table = await screen.findByRole("table", { name: tx("review:compare.label") });
    await waitFor(() => expect(within(table).getAllByRole("row")[1]).toHaveTextContent("gone copy.mp3"));
    expect(screen.getByText(tx("review:reason.notOnDisk"))).toBeInTheDocument();
    // The linked file that is gone says so.
    expect(within(table).getAllByRole("row")[2]).toHaveTextContent(tx("library:fileMissing"));
    expect(within(clear).getByRole("option")).toHaveAttribute("aria-selected", "true");
  });

  it("moves through a group with the arrow keys, one tab stop for the list", async () => {
    backend();
    renderScreen();
    const needs = await screen.findByRole("region", { name: tx("review:group.needsCloserLook") });
    const options = await within(needs).findAllByRole("option");
    expect(options.map((o) => o.tabIndex)).toEqual([0, -1]);
    options[0]?.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(options[1]).toHaveFocus();
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    expect(options.map((o) => o.tabIndex)).toEqual([-1, 0]);
    await userEvent.keyboard("{ArrowUp}");
    expect(options[0]).toHaveFocus();
  });

  it("loads the next page of a group on request", async () => {
    const many = Array.from({ length: 150 }, (_, n) => item("versionDoubt", 100 + n * 2, [`Tune ${n}`, `Tune ${n} b`]));
    const calls = backend({
      counts: { needsCloserLook: 150, clearBestFile: 0, qualityProblems: 0, total: 150 },
      pages: { needsCloserLook: many, clearBestFile: [] },
    });
    renderScreen();
    const needs = await screen.findByRole("region", { name: tx("review:group.needsCloserLook") });
    await waitFor(() => expect(within(needs).getAllByRole("option")).toHaveLength(100));
    await userEvent.click(within(needs).getByRole("button", { name: tx("review:queue.more") }));
    await waitFor(() => expect(within(needs).getAllByRole("option")).toHaveLength(150));
    expect(within(needs).queryByRole("button", { name: tx("review:queue.more") })).toBeNull();
    const offsets = calls.filter((c) => c.cmd === "review_items").map((c) => (c.args as { offset: number }).offset);
    // (Page 0 may be asked for twice: this fake answers "gone" for items it has no comparison for.)
    expect([...new Set(offsets)]).toEqual([0, 100]);
  });

  it("says what's absent when nothing waits, and still shows the Missing list", async () => {
    backend({ counts: { needsCloserLook: 0, clearBestFile: 0, qualityProblems: 0, total: 0 } });
    renderScreen();
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
    expect(await screen.findByText(tx("review:missing.empty"))).toBeInTheDocument();
    expect(screen.queryByRole("table", { name: tx("review:compare.label") })).toBeNull();
  });

  it("says the queue can't be read when the backend fails", async () => {
    backend({ counts: "broken" });
    renderScreen();
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:database"));
  });

  it("shows no suggestion for an item that has none, only the file's verdict", async () => {
    backend({
      counts: { needsCloserLook: 0, clearBestFile: 0, qualityProblems: 1, total: 1 },
      pages: { needsCloserLook: [], clearBestFile: [], qualityProblems: [QUALITY] },
    });
    renderScreen();
    const table = await screen.findByRole("table", { name: tx("review:compare.label") });
    expect(within(table).getAllByRole("row")[1]).toHaveTextContent(tx("review:verdict.truncated"));
    expect(screen.queryByText(tx("review:mark.suggested"))).toBeNull();
    expect(screen.queryByText(tx("review:compare.suggested", { name: "flagged.mp3" }))).toBeNull();
  });
});

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { LibraryTrack, Overview, Preflight, ReviewCounts, SendState } from "../bindings";
import { tx } from "../test/tx";

// The Overview's top half (1cB-11): the header with Send, what's waiting, and
// the progress line.

/** Matches text that contains `text`, whatever characters it holds. */
function contains(text: string): RegExp {
  return new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, String.raw`\$&`));
}

function libraryTrack(id: number): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: null,
    file: { id: 1, path: String.raw`E:\Music\tune.mp3`, name: "tune.mp3", present: true, driveConnected: true },
    fragile: null,
    sourceMissing: false,
    openConflicts: 0,
    addedAt: "2026-10-01T10:00:00.000Z",
    betterFile: null,
  };
}

const NOTHING_WAITING: Overview = {
  tracks: 3,
  totalLengthMs: 600_000,
  inCrates: 3,
  inNoCrate: 0,
  analyzedByRekordbox: 3,
  notAnalyzed: 0,
  rekordboxRead: true,
  lowQuality: 0,
  lastSentMs: Date.UTC(2026, 9, 20, 12, 0),
  allMusic: 3,
  addedThisWeek: 0,
  decisionsThisWeek: 0,
  genres: [],
  noGenre: 0,
  missingTags: 0,
  missingArt: 0,
  counting: { files: false, rekordbox: false, quality: false },
};

/** The to-do list's own region (the sidebar has lists of its own). */
function todoList() {
  return screen.getByRole("region", { name: tx("overview:todo.title") });
}

const NO_REVIEW: ReviewCounts = {
  needsCloserLook: 0,
  clearBestFile: 0,
  qualityProblems: 0,
  conflicts: 0,
  total: 0,
};

function preflight(over: Partial<Preflight>): Preflight {
  return {
    token: "t1",
    export: { path: String.raw`C:\export.xml`, modifiedMs: 0, readAt: "2026-10-27T10:00:00.000Z", notStored: 0 },
    newTracks: 0,
    knownTracks: 0,
    leftOut: [],
    fileMissing: [],
    noFileAtLocation: [],
    otherFile: [],
    noKind: [],
    losesEntries: [],
    conflictsWaiting: 0,
    refusal: null,
    nothingToSend: false,
    canSend: true,
    exportOlderThanLastSend: false,
    needsConfirm: false,
    ...over,
  } as Preflight;
}

interface World {
  library?: number;
  /** The Library can't be read. */
  libraryBroken?: boolean;
  overview?: Partial<Overview>;
  /** The Overview command fails. */
  overviewBroken?: boolean;
  /** The Overview command doesn't answer until `release()`. */
  overviewHeld?: boolean;
  review?: Partial<ReviewCounts>;
  preflight?: Preflight | null;
}

/** Stands in for the Rust side, and keeps the names of the commands asked. */
function fakeBackend(world: World = {}) {
  const calls: string[] = [];
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  mockIPC(
    async (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "library_tracks":
          if (world.libraryBroken) throw { kind: "database", params: {} };
          return Array.from({ length: world.library ?? 3 }, (_, n) => libraryTrack(n + 1));
        case "overview":
          if (world.overviewHeld) await gate;
          if (world.overviewBroken) throw { kind: "database", params: {} };
          return { ...NOTHING_WAITING, ...world.overview };
        case "review_counts":
          return { ...NO_REVIEW, ...world.review };
        case "send_state": {
          const state: SendState = {
            revision: 0,
            filePath: String.raw`C:\data\send.xml`,
            exportPath: null,
            preflight: world.preflight ?? null,
            step: null,
            failure: null,
            sent: null,
          };
          return state;
        }
        case "rekordbox_offer":
          return { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
        case "after_send_lists":
          return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [] };
        case "rekordbox_xml_source":
          return {
            path: null,
            watch: false,
            lastRead: null,
            lastFailure: null,
            newerExport: null,
            exportFolder: null,
          };
        case "music_folders":
          return [];
        case "read_online_only_files":
          return false;
        case "key_notation":
          return "camelot";
        case "table_columns":
          return null;
        case "track_table_page":
          return { total: 0, offset: 0, rows: [], behind: false };
        case "list_crates":
          return [];
        case "missing_tracks":
          return { total: 0, groups: [] };
        case "review_conflict_fields":
          return [];
        case "review_items":
          return { total: 0, items: [] };
        default:
          // The shell's own commands (Activity) aren't this test's business.
          throw { kind: "internal", params: {} };
      }
    },
    { shouldMockEvents: true },
  );
  return { calls, release };
}

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

/** The send's two steps: both write or read on the user's own click. */
const SEND_STEPS = ["write_send", "prepare_send"];

describe("Send on the Overview", () => {
  it("opens the send review and doesn't send", async () => {
    const { calls } = fakeBackend();
    renderApp("/overview");
    await userEvent.click(await screen.findByRole("button", { name: tx("send:open") }));
    expect(await screen.findByRole("heading", { level: 2, name: tx("send:title") })).toBeInTheDocument();
    // Opening the review reads nothing and writes nothing: the user's own steps do.
    expect(calls.filter((cmd) => SEND_STEPS.includes(cmd))).toEqual([]);
  });

  it("is the Overview's one filled button", async () => {
    fakeBackend({ library: 3 });
    renderApp("/overview");
    const send = await screen.findByRole("button", { name: tx("send:open") });
    // Every filled button takes the class Send takes.
    const filled = screen.getAllByRole("button").filter((button) => button.className === send.className);
    expect(filled).toEqual([send]);
  });

  it("is greyed out, and names the reason, when the Library can't be read", async () => {
    fakeBackend({ libraryBroken: true });
    renderApp("/overview");
    const send = await screen.findByRole("button", { name: tx("send:open") });
    expect(send).toBeDisabled();
    expect(send).toHaveAccessibleDescription(tx("overview:header.unreadable"));
  });

  it("is available once the Library is read", async () => {
    fakeBackend();
    renderApp("/overview");
    expect(await screen.findByRole("button", { name: tx("send:open") })).toBeEnabled();
    expect(screen.queryByText(tx("overview:header.unreadable"))).toBeNull();
  });

  it("stays available, and says what the last read found, when it found nothing to send", async () => {
    fakeBackend({ preflight: preflight({ nothingToSend: true, canSend: false }) });
    renderApp("/overview");
    expect(await screen.findByRole("button", { name: tx("send:open") })).toBeEnabled();
    expect(await screen.findByText(contains(tx("send:review.nothing")))).toBeInTheDocument();
  });

  it("stays available, and says why, when the last read refused the send", async () => {
    fakeBackend({
      preflight: preflight({ refusal: { reason: "incompleteExport", path: [] }, canSend: false }),
    });
    renderApp("/overview");
    expect(await screen.findByRole("button", { name: tx("send:open") })).toBeEnabled();
    expect(
      await screen.findByText(contains(tx("send:review.refused.incompleteExport"))),
    ).toBeInTheDocument();
  });
});

describe("the Overview's header", () => {
  it("says how big the Library is and when it was last sent", async () => {
    fakeBackend({ library: 3 });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:header.library", { count: 3 }))).toBeInTheDocument();
    const lastSent = tx("overview:header.lastSent", { when: "" }).trim();
    expect(await screen.findByText(contains(lastSent))).toBeInTheDocument();
  });

  it("says when the Library was never sent", async () => {
    fakeBackend({ overview: { lastSentMs: null } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:header.neverSent"))).toBeInTheDocument();
  });
});

interface Line {
  name: string;
  overview?: Partial<Overview>;
  review?: Partial<ReviewCounts>;
  text: string;
  door: string;
}

/** Each line of the to-do list: what makes it appear, what it says, and where it goes. */
const LINES: Line[] = [
  {
    name: "reviewNeedsCloserLook",
    review: { needsCloserLook: 3 },
    text: tx("overview:todo.review", { count: 3, group: tx("review:group.needsCloserLook") }),
    door: "/review",
  },
  {
    name: "reviewClearBestFile",
    review: { clearBestFile: 4 },
    text: tx("overview:todo.review", { count: 4, group: tx("review:group.clearBestFile") }),
    door: "/review",
  },
  {
    name: "reviewQualityProblems",
    review: { qualityProblems: 2 },
    text: tx("overview:todo.review", { count: 2, group: tx("review:group.qualityProblems") }),
    door: "/review",
  },
  {
    name: "reviewConflicts",
    review: { conflicts: 5 },
    text: tx("overview:todo.review", { count: 5, group: tx("conflicts:group") }),
    door: "/review",
  },
  {
    name: "lowQuality",
    overview: { lowQuality: 6 },
    text: tx("overview:todo.lowQuality", { count: 6 }),
    door: "/library",
  },
  {
    name: "missingTags",
    overview: { missingTags: 7 },
    text: tx("overview:todo.missingTags", { count: 7 }),
    door: "/library",
  },
  {
    name: "missingArt",
    overview: { missingArt: 8 },
    text: tx("overview:todo.missingArt", { count: 8 }),
    door: "/library",
  },
  {
    name: "notAnalyzed",
    overview: { notAnalyzed: 9 },
    text: tx("overview:todo.notAnalyzed", { count: 9 }),
    door: "/library",
  },
  {
    name: "inNoCrate",
    overview: { inNoCrate: 1 },
    text: tx("overview:todo.inNoCrate", { count: 1 }),
    door: "/library",
  },
];

describe("what to do next", () => {
  it.each(LINES)("shows its count and goes to the right place: $name", async (line) => {
    fakeBackend({ overview: line.overview, review: line.review });
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: line.text }));
    await waitFor(() => expect(router.state.location.pathname).toBe(line.door));
  });

  it("shows only the lines with something waiting, Review first", async () => {
    fakeBackend({
      review: { needsCloserLook: 3, conflicts: 5 },
      overview: { missingTags: 7, inNoCrate: 2 },
    });
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("overview:todo.inNoCrate", { count: 2 }) });
    const lines = within(todoList())
      .getAllByRole("link")
      .map((link) => link.textContent);
    expect(lines).toEqual([
      tx("overview:todo.review", { count: 3, group: tx("review:group.needsCloserLook") }),
      tx("overview:todo.review", { count: 5, group: tx("conflicts:group") }),
      tx("overview:todo.missingTags", { count: 7 }),
      tx("overview:todo.inNoCrate", { count: 2 }),
    ]);
    expect(screen.queryByText(tx("overview:todo.empty"))).toBeNull();
  });

  it("shows the empty state, naming what's absent, when nothing waits", async () => {
    fakeBackend();
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:todo.empty"))).toBeInTheDocument();
    expect(within(todoList()).queryByRole("list")).toBeNull();
    expect(screen.queryByText(tx("overview:todo.counting"))).toBeNull();
  });

  it("doesn't count tracks as not analyzed before rekordbox has been read", async () => {
    fakeBackend({ overview: { rekordboxRead: false, notAnalyzed: 3 } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:todo.empty"))).toBeInTheDocument();
  });

  it("says it is still counting, rather than calling the list empty, while background work runs", async () => {
    fakeBackend({ overview: { counting: { files: true, rekordbox: false, quality: false } } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:todo.counting"))).toBeInTheDocument();
    expect(screen.queryByText(tx("overview:todo.empty"))).toBeNull();
  });

  it("marks a line whose number is still being counted", async () => {
    fakeBackend({
      overview: { missingTags: 4, counting: { files: true, rekordbox: false, quality: false } },
    });
    renderApp("/overview");
    const link = await screen.findByRole("link", {
      name: contains(tx("overview:todo.missingTags", { count: 4 })),
    });
    expect(link).toHaveTextContent(tx("overview:todo.lineCounting"));
  });

  it("shows the Review lines while the slower numbers are still on their way", async () => {
    const { release } = fakeBackend({ overviewHeld: true, review: { qualityProblems: 2 } });
    renderApp("/overview");
    const name = tx("overview:todo.review", { count: 2, group: tx("review:group.qualityProblems") });
    expect(await screen.findByRole("link", { name })).toBeInTheDocument();
    expect(screen.getByText(tx("overview:todo.counting"))).toBeInTheDocument();
    expect(screen.queryByText(tx("overview:todo.empty"))).toBeNull();

    release();
    await waitFor(() => expect(screen.queryByText(tx("overview:todo.counting"))).toBeNull());
    expect(screen.getByRole("link", { name })).toBeInTheDocument();
  });

  it("says so when the numbers can't be read, and doesn't call the list empty", async () => {
    fakeBackend({ overviewBroken: true });
    renderApp("/overview");
    expect(await screen.findByText(tx("errors:database"))).toHaveAttribute("role", "alert");
    expect(screen.queryByText(tx("overview:todo.empty"))).toBeNull();
  });
});

describe("the progress line", () => {
  it("shows the numbers the command counted", async () => {
    fakeBackend({ overview: { addedThisWeek: 4, decisionsThisWeek: 12 } });
    renderApp("/overview");
    expect(
      await screen.findByText(
        tx("overview:progress.line", {
          added: tx("overview:progress.added", { count: 4 }),
          decisions: tx("overview:progress.decisions", { count: 12 }),
        }),
      ),
    ).toBeInTheDocument();
  });

  it("says plainly when nothing was added, and when one decision was made", async () => {
    fakeBackend({ overview: { addedThisWeek: 0, decisionsThisWeek: 1 } });
    renderApp("/overview");
    expect(
      await screen.findByText(
        tx("overview:progress.line", {
          added: tx("overview:progress.added", { count: 0 }),
          decisions: tx("overview:progress.decisions", { count: 1 }),
        }),
      ),
    ).toBeInTheDocument();
  });

  it("isn't drawn until the numbers arrive", async () => {
    const { release } = fakeBackend({ overviewHeld: true });
    renderApp("/overview");
    await screen.findByText(tx("overview:todo.counting"));
    const progress = contains(tx("overview:progress.added", { count: 0 }));
    expect(screen.queryByText(progress)).toBeNull();
    release();
    expect(await screen.findByText(progress)).toBeInTheDocument();
  });
});

describe("the Overview while the Library is empty", () => {
  it("shows no header, list or progress line, and doesn't ask for the numbers", async () => {
    const { calls } = fakeBackend({ library: 0 });
    renderApp("/overview");
    expect(await screen.findByRole("heading", { name: tx("firstRun:title") })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: tx("overview:todo.title") })).toBeNull();
    expect(screen.queryByRole("button", { name: tx("send:open") })).toBeNull();
    expect(calls).not.toContain("overview");
  });
});

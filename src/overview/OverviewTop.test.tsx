import { emit } from "@tauri-apps/api/event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { act, cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { JobUpdate, Overview, ReviewCounts } from "../bindings";
import { fakeBackend, preflight } from "../test/overviewBackend";
import { tx } from "../test/tx";

// The Overview's top half (1cB-11): the header with Send, what's waiting, and
// the progress line.

/** Matches text that contains `text`, whatever characters it holds. */
function contains(text: string): RegExp {
  return new RegExp(text.replace(/[.*+?^${}()|[\]\\]/g, String.raw`\$&`));
}

/** The to-do list's own region (the sidebar has lists of its own). */
function todoList() {
  return screen.getByRole("region", { name: tx("overview:todo.title") });
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

  it("is the Overview's one filled button, the quiet ones being quiet", async () => {
    // Everything on the screen that can be filled is there: a newer export to
    // read and rekordbox tracks to add.
    fakeBackend({ library: 3, newerExport: true, rekordboxToAdd: 2 });
    renderApp("/overview");
    const send = await screen.findByRole("button", { name: tx("send:open") });
    const readNewer = await screen.findByRole("button", { name: tx("rekordbox:readNewer") });
    const add = await screen.findByRole("button", { name: tx("rekordboxOffer:add") });
    // Each stylesheet names its filled buttons "primary" (CSS modules keep the name in the class).
    const filled = screen.getAllByRole("button").filter((button) => /primary/.test(button.className));
    expect(filled).toEqual([send]);
    expect(readNewer).toBeEnabled();
    expect(add).toBeEnabled();
  });

  it("makes Read newer export the filled button only while the Library has no tracks", async () => {
    fakeBackend({ library: 0, newerExport: true });
    renderApp("/overview");
    const readNewer = await screen.findByRole("button", { name: tx("rekordbox:readNewer") });
    expect(readNewer.className).toMatch(/primary/);
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

describe("the check the header repeats", () => {
  it("says when the export was read on a line of its own, never after a full stop", async () => {
    fakeBackend({
      preflight: preflight({ refusal: { reason: "incompleteExport", path: [] }, canSend: false }),
    });
    renderApp("/overview");
    const refusal = await screen.findByText(tx("send:review.refused.incompleteExport"));
    // The refusal is whole on its own line; the export's date is the line after it, not tacked on.
    const exportRead = contains(tx("overview:header.exportRead", { when: "" }).trim());
    expect(refusal).not.toHaveTextContent(exportRead);
    expect(refusal.nextElementSibling).toHaveTextContent(exportRead);
  });

  it("stops saying there is nothing to send once the Library has changed", async () => {
    const world = { library: 3, preflight: preflight({ nothingToSend: true, canSend: false }) };
    fakeBackend(world);
    renderApp("/overview");
    expect(await screen.findByText(contains(tx("send:review.nothing")))).toBeInTheDocument();

    // A track is added: the check was made on a Library that is gone.
    world.library = 4;
    await act(() => emit("job-updates", [endedJob()]));
    await waitFor(() => expect(screen.queryByText(contains(tx("send:review.nothing")))).toBeNull());
    expect(screen.getByRole("button", { name: tx("send:open") })).toBeEnabled();
  });

  it("stops saying it once a track is retitled, though the Library holds as many tracks as before", async () => {
    const world = { library: 3, libraryStamp: 1, preflight: preflight({ nothingToSend: true, canSend: false }) };
    fakeBackend(world);
    renderApp("/overview");
    expect(await screen.findByText(contains(tx("send:review.nothing")))).toBeInTheDocument();

    // The count stays 3; only the change stamp moves, as it does for a retitle.
    world.libraryStamp = 2;
    await act(() => emit("job-updates", [endedJob()]));
    await waitFor(() => expect(screen.queryByText(contains(tx("send:review.nothing")))).toBeNull());
    expect(screen.getByRole("button", { name: tx("send:open") })).toBeEnabled();
  });

  it("keeps saying it while the Library is the same after a job ends", async () => {
    fakeBackend({ library: 3, preflight: preflight({ nothingToSend: true, canSend: false }) });
    renderApp("/overview");
    expect(await screen.findByText(contains(tx("send:review.nothing")))).toBeInTheDocument();

    await act(() => emit("job-updates", [endedJob()]));
    // Give the lists time to come back (they are asked again when a job ends).
    await screen.findByText(tx("overview:todo.empty"));
    expect(screen.getByText(contains(tx("send:review.nothing")))).toBeInTheDocument();
  });
});

function endedJob(): JobUpdate {
  return { seq: 9, id: 9, kind: "scan", status: "done", progress: null, priority: 0 };
}

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
  {
    name: "outsideLibrary",
    overview: { outsideLibrary: 12 },
    text: tx("overview:todo.outsideLibrary", { count: 12 }),
    door: "/all-music",
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
    fakeBackend({ overview: { counting: { files: true, rekordbox: false, quality: false, index: false } } });
    renderApp("/overview");
    expect(await screen.findByText(tx("overview:todo.counting"))).toBeInTheDocument();
    expect(screen.queryByText(tx("overview:todo.empty"))).toBeNull();
  });

  it("marks a line whose number is still being counted", async () => {
    fakeBackend({
      overview: { missingTags: 4, counting: { files: true, rekordbox: false, quality: false, index: false } },
    });
    renderApp("/overview");
    const link = await screen.findByRole("link", {
      name: contains(tx("overview:todo.missingTags", { count: 4 })),
    });
    expect(link).toHaveTextContent(tx("overview:todo.lineCounting"));
  });

  it("marks the low-quality, missing-tags and missing-art lines, and the list, while the index is behind", async () => {
    fakeBackend({
      overview: {
        lowQuality: 3,
        missingTags: 4,
        missingArt: 2,
        counting: { files: false, rekordbox: false, quality: false, index: true },
      },
    });
    renderApp("/overview");
    for (const [key, count] of [["lowQuality", 3], ["missingTags", 4], ["missingArt", 2]] as const) {
      const link = await screen.findByRole("link", {
        name: contains(tx(`overview:todo.${key}`, { count })),
      });
      expect(link).toHaveTextContent(tx("overview:todo.lineCounting"));
    }
    expect(screen.getByText(tx("overview:todo.counting"))).toBeInTheDocument();
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

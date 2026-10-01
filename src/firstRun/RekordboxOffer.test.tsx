import { QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { AddSummary, JobUpdate, Offer, PlaylistChoice, UndoOutcome } from "../bindings";
import "../i18n";
import { RekordboxOffer } from "./RekordboxOffer";
import type { PlaylistPath } from "./useRekordboxOffer";

function offer(toAdd: number, fields: Partial<Offer> = {}): Offer {
  return {
    toAdd,
    alreadyInLibrary: 0,
    waitingInMissing: 0,
    waitingForConfirmation: 0,
    ...fields,
  };
}

/**
 * Stands in for the Rust side. `whole` is the offer for the whole
 * collection; `inPlaylists` answers a narrowed one. Every call is recorded.
 */
function fakeBackend(options: {
  whole: Offer;
  inPlaylists?: (chosen: PlaylistPath[]) => Offer;
  playlists?: PlaylistChoice[];
  summary?: AddSummary;
  undo?: UndoOutcome;
  /** Commands that fail, as the database being unreadable. */
  broken?: string[];
  /** Commands that don't answer until `release()` is called. */
  held?: string[];
}) {
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const backend = {
    whole: options.whole,
    release,
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
    count: (cmd: string) => backend.calls.filter((call) => call.cmd === cmd).length,
    last: (cmd: string) => backend.calls.filter((call) => call.cmd === cmd).at(-1)?.args,
  };
  mockIPC(
    async (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      backend.calls.push({ cmd, args: a });
      if (options.held?.includes(cmd)) await gate;
      if (options.broken?.includes(cmd)) throw { kind: "database", params: {} };
      if (cmd === "rekordbox_offer") {
        const chosen = a.playlists as PlaylistPath[] | null;
        if (chosen === null) return backend.whole;
        return options.inPlaylists?.(chosen) ?? offer(0);
      }
      if (cmd === "rekordbox_playlists") return options.playlists ?? [];
      if (cmd === "add_rekordbox_tracks") return options.summary;
      if (cmd === "undo_add_rekordbox_tracks") return options.undo;
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return backend;
}

function renderOffer(firstRun = false) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <RekordboxOffer firstRun={firstRun} />
    </QueryClientProvider>,
  );
}

const SUMMARY: AddSummary = {
  added: 3,
  alreadyInLibrary: 5,
  waitingInMissing: 2,
  waitingForConfirmation: 1,
  operationId: 9,
};

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the offer to add rekordbox tracks", () => {
  it("says how many rekordbox tracks aren't in the Library and adds nothing by itself", async () => {
    const backend = fakeBackend({ whole: offer(3) });
    renderOffer();
    expect(await screen.findByRole("status")).toHaveTextContent(
      "3 rekordbox tracks aren't in your Library",
    );
    expect(screen.getByRole("button", { name: "Add them" })).toBeEnabled();
    expect(backend.count("add_rekordbox_tracks")).toBe(0);
  });

  it("writes one track in the singular", async () => {
    fakeBackend({ whole: offer(1) });
    renderOffer();
    expect(await screen.findByRole("status")).toHaveTextContent(
      "1 rekordbox track isn't in your Library",
    );
    expect(screen.getByRole("button", { name: "Add it" })).toBeInTheDocument();
  });

  it("shows nothing when there's nothing to add, whatever is waiting", async () => {
    const backend = fakeBackend({
      whole: offer(0, { alreadyInLibrary: 4, waitingInMissing: 2, waitingForConfirmation: 1 }),
    });
    const { container } = renderOffer();
    await waitFor(() => expect(backend.count("rekordbox_offer")).toBeGreaterThan(0));
    await waitFor(() => expect(container).toBeEmptyDOMElement());
  });

  it("says so when the offer can't be read, instead of looking like nothing to add", async () => {
    fakeBackend({ whole: offer(3), broken: ["rekordbox_offer"] });
    renderOffer();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't read or save the Library. Try again.",
    );
    expect(screen.queryByRole("button", { name: "Add them" })).toBeNull();
  });

  it("marks the add button busy while the add runs", async () => {
    const backend = fakeBackend({
      whole: offer(3),
      summary: SUMMARY,
      held: ["add_rekordbox_tracks"],
    });
    renderOffer();
    const add = await screen.findByRole("button", { name: "Add them" });
    expect(add).not.toHaveAttribute("aria-busy", "true");
    await userEvent.click(add);
    await waitFor(() => expect(add).toHaveAttribute("aria-busy", "true"));
    expect(add).toBeDisabled();

    backend.release();
    expect(await screen.findByRole("button", { name: "Undo" })).toBeEnabled();
    expect(backend.count("add_rekordbox_tracks")).toBe(1);
  });

  it("marks the undo button busy while the undo runs", async () => {
    const backend = fakeBackend({
      whole: offer(3),
      summary: SUMMARY,
      undo: { status: "undone", operation: { id: 9, kind: "add_rekordbox_tracks" } },
      held: ["undo_add_rekordbox_tracks"],
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add them" }));
    const undo = await screen.findByRole("button", { name: "Undo" });
    await userEvent.click(undo);
    await waitFor(() => expect(undo).toHaveAttribute("aria-busy", "true"));
    expect(undo).toBeDisabled();

    backend.release();
    expect(await screen.findByText("Add undone")).toBeInTheDocument();
  });

  it("is titled Start from rekordbox on the first run", async () => {
    fakeBackend({ whole: offer(2) });
    renderOffer(true);
    expect(
      await screen.findByRole("heading", { name: "Start from rekordbox" }),
    ).toBeInTheDocument();
  });

  it("adds the whole collection when asked, then shows the summary's four numbers", async () => {
    const backend = fakeBackend({ whole: offer(3), summary: SUMMARY });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add them" }));

    const summary = await screen.findByRole("list");
    expect(within(summary).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "3 tracks added to your Library",
      "5 already in your Library",
      "2 waiting in Missing (no file found)",
      "1 waiting for a match to be confirmed",
    ]);
    expect(backend.count("add_rekordbox_tracks")).toBe(1);
    expect(backend.last("add_rekordbox_tracks")).toEqual({ playlists: null });
  });

  it("leaves a zero out of the summary", async () => {
    fakeBackend({
      whole: offer(1),
      summary: { ...SUMMARY, added: 1, alreadyInLibrary: 0, waitingInMissing: 0 },
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add it" }));
    const summary = await screen.findByRole("list");
    expect(within(summary).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "1 track added to your Library",
      "1 waiting for a match to be confirmed",
    ]);
  });

  it("undoes the whole add with one undo, by its operation", async () => {
    const backend = fakeBackend({
      whole: offer(3),
      summary: SUMMARY,
      undo: { status: "undone", operation: { id: 9, kind: "add_rekordbox_tracks" } },
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add them" }));
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));

    expect(await screen.findByText("Add undone")).toBeInTheDocument();
    expect(backend.last("undo_add_rekordbox_tracks")).toEqual({ operationId: 9 });
    expect(backend.count("undo_add_rekordbox_tracks")).toBe(1);
    expect(screen.queryByRole("button", { name: "Undo" })).toBeNull();
  });

  it("says so when the add can't be undone any more", async () => {
    fakeBackend({ whole: offer(3), summary: SUMMARY, undo: { status: "nothingToUndo" } });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add them" }));
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Can't undo: the Library has changed since",
    );
  });

  it("goes back to the offer when the summary is closed", async () => {
    const backend = fakeBackend({ whole: offer(3), summary: SUMMARY });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Add them" }));
    await screen.findByRole("list");
    backend.whole = offer(0);
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("asks again when a background task ends, since a read or a relink can change it", async () => {
    const backend = fakeBackend({ whole: offer(0) });
    renderOffer();
    await waitFor(() => expect(backend.count("rekordbox_offer")).toBeGreaterThan(0));
    expect(screen.queryByRole("status")).toBeNull();

    backend.whole = offer(4);
    const relink: JobUpdate = {
      seq: 1,
      id: 7,
      kind: "relink",
      status: "done",
      progress: 1,
      priority: 0,
    };
    await act(() => emit("job-updates", [relink]));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "4 rekordbox tracks aren't in your Library",
    );
    expect(backend.count("add_rekordbox_tracks")).toBe(0);
  });
});

describe("the offer narrowed to chosen playlists", () => {
  const PLAYLISTS: PlaylistChoice[] = [
    { path: ["Peak"], tracks: 12 },
    { path: ["Sets", "Closing"], tracks: 1 },
    { path: ["Sets", "Warmup"], tracks: 8 },
  ];

  function narrowedBackend() {
    return fakeBackend({
      whole: offer(20),
      playlists: PLAYLISTS,
      inPlaylists: (chosen) => offer(chosen.length === 0 ? 0 : chosen.length * 4),
      summary: { ...SUMMARY, added: 8 },
    });
  }

  it("shows the playlist tree, each folder named once above its playlists", async () => {
    narrowedBackend();
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));

    const tree = within(await screen.findByRole("group", { name: "rekordbox playlists" }));
    expect((await tree.findAllByRole("listitem")).map((item) => item.textContent)).toEqual([
      "Peak(12 tracks)",
      "Sets",
      "Closing(1 track)",
      "Warmup(8 tracks)",
    ]);
    expect(tree.getByText("Only the tracks are added, not the playlists.")).toBeInTheDocument();
    // Nothing chosen yet: nothing to add.
    expect(screen.getByRole("status")).toHaveTextContent("No playlists chosen");
    expect(screen.getByRole("button", { name: "Add them" })).toBeDisabled();
  });

  it("doesn't say there are no playlists while the list is still loading", async () => {
    const backend = fakeBackend({
      whole: offer(20),
      playlists: PLAYLISTS,
      held: ["rekordbox_playlists"],
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));
    await waitFor(() => expect(backend.count("rekordbox_playlists")).toBe(1));
    expect(screen.queryByText("No rekordbox playlists")).toBeNull();
    expect(screen.queryByText("No playlists chosen")).toBeNull();

    backend.release();
    expect(await screen.findByRole("checkbox", { name: /Peak/ })).toBeInTheDocument();
  });

  it("says so when the playlists can't be read, instead of saying there are none", async () => {
    fakeBackend({ whole: offer(20), broken: ["rekordbox_playlists"] });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Couldn't read or save the Library. Try again.",
    );
    expect(screen.queryByText("No rekordbox playlists")).toBeNull();
  });

  it("says there are no playlists only once the list has loaded empty", async () => {
    fakeBackend({ whole: offer(20), playlists: [] });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));
    expect(await screen.findByText("No rekordbox playlists")).toBeInTheDocument();
  });

  it("offers and adds only the tracks in the ticked playlists", async () => {
    const backend = narrowedBackend();
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));
    await userEvent.click(await screen.findByRole("checkbox", { name: /Warmup/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: /Peak/ }));

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "8 tracks in the chosen playlists aren't in your Library",
      ),
    );
    expect(backend.count("add_rekordbox_tracks")).toBe(0);

    await userEvent.click(screen.getByRole("button", { name: "Add them" }));
    await screen.findByRole("list");
    expect(backend.last("add_rekordbox_tracks")).toEqual({
      playlists: [["Sets", "Warmup"], ["Peak"]],
    });
  });

  it("forgets the pick when the whole collection is chosen again", async () => {
    const backend = narrowedBackend();
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: "Choose playlists" }));
    await userEvent.click(await screen.findByRole("checkbox", { name: /Peak/ }));
    await userEvent.click(screen.getByRole("button", { name: "Whole collection" }));

    expect(screen.getByRole("status")).toHaveTextContent(
      "20 rekordbox tracks aren't in your Library",
    );
    await userEvent.click(screen.getByRole("button", { name: "Choose playlists" }));
    expect(await screen.findByRole("checkbox", { name: /Peak/ })).not.toBeChecked();
    expect(backend.count("add_rekordbox_tracks")).toBe(0);
  });
});

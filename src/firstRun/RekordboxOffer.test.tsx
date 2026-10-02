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
import { tx } from "../test/tx";

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
      tx("rekordboxOffer:count", { count: 3 }),
    );
    expect(screen.getByRole("button", { name: tx("rekordboxOffer:add") })).toBeEnabled();
    expect(backend.count("add_rekordbox_tracks")).toBe(0);
  });

  it("writes one track in the singular", async () => {
    fakeBackend({ whole: offer(1) });
    renderOffer();
    expect(await screen.findByRole("status")).toHaveTextContent(
      tx("rekordboxOffer:count", { count: 1 }),
    );
    expect(screen.getByRole("button", { name: tx("rekordboxOffer:add") })).toBeInTheDocument();
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
      tx("errors:database"),
    );
    expect(screen.queryByRole("button", { name: tx("rekordboxOffer:add") })).toBeNull();
  });

  it("marks the add button busy while the add runs", async () => {
    const backend = fakeBackend({
      whole: offer(3),
      summary: SUMMARY,
      held: ["add_rekordbox_tracks"],
    });
    renderOffer();
    const add = await screen.findByRole("button", { name: tx("rekordboxOffer:add") });
    expect(add).not.toHaveAttribute("aria-busy", "true");
    await userEvent.click(add);
    await waitFor(() => expect(add).toHaveAttribute("aria-busy", "true"));
    expect(add).toBeDisabled();

    backend.release();
    expect(await screen.findByRole("button", { name: tx("rekordboxOffer:summary.undo") })).toBeEnabled();
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
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));
    const undo = await screen.findByRole("button", { name: tx("rekordboxOffer:summary.undo") });
    await userEvent.click(undo);
    await waitFor(() => expect(undo).toHaveAttribute("aria-busy", "true"));
    expect(undo).toBeDisabled();

    backend.release();
    expect(await screen.findByText(tx("rekordboxOffer:summary.undone"))).toBeInTheDocument();
  });

  it("is titled Start from rekordbox on the first run", async () => {
    fakeBackend({ whole: offer(2) });
    renderOffer(true);
    expect(
      await screen.findByRole("heading", { name: tx("firstRun:fromRekordbox.title") }),
    ).toBeInTheDocument();
  });

  it("adds the whole collection when asked, then shows the summary's four numbers", async () => {
    const backend = fakeBackend({ whole: offer(3), summary: SUMMARY });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));

    const summary = await screen.findByRole("list");
    expect(within(summary).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      tx("rekordboxOffer:summary.added", { count: 3 }),
      tx("rekordboxOffer:summary.alreadyInLibrary", { count: 5 }),
      tx("rekordboxOffer:summary.waitingInMissing", { count: 2 }),
      tx("rekordboxOffer:summary.waitingForConfirmation", { count: 1 }),
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
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));
    const summary = await screen.findByRole("list");
    expect(within(summary).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      tx("rekordboxOffer:summary.added", { count: 1 }),
      tx("rekordboxOffer:summary.waitingForConfirmation", { count: 1 }),
    ]);
  });

  it("undoes the whole add with one undo, by its operation", async () => {
    const backend = fakeBackend({
      whole: offer(3),
      summary: SUMMARY,
      undo: { status: "undone", operation: { id: 9, kind: "add_rekordbox_tracks" } },
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:summary.undo") }));

    expect(await screen.findByText(tx("rekordboxOffer:summary.undone"))).toBeInTheDocument();
    expect(backend.last("undo_add_rekordbox_tracks")).toEqual({ operationId: 9 });
    expect(backend.count("undo_add_rekordbox_tracks")).toBe(1);
    expect(screen.queryByRole("button", { name: tx("rekordboxOffer:summary.undo") })).toBeNull();
  });

  it("says so when the add can't be undone any more", async () => {
    fakeBackend({ whole: offer(3), summary: SUMMARY, undo: { status: "nothingToUndo" } });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:summary.undo") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("rekordboxOffer:summary.cantUndo"),
    );
  });

  it("goes back to the offer when the summary is closed", async () => {
    const backend = fakeBackend({ whole: offer(3), summary: SUMMARY });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:add") }));
    await screen.findByRole("list");
    backend.whole = offer(0);
    await userEvent.click(screen.getByRole("button", { name: tx("rekordboxOffer:summary.done") }));
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("listens for background tasks once, however many offers it reads", async () => {
    fakeBackend({ whole: offer(3), playlists: [{ path: ["Peak"], tracks: 3 }] });
    // Count the listeners the app registers, on the way to the mock.
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: { invoke: (cmd: string, ...rest: unknown[]) => Promise<unknown> };
      }
    ).__TAURI_INTERNALS__;
    const invoke = internals.invoke;
    const listened: unknown[] = [];
    internals.invoke = (cmd, ...rest) => {
      if (cmd === "plugin:event|listen") listened.push((rest[0] as { event: unknown }).event);
      return invoke(cmd, ...rest);
    };

    renderOffer();
    // Both offer queries are in use: the whole collection and the pick.
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    await userEvent.click(await screen.findByRole("checkbox", { name: /Peak/ }));
    expect(listened).toEqual(["job-updates"]);
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
      tx("rekordboxOffer:count", { count: 4 }),
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
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));

    const tree = within(await screen.findByRole("group", { name: tx("rekordboxOffer:playlists.title") }));
    expect((await tree.findAllByRole("listitem")).map((item) => item.textContent)).toEqual([
      `Peak${tx("rekordboxOffer:playlists.tracks", { count: 12 })}`,
      "Sets",
      `Closing${tx("rekordboxOffer:playlists.tracks", { count: 1 })}`,
      `Warmup${tx("rekordboxOffer:playlists.tracks", { count: 8 })}`,
    ]);
    expect(tree.getByText(tx("rekordboxOffer:playlists.note"))).toBeInTheDocument();
    // Nothing chosen yet: nothing to add.
    expect(screen.getByRole("status")).toHaveTextContent(tx("rekordboxOffer:noPlaylistsChosen"));
    expect(screen.getByRole("button", { name: tx("rekordboxOffer:add") })).toBeDisabled();
  });

  it("doesn't say there are no playlists while the list is still loading", async () => {
    const backend = fakeBackend({
      whole: offer(20),
      playlists: PLAYLISTS,
      held: ["rekordbox_playlists"],
    });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    await waitFor(() => expect(backend.count("rekordbox_playlists")).toBe(1));
    expect(screen.queryByText(tx("rekordboxOffer:playlists.empty"))).toBeNull();
    expect(screen.queryByText(tx("rekordboxOffer:noPlaylistsChosen"))).toBeNull();

    backend.release();
    expect(await screen.findByRole("checkbox", { name: /Peak/ })).toBeInTheDocument();
  });

  it("says so when the playlists can't be read, instead of saying there are none", async () => {
    fakeBackend({ whole: offer(20), broken: ["rekordbox_playlists"] });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("errors:database"),
    );
    expect(screen.queryByText(tx("rekordboxOffer:playlists.empty"))).toBeNull();
  });

  it("says there are no playlists only once the list has loaded empty", async () => {
    fakeBackend({ whole: offer(20), playlists: [] });
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    expect(await screen.findByText(tx("rekordboxOffer:playlists.empty"))).toBeInTheDocument();
  });

  it("offers and adds only the tracks in the ticked playlists", async () => {
    const backend = narrowedBackend();
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    await userEvent.click(await screen.findByRole("checkbox", { name: /Warmup/ }));
    await userEvent.click(screen.getByRole("checkbox", { name: /Peak/ }));

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        tx("rekordboxOffer:countInPlaylists", { count: 8 }),
      ),
    );
    expect(backend.count("add_rekordbox_tracks")).toBe(0);

    await userEvent.click(screen.getByRole("button", { name: tx("rekordboxOffer:add") }));
    await screen.findByRole("list");
    expect(backend.last("add_rekordbox_tracks")).toEqual({
      playlists: [["Sets", "Warmup"], ["Peak"]],
    });
  });

  it("forgets the pick when the whole collection is chosen again", async () => {
    const backend = narrowedBackend();
    renderOffer();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    await userEvent.click(await screen.findByRole("checkbox", { name: /Peak/ }));
    await userEvent.click(screen.getByRole("button", { name: tx("rekordboxOffer:wholeCollection") }));

    expect(screen.getByRole("status")).toHaveTextContent(
      tx("rekordboxOffer:count", { count: 20 }),
    );
    await userEvent.click(screen.getByRole("button", { name: tx("rekordboxOffer:choosePlaylists") }));
    expect(await screen.findByRole("checkbox", { name: /Peak/ })).not.toBeChecked();
    expect(backend.count("add_rekordbox_tracks")).toBe(0);
  });
});

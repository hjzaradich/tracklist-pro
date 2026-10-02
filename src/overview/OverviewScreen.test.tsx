import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { AddSummary, LibraryTrack, Offer } from "../bindings";
import { tx } from "../test/tx";

function offer(toAdd: number): Offer {
  return { toAdd, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
}

function libraryTrack(id: number): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: null,
    file: { path: String.raw`E:\Music\tune.mp3`, name: "tune.mp3", present: true, driveConnected: true },
    fragile: null,
    sourceMissing: false,
    openConflicts: 0,
    addedAt: "2026-10-01T10:00:00.000Z",
  };
}

/** Stands in for the Rust side: a Library of `library` tracks and an offer of `toAdd`. */
function fakeBackend(state: {
  library: number;
  toAdd: number;
  /** The Library can't be read. */
  libraryBroken?: boolean;
  /** The Library doesn't answer until `release()` is called. */
  libraryHeld?: boolean;
}) {
  let release = () => {};
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const backend = { ...state, adds: 0, release };
  mockIPC(
    async (cmd) => {
      if (cmd === "library_tracks") {
        if (state.libraryHeld) await gate;
        if (state.libraryBroken) throw { kind: "database", params: {} };
        return Array.from({ length: backend.library }, (_, n) => libraryTrack(n + 1));
      }
      if (cmd === "rekordbox_offer") return offer(backend.toAdd);
      if (cmd === "add_rekordbox_tracks") {
        backend.adds += 1;
        const summary: AddSummary = {
          added: backend.toAdd,
          alreadyInLibrary: 0,
          waitingInMissing: 0,
          waitingForConfirmation: 0,
          operationId: 1,
        };
        backend.library += backend.toAdd;
        backend.toAdd = 0;
        return summary;
      }
      if (cmd === "send_state") {
        return {
          revision: 0,
          filePath: String.raw`C:\data\send.xml`,
          exportPath: null,
          preflight: null,
          step: null,
          failure: null,
          sent: null,
        };
      }
      if (cmd === "after_send_lists") {
        return {
          playlistsChecked: true,
          stalePlaylists: [
            { path: ["Crates", "Old name"], kind: "playlist", playlistsInside: 0, empty: false },
          ],
          manualRemovals: [],
        };
      }
      if (cmd === "music_folders") return [];
      if (cmd === "read_online_only_files") return false;
      if (cmd === "all_music_tracks") return { total: 0, tracks: [] };
      if (cmd === "rekordbox_xml_source") {
        return {
          path: null,
          watch: false,
          lastRead: null,
          lastFailure: null,
          newerExport: null,
          exportFolder: null,
        };
      }
      // The shell's own commands (Activity) aren't this test's business.
      throw { kind: "internal", params: {} };
    },
    { shouldMockEvents: true },
  );
  return backend;
}

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the Overview while the Library is empty (first run)", () => {
  it("leads with picking music folders, then the rekordbox export", async () => {
    fakeBackend({ library: 0, toAdd: 0 });
    renderApp("/overview");
    expect(await screen.findByRole("heading", { name: tx("firstRun:title") })).toBeInTheDocument();
    const panels = screen
      .getAllByRole("heading", { level: 2 })
      .map((heading) => heading.textContent);
    expect(panels).toEqual([
      tx("firstRun:title"),
      tx("firstRun:folders.title"),
      tx("rekordbox:title"),
      tx("firstRun:fresh.title"),
    ]);
    expect(await screen.findByText(tx("musicFolderStatus:empty"))).toBeInTheDocument();
  });

  it("offers Start from rekordbox once a read leaves tracks to add, and adds only when asked", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 2 });
    renderApp("/overview");
    expect(
      await screen.findByRole("heading", { name: tx("firstRun:fromRekordbox.title") }),
    ).toBeInTheDocument();
    expect(screen.getByText(tx("rekordboxOffer:count", { count: 2 }))).toBeInTheDocument();
    expect(backend.adds).toBe(0);

    await userEvent.click(screen.getByRole("button", { name: tx("rekordboxOffer:add") }));
    // The summary stays up although the Library is no longer empty.
    expect(await screen.findByText(tx("rekordboxOffer:summary.added", { count: 2 }))).toBeInTheDocument();
    expect(backend.adds).toBe(1);
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: tx("firstRun:title") })).toBeNull(),
    );
    expect(screen.getByText(tx("rekordboxOffer:summary.added", { count: 2 }))).toBeInTheDocument();
  });

  it("starts fresh by going to All music, with nothing stored and nothing added", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 0 });
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: tx("firstRun:fresh.action") }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/all-music"));
    expect(await screen.findByText(tx("allMusic:empty"))).toBeInTheDocument();
    expect(backend.adds).toBe(0);
    expect(localStorage.length).toBe(0);
  });
});

describe("the Overview before it knows the Library", () => {
  it("shows neither the first-run flow nor the Library's own view while the Library loads", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 0, libraryHeld: true });
    renderApp("/overview");
    // The rest of the screen is up.
    expect(
      await screen.findByRole("heading", { name: tx("rekordbox:title") }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: tx("firstRun:title") })).toBeNull();
    expect(screen.queryByRole("heading", { name: tx("firstRun:folders.title") })).toBeNull();
    expect(screen.queryByText(tx("overview:empty"))).toBeNull();

    backend.release();
    expect(await screen.findByRole("heading", { name: tx("firstRun:title") })).toBeInTheDocument();
  });

  it("says so when the Library can't be read, and doesn't show the first-run flow", async () => {
    fakeBackend({ library: 0, toAdd: 0, libraryBroken: true });
    renderApp("/overview");
    expect(
      await screen.findByText(tx("errors:database")),
    ).toHaveAttribute("role", "alert");
    expect(screen.queryByRole("heading", { name: tx("firstRun:title") })).toBeNull();
    expect(screen.queryByRole("heading", { name: tx("firstRun:fresh.title") })).toBeNull();
    expect(screen.queryByText(tx("overview:empty"))).toBeNull();
  });
});

describe("the Overview once the Library has tracks", () => {
  it("shows the offer after a read that leaves rekordbox tracks to add", async () => {
    fakeBackend({ library: 3, toAdd: 5 });
    renderApp("/overview");
    expect(
      await screen.findByText(tx("rekordboxOffer:count", { count: 5 })),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: tx("rekordboxOffer:title") })).toBeInTheDocument();
    // Not the first run any more.
    expect(screen.queryByRole("heading", { name: tx("firstRun:title") })).toBeNull();
    expect(screen.queryByRole("heading", { name: tx("firstRun:fresh.title") })).toBeNull();
  });

  it("keeps the Music folders panel, below the rest, so folders can still be added", async () => {
    fakeBackend({ library: 3, toAdd: 0 });
    renderApp("/overview");
    const folders = await screen.findByRole("heading", { name: tx("firstRun:folders.title") });
    const rekordbox = screen.getByRole("heading", { name: tx("rekordbox:title") });
    expect(
      rekordbox.compareDocumentPosition(folders) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(screen.getByRole("button", { name: tx("firstRun:folders.add") })).toBeEnabled();
    expect(
      screen.getByRole("checkbox", { name: tx("musicFolderStatus:readOnlineOnly") }),
    ).toBeInTheDocument();
  });

  it("shows no offer when the count is zero", async () => {
    fakeBackend({ library: 3, toAdd: 0 });
    renderApp("/overview");
    expect(
      await screen.findByText(tx("overview:empty")),
    ).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: tx("rekordbox:title") })).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("heading", { name: tx("rekordboxOffer:title") })).toBeNull());
    expect(screen.queryByText(tx("rekordboxOffer:count", { count: 0 }))).toBeNull();
    expect(screen.queryByRole("button", { name: tx("rekordboxOffer:add") })).toBeNull();
  });
});

describe("Send to rekordbox on the Overview", () => {
  it("opens the guided send once the Library has tracks, and returns when it's closed", async () => {
    fakeBackend({ library: 2, toAdd: 0 });
    renderApp("/overview");
    await userEvent.click(await screen.findByRole("button", { name: tx("send:open") }));
    expect(
      await screen.findByRole("heading", { level: 2, name: tx("send:title") }),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 3, name: tx("send:write.title") })).toBeInTheDocument();
    // The checklist takes the Overview's place.
    expect(screen.queryByRole("heading", { name: tx("rekordbox:title") })).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: tx("send:close") }));
    expect(await screen.findByRole("button", { name: tx("send:open") })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: tx("rekordbox:title") })).toBeInTheDocument();
  });

  it("shows the after-send lists in the guided send's last step", async () => {
    fakeBackend({ library: 2, toAdd: 0 });
    renderApp("/overview");
    await userEvent.click(await screen.findByRole("button", { name: tx("send:open") }));
    expect(await screen.findByText("Crates > Old name")).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:removals.empty"))).toBeInTheDocument();
  });

  it("isn't offered while the Library is empty", async () => {
    fakeBackend({ library: 0, toAdd: 0 });
    renderApp("/overview");
    expect(await screen.findByRole("heading", { name: tx("firstRun:title") })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: tx("send:open") })).toBeNull();
  });

  it("isn't offered, and neither is adding rekordbox tracks, until the Library is known", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 5, libraryHeld: true });
    renderApp("/overview");
    expect(
      await screen.findByRole("heading", { name: tx("rekordbox:title") }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: tx("send:open") })).toBeNull();
    // The offer's title depends on whether the Library is empty, so it
    // waits too: no regular title flashes before the first-run one.
    expect(screen.queryByRole("heading", { name: tx("rekordboxOffer:title") })).toBeNull();
    expect(screen.queryByRole("heading", { name: tx("firstRun:fromRekordbox.title") })).toBeNull();

    backend.release();
    expect(await screen.findByRole("heading", { name: tx("firstRun:fromRekordbox.title") })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: tx("rekordboxOffer:title") })).toBeNull();
  });
});

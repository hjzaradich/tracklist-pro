import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import type { AddSummary, LibraryTrack, Offer } from "../bindings";

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
      if (cmd === "music_folders") return [];
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
    expect(await screen.findByRole("heading", { name: "Start your Library" })).toBeInTheDocument();
    const panels = screen
      .getAllByRole("heading", { level: 2 })
      .map((heading) => heading.textContent);
    expect(panels).toEqual([
      "Start your Library",
      "Music folders",
      "rekordbox collection",
      "Start fresh",
    ]);
    expect(await screen.findByText("No music folders")).toBeInTheDocument();
  });

  it("offers Start from rekordbox once a read leaves tracks to add, and adds only when asked", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 2 });
    renderApp("/overview");
    expect(
      await screen.findByRole("heading", { name: "Start from rekordbox" }),
    ).toBeInTheDocument();
    expect(screen.getByText("2 rekordbox tracks aren't in your Library")).toBeInTheDocument();
    expect(backend.adds).toBe(0);

    await userEvent.click(screen.getByRole("button", { name: "Add" }));
    // The summary stays up although the Library is no longer empty.
    expect(await screen.findByText("2 tracks added to your Library")).toBeInTheDocument();
    expect(backend.adds).toBe(1);
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Start your Library" })).toBeNull(),
    );
    expect(screen.getByText("2 tracks added to your Library")).toBeInTheDocument();
  });

  it("starts fresh by going to All music, with nothing stored and nothing added", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 0 });
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: "Go to All music" }));
    await waitFor(() => expect(router.state.location.pathname).toBe("/all-music"));
    expect(await screen.findByText("No tracks in All music")).toBeInTheDocument();
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
      await screen.findByRole("heading", { name: "rekordbox collection" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "Start your Library" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Music folders" })).toBeNull();
    expect(screen.queryByText("Your Library at a glance, and what to do next.")).toBeNull();

    backend.release();
    expect(await screen.findByRole("heading", { name: "Start your Library" })).toBeInTheDocument();
  });

  it("says so when the Library can't be read, and doesn't show the first-run flow", async () => {
    fakeBackend({ library: 0, toAdd: 0, libraryBroken: true });
    renderApp("/overview");
    expect(
      await screen.findByText("Couldn't read or save the Library. Try again."),
    ).toHaveAttribute("role", "alert");
    expect(screen.queryByRole("heading", { name: "Start your Library" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Start fresh" })).toBeNull();
    expect(screen.queryByText("Your Library at a glance, and what to do next.")).toBeNull();
  });
});

describe("the Overview once the Library has tracks", () => {
  it("shows the offer after a read that leaves rekordbox tracks to add", async () => {
    fakeBackend({ library: 3, toAdd: 5 });
    renderApp("/overview");
    expect(
      await screen.findByText("5 rekordbox tracks aren't in your Library"),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "rekordbox tracks" })).toBeInTheDocument();
    // Not the first run any more.
    expect(screen.queryByRole("heading", { name: "Start your Library" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Music folders" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Start fresh" })).toBeNull();
  });

  it("shows no offer when the count is zero", async () => {
    fakeBackend({ library: 3, toAdd: 0 });
    renderApp("/overview");
    expect(
      await screen.findByText("Your Library at a glance, and what to do next."),
    ).toBeInTheDocument();
    expect(await screen.findByRole("heading", { name: "rekordbox collection" })).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole("heading", { name: "rekordbox tracks" })).toBeNull());
    expect(screen.queryByText(/in your Library$/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Add" })).toBeNull();
  });
});

describe("Send to rekordbox on the Overview", () => {
  it("opens the guided send once the Library has tracks, and returns when it's closed", async () => {
    fakeBackend({ library: 2, toAdd: 0 });
    renderApp("/overview");
    await userEvent.click(await screen.findByRole("button", { name: "Send to rekordbox" }));
    expect(
      await screen.findByRole("heading", { level: 2, name: "Send to rekordbox" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("heading", { level: 3, name: "Write file" })).toBeInTheDocument();
    // The checklist takes the Overview's place.
    expect(screen.queryByRole("heading", { name: "rekordbox collection" })).toBeNull();

    await userEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(await screen.findByRole("button", { name: "Send to rekordbox" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "rekordbox collection" })).toBeInTheDocument();
  });

  it("isn't offered while the Library is empty", async () => {
    fakeBackend({ library: 0, toAdd: 0 });
    renderApp("/overview");
    expect(await screen.findByRole("heading", { name: "Start your Library" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send to rekordbox" })).toBeNull();
  });

  it("isn't offered, and neither is adding rekordbox tracks, until the Library is known", async () => {
    const backend = fakeBackend({ library: 0, toAdd: 5, libraryHeld: true });
    renderApp("/overview");
    expect(
      await screen.findByRole("heading", { name: "rekordbox collection" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Send to rekordbox" })).toBeNull();
    // The offer's title depends on whether the Library is empty, so it
    // waits too: no regular title flashes before the first-run one.
    expect(screen.queryByRole("heading", { name: "rekordbox tracks" })).toBeNull();
    expect(screen.queryByRole("heading", { name: "Start from rekordbox" })).toBeNull();

    backend.release();
    expect(await screen.findByRole("heading", { name: "Start from rekordbox" })).toBeInTheDocument();
    expect(screen.queryByRole("heading", { name: "rekordbox tracks" })).toBeNull();
  });
});

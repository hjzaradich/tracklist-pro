import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { AllMusicTrack } from "../bindings";
import "../i18n";
import { AllMusicScreen } from "./AllMusicScreen";

function track(id: number, fields: Partial<AllMusicTrack> = {}): AllMusicTrack {
  return {
    recordingId: id,
    title: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    file: { path: String.raw`E:\Music\tune ` + id + ".mp3", name: `tune ${id}.mp3`, present: true, driveConnected: true },
    inLibrary: false,
    ...fields,
  };
}

/**
 * Stands in for the Rust side: All music holds `tracks`; a search keeps the
 * titles holding it; adding a track puts it in the Library, or is refused.
 */
function fakeBackend(tracks: AllMusicTrack[], options: { total?: number; refuse?: unknown } = {}) {
  const backend = { tracks, searches: [] as string[], added: [] as number[] };
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    if (cmd === "all_music_tracks") {
      const search = (a.search as string | null) ?? "";
      backend.searches.push(search);
      const matching = backend.tracks.filter((t) =>
        (t.title ?? "").toLowerCase().includes(search.toLowerCase()),
      );
      return { total: options.total ?? matching.length, tracks: matching };
    }
    if (cmd === "promote_track") {
      if (options.refuse) throw options.refuse;
      const id = a.recordingId as number;
      backend.added.push(id);
      backend.tracks = backend.tracks.map((t) =>
        t.recordingId === id ? { ...t, inLibrary: true } : t,
      );
      return { libraryTrack: {}, added: true };
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <AllMusicScreen />
    </QueryClientProvider>,
  );
}

/** The list's body rows, once loaded. */
async function rows() {
  const table = await screen.findByRole("table");
  return within(table).getAllByRole("row").slice(1);
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the All music screen", () => {
  it("names what's absent when All music has no tracks", async () => {
    fakeBackend([]);
    renderScreen();
    expect(await screen.findByText("No tracks in All music")).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("lists each track's title, artist and file, with Add to Library", async () => {
    fakeBackend([track(1), track(2, { title: null, artist: null })]);
    renderScreen();
    const [first, second] = await rows();
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      "Title",
      "Artist",
      "File",
      "Library",
    ]);
    expect(within(first).getAllByRole("cell").map((cell) => cell.textContent)).toEqual([
      "Synthetic Tune 1",
      "Made Up Artist 1",
      String.raw`E:\Music\tune 1.mp3`,
      "Add to Library",
    ]);
    // No title: the file's name stands in.
    expect(within(second).getAllByRole("cell")[0]).toHaveTextContent("tune 2.mp3");
  });

  it("adds one track to the Library when asked, and then says it's in the Library", async () => {
    const backend = fakeBackend([track(1), track(2)]);
    renderScreen();
    const [, second] = await rows();
    expect(backend.added).toEqual([]);

    await userEvent.click(within(second).getByRole("button", { name: "Add to Library" }));
    await waitFor(async () => expect((await rows())[1]).toHaveTextContent("In Library"));
    expect(backend.added).toEqual([2]);
    const [first, added] = await rows();
    expect(within(added).queryByRole("button")).toBeNull();
    expect(within(first).getByRole("button", { name: "Add to Library" })).toBeEnabled();
  });

  it("shows a track that's in the Library without the action", async () => {
    fakeBackend([track(1, { inLibrary: true })]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent("In Library");
    expect(within(row).queryByRole("button")).toBeNull();
  });

  it("says why a track couldn't be added", async () => {
    fakeBackend([track(1)], { refuse: { kind: "libraryNoFile", params: {} } });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: "Add to Library" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Can't add: this track has no file",
    );
  });

  it("narrows the list by the search, and names what's absent when nothing matches", async () => {
    const backend = fakeBackend([track(1), track(2)]);
    renderScreen();
    await rows();
    await userEvent.type(screen.getByRole("searchbox", { name: "Search All music" }), "Tune 2");
    await waitFor(async () => expect(await rows()).toHaveLength(1));
    expect((await rows())[0]).toHaveTextContent("Synthetic Tune 2");
    expect(backend.searches.at(-1)).toBe("Tune 2");

    await userEvent.type(screen.getByRole("searchbox"), " zzz");
    expect(await screen.findByText("No tracks match")).toBeInTheDocument();
  });

  it("says how many tracks match when the list holds only some of them", async () => {
    fakeBackend([track(1), track(2)], { total: 1234 });
    renderScreen();
    await rows();
    expect(screen.getByText("Showing 2 of 1,234 tracks")).toBeInTheDocument();
  });
});

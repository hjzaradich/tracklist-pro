import { QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { useActivityStore } from "../activity/activityStore";
import type { AllMusicTrack, JobUpdate } from "../bindings";
import { useReloadListsWhenJobsEnd } from "../shell/useReloadListsWhenJobsEnd";
import "../i18n";
import { AllMusicScreen } from "./AllMusicScreen";
import type { Schedule } from "./useDebounced";
import { tx } from "../test/tx";

/** Typing has "stopped" at once: the search is asked for on every change. */
const AT_ONCE: Schedule = (run) => {
  run();
  return () => {};
};

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
  }, { shouldMockEvents: true });
  return backend;
}

function job(kind: JobUpdate["kind"], status: JobUpdate["status"], id = 7): JobUpdate {
  return { seq: id, id, kind, status, progress: null, priority: 0 };
}

/** All music inside what the shell does for every screen. */
function InTheShell() {
  useReloadListsWhenJobsEnd();
  return <AllMusicScreen schedule={AT_ONCE} />;
}

function renderScreen(schedule: Schedule = AT_ONCE) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <AllMusicScreen schedule={schedule} />
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
    expect(await screen.findByText(tx("allMusic:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("lists each track's title, artist and file, with Add to Library", async () => {
    fakeBackend([track(1), track(2, { title: null, artist: null })]);
    renderScreen();
    const [first, second] = await rows();
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      tx("allMusic:columns.title"),
      tx("allMusic:columns.artist"),
      tx("allMusic:columns.file"),
      tx("allMusic:columns.library"),
    ]);
    expect(within(first).getAllByRole("cell").map((cell) => cell.textContent)).toEqual([
      "Synthetic Tune 1",
      "Made Up Artist 1",
      String.raw`E:\Music\tune 1.mp3`,
      tx("allMusic:add"),
    ]);
    // No title: the file's name stands in.
    expect(within(second).getAllByRole("cell")[0]).toHaveTextContent("tune 2.mp3");
  });

  it("adds one track to the Library when asked, and then says it's in the Library", async () => {
    const backend = fakeBackend([track(1), track(2)]);
    renderScreen();
    const [, second] = await rows();
    expect(backend.added).toEqual([]);

    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));
    await waitFor(async () => expect((await rows())[1]).toHaveTextContent(tx("allMusic:inLibrary")));
    expect(backend.added).toEqual([2]);
    const [first, added] = await rows();
    expect(within(added).queryByRole("button")).toBeNull();
    expect(within(first).getByRole("button", { name: tx("allMusic:add") })).toBeEnabled();
  });

  it("shows a track that's in the Library without the action", async () => {
    fakeBackend([track(1, { inLibrary: true })]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent(tx("allMusic:inLibrary"));
    expect(within(row).queryByRole("button")).toBeNull();
  });

  it("says why a track couldn't be added", async () => {
    fakeBackend([track(1)], { refuse: { kind: "libraryNoFile", params: {} } });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: tx("allMusic:add") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("library:error.noFile"));
  });

  it("narrows the list by the search, and names what's absent when nothing matches", async () => {
    const backend = fakeBackend([track(1), track(2)]);
    renderScreen();
    await rows();
    await userEvent.type(screen.getByRole("searchbox", { name: tx("allMusic:search") }), "Tune 2");
    await waitFor(async () => expect(await rows()).toHaveLength(1));
    expect((await rows())[0]).toHaveTextContent("Synthetic Tune 2");
    expect(backend.searches.at(-1)).toBe("Tune 2");

    await userEvent.type(screen.getByRole("searchbox"), " zzz");
    expect(await screen.findByText(tx("allMusic:noMatch"))).toBeInTheDocument();
  });

  it("asks for a search once typing has stopped, not on every keystroke", async () => {
    const backend = fakeBackend([track(1), track(2)]);
    // Stands in for the clock: holds what's scheduled until the test runs it.
    const waiting: (() => void)[] = [];
    let calledOff = 0;
    const held: Schedule = (run) => {
      waiting.push(run);
      return () => {
        calledOff += 1;
        waiting.splice(waiting.indexOf(run), 1);
      };
    };
    renderScreen(held);
    await rows();
    const before = waiting.length;

    await userEvent.type(screen.getByRole("searchbox"), "Tune 2");
    expect(screen.getByRole("searchbox")).toHaveValue("Tune 2");
    // Six keystrokes: each called the one before off, and nothing was asked.
    expect(calledOff).toBeGreaterThanOrEqual(5);
    expect(waiting).toHaveLength(Math.max(before, 1));
    expect(backend.searches).toEqual([""]);
    expect(await rows()).toHaveLength(2);

    // Typing has stopped.
    act(() => waiting.at(-1)?.());
    await waitFor(async () => expect(await rows()).toHaveLength(1));
    expect(backend.searches).toEqual(["", "Tune 2"]);
  });

  it("says how many tracks match when the list holds only some of them", async () => {
    fakeBackend([track(1), track(2)], { total: 1234 });
    renderScreen();
    await rows();
    expect(screen.getByText(tx("allMusic:showing", { shown: 2, total: 1234 }))).toBeInTheDocument();
  });
});

describe("All music while a scan is still filling it", () => {
  afterEach(() => useActivityStore.getState().reset());

  it("says what the scan is doing instead of claiming there are no tracks", async () => {
    fakeBackend([]);
    useActivityStore.getState().applySnapshot({ seq: 7, jobs: [job("read", "running")] });
    renderScreen();
    expect(await screen.findByText(tx("activity:task.read"))).toBeInTheDocument();
    expect(screen.queryByText(tx("allMusic:empty"))).toBeNull();
  });

  it("says there are no tracks once nothing that could add one is under way", async () => {
    fakeBackend([]);
    // A relink puts no track in All music.
    useActivityStore.getState().applySnapshot({ seq: 7, jobs: [job("relink", "running")] });
    renderScreen();
    expect(await screen.findByText(tx("allMusic:empty"))).toBeInTheDocument();
  });

  it("still says no tracks match a search", async () => {
    fakeBackend([]);
    useActivityStore.getState().applySnapshot({ seq: 7, jobs: [job("scan", "running")] });
    renderScreen();
    await userEvent.type(screen.getByRole("searchbox"), "tune");
    expect(await screen.findByText(tx("allMusic:noMatch"))).toBeInTheDocument();
  });

  it("loads the tracks when the background task ends, without leaving the screen", async () => {
    const backend = fakeBackend([]);
    render(
      <QueryClientProvider client={createQueryClient()}>
        <InTheShell />
      </QueryClientProvider>,
    );
    expect(await screen.findByText(tx("allMusic:empty"))).toBeInTheDocument();

    backend.tracks = [track(1), track(2)];
    await act(() => emit("job-updates", [job("group", "done")]));
    expect(await rows()).toHaveLength(2);
  });

  it("doesn't ask again while a task is only making progress", async () => {
    const backend = fakeBackend([]);
    render(
      <QueryClientProvider client={createQueryClient()}>
        <InTheShell />
      </QueryClientProvider>,
    );
    await screen.findByText(tx("allMusic:empty"));
    const asked = backend.searches.length;
    await act(() => emit("job-updates", [job("read", "running")]));
    expect(backend.searches).toHaveLength(asked);
  });
});

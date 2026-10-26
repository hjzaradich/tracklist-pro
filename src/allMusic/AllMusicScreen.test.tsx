import { QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { useActivityStore } from "../activity/activityStore";
import type { AllMusicTrack, JobUpdate, TableRow } from "../bindings";
import { usePlayer } from "../player/playerStore";
import { useReloadListsWhenJobsEnd } from "../shell/useReloadListsWhenJobsEnd";
import "../i18n";
import { AllMusicScreen } from "./AllMusicScreen";
import type { Schedule } from "./useDebounced";
import { pageOf, queryOf, tableRow } from "../test/trackTableIpc";
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
    file: { id, path: String.raw`E:\Music\tune ` + id + ".mp3", name: `tune ${id}.mp3`, present: true, driveConnected: true },
    inLibrary: false,
    matchNotConfirmed: false,
    ...fields,
  };
}

/** The row of the track table for a track of All music. */
function allRow(track: AllMusicTrack): TableRow {
  return tableRow(track.recordingId, {
    recordingId: track.recordingId,
    libraryTrackId: null,
    title: track.title,
    artist: track.artist,
    file: track.file,
    inLibrary: track.inLibrary,
    matchNotConfirmed: track.matchNotConfirmed,
  });
}

/**
 * Stands in for the Rust side: All music holds `tracks`; a search keeps the
 * titles holding it; adding a track puts it in the Library, or is refused.
 */
function fakeBackend(
  tracks: AllMusicTrack[],
  options: {
    refuse?: unknown;
    lowBitrate?: Record<number, number>;
    /** Called with the file id of each preview check. */
    onPreview?: (fileId: number) => void;
  } = {},
) {
  const backend = {
    tracks,
    searches: [] as string[],
    added: [] as number[],
    /** Every add asked for, with the confirmation it carried. */
    asked: [] as { recordingId: number; confirmLowBitrate: boolean }[],
  };
  mockIPC((cmd, args) => {
    const a = (args ?? {}) as Record<string, unknown>;
    if (cmd === "key_notation") return "camelot";
    if (cmd === "track_table_page") {
      const query = queryOf(args);
      const search = query.search ?? "";
      // One note per search: its first window.
      if (query.offset === 0) backend.searches.push(search);
      const matching = backend.tracks.filter((t) =>
        (t.title ?? "").toLowerCase().includes(search.toLowerCase()),
      );
      return pageOf(matching.map(allRow), query);
    }
    if (cmd === "promote_track") {
      if (options.refuse) throw options.refuse;
      const id = a.recordingId as number;
      const confirmLowBitrate = a.confirmLowBitrate as boolean;
      backend.asked.push({ recordingId: id, confirmLowBitrate });
      // The rule is the backend's: a low-bitrate file needs the confirmation.
      const kbps = options.lowBitrate?.[id];
      if (kbps !== undefined && !confirmLowBitrate) {
        return { outcome: "needsConfirmation", warning: { code: "lowBitrate", kbps } };
      }
      backend.added.push(id);
      backend.tracks = backend.tracks.map((t) =>
        t.recordingId === id ? { ...t, inLibrary: true } : t,
      );
      return { outcome: "done", libraryTrack: {}, added: true };
    }
    if (cmd === "preview_check") {
      options.onPreview?.(a.fileId as number);
      return null;
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

/** The table's body rows, once loaded. */
async function rows() {
  const table = await screen.findByRole("grid");
  return within(table).getAllByRole("row").slice(1);
}

afterEach(() => {
  cleanup();
  clearMocks();
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
});

describe("the All music screen", () => {
  it("names what's absent when All music has no tracks", async () => {
    fakeBackend([]);
    renderScreen();
    expect(await screen.findByText(tx("allMusic:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("grid")).toBeNull();
  });

  it("lists each track's title, artist and file, with Add to Library", async () => {
    fakeBackend([track(1), track(2, { title: null, artist: null })]);
    renderScreen();
    const [first, second] = await rows();
    // The Library's columns and, for All music, the file's name and folder.
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      tx("trackTable:columns.title"),
      tx("trackTable:columns.artist"),
      tx("trackTable:columns.bpm"),
      tx("trackTable:columns.key"),
      tx("trackTable:columns.time"),
      tx("trackTable:columns.genre"),
      tx("trackTable:columns.rating"),
      tx("trackTable:columns.dateAdded"),
      tx("trackTable:columns.file"),
      tx("trackTable:columns.folder"),
      tx("trackTable:columns.actions"),
    ]);
    const cells = within(first).getAllByRole("gridcell");
    // The play button (an icon, named by its label) comes first in the
    // title's cell: no words of its own.
    expect(cells[0]).toHaveTextContent("Synthetic Tune 1");
    expect(cells[1]).toHaveTextContent("Made Up Artist 1");
    expect(cells[8]).toHaveTextContent("tune 1.mp3");
    expect(cells[9]).toHaveTextContent(String.raw`E:\Music`);
    expect(cells[10]).toHaveTextContent(tx("allMusic:add"));
    // No title: the file's name stands in.
    expect(within(second).getAllByRole("gridcell")[0]).toHaveTextContent("tune 2.mp3");
  });

  it("has a play button on each track with a file, which gives the player that file", async () => {
    // jsdom plays nothing.
    vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => {});
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
    vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
    mockConvertFileSrc("windows");
    const asked: number[] = [];
    fakeBackend([track(1), track(2, { title: null, file: { ...track(2).file!, id: 77 } })], {
      onPreview: (id) => asked.push(id),
    });
    renderScreen();
    const [first, second] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("player:playNamed", { name: "Synthetic Tune 1" }) }));
    // The track's own file, by its id, not a path.
    await waitFor(() => expect(asked).toEqual([1]));
    // With no title the file's name stands in.
    await userEvent.click(within(second).getByRole("button", { name: tx("player:playNamed", { name: "tune 2.mp3" }) }));
    await waitFor(() => expect(asked).toEqual([1, 77]));
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
    expect(within(added).queryByRole("button", { name: tx("allMusic:add") })).toBeNull();
    expect(within(first).getByRole("button", { name: tx("allMusic:add") })).toBeEnabled();
  });

  it("shows a track that's in the Library without the action", async () => {
    fakeBackend([track(1, { inLibrary: true })]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent(tx("allMusic:inLibrary"));
    expect(within(row).queryByRole("button", { name: tx("allMusic:add") })).toBeNull();
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
    expect(await screen.findByText(tx("trackTable:noMatch"))).toBeInTheDocument();
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

  it("says how many tracks there are, and how many a search leaves", async () => {
    fakeBackend([track(1), track(2)]);
    renderScreen();
    await rows();
    expect(screen.getByText(tx("allMusic:trackCount", { count: 2 }))).toBeInTheDocument();
    await userEvent.type(screen.getByRole("searchbox", { name: tx("allMusic:search") }), "Tune 2");
    expect(await screen.findByText(tx("allMusic:trackCount", { count: 1 }))).toBeInTheDocument();
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
    expect(await screen.findByText(tx("trackTable:noMatch"))).toBeInTheDocument();
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

describe("a track whose rekordbox match isn't confirmed", () => {
  it("can't be added: the button is greyed out and the row says why", async () => {
    const backend = fakeBackend([track(1), track(2, { matchNotConfirmed: true })]);
    renderScreen();
    const [free, held] = await rows();

    expect(within(free).getByRole("button", { name: tx("allMusic:add") })).toBeEnabled();
    expect(within(free).queryByText(tx("allMusic:matchNotConfirmed"))).toBeNull();

    const button = within(held).getByRole("button", { name: tx("allMusic:add") });
    expect(button).toBeDisabled();
    // The reason is on the row before any click, and tied to the button.
    expect(button).toHaveAccessibleDescription(tx("allMusic:matchNotConfirmed"));
    await userEvent.click(button);
    expect(backend.added).toEqual([]);
  });
});

describe("the All music table's cut-off text", () => {
  const longTitle =
    "A made-up title that keeps going far past any column width (Extended Club Mix With The Long Intro)";
  const longArtist = "A Made Up Ensemble Of Many Collaborators feat. Another Very Long Guest Artist";
  const longFolder = String.raw`E:\Music\Made Up Folder\A folder with a name that keeps going far past any column`;

  it("a cut-off title cell carries its full text", async () => {
    fakeBackend([
      track(1, {
        title: longTitle,
        artist: longArtist,
        file: { id: 1, path: longFolder + String.raw`\x.mp3`, name: "x.mp3", present: true, driveConnected: true },
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    const cells = within(row).getAllByRole("gridcell");
    expect(within(cells[0]).getByText(longTitle)).toHaveAttribute("title", longTitle);
    expect(cells[1]).toHaveAttribute("title", longArtist);
    expect(cells[8]).toHaveAttribute("title", "x.mp3");
    expect(cells[9]).toHaveAttribute("title", longFolder);
  });

  it("a title that is the file's name carries that name", async () => {
    fakeBackend([track(1, { title: null, artist: null })]);
    renderScreen();
    const [row] = await rows();
    const cells = within(row).getAllByRole("gridcell");
    expect(within(cells[0]).getByText("tune 1.mp3")).toHaveAttribute("title", "tune 1.mp3");
    expect(cells[1]).not.toHaveAttribute("title");
  });
});

describe("adding a low-bitrate file", () => {
  /** The warning box, once the backend has asked for the confirmation. */
  async function warning() {
    return screen.findByRole("alertdialog");
  }

  it("shows the warning with exactly two choices, and adds nothing yet", async () => {
    const backend = fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [, second] = await rows();
    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));

    const box = await warning();
    expect(box).toHaveTextContent(tx("allMusic:lowBitrate.title", { title: "Synthetic Tune 2" }));
    expect(box).toHaveTextContent(tx("allMusic:lowBitrate.detail", { kbps: 128 }));
    expect(within(box).getAllByRole("button").map((button) => button.textContent)).toEqual([
      tx("allMusic:lowBitrate.addAnyway"),
      tx("allMusic:lowBitrate.cancel"),
    ]);
    // Asked once, without the confirmation; nothing is in the Library.
    expect(backend.asked).toEqual([{ recordingId: 2, confirmLowBitrate: false }]);
    expect(backend.added).toEqual([]);
    // The safe choice has focus.
    expect(within(box).getByRole("button", { name: tx("allMusic:lowBitrate.cancel") })).toHaveFocus();
  });

  it("calls nothing further when the user chooses Cancel", async () => {
    const backend = fakeBackend([track(1)], { lowBitrate: { 1: 96 } });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: tx("allMusic:add") }));
    const box = await warning();

    await userEvent.click(within(box).getByRole("button", { name: tx("allMusic:lowBitrate.cancel") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(backend.asked).toEqual([{ recordingId: 1, confirmLowBitrate: false }]);
    expect(backend.added).toEqual([]);
    // The track is still there to add, and the list was not reloaded into a different state.
    const [row] = await rows();
    expect(within(row).getByRole("button", { name: tx("allMusic:add") })).toBeEnabled();
    expect(row).not.toHaveTextContent(tx("allMusic:inLibrary"));
  });

  it("cancels on Escape too", async () => {
    const backend = fakeBackend([track(1)], { lowBitrate: { 1: 96 } });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: tx("allMusic:add") }));
    await warning();

    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(backend.asked).toHaveLength(1);
    expect(backend.added).toEqual([]);
  });

  it("cancels on Escape wherever focus is, and gives focus back to the Add button it came from", async () => {
    const backend = fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [, second] = await rows();
    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));
    await warning();

    // Focus is outside the box, in the search field.
    const search = screen.getByRole("searchbox", { name: tx("allMusic:search") });
    search.focus();
    expect(search).toHaveFocus();
    await userEvent.keyboard("{Escape}");

    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(backend.asked).toHaveLength(1);
    expect(backend.added).toEqual([]);
    const [, again] = await rows();
    expect(within(again).getByRole("button", { name: tx("allMusic:add") })).toHaveFocus();
  });

  it("gives focus back to the Add button after Cancel too", async () => {
    fakeBackend([track(1)], { lowBitrate: { 1: 96 } });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: tx("allMusic:add") }));
    const box = await warning();
    await userEvent.click(within(box).getByRole("button", { name: tx("allMusic:lowBitrate.cancel") }));
    expect(await screen.findByRole("button", { name: tx("allMusic:add") })).toHaveFocus();
  });

  it("closes the warning when the track leaves the list, instead of asking about nothing", async () => {
    const backend = fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [, second] = await rows();
    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));
    await warning();

    // A search that no longer lists the track.
    await userEvent.type(screen.getByRole("searchbox", { name: tx("allMusic:search") }), "Tune 1");
    await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    expect(backend.added).toEqual([]);
  });

  it("adds the track with the confirmation when the user chooses Add anyway", async () => {
    const backend = fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [, second] = await rows();
    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));
    const box = await warning();

    await userEvent.click(within(box).getByRole("button", { name: tx("allMusic:lowBitrate.addAnyway") }));
    await waitFor(async () => expect((await rows())[1]).toHaveTextContent(tx("allMusic:inLibrary")));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(backend.asked).toEqual([
      { recordingId: 2, confirmLowBitrate: false },
      { recordingId: 2, confirmLowBitrate: true },
    ]);
    expect(backend.added).toEqual([2]);
  });

  it("shows no warning for a track the backend adds at once", async () => {
    const backend = fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("allMusic:add") }));
    await waitFor(async () => expect((await rows())[0]).toHaveTextContent(tx("allMusic:inLibrary")));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(backend.asked).toEqual([{ recordingId: 1, confirmLowBitrate: false }]);
  });

  it("holds the other Add buttons while the question is open", async () => {
    fakeBackend([track(1), track(2)], { lowBitrate: { 2: 128 } });
    renderScreen();
    const [first, second] = await rows();
    await userEvent.click(within(second).getByRole("button", { name: tx("allMusic:add") }));
    await warning();
    expect(within(first).getByRole("button", { name: tx("allMusic:add") })).toBeDisabled();
  });
});

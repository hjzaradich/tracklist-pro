import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LibraryTrack } from "../bindings";
import "../i18n";
import { LibraryScreen } from "./LibraryScreen";
import { pageOf, queryOf, rowOfLibraryTrack } from "../test/trackTableIpc";
import { expectOnlyTheTopBarUndo, TopBarUndo } from "../test/topBarUndo";
import { tx } from "../test/tx";

function track(id: number, fields: Partial<LibraryTrack> = {}): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    file: {
      id,
      path: String.raw`E:\Music\tune ` + id + ".mp3",
      name: `tune ${id}.mp3`,
      present: true,
      driveConnected: true,
    },
    sourceMissing: false,
    openConflicts: 0,
    fragile: null,
    addedAt: "2026-09-30T10:00:00.000Z",
    betterFile: null,
    ...fields,
  };
}

/** What a row's "Add to crate" says while there are no crates to choose. */
const noCrates = tx("crates:addTrack.none");

/** A linked file the last scan didn't find, on a drive that's connected. */
const goneFile = {
  id: 1,
  path: String.raw`E:\Music\gone.mp3`,
  name: "gone.mp3",
  present: false,
  driveConnected: true,
};

/** Stands in for the Rust side: the Library holds `tracks`, or can't be read. */
function library(tracks: LibraryTrack[] | "broken") {
  mockIPC((cmd, args) => {
    if (cmd === "list_crates") return [];
    if (cmd === "key_notation") return "camelot";
    if (cmd === "table_columns") return null;
    if (cmd !== "track_table_page") throw new Error(`unexpected command ${cmd}`);
    if (tracks === "broken") throw { kind: "database", params: {} };
    return pageOf(tracks.map(rowOfLibraryTrack), queryOf(args));
  });
}

/** `topBar`: the top bar's Undo beside the screen, for the tests of where Undo is. */
function renderScreen(topBar = false) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      {topBar && <TopBarUndo />}
      <LibraryScreen />
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
});

describe("the Library screen", () => {
  it("names what's absent when there are no Library tracks", async () => {
    library([]);
    renderScreen();
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("grid")).toBeNull();
  });

  it("lists each Library track's title, artist and file, in the order the backend gave", async () => {
    library([track(2), track(1)]);
    renderScreen();
    const [first, second] = await rows();
    // The six columns shown at first (owner decision, 2026-10-30); Rating, Date added, File and Folder are in the column editor.
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      tx("trackTable:columns.title"),
      tx("trackTable:columns.artist"),
      tx("trackTable:columns.bpm"),
      tx("trackTable:columns.key"),
      tx("trackTable:columns.time"),
      tx("trackTable:columns.genre"),
      tx("trackTable:columns.actions"),
    ]);
    const cells = within(first).getAllByRole("gridcell");
    expect(cells[0]).toHaveTextContent("Synthetic Tune 2");
    expect(cells[1]).toHaveTextContent("Made Up Artist 2");
    expect(cells.at(-1)).toHaveTextContent(`${noCrates}${tx("library:remove.button")}`);
    expect(second).toHaveTextContent("Synthetic Tune 1");
    expect(screen.queryByText(tx("library:empty"))).toBeNull();
    // Nothing to say about a track whose file is there.
    expect(first).not.toHaveTextContent(tx("library:fileMissing"));
  });

  it("says how many tracks the Library holds, above the list", async () => {
    library([track(1), track(2), track(3)]);
    renderScreen();
    const count = await screen.findByText(tx("library:trackCount", { count: 3 }));
    const table = screen.getByRole("grid");
    expect(count.compareDocumentPosition(table) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it("counts a single track as one track", async () => {
    library([track(1)]);
    renderScreen();
    expect(await screen.findByText(tx("library:trackCount", { count: 1 }))).toBeInTheDocument();
  });

  it("shows no count for an empty Library: it says there are no Library tracks", async () => {
    library([]);
    renderScreen();
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
    expect(screen.queryByText(tx("library:trackCount", { count: 0 }))).toBeNull();
  });

  it("shows the file's name as the title of a track that has none", async () => {
    library([track(1, { title: null, artist: null })]);
    renderScreen();
    const [row] = await rows();
    const cells = within(row).getAllByRole("gridcell");
    expect(cells[0]).toHaveTextContent("tune 1.mp3");
    expect(cells[1]).toHaveTextContent("");
  });

  it("says so next to a track whose file is missing", async () => {
    const gone = track(2, { sourceMissing: true, file: goneFile });
    library([track(1), gone]);
    renderScreen();
    const [here, missing] = await rows();
    expect(missing).toHaveTextContent(tx("library:fileMissing"));
    expect(here).not.toHaveTextContent(tx("library:fileMissing"));
  });

  it.each(["downloads", "temp", "external", "network"] as const)(
    "says why a file in a %s location is fragile",
    async (reason) => {
      library([track(1, { fragile: reason })]);
      renderScreen();
      const [row] = await rows();
      expect(row).toHaveTextContent(tx(`library:fragile.${reason}`));
    },
  );

  it("shows no note for a track whose file isn't in a fragile location", async () => {
    library([track(1)]);
    renderScreen();
    const [row] = await rows();
    for (const reason of ["downloads", "temp", "external", "network"]) {
      expect(row).not.toHaveTextContent(tx(`library:fragile.${reason}`));
    }
    expect(row).not.toHaveTextContent(tx("library:fileMissing"));
    expect(row).not.toHaveTextContent(tx("library:driveNotConnected"));
  });

  it("shows both notes when the file is missing and was somewhere fragile", async () => {
    library([
      track(1, {
        fragile: "external",
        sourceMissing: true,
        file: goneFile,
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent(tx("library:fileMissing"));
    expect(row).toHaveTextContent(tx("library:fragile.external"));
  });

  it("says the drive isn't connected, not that the file is missing, for a file on an unplugged drive", async () => {
    library([
      track(1, {
        file: {
          id: 1,
          path: String.raw`G:\Music\a.mp3`,
          name: "a.mp3",
          present: true,
          driveConnected: false,
        },
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent(tx("library:driveNotConnected"));
    expect(row).not.toHaveTextContent(tx("library:fileMissing"));
  });

  it("shows several notes on one row", async () => {
    library([
      track(1, {
        fragile: "external",
        sourceMissing: true,
        file: { ...goneFile, driveConnected: false },
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveTextContent(tx("library:fileMissing"));
    expect(row).toHaveTextContent(tx("library:driveNotConnected"));
    expect(row).toHaveTextContent(tx("library:fragile.external"));
  });

  it("shows an error message, not an empty Library, when the list can't be loaded", async () => {
    library("broken");
    renderScreen();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("errors:database"),
    );
    expect(screen.queryByText(tx("library:empty"))).toBeNull();
  });
});

/**
 * A Library that removes tracks like the Rust side, and names the removal as
 * the next step to undo (for the top bar's Undo): `calls` records every
 * command, and the file on disk is never part of it.
 */
function removable(initial: LibraryTrack[]) {
  let tracks = initial;
  let last: LibraryTrack | null = null;
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    calls.push(cmd);
    if (cmd === "list_crates") return [];
    if (cmd === "key_notation") return "camelot";
    if (cmd === "table_columns") return null;
    if (cmd === "track_table_page") return pageOf(tracks.map(rowOfLibraryTrack), queryOf(args));
    if (cmd === "remove_library_track") {
      last = tracks.find((t) => t.id === (args as { id: number }).id) ?? null;
      tracks = tracks.filter((t) => t !== last);
      return null;
    }
    if (cmd === "next_undo_operation") {
      const operation =
        last === null ? null : { id: 1, kind: "remove_from_library", details: { name: null, from: null, tracks: null, newToLibrary: null, newCrate: null } };
      return { operation, refusal: null };
    }
    if (cmd === "undo_last_operation") {
      if (last === null) return { status: "nothingToUndo" };
      tracks = [...tracks, last].sort((a, b) => a.id - b.id);
      last = null;
      return { status: "undone", operation: { id: 1, kind: "x", details: { name: null, from: null, tracks: null, newToLibrary: null, newCrate: null } } };
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return calls;
}

describe("removing a track from the Library", () => {
  it("asks first, and removes nothing until the user confirms", async () => {
    const calls = removable([track(1), track(2)]);
    renderScreen();
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(tx("library:remove.title", { title: "Synthetic Tune 1" }));
    expect(dialog).toHaveTextContent(tx("library:remove.detail"));
    expect(calls).not.toContain("remove_library_track");

    await userEvent.click(within(dialog).getByRole("button", { name: tx("library:remove.cancel") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls).not.toContain("remove_library_track");
    expect(await rows()).toHaveLength(2);
  });

  it("removes the chosen track once confirmed, and only that one", async () => {
    const calls = removable([track(1), track(2)]);
    renderScreen();
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );

    expect(await screen.findByRole("status")).toHaveTextContent(tx("library:remove.done"));
    await waitFor(async () => expect(await rows()).toHaveLength(1));
    expect((await rows())[0]).toHaveTextContent("Synthetic Tune 2");
    expect(calls.filter((c) => c === "remove_library_track")).toHaveLength(1);
  });

  it("says the track was removed and offers no Undo there, only the top bar's", async () => {
    removable([track(1), track(2)]);
    renderScreen(true);
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    const line = await screen.findByRole("status");
    expect(line).toHaveTextContent(tx("library:remove.done"));
    expect(within(line).queryByRole("button")).toBeNull();
    await expectOnlyTheTopBarUndo();
  });

  it("drops the line when the top bar's Undo takes the removal back", async () => {
    removable([track(1), track(2)]);
    renderScreen(true);
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(tx("library:remove.done"));
    await userEvent.click(await expectOnlyTheTopBarUndo());
    await waitFor(async () => expect(await rows()).toHaveLength(2));
    await waitFor(() => expect(screen.queryByText(tx("library:remove.done"))).toBeNull());
  });

  it("says so, with only the top bar's Undo, when the last track was removed and the list is empty", async () => {
    removable([track(1)]);
    renderScreen(true);
    const [only] = await rows();
    await userEvent.click(within(only).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(tx("library:remove.done"));
    await expectOnlyTheTopBarUndo();
  });

  it("names the track on each row's Remove button", async () => {
    removable([track(1), track(2)]);
    renderScreen();
    await rows();
    expect(screen.getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 2" }) })).toBeInTheDocument();
  });

  it("gives the dialog focus, on Cancel", async () => {
    removable([track(1)]);
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    const dialog = await screen.findByRole("alertdialog");
    expect(within(dialog).getByRole("button", { name: tx("library:remove.cancel") })).toHaveFocus();
  });

  it("says how many open conflicts removing the track drops", async () => {
    removable([track(1, { openConflicts: 2 }), track(2, { openConflicts: 1 }), track(3)]);
    renderScreen();
    const [first, second, third] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    expect(await screen.findByRole("alertdialog")).toHaveTextContent(
      tx("library:remove.conflicts", { count: 2 }),
    );
    await userEvent.click(screen.getByRole("button", { name: tx("library:remove.cancel") }));
    await userEvent.click(within(second).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 2" }) }));
    expect(await screen.findByRole("alertdialog")).toHaveTextContent(
      tx("library:remove.conflicts", { count: 1 }),
    );
    await userEvent.click(screen.getByRole("button", { name: tx("library:remove.cancel") }));
    await userEvent.click(within(third).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 3" }) }));
    const noConflicts = await screen.findByRole("alertdialog");
    expect(noConflicts).toHaveAccessibleDescription(tx("library:remove.detail"));
    expect(noConflicts).not.toHaveTextContent(tx("library:remove.conflicts", { count: 0 }));
  });

  it("cancels the dialog on Escape without removing anything", async () => {
    const calls = removable([track(1)]);
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await screen.findByRole("alertdialog");
    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls).not.toContain("remove_library_track");
  });

  it("includes the conflicts line in the dialog's description", async () => {
    removable([track(1, { openConflicts: 2 })]);
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    expect(await screen.findByRole("alertdialog")).toHaveAccessibleDescription(
      `${tx("library:remove.detail")} ${tx("library:remove.conflicts", { count: 2 })}`,
    );
  });

});

/**
 * A Library with crates to add to, like the Rust side: `calls` records the
 * crate commands with their arguments. A track already in the crate is
 * skipped; an add that changed something is the next step to undo (for the
 * top bar's Undo).
 */
function crateBackend(crates: { id: number; name: string; trackIds: number[] }[]) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  let last: { id: number; added: number[] } | null = null;
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    if (cmd === "key_notation") return "camelot";
    if (cmd === "table_columns") return null;
    if (cmd === "track_table_page") {
      return pageOf([track(1), track(2)].map(rowOfLibraryTrack), queryOf(args));
    }
    if (cmd === "list_crates") {
      return crates.map((c) => ({
        id: c.id,
        parentId: null,
        kind: "static",
        name: c.name,
        trackCount: c.trackIds.length,
      }));
    }
    calls.push({ cmd, args });
    if (cmd === "add_tracks_to_crate") {
      const target = crates.find((c) => c.id === args.id);
      if (target === undefined) throw { kind: "crateNotFound", params: {} };
      const wanted = args.tracks as number[];
      const fresh = wanted.filter((id) => !target.trackIds.includes(id));
      target.trackIds.push(...fresh);
      last = { id: target.id, added: fresh };
      return {
        changed: fresh.length,
        skipped: wanted.length - fresh.length,
        // As the Rust side answers: an add that changed nothing records nothing.
        operationId: fresh.length > 0 ? 1 : null,
      };
    }
    if (cmd === "undo_last_operation") {
      last = null;
      return { status: "undone", operation: { id: 1, kind: "x", details: { name: null, from: null, tracks: null, newToLibrary: null, newCrate: null } } };
    }
    if (cmd === "next_undo_operation") {
      const operation =
        last === null || last.added.length === 0
          ? null
          : { id: 1, kind: "add_to_crate", details: { name: "Warm up", from: null, tracks: last.added.length, newToLibrary: null, newCrate: null } };
      return { operation, refusal: null };
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return calls;
}

/** The "Add to crate" choice of the row titled `title`. */
async function addChoice(title: string) {
  return screen.findByRole("combobox", { name: tx("crates:addTrack.labelFor", { title }) });
}

describe("adding a track to a crate", () => {
  it("offers every crate on each row, and says there are none when there are none", async () => {
    crateBackend([]);
    const first = renderScreen();
    const choice = await addChoice("Synthetic Tune 1");
    expect(choice).toBeDisabled();
    expect(within(choice).getAllByRole("option").map((o) => o.textContent)).toEqual([
      tx("crates:addTrack.none"),
    ]);
    first.unmount();

    crateBackend([
      { id: 1, name: "Warm up", trackIds: [] },
      { id: 2, name: "Peak time", trackIds: [] },
    ]);
    renderScreen();
    const second = await addChoice("Synthetic Tune 2");
    await waitFor(() => expect(second).toBeEnabled());
    expect(within(second).getAllByRole("option").map((o) => o.textContent)).toEqual([
      tx("crates:addTrack.label"),
      "Warm up",
      "Peak time",
    ]);
  });

  it("adds that track, and only that track, to the chosen crate", async () => {
    const calls = crateBackend([
      { id: 1, name: "Warm up", trackIds: [] },
      { id: 2, name: "Peak time", trackIds: [] },
    ]);
    renderScreen();
    const choice = await addChoice("Synthetic Tune 2");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Peak time");

    expect(await screen.findByRole("status")).toHaveTextContent(
      tx("crates:addTrack.added", { crate: "Peak time" }),
    );
    expect(calls.filter((c) => c.cmd === "add_tracks_to_crate")).toEqual([
      { cmd: "add_tracks_to_crate", args: { id: 2, tracks: [2] } },
    ]);
    // The row goes back to its first line, ready for the next track.
    expect(choice).toHaveValue("");
  });

  it("says a track is already in the crate", async () => {
    crateBackend([{ id: 1, name: "Warm up", trackIds: [1] }]);
    renderScreen();
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");

    expect(await screen.findByRole("status")).toHaveTextContent(
      tx("crates:addTrack.alreadyIn", { crate: "Warm up" }),
    );
  });

  it("says the track was added and offers no Undo there, only the top bar's", async () => {
    crateBackend([{ id: 1, name: "Warm up", trackIds: [] }]);
    renderScreen(true);
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");

    const line = await screen.findByRole("status");
    expect(line).toHaveTextContent(tx("crates:addTrack.added", { crate: "Warm up" }));
    expect(within(line).queryByRole("button")).toBeNull();
    await expectOnlyTheTopBarUndo();
  });

  it("drops the line when the top bar's Undo takes the add back", async () => {
    crateBackend([{ id: 1, name: "Warm up", trackIds: [] }]);
    renderScreen(true);
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");
    expect(await screen.findByRole("status")).toHaveTextContent(
      tx("crates:addTrack.added", { crate: "Warm up" }),
    );
    await userEvent.click(await expectOnlyTheTopBarUndo());
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  });

  it("shows why an add was refused", async () => {
    mockIPC((cmd, args) => {
      if (cmd === "key_notation") return "camelot";
      if (cmd === "table_columns") return null;
      if (cmd === "track_table_page") return pageOf([rowOfLibraryTrack(track(1))], queryOf(args));
      if (cmd === "list_crates") {
        return [{ id: 1, parentId: null, kind: "static", name: "Warm up", trackCount: 0 }];
      }
      if (cmd === "add_tracks_to_crate") throw { kind: "crateNotFound", params: {} };
      throw new Error(`unexpected command ${cmd}`);
    });
    renderScreen();
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("crates:error.notFound"));
  });
});

describe("the Library table's cut-off text", () => {
  const longTitle =
    "A made-up title that keeps going far past any column width (Extended Club Mix With The Long Intro)";
  const longArtist = "A Made Up Ensemble Of Many Collaborators feat. Another Very Long Guest Artist";

  it("a cut-off title cell carries its full text", async () => {
    library([track(1, { title: longTitle, artist: longArtist })]);
    renderScreen();
    const [row] = await rows();
    const [title, artist] = within(row).getAllByRole("gridcell");
    expect(within(title).getByText(longTitle)).toHaveAttribute("title", longTitle);
    expect(artist).toHaveAttribute("title", longArtist);
  });

  it("a title that is the file's name carries that name", async () => {
    library([track(1, { title: null, artist: null })]);
    renderScreen();
    const [row] = await rows();
    const [title, artist] = within(row).getAllByRole("gridcell");
    expect(within(title).getByText("tune 1.mp3")).toHaveAttribute("title", "tune 1.mp3");
    // No artist: nothing to say on hover.
    expect(artist).not.toHaveAttribute("title");
  });
});

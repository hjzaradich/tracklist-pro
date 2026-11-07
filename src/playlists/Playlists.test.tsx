import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import { crate, libraryRows } from "../test/crateBackend";
import { type FakePlaylist, playlistBackend } from "../test/playlistBackend";
import { tx } from "../test/tx";

// The Playlists screen and the sidebar's list (3C-P1b; ROADMAP 3.5). The whole
// app is rendered: the drop lands in the sidebar, Undo is in the top bar, and
// the line over the table says what an add did.

const ROW = 28;

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
  // The table's scroll area: 800 by 400, at the window's corner.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const grid = this.getAttribute("role") === "grid";
    return { x: 0, y: 0, top: 0, left: 0, right: grid ? 800 : 0, bottom: grid ? 400 : 0, width: grid ? 800 : 0, height: grid ? 400 : 0, toJSON: () => ({}) };
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
  document.body.innerHTML = "";
});

const TUNES = libraryRows(5);
/** Arc: Tune 0, 1, 2, then Tune 0 again; entry ids 1 to 4. */
const ARC: FakePlaylist = {
  id: 1,
  name: "Arc",
  origin: "app",
  sourcePath: null,
  entries: [
    [1, 100],
    [2, 101],
    [3, 102],
    [4, 100],
  ],
};
const WARMUP: FakePlaylist = { id: 2, name: "Warmup", origin: "app", sourcePath: null, entries: [] };

const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const rowAt = (index: number) => {
  const found = rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === index);
  if (!found) throw new Error(`row ${index} is not drawn`);
  return found;
};
/** What each row shows: its `#` and its title. */
const shown = () =>
  rows().map((row) => {
    const cells = within(row).getAllByRole("gridcell");
    return [cells[0]?.textContent, /Tune \d+/.exec(row.textContent ?? "")?.[0]];
  });

async function openPlaylist(options: Partial<Parameters<typeof playlistBackend>[0]> = {}, id = 1) {
  const backend = playlistBackend({ rows: TUNES, playlists: [ARC, WARMUP], ...options });
  renderApp(`/playlists?playlist=${id}`);
  await screen.findByRole("heading", { level: 2 });
  // A playlist with nothing in it has no table to wait for.
  if (backend.state.playlists.find((p) => p.id === id)?.entries.length) {
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(0));
  }
  return backend;
}

const button = (key: string, params?: Record<string, string | number>) =>
  screen.findByRole("button", { name: tx(key, params) });

/** Holds a row, moves the pointer to `y` over the table and lets go there. */
function dragRowTo(index: number, y: number, release = true) {
  fireEvent.pointerDown(rowAt(index), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
  fireEvent.pointerMove(document.body, { clientX: 320, clientY: y, pointerId: 1 });
  if (release) fireEvent.pointerUp(document.body, { clientX: 320, clientY: y, pointerId: 1 });
}

describe("the playlists in the sidebar", () => {
  it("lists every playlist under the Playlists link, and says when there are none", async () => {
    playlistBackend({ rows: TUNES, playlists: [ARC, WARMUP] });
    renderApp("/overview");
    const list = await screen.findByRole("list", { name: tx("playlists:list.label") });
    expect(within(list).getAllByRole("link").map((link) => link.textContent)).toEqual(["Arc", "Warmup"]);
    expect(screen.getByRole("link", { name: tx("playlists:title") })).toBeInTheDocument();
    cleanup();

    playlistBackend({ rows: TUNES, playlists: [] });
    renderApp("/overview");
    expect(await screen.findByText(tx("playlists:empty"))).toBeInTheDocument();
  });

  it("a playlist in the list opens its tracks", async () => {
    playlistBackend({ rows: TUNES, playlists: [ARC, WARMUP] });
    renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: "Arc" }));
    await screen.findByRole("grid");
    await waitFor(() => expect(shown()).toHaveLength(4));
  });

  it("tracks dropped on a playlist go to its end as one step, and the table says so", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC, WARMUP] });
    renderApp("/library");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(2));
    const target = (await screen.findByRole("link", { name: "Warmup" })).parentElement as HTMLElement;
    fireEvent.pointerDown(rowAt(1), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(document.body, { clientX: 320, clientY: 120, pointerId: 1 });
    fireEvent.pointerEnter(target, { pointerId: 1 });
    expect(target).toHaveAttribute("data-drop", "target");
    fireEvent.pointerUp(target, { pointerId: 1 });

    expect(await screen.findByText(tx("playlists:addSelection.added", { count: 1, playlist: "Warmup" }))).toBeInTheDocument();
    expect(backend.called("add_recordings_to_playlist")[0]?.args).toMatchObject({ id: 2, recordings: [101], at: null });
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.addToPlaylist", { count: 1, name: "Warmup" }) }),
    ).toBeInTheDocument();
  });
});

describe("the Playlists screen", () => {
  it("shows the tracks in the playlist's order with their place in a # column", async () => {
    await openPlaylist();
    expect(shown()).toEqual([
      ["1", "Tune 0"],
      ["2", "Tune 1"],
      ["3", "Tune 2"],
      ["4", "Tune 0"],
    ]);
    expect(screen.getByRole("columnheader", { name: tx("trackTable:columns.position") })).toBeInTheDocument();
  });

  it("a track that is in the playlist twice is two rows, each with its own remove", async () => {
    const backend = await openPlaylist();
    expect(rows().filter((row) => row.textContent?.includes("Tune 0"))).toHaveLength(2);
    await userEvent.click(within(rowAt(3)).getByRole("button", { name: tx("playlists:removeTrack.buttonFor", { title: "Tune 0" }) }));
    await waitFor(() => expect(shown()).toHaveLength(3));
    expect(backend.called("remove_from_playlist")[0]?.args).toMatchObject({ id: 1, entries: [4] });
    expect(shown().map((row) => row[1])).toEqual(["Tune 0", "Tune 1", "Tune 2"]);
    expect(await screen.findByText(tx("playlists:done.removed", { title: "Tune 0" }))).toBeInTheDocument();
    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.removeFromPlaylist", { count: 1, name: "Arc" }) }),
    );
    await waitFor(() => expect(shown()).toHaveLength(4));
  });

  it("dragging a row to another place moves it with its entry id as one step, and the line shows where", async () => {
    const backend = await openPlaylist();
    // Over the table, between the second and third rows (the header is the first row).
    dragRowTo(0, ROW + 2 * ROW, false);
    expect(screen.getByTestId("insert-line")).toHaveStyle({ transform: `translateY(${2 * ROW}px)` });
    fireEvent.pointerUp(document.body, { clientX: 320, clientY: 3 * ROW, pointerId: 1 });
    await waitFor(() => expect(backend.called("move_in_playlist")).toHaveLength(1));
    expect(backend.called("move_in_playlist")[0]?.args).toMatchObject({ id: 1, entries: [1], before: 3 });
    await waitFor(() => expect(shown().map((row) => row[1])).toEqual(["Tune 1", "Tune 0", "Tune 2", "Tune 0"]));
    expect(screen.queryByTestId("insert-line")).toBeNull();
    // The repeat did not move with it: it is its own entry.
    expect(backend.state.playlists[0]?.entries.map(([entry]) => entry)).toEqual([2, 1, 3, 4]);

    const undo = await screen.findByRole("button", { name: tx("shell:undo.action.moveInPlaylist", { count: 1, name: "Arc" }) });
    await userEvent.click(undo);
    await waitFor(() => expect(shown().map((row) => row[1])).toEqual(["Tune 0", "Tune 1", "Tune 2", "Tune 0"]));
  });

  it("dropping below the last row moves to the end, and Escape gives the drag up", async () => {
    const backend = await openPlaylist();
    dragRowTo(0, 300, false);
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.pointerUp(document.body, { clientX: 320, clientY: 300, pointerId: 1 });
    expect(backend.called("move_in_playlist")).toHaveLength(0);
    expect(screen.queryByTestId("insert-line")).toBeNull();

    dragRowTo(0, 300);
    await waitFor(() => expect(backend.called("move_in_playlist")).toHaveLength(1));
    expect(backend.called("move_in_playlist")[0]?.args).toMatchObject({ entries: [1], before: null });
  });

  it("while the list is sorted by a column, rows cannot be dragged to another place", async () => {
    const backend = await openPlaylist();
    await userEvent.click(
      within(screen.getByRole("columnheader", { name: tx("trackTable:columns.title") })).getByRole("button"),
    );
    await waitFor(() => expect(backend.called("track_table_page").some((c) => JSON.stringify(c.args).includes('"title"'))).toBe(true));
    dragRowTo(0, 3 * ROW, false);
    expect(screen.queryByTestId("insert-line")).toBeNull();
    fireEvent.pointerUp(document.body, { clientX: 320, clientY: 3 * ROW, pointerId: 1 });
    expect(backend.called("move_in_playlist")).toHaveLength(0);
  });

  it("while a filter is on, rows cannot be dragged to another place", async () => {
    const backend = await openPlaylist();
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.add") }));
    await userEvent.click(await screen.findByRole("button", { name: tx("trackTable:filters.missing") }));
    await screen.findByRole("list", { name: tx("trackTable:filters.label") });
    dragRowTo(0, 3 * ROW, false);
    expect(screen.queryByTestId("insert-line")).toBeNull();
    fireEvent.pointerUp(document.body, { clientX: 320, clientY: 3 * ROW, pointerId: 1 });
    expect(backend.called("move_in_playlist")).toHaveLength(0);
  });

  it("dragging several selected rows moves every one of them, by entry id, in the order shown", async () => {
    const distinct: FakePlaylist = {
      ...ARC,
      entries: [
        [1, 100],
        [2, 101],
        [3, 102],
        [4, 103],
      ],
    };
    const backend = await openPlaylist({ playlists: [distinct, WARMUP] });
    fireEvent.click(rowAt(0));
    fireEvent.click(rowAt(2), { ctrlKey: true });
    // Between the third and fourth rows.
    dragRowTo(0, ROW + 3 * ROW);
    await waitFor(() => expect(backend.called("move_in_playlist")).toHaveLength(1));
    expect(backend.called("move_in_playlist")[0]?.args).toMatchObject({ id: 1, entries: [1, 3], before: 4 });
    await waitFor(() => expect(shown().map((row) => row[1])).toEqual(["Tune 1", "Tune 0", "Tune 2", "Tune 3"]));
  });

  it("a selection that reaches rows not loaded is never partly moved: the drag shows no line and moves nothing", async () => {
    const long: FakePlaylist = {
      ...ARC,
      entries: Array.from({ length: 450 }, (_, n): [number, number] => [n + 1, 100 + n]),
    };
    const backend = await openPlaylist({ rows: libraryRows(450), playlists: [long, WARMUP] });
    fireEvent.click(rowAt(0));
    await userEvent.keyboard("{Control>}a{/Control}");
    await waitFor(() => expect(backend.called("track_table_ids").length).toBeGreaterThan(0));
    await waitFor(() => expect(rowAt(0)).toHaveAttribute("aria-selected", "true"));
    dragRowTo(0, 3 * ROW, false);
    expect(screen.queryByTestId("insert-line")).toBeNull();
    fireEvent.pointerUp(document.body, { clientX: 320, clientY: 3 * ROW, pointerId: 1 });
    expect(backend.called("move_in_playlist")).toHaveLength(0);
  });

  it("a drag that ends outside the table moves nothing", async () => {
    const backend = await openPlaylist();
    dragRowTo(0, 900);
    expect(backend.called("move_in_playlist")).toHaveLength(0);
  });

  it("makes a playlist, renames it and deletes it, each a step with its own Undo", async () => {
    const backend = await openPlaylist({ playlists: [WARMUP] }, 2);
    await userEvent.click(await button("playlists:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("playlists:new.label")), "Closing{Enter}");
    expect(await screen.findByText(tx("playlists:done.created", { name: "Closing" }))).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: tx("shell:undo.action.createPlaylist", { name: "Closing" }) })).toBeInTheDocument();

    await userEvent.click(await button("playlists:rename.button"));
    const box = await screen.findByLabelText(tx("playlists:rename.label"));
    await userEvent.clear(box);
    await userEvent.type(box, "Closers{Enter}");
    expect(await screen.findByText(tx("playlists:done.renamed", { name: "Closers" }))).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.renamePlaylist", { from: "Closing", name: "Closers" }) }),
    ).toBeInTheDocument();

    await userEvent.click(await button("playlists:delete.button"));
    expect(backend.called("delete_playlist")).toHaveLength(0);
    const confirm = await screen.findByRole("alertdialog");
    await userEvent.click(within(confirm).getByRole("button", { name: tx("playlists:delete.confirm") }));
    expect(await screen.findByText(tx("playlists:done.deleted", { name: "Closers" }))).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.deletePlaylist", { name: "Closers" }) }),
    ).toBeInTheDocument();
  });

  it("a name already used is refused where it was typed", async () => {
    await openPlaylist({ playlists: [WARMUP] }, 2);
    await userEvent.click(await button("playlists:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("playlists:new.label")), "warmup{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("playlists:error.nameTaken", { name: "Warmup" }));
  });

  it("the add dialog lists playlists and adds the selected tracks to the end of the one chosen", async () => {
    const backend = playlistBackend({ rows: TUNES, playlists: [ARC, WARMUP] });
    renderApp("/library");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(2));
    fireEvent.click(rowAt(2));
    await userEvent.keyboard("c");
    const choice = await screen.findByRole("option", { name: new RegExp("Warmup$") });
    await userEvent.click(choice);
    await waitFor(() => expect(backend.called("add_recordings_to_playlist")).toHaveLength(1));
    expect(backend.called("add_recordings_to_playlist")[0]?.args).toMatchObject({ id: 2, recordings: [102] });
    expect(await screen.findByText(tx("playlists:addSelection.added", { count: 1, playlist: "Warmup" }))).toBeInTheDocument();
  });

  it("a track outside the Library joins it with the add, as for a crate, and one undo takes both back", async () => {
    const stray = { ...libraryRows(6)[5]!, libraryTrackId: null, inLibrary: false };
    const backend = playlistBackend({ rows: [...TUNES, stray], playlists: [WARMUP] });
    renderApp("/all-music");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(2));
    fireEvent.click(rowAt(5));
    await userEvent.keyboard("c");
    await userEvent.click(await screen.findByRole("option", { name: new RegExp("Warmup$") }));
    await waitFor(() => expect(backend.state.playlists[0]?.entries).toHaveLength(1));
    const line = await screen.findByText(tx("playlists:addSelection.added", { count: 1, playlist: "Warmup" }), { exact: false });
    expect(line.textContent).toContain(tx("crates:addSelection.alsoInLibrary", { count: 1 }));
    expect(stray.libraryTrackId).not.toBeNull();
    const undo = await screen.findByRole("button", {
      name: tx("shell:undo.action.addToPlaylistLibrary", { count: 1, name: "Warmup", library: 1 }),
    });
    await userEvent.click(undo);
    await waitFor(() => expect(backend.state.playlists[0]?.entries).toHaveLength(0));
    expect(stray.libraryTrackId).toBeNull();
  });

  it("a track the Library add refuses is left out of a playlist add, with the reason", async () => {
    const stray = { ...libraryRows(6)[5]!, libraryTrackId: null, inLibrary: false };
    const backend = playlistBackend({
      rows: [...TUNES, stray],
      playlists: [WARMUP],
      refused: { [stray.recordingId]: "fileMissing" },
    });
    renderApp("/all-music");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(2));
    fireEvent.click(rowAt(5));
    await userEvent.keyboard("c");
    await userEvent.click(await screen.findByRole("option", { name: new RegExp("Warmup$") }));
    const note = tx("crates:addSelection.leftOut.fileMissing", { count: 1 });
    await waitFor(() =>
      expect(screen.getAllByRole("status").some((line) => line.textContent?.includes(note))).toBe(true),
    );
    expect(backend.state.playlists[0]?.entries).toHaveLength(0);
    expect(backend.state.history).toHaveLength(0);
  });
});

describe("converting between playlists, crates and rekordbox playlists", () => {
  it("Save as crate makes a crate of the playlist and says so", async () => {
    const backend = await openPlaylist();
    await userEvent.click(await button("playlists:saveAsCrate.button"));
    await userEvent.click(await button("playlists:saveAsCrate.confirm"));
    expect(await screen.findByText(tx("playlists:done.savedAsCrate", { name: "Arc" }))).toBeInTheDocument();
    expect(backend.called("save_playlist_as_crate")[0]?.args).toMatchObject({ id: 1, name: "Arc" });
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.savePlaylistAsCrate", { name: "Arc" }) }),
    ).toBeInTheDocument();
  });

  it("Start a playlist from this crate takes the crate table's order and opens the new playlist", async () => {
    const backend = playlistBackend({
      rows: TUNES,
      playlists: [],
      crates: [crate(7, "Peak")],
    });
    renderApp("/crates?crate=7");
    await screen.findByRole("grid");
    await userEvent.click(await button("playlists:fromCrate.button"));
    await userEvent.click(await button("playlists:fromCrate.confirm"));
    await waitFor(() => expect(backend.called("playlist_from_crate")).toHaveLength(1));
    // The order asked for is the table's rows as listed, by Library track.
    expect(backend.called("playlist_from_crate")[0]?.args).toMatchObject({
      crateId: 7,
      name: "Peak",
      order: [1, 2, 3, 4, 5],
    });
    expect(await screen.findByRole("heading", { level: 2, name: "Peak" })).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.playlistFromCrate", { name: "Peak" }) }),
    ).toBeInTheDocument();
  });

  it("a rekordbox playlist is picked from the last read and copied, saying how many entries could not be matched", async () => {
    const backend = await openPlaylist({
      playlists: [WARMUP],
      rekordbox: [
        { path: ["Friday", "Peak"], entries: 5 },
        { path: ["Chill"], entries: 2 },
      ],
      unmatched: 2,
    }, 2);
    await userEvent.click(await button("playlists:fromRekordbox.button"));
    await userEvent.click(await screen.findByRole("button", { name: /Friday › Peak/ }));
    await userEvent.click(await button("playlists:fromRekordbox.confirm"));
    expect(
      await screen.findByText(tx("playlists:done.copiedLeftOut", { name: "Peak", count: 3, left: 2 })),
    ).toBeInTheDocument();
    expect(backend.called("playlist_from_rekordbox")[0]?.args).toMatchObject({ path: ["Friday", "Peak"], name: "Peak" });
    // The new playlist is open, and says where it came from.
    expect(await screen.findByText(tx("playlists:copiedFrom", { path: "Friday › Peak" }))).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: tx("shell:undo.action.playlistFromRekordbox", { name: "Peak" }) }),
    ).toBeInTheDocument();
  });

  it("says when the last read has no rekordbox playlists", async () => {
    await openPlaylist({ playlists: [WARMUP], rekordbox: [] }, 2);
    await userEvent.click(await button("playlists:fromRekordbox.button"));
    expect(await screen.findByText(tx("playlists:fromRekordbox.none"))).toBeInTheDocument();
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LibraryTrack } from "../bindings";
import "../i18n";
import { LibraryScreen } from "./LibraryScreen";
import { tx } from "../test/tx";

function track(id: number, fields: Partial<LibraryTrack> = {}): LibraryTrack {
  return {
    id,
    recordingId: id + 100,
    kind: "linked",
    title: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    file: {
      path: String.raw`E:\Music\tune ` + id + ".mp3",
      name: `tune ${id}.mp3`,
      present: true,
      driveConnected: true,
    },
    sourceMissing: false,
    openConflicts: 0,
    fragile: null,
    addedAt: "2026-09-30T10:00:00.000Z",
    ...fields,
  };
}

/** What a row's "Add to crate" says while there are no crates to choose. */
const noCrates = tx("crates:addTrack.none");

/** A linked file the last scan didn't find, on a drive that's connected. */
const goneFile = {
  path: String.raw`E:\Music\gone.mp3`,
  name: "gone.mp3",
  present: false,
  driveConnected: true,
};

/** Stands in for the Rust side: the Library holds `tracks`, or can't be read. */
function library(tracks: LibraryTrack[] | "broken") {
  mockIPC((cmd) => {
    if (cmd === "list_crates") return [];
    if (cmd !== "library_tracks") throw new Error(`unexpected command ${cmd}`);
    if (tracks === "broken") throw { kind: "database", params: {} };
    return tracks;
  });
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <LibraryScreen />
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

describe("the Library screen", () => {
  it("names what's absent when there are no Library tracks", async () => {
    library([]);
    renderScreen();
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("lists each Library track's title, artist and file, in the order the backend gave", async () => {
    library([track(2), track(1)]);
    renderScreen();
    const [first, second] = await rows();
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      tx("library:columns.title"),
      tx("library:columns.artist"),
      tx("library:columns.file"),
      tx("library:columns.actions"),
    ]);
    expect(
      within(first)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["Synthetic Tune 2", "Made Up Artist 2", String.raw`E:\Music\tune 2.mp3`, `${noCrates}${tx("library:remove.button")}`]);
    expect(second).toHaveTextContent("Synthetic Tune 1");
    expect(screen.queryByText(tx("library:empty"))).toBeNull();
    // Nothing to say about a track whose file is there.
    expect(first).not.toHaveAttribute("aria-describedby");
  });

  it("shows the file's name as the title of a track that has none", async () => {
    library([track(1, { title: null, artist: null })]);
    renderScreen();
    const [row] = await rows();
    expect(
      within(row)
        .getAllByRole("cell")
        .map((cell) => cell.textContent),
    ).toEqual(["tune 1.mp3", "", String.raw`E:\Music\tune 1.mp3`, `${noCrates}${tx("library:remove.button")}`]);
  });

  it("says so next to a track whose file is missing", async () => {
    const gone = track(2, { sourceMissing: true, file: goneFile });
    library([track(1), gone]);
    renderScreen();
    const [here, missing] = await rows();
    expect(missing).toHaveAccessibleDescription(tx("library:fileMissing"));
    expect(missing).toHaveTextContent(String.raw`E:\Music\gone.mp3`);
    expect(missing).toHaveAttribute("data-file-present", "false");
    expect(here).not.toHaveTextContent(tx("library:fileMissing"));
    expect(here).toHaveAttribute("data-file-present", "true");
  });

  it.each(["downloads", "temp", "external", "network"] as const)(
    "says why a file in a %s location is fragile",
    async (reason) => {
      library([track(1, { fragile: reason })]);
      renderScreen();
      const [row] = await rows();
      expect(row).toHaveAccessibleDescription(tx(`library:fragile.${reason}`));
    },
  );

  it("shows no note for a track whose file isn't in a fragile location", async () => {
    library([track(1)]);
    renderScreen();
    const [row] = await rows();
    expect(row).not.toHaveAttribute("aria-describedby");
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
          path: String.raw`G:\Music\a.mp3`,
          name: "a.mp3",
          present: true,
          driveConnected: false,
        },
      }),
    ]);
    renderScreen();
    const [row] = await rows();
    expect(row).toHaveAccessibleDescription(tx("library:driveNotConnected"));
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
 * A Library that removes and restores tracks like the Rust side: `calls`
 * records every command, and the file on disk is never part of it.
 */
function removable(initial: LibraryTrack[]) {
  let tracks = initial;
  let last: LibraryTrack | null = null;
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    calls.push(cmd);
    if (cmd === "list_crates") return [];
    if (cmd === "library_tracks") return tracks;
    if (cmd === "remove_library_track") {
      last = tracks.find((t) => t.id === (args as { id: number }).id) ?? null;
      tracks = tracks.filter((t) => t !== last);
      return null;
    }
    if (cmd === "undo_last_operation") {
      if (last === null) return { status: "nothingToUndo" };
      tracks = [...tracks, last].sort((a, b) => a.id - b.id);
      return { status: "undone", operation: { id: 1, kind: "remove_from_library" } };
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

  it("puts the track back when the user undoes the removal", async () => {
    removable([track(1), track(2)]);
    renderScreen();
    const [first] = await rows();
    await userEvent.click(within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    await userEvent.click(await screen.findByRole("button", { name: tx("library:remove.undo") }));

    expect(await screen.findByText(tx("library:remove.undone"))).toBeInTheDocument();
    await waitFor(async () => expect(await rows()).toHaveLength(2));
    expect(screen.queryByRole("button", { name: tx("library:remove.undo") })).toBeNull();
  });

  it("offers undo even when the last track was removed and the list is empty", async () => {
    removable([track(1)]);
    renderScreen();
    const [only] = await rows();
    await userEvent.click(within(only).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("library:remove.undo") })).toBeInTheDocument();
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

  it("says so, and keeps the track out, when undo is refused", async () => {
    let tracks = [track(1)];
    mockIPC((cmd) => {
    if (cmd === "list_crates") return [];
    if (cmd === "library_tracks") return tracks;
      if (cmd === "remove_library_track") {
        tracks = [];
        return null;
      }
      if (cmd === "undo_last_operation") {
        return {
          status: "refused",
          operation: { id: 1, kind: "remove_from_library" },
          conflicts: [],
        };
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    await userEvent.click(await screen.findByRole("button", { name: tx("library:remove.undo") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("library:remove.undoRefused"));
    expect(await screen.findByText(tx("library:empty"))).toBeInTheDocument();
  });

  it("disables Undo while the undo is running", async () => {
    let finish: (v: unknown) => void = () => {};
    const pending = new Promise((resolve) => (finish = resolve));
    let tracks = [track(1)];
    mockIPC((cmd) => {
    if (cmd === "list_crates") return [];
    if (cmd === "library_tracks") return tracks;
      if (cmd === "remove_library_track") {
        tracks = [];
        return null;
      }
      if (cmd === "undo_last_operation") return pending;
      throw new Error(`unexpected command ${cmd}`);
    });
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    const undo = await screen.findByRole("button", { name: tx("library:remove.undo") });
    await userEvent.click(undo);
    await waitFor(() => expect(undo).toBeDisabled());
    finish({ status: "nothingToUndo" });
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

  it("greys out Undo, with no message, when there turns out to be nothing to undo", async () => {
    let tracks = [track(1)];
    mockIPC((cmd) => {
    if (cmd === "list_crates") return [];
    if (cmd === "library_tracks") return tracks;
      if (cmd === "remove_library_track") {
        tracks = [];
        return null;
      }
      if (cmd === "undo_last_operation") return { status: "nothingToUndo" };
      throw new Error(`unexpected command ${cmd}`);
    });
    renderScreen();
    const [row] = await rows();
    await userEvent.click(within(row).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
    );
    const undo = await screen.findByRole("button", { name: tx("library:remove.undo") });
    expect(undo).toBeEnabled();
    await userEvent.click(undo);
    await waitFor(() => expect(screen.getByRole("button", { name: tx("library:remove.undo") })).toBeDisabled());
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByText(tx("library:remove.undoRefused"))).toBeNull();
  });
});

/**
 * A Library with crates to add to, like the Rust side: `calls` records the
 * crate commands with their arguments. A track already in the crate is
 * skipped; an undo takes the last add back out.
 */
function crateBackend(crates: { id: number; name: string; trackIds: number[] }[]) {
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  let last: { id: number; added: number[] } | null = null;
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    if (cmd === "library_tracks") return [track(1), track(2)];
    if (cmd === "list_crates") {
      return crates.map((c) => ({ id: c.id, name: c.name, trackCount: c.trackIds.length }));
    }
    calls.push({ cmd, args });
    if (cmd === "add_tracks_to_crate") {
      const target = crates.find((c) => c.id === args.id);
      if (target === undefined) throw { kind: "crateNotFound", params: {} };
      const wanted = args.tracks as number[];
      const fresh = wanted.filter((id) => !target.trackIds.includes(id));
      target.trackIds.push(...fresh);
      last = { id: target.id, added: fresh };
      return { changed: fresh.length, skipped: wanted.length - fresh.length };
    }
    if (cmd === "undo_last_operation") {
      if (last === null) return { status: "nothingToUndo" };
      const target = crates.find((c) => c.id === last?.id);
      if (target) target.trackIds = target.trackIds.filter((id) => !last?.added.includes(id));
      last = null;
      return { status: "undone", operation: { id: 1, kind: "add_to_crate" } };
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

  it("says a track is already in the crate, with nothing to undo", async () => {
    crateBackend([{ id: 1, name: "Warm up", trackIds: [1] }]);
    renderScreen();
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");

    expect(await screen.findByRole("status")).toHaveTextContent(
      tx("crates:addTrack.alreadyIn", { crate: "Warm up" }),
    );
    expect(screen.queryByRole("button", { name: tx("crates:undo") })).toBeNull();
  });

  it("takes the track back out when the user undoes the add", async () => {
    const calls = crateBackend([{ id: 1, name: "Warm up", trackIds: [] }]);
    renderScreen();
    const choice = await addChoice("Synthetic Tune 1");
    await waitFor(() => expect(choice).toBeEnabled());
    await userEvent.selectOptions(choice, "Warm up");
    await userEvent.click(await screen.findByRole("button", { name: tx("crates:undo") }));

    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:undone"));
    expect(calls.map((c) => c.cmd)).toContain("undo_last_operation");
  });

  it("shows why an add was refused", async () => {
    mockIPC((cmd) => {
      if (cmd === "library_tracks") return [track(1)];
      if (cmd === "list_crates") return [{ id: 1, name: "Warm up", trackCount: 0 }];
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

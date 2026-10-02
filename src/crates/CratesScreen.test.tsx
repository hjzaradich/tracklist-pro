import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { Crate, LibraryTrack } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { CratesScreen } from "./CratesScreen";

function track(id: number): LibraryTrack {
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
  };
}

/**
 * Stands in for the Rust side: crates and what's in them, the commands the
 * screen calls (recorded in `calls` with their arguments), and a one-step
 * undo that puts the crates back as they were.
 */
function backend(
  initial: { crate: Crate; tracks: LibraryTrack[] }[],
  // `gone`: tracks the backend no longer has in any crate, though the screen
  // still lists them. `failing`: commands that are refused.
  options: { gone?: number[]; failing?: string[] } = {},
) {
  let crates = initial;
  let before: typeof crates | null = null;
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  let nextId = 100;
  const refuse = (kind: string, params: Record<string, string> = {}) => {
    throw { kind, params };
  };
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    if (options.failing?.includes(cmd)) return refuse("crateNotFound");
    const named = (name: string) => crates.find((c) => c.crate.name.toLowerCase() === name.trim().toLowerCase());
    switch (cmd) {
      case "list_crates":
        return crates.map((c) => ({ ...c.crate, trackCount: c.tracks.length }));
      case "crate_tracks":
        return crates.find((c) => c.crate.id === args.id)?.tracks ?? refuse("crateNotFound");
      case "create_crate": {
        const name = String(args.name).trim();
        if (name === "") return refuse("crateNameEmpty");
        const existing = named(name);
        if (existing) return refuse("crateNameTaken", { name: existing.crate.name });
        before = crates;
        const id = nextId++;
        crates = [...crates, { crate: { id, name, trackCount: 0 }, tracks: [] }];
        return id;
      }
      case "rename_crate": {
        const target = crates.find((c) => c.crate.id === args.id);
        const name = String(args.name).trim();
        // The name it has: nothing is recorded.
        if (target?.crate.name === name) return null;
        const existing = named(name);
        if (existing && existing !== target) {
          return refuse("crateNameTaken", { name: existing.crate.name });
        }
        before = crates;
        crates = crates.map((c) =>
          c.crate.id === args.id ? { ...c, crate: { ...c.crate, name } } : c,
        );
        return 7;
      }
      case "delete_crate":
        before = crates;
        crates = crates.filter((c) => c.crate.id !== args.id);
        return null;
      case "remove_tracks_from_crate": {
        before = crates;
        const wanted = args.tracks as number[];
        if (wanted.every((id) => options.gone?.includes(id))) {
          return { changed: 0, skipped: wanted.length, operationId: null };
        }
        crates = crates.map((c) =>
          c.crate.id === args.id
            ? { ...c, tracks: c.tracks.filter((t) => !wanted.includes(t.id)) }
            : c,
        );
        return { changed: wanted.length, skipped: 0, operationId: 8 };
      }
      case "undo_last_operation":
        if (before === null) return { status: "nothingToUndo" };
        crates = before;
        before = null;
        return { status: "undone", operation: { id: 1, kind: "x" } };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return calls;
}

function crateWith(id: number, name: string, tracks: LibraryTrack[]) {
  return { crate: { id, name, trackCount: tracks.length }, tracks };
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <CratesScreen />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  clearMocks();
});

const button = (key: string) => screen.findByRole("button", { name: tx(key) });

describe("the Crates screen", () => {
  it("names what's absent when there are no crates, and offers a new one", async () => {
    backend([]);
    renderScreen();
    expect(await screen.findByText(tx("crates:empty"))).toBeInTheDocument();
    expect(await button("crates:new.button")).toBeEnabled();
    expect(screen.queryByRole("list")).toBeNull();
  });

  it("lists each crate with its track count", async () => {
    backend([
      crateWith(1, "Warm up", []),
      crateWith(2, "Peak time", [track(1)]),
      crateWith(3, "Closing", [track(1), track(2)]),
    ]);
    renderScreen();
    const list = await screen.findByRole("list", { name: tx("crates:list.label") });
    const items = within(list).getAllByRole("listitem");
    expect(items.map((item) => item.textContent)).toEqual([
      `Warm up${tx("crates:trackCount", { count: 0 })}`,
      `Peak time${tx("crates:trackCount", { count: 1 })}`,
      `Closing${tx("crates:trackCount", { count: 2 })}`,
    ]);
    // The three counts are three different phrases.
    expect(new Set([0, 1, 2].map((count) => tx("crates:trackCount", { count }))).size).toBe(3);
  });

  it("asks which crate to look at until one is chosen", async () => {
    backend([crateWith(1, "Warm up", [track(1)])]);
    renderScreen();
    expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("shows the chosen crate's tracks in the Library list's columns, in the order the backend gave", async () => {
    backend([crateWith(1, "Warm up", [track(2), track(1)]), crateWith(2, "Other", [track(3)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    const table = await screen.findByRole("table");
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Warm up");
    expect(screen.getAllByRole("columnheader").map((header) => header.textContent)).toEqual([
      tx("library:columns.title"),
      tx("library:columns.artist"),
      tx("library:columns.file"),
      tx("library:columns.actions"),
    ]);
    const rows = within(table).getAllByRole("row").slice(1);
    expect(rows[0]).toHaveTextContent("Synthetic Tune 2");
    expect(rows[0]).toHaveTextContent("Made Up Artist 2");
    expect(rows[0]).toHaveTextContent(String.raw`E:\Music\tune 2.mp3`);
    expect(rows[1]).toHaveTextContent("Synthetic Tune 1");
    expect(rows).toHaveLength(2);
  });

  it("names what's absent in a crate with no tracks", async () => {
    backend([crateWith(1, "Warm up", [])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    expect(await screen.findByText(tx("crates:noTracks"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("takes a track out of the crate with its Remove, and only that track", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1), track(2)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(
      await screen.findByRole("button", {
        name: tx("crates:removeTrack.buttonFor", { title: "Synthetic Tune 1" }),
      }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.removed"));
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(2));
    expect(screen.getByRole("table")).not.toHaveTextContent("Synthetic Tune 1");
    expect(calls.filter((c) => c.cmd === "remove_tracks_from_crate")).toEqual([
      { cmd: "remove_tracks_from_crate", args: { id: 1, tracks: [1] } },
    ]);
  });

  it("puts a removed track back when the user undoes it", async () => {
    backend([crateWith(1, "Warm up", [track(1), track(2)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(
      await screen.findByRole("button", {
        name: tx("crates:removeTrack.buttonFor", { title: "Synthetic Tune 1" }),
      }),
    );
    await userEvent.click(await button("crates:undo"));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:undone"));
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(3));
  });
});

describe("making a crate", () => {
  it("asks for a name, makes the crate, and shows it chosen", async () => {
    const calls = backend([]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "Warm up");
    await userEvent.click(await button("crates:new.confirm"));

    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Warm up");
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.created"));
    expect(calls.filter((c) => c.cmd === "create_crate")).toEqual([
      { cmd: "create_crate", args: { name: "Warm up" } },
    ]);
    expect(screen.queryByLabelText(tx("crates:new.label"))).toBeNull();
  });

  it("says why a name was refused and keeps the form open", async () => {
    backend([crateWith(1, "Warm up", [])]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "WARM UP ");
    await userEvent.click(await button("crates:new.confirm"));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("crates:error.nameTaken", { name: "Warm up" }),
    );
    expect(screen.getByLabelText(tx("crates:new.label"))).toHaveValue("WARM UP ");
  });

  it("goes back without making anything", async () => {
    const calls = backend([]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.click(await button("crates:new.cancel"));
    expect(screen.queryByLabelText(tx("crates:new.label"))).toBeNull();
    expect(calls.map((c) => c.cmd)).not.toContain("create_crate");
  });

  it("puts the crate away again when the user undoes it", async () => {
    backend([]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "Warm up");
    await userEvent.click(await button("crates:new.confirm"));
    await userEvent.click(await button("crates:undo"));
    expect(await screen.findByText(tx("crates:empty"))).toBeInTheDocument();
  });
});

describe("renaming a crate", () => {
  it("starts from the current name and sends the new one", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:rename.button"));
    const input = await screen.findByLabelText(tx("crates:rename.label"));
    expect(input).toHaveValue("Warm up");
    await userEvent.clear(input);
    await userEvent.type(input, "Warm up 2");
    // The form's button has the header button's name: look inside the form.
    const form = input.closest("form") as HTMLElement;
    await userEvent.click(within(form).getByRole("button", { name: tx("crates:rename.confirm") }));

    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Warm up 2");
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.renamed"));
    expect(calls.filter((c) => c.cmd === "rename_crate")).toEqual([
      { cmd: "rename_crate", args: { id: 1, name: "Warm up 2" } },
    ]);
  });
});

describe("deleting a crate", () => {
  it("asks first, and deletes nothing until the user confirms", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:delete.button"));

    const dialog = await screen.findByRole("alertdialog");
    expect(dialog).toHaveTextContent(tx("crates:delete.title", { name: "Warm up" }));
    expect(dialog).toHaveTextContent(tx("crates:delete.detail"));
    expect(calls.map((c) => c.cmd)).not.toContain("delete_crate");

    await userEvent.click(within(dialog).getByRole("button", { name: tx("crates:delete.cancel") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls.map((c) => c.cmd)).not.toContain("delete_crate");
  });

  it("deletes the crate once confirmed, and brings it back on undo", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1)]), crateWith(2, "Other", [])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:delete.button"));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: tx("crates:delete.confirm"),
      }),
    );

    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.deleted"));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Warm up/ })).toBeNull());
    expect(calls.filter((c) => c.cmd === "delete_crate")).toEqual([
      { cmd: "delete_crate", args: { id: 1 } },
    ]);

    await userEvent.click(await button("crates:undo"));
    expect(await screen.findByRole("button", { name: /Warm up/ })).toBeInTheDocument();
  });
});

describe("undo", () => {
  it("says so, and changes nothing, when the Library has changed since", async () => {
    backend([]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "Warm up");
    await userEvent.click(await button("crates:new.confirm"));
    // The backend refuses the undo.
    mockIPC((cmd) => {
      if (cmd === "undo_last_operation") {
        return { status: "refused", operation: { id: 1, kind: "create_crate" }, conflicts: [] };
      }
      if (cmd === "list_crates") return [{ id: 100, name: "Warm up", trackCount: 0 }];
      if (cmd === "crate_tracks") return [];
      throw new Error(`unexpected command ${cmd}`);
    });
    await userEvent.click(await button("crates:undo"));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("crates:undoRefused"));
    // The crate is still there.
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Warm up");
  });
});

/** The Rename form's own confirm button (the header's button has the same name). */
async function confirmRename() {
  const input = await screen.findByLabelText(tx("crates:rename.label"));
  await userEvent.click(
    within(input.closest("form") as HTMLElement).getByRole("button", {
      name: tx("crates:rename.confirm"),
    }),
  );
}

describe("a change that records nothing", () => {
  it("offers no Undo after renaming a crate to the name it has, since Undo would take back something else", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:rename.button"));
    await confirmRename();

    await waitFor(() => expect(screen.queryByLabelText(tx("crates:rename.label"))).toBeNull());
    expect(calls.filter((c) => c.cmd === "rename_crate")).toHaveLength(1);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: tx("crates:undo") })).toBeNull();
  });

  it("offers no Undo after removing a track that was gone already", async () => {
    const calls = backend([crateWith(1, "Warm up", [track(1), track(2)])], { gone: [1] });
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(
      await screen.findByRole("button", {
        name: tx("crates:removeTrack.buttonFor", { title: "Synthetic Tune 1" }),
      }),
    );

    await waitFor(() =>
      expect(calls.filter((c) => c.cmd === "remove_tracks_from_crate")).toHaveLength(1),
    );
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("button", { name: tx("crates:undo") })).toBeNull();
  });

  it("leaves an earlier change's strip as it was", async () => {
    backend([crateWith(1, "Warm up", [track(1)])]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:rename.button"));
    const input = await screen.findByLabelText(tx("crates:rename.label"));
    await userEvent.clear(input);
    await userEvent.type(input, "Warm up 2");
    await confirmRename();
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.renamed"));

    // Renaming to the name it has records nothing.
    await userEvent.click(await button("crates:rename.button"));
    await confirmRename();
    await waitFor(() => expect(screen.queryByLabelText(tx("crates:rename.label"))).toBeNull());
    expect(screen.getByRole("status")).toHaveTextContent(tx("crates:done.renamed"));
  });
});

describe("errors", () => {
  it("clears a refused name when the user goes back", async () => {
    backend([crateWith(1, "Warm up", [])]);
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "warm up");
    await userEvent.click(await button("crates:new.confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("crates:error.nameTaken", { name: "Warm up" }),
    );

    await userEvent.click(await button("crates:new.cancel"));
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("shows the newest error alone, not an older one that is still on screen", async () => {
    backend([crateWith(1, "Warm up", []), crateWith(2, "Peak time", [])], {
      failing: ["delete_crate"],
    });
    renderScreen();
    // A refused name in the New form...
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "PEAK TIME");
    await userEvent.click(await button("crates:new.confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("crates:error.nameTaken", { name: "Peak time" }),
    );
    // ...is gone once the user does something else, which also fails.
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    expect(screen.queryByRole("alert")).toBeNull();
    await userEvent.click(await button("crates:delete.button"));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: tx("crates:delete.confirm"),
      }),
    );
    const alerts = await screen.findAllByRole("alert");
    expect(alerts.map((alert) => alert.textContent)).toEqual([tx("crates:error.notFound")]);

    // And a new attempt clears that one too.
    await userEvent.click(await button("crates:rename.button"));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

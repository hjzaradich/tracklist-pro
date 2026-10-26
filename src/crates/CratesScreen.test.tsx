import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import { useState } from "react";
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
  // The operation the last change recorded, as the commands answer it.
  let lastOperation = 9;
  const calls: { cmd: string; args: Record<string, unknown> }[] = [];
  let nextId = 100;
  const refuse = (kind: string, params: Record<string, string> = {}) => {
    throw { kind, params };
  };
  mockIPC((cmd, rawArgs) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    if (options.failing?.includes(cmd)) return refuse("crateNotFound");
    const named = (name: string, parent: number | null = null) =>
      crates.find(
        (c) =>
          c.crate.parentId === parent &&
          c.crate.name.toLowerCase() === name.trim().toLowerCase(),
      );
    // The ids of everything somewhere below a folder.
    const below = (id: number): number[] => {
      const kids = crates.filter((c) => c.crate.parentId === id).map((c) => c.crate.id);
      return [...kids, ...kids.flatMap(below)];
    };
    switch (cmd) {
      case "list_crates":
        return crates.map((c) => ({ ...c.crate, trackCount: c.tracks.length }));
      case "crate_tracks":
        return crates.find((c) => c.crate.id === args.id)?.tracks ?? refuse("crateNotFound");
      case "create_crate":
      case "create_crate_folder": {
        const name = String(args.name).trim();
        const parent = (args.parent ?? null) as number | null;
        if (name === "") return refuse("crateNameEmpty");
        const existing = named(name, parent);
        if (existing) return refuse("crateNameTaken", { name: existing.crate.name });
        before = crates;
        lastOperation = 9;
        const id = nextId++;
        const kind = cmd === "create_crate" ? "static" : "folder";
        crates = [...crates, { crate: { id, parentId: parent, kind, name, trackCount: 0 }, tracks: [] }];
        return id;
      }
      case "move_crate": {
        const node = crates.find((c) => c.crate.id === args.id);
        const folder = (args.folder ?? null) as number | null;
        if (node === undefined) return refuse("crateNotFound");
        if (folder !== null && (folder === node.crate.id || below(node.crate.id).includes(folder))) {
          return refuse("crateMoveIntoItself");
        }
        if (node.crate.parentId === folder) return null;
        const clash = named(node.crate.name, folder);
        if (clash) return refuse("crateNameTaken", { name: clash.crate.name });
        before = crates;
        lastOperation = 6;
        crates = crates.map((c) =>
          c === node ? { ...c, crate: { ...c.crate, parentId: folder } } : c,
        );
        return 6;
      }
      case "crate_folder_contents": {
        const ids = below(args.id as number);
        const inside = crates.filter((c) => ids.includes(c.crate.id));
        const distinct = new Set(inside.flatMap((c) => c.tracks.map((t) => t.id)));
        return {
          crates: inside.filter((c) => c.crate.kind === "static").length,
          folders: inside.filter((c) => c.crate.kind === "folder").length,
          tracks: distinct.size,
        };
      }
      case "rename_crate": {
        const target = crates.find((c) => c.crate.id === args.id);
        const name = String(args.name).trim();
        // The name it has: nothing is recorded.
        if (target?.crate.name === name) return null;
        const existing = named(name, target?.crate.parentId ?? null);
        if (existing && existing !== target) {
          return refuse("crateNameTaken", { name: existing.crate.name });
        }
        before = crates;
        crates = crates.map((c) =>
          c.crate.id === args.id ? { ...c, crate: { ...c.crate, name } } : c,
        );
        lastOperation = 7;
        return 7;
      }
      case "delete_crate": {
        before = crates;
        lastOperation = 9;
        const gone = [args.id as number, ...below(args.id as number)];
        crates = crates.filter((c) => !gone.includes(c.crate.id));
        return null;
      }
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
        lastOperation = 8;
        return { changed: wanted.length, skipped: 0, operationId: 8 };
      }
      case "next_undo_operation":
        return {
          operation: before === null ? null : { id: lastOperation, kind: "x", details: { name: null, from: null, tracks: null } },
          refusal: null,
        };
      case "undo_last_operation":
        if (before === null) return { status: "nothingToUndo" };
        crates = before;
        before = null;
        return { status: "undone", operation: { id: 1, kind: "x", details: { name: null, from: null, tracks: null } } };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return calls;
}

function crateWith(id: number, name: string, tracks: LibraryTrack[], parentId: number | null = null) {
  return { crate: { id, parentId, kind: "static" as const, name, trackCount: tracks.length }, tracks };
}

function folderWith(id: number, name: string, parentId: number | null = null) {
  return { crate: { id, parentId, kind: "folder" as const, name, trackCount: 0 }, tracks: [] };
}

/** The screen on a page that remembers the choice, as the route does. */
function Page({ chosen = null }: { chosen?: number | null }) {
  const [id, setId] = useState<number | null>(chosen);
  return <CratesScreen chosen={id} onChoose={setId} />;
}

function renderScreen(chosen: number | null = null) {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <Page chosen={chosen} />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  clearMocks();
});

const button = (key: string) => screen.findByRole("button", { name: tx(key) });

/** The Undo offered after a change, once it knows which change it is for. */
async function readyUndo() {
  const undo = await button("crates:undo");
  await waitFor(() => expect(undo).toBeEnabled());
  return undo;
}

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
    await userEvent.click(await readyUndo());
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
      { cmd: "create_crate", args: { name: "Warm up", parent: null } },
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
    await userEvent.click(await readyUndo());
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

    await userEvent.click(await readyUndo());
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
    const undo = await readyUndo();
    // The backend refuses the undo.
    mockIPC((cmd) => {
      if (cmd === "undo_last_operation") {
        return {
          status: "refused",
          operation: { id: 1, kind: "create_crate", details: { name: null, from: null, tracks: null } },
          reason: { code: "changedSince" },
          conflicts: [],
        };
      }
      if (cmd === "list_crates") {
        return [{ id: 100, parentId: null, kind: "static", name: "Warm up", trackCount: 0 }];
      }
      if (cmd === "crate_tracks") return [];
      throw new Error(`unexpected command ${cmd}`);
    });
    await userEvent.click(undo);
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

describe("the Undo offered after creating or deleting a crate, before it knows which change it is for", () => {
  /**
   * A backend with one crate, whose answer to "what is the next step to
   * undo" is `next`. Records the operation each undo was asked for.
   */
  function askingBackend(next: () => unknown) {
    let crates = [{ id: 1, parentId: null, kind: "static", name: "Warm up", trackCount: 0 }];
    const undone: unknown[] = [];
    mockIPC((cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      if (cmd === "list_crates") return crates;
      if (cmd === "crate_tracks") return [];
      if (cmd === "create_crate") {
        crates = [
          ...crates,
          { id: 2, parentId: null, kind: "static", name: String(a.name), trackCount: 0 },
        ];
        return 2;
      }
      if (cmd === "delete_crate") {
        crates = crates.filter((c) => c.id !== a.id);
        return null;
      }
      if (cmd === "next_undo_operation") return next();
      if (cmd === "undo_last_operation") {
        undone.push(a.operationId);
        return { status: "nothingToUndo" };
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    return undone;
  }

  async function createCrate() {
    renderScreen();
    await userEvent.click(await button("crates:new.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:new.label")), "Peak time");
    await userEvent.click(await button("crates:new.confirm"));
  }

  async function deleteCrate() {
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Warm up/ }));
    await userEvent.click(await button("crates:delete.button"));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: tx("crates:delete.confirm"),
      }),
    );
  }

  it.each([
    ["creating", createCrate, "create_crate"],
    ["deleting", deleteCrate, "delete_crate"],
  ] as const)(
    "after %s a crate it is greyed out until the answer is in, then undoes exactly that change",
    async (_what, change, kind) => {
      let answer: (value: unknown) => void = () => {};
      const undone = askingBackend(() => new Promise((resolve) => (answer = resolve)));
      await change();
      const undo = await button("crates:undo");
      expect(undo).toBeDisabled();
      // A press now does nothing: it can't take back whatever is newest.
      await userEvent.click(undo);
      expect(undone).toEqual([]);

      answer({
        operation: { id: 42, kind, details: { name: null, from: null, tracks: null } },
        refusal: null,
      });
      await waitFor(() => expect(undo).toBeEnabled());
      await userEvent.click(undo);
      await waitFor(() => expect(undone).toEqual([42]));
    },
  );

  it.each([
    ["creating", createCrate],
    ["deleting", deleteCrate],
  ] as const)("after %s a crate it stays greyed out when the answer can't be had", async (_what, change) => {
    let asked = 0;
    const undone = askingBackend(() => {
      asked += 1;
      throw { kind: "database", params: {} };
    });
    await change();
    const undo = await button("crates:undo");
    await waitFor(() => expect(asked).toBeGreaterThan(0));
    await act(async () => {});
    expect(undo).toBeDisabled();
    await userEvent.click(undo);
    expect(undone).toEqual([]);
  });
});

/** A Friday residency folder with a crate, a folder inside it and a crate in that. */
function residency() {
  return [
    folderWith(1, "Friday residency"),
    crateWith(2, "Warmup", [track(1), track(2)], 1),
    folderWith(3, "Late", 1),
    crateWith(4, "Peak time", [track(2), track(3)], 3),
    crateWith(5, "Loose", [track(1)]),
  ];
}

describe("folders in the tree", () => {
  it("lists a folder, then what is in it one level in, and counts only crates' tracks", async () => {
    backend(residency());
    renderScreen();
    const list = await screen.findByRole("list", { name: tx("crates:list.label") });
    const rows = within(list)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(rows).toEqual([
      "Friday residency",
      `Warmup${tx("crates:trackCount", { count: 2 })}`,
      "Late",
      `Peak time${tx("crates:trackCount", { count: 2 })}`,
      `Loose${tx("crates:trackCount", { count: 1 })}`,
    ]);
    // What is in a folder is nested inside its own row.
    const folder = within(list).getByRole("button", { name: "Friday residency" });
    expect(within(folder.closest("li") as HTMLElement).getAllByRole("listitem")).toHaveLength(3);
  });

  it("shows the rekordbox path of the chosen crate or folder", async () => {
    backend(residency());
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: /Peak time/ }));
    expect(await screen.findByLabelText(tx("crates:path.label"))).toHaveTextContent(
      `${tx("crates:path.root")} › Friday residency › Late › Peak time`,
    );
    await userEvent.click(screen.getByRole("button", { name: "Friday residency" }));
    expect(await screen.findByLabelText(tx("crates:path.label"))).toHaveTextContent(
      `${tx("crates:path.root")} › Friday residency`,
    );
    await userEvent.click(screen.getByRole("button", { name: /Loose/ }));
    expect(await screen.findByLabelText(tx("crates:path.label"))).toHaveTextContent(
      `${tx("crates:path.root")} › Loose`,
    );
  });

  it("shows a folder without tracks, saying what to do or that it is empty", async () => {
    backend([...residency(), folderWith(6, "Nothing yet")]);
    renderScreen();
    await userEvent.click(await screen.findByRole("button", { name: "Friday residency" }));
    expect(await screen.findByText(tx("crates:folder.select"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Nothing yet" }));
    expect(await screen.findByText(tx("crates:folder.empty"))).toBeInTheDocument();
  });

  it("shows nothing chosen, and no error, when the chosen id names nothing", async () => {
    const calls = backend(residency());
    renderScreen(999);
    expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    // Nothing was asked of the backend about the missing crate.
    expect(calls.map((c) => c.cmd)).not.toContain("crate_tracks");
    expect(calls.map((c) => c.cmd)).not.toContain("crate_folder_contents");
  });

  it("falls back to nothing chosen when the chosen crate is deleted", async () => {
    backend(residency());
    renderScreen(5);
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Loose");
    await userEvent.click(await button("crates:delete.button"));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: tx("crates:delete.confirm"),
      }),
    );
    expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("making a folder", () => {
  it("makes a folder at the top and shows it chosen, with Undo", async () => {
    const calls = backend([crateWith(1, "Loose", [])]);
    renderScreen();
    await userEvent.click(await button("crates:newFolder.button"));
    await userEvent.type(await screen.findByLabelText(tx("crates:newFolder.label")), "Sets");
    await userEvent.click(await button("crates:newFolder.confirm"));
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Sets");
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.createdFolder"));
    expect(calls.filter((c) => c.cmd === "create_crate_folder")).toEqual([
      { cmd: "create_crate_folder", args: { name: "Sets", parent: null } },
    ]);
    await userEvent.click(await readyUndo());
    await waitFor(() => expect(screen.queryByRole("button", { name: "Sets" })).toBeNull());
  });

  it("makes a crate and a folder inside the chosen folder", async () => {
    const calls = backend(residency());
    renderScreen(1);
    await userEvent.click(await button("crates:newInside.button"));
    await userEvent.type(
      await screen.findByLabelText(tx("crates:newInside.label", { name: "Friday residency" })),
      "Closing",
    );
    await userEvent.click(await button("crates:new.confirm"));
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Closing");

    await userEvent.click(screen.getByRole("button", { name: "Friday residency" }));
    await userEvent.click(await button("crates:newFolderInside.button"));
    await userEvent.type(
      await screen.findByLabelText(tx("crates:newFolderInside.label", { name: "Friday residency" })),
      "After hours",
    );
    await userEvent.click(await button("crates:newFolder.confirm"));
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("After hours");
    expect(calls.filter((c) => c.cmd.startsWith("create_"))).toEqual([
      { cmd: "create_crate", args: { name: "Closing", parent: 1 } },
      { cmd: "create_crate_folder", args: { name: "After hours", parent: 1 } },
    ]);
  });

  it("says why a name is refused: the same name in the same folder", async () => {
    backend(residency());
    renderScreen(1);
    await userEvent.click(await button("crates:newInside.button"));
    await userEvent.type(
      await screen.findByLabelText(tx("crates:newInside.label", { name: "Friday residency" })),
      "WARMUP",
    );
    await userEvent.click(await button("crates:new.confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("crates:error.nameTaken", { name: "Warmup" }),
    );
  });
});

describe("deleting a folder", () => {
  async function askToDelete(name: string) {
    await userEvent.click(await screen.findByRole("button", { name }));
    await userEvent.click(await button("crates:delete.button"));
    return screen.findByRole("alertdialog");
  }

  it("says how many crates, folders and different tracks go, and keeps the tracks", async () => {
    const calls = backend(residency());
    renderScreen();
    const dialog = await askToDelete("Friday residency");
    expect(dialog).toHaveTextContent(tx("crates:delete.folderTitle", { name: "Friday residency" }));
    // Two crates, one folder, and three different tracks (one is in two crates).
    await waitFor(() => expect(dialog).toHaveTextContent(tx("crates:delete.crates", { count: 2 })));
    expect(dialog).toHaveTextContent(tx("crates:delete.folders", { count: 1 }));
    expect(dialog).toHaveTextContent(tx("crates:delete.tracks", { count: 3 }));
    expect(dialog).toHaveTextContent(tx("crates:delete.detail"));
    expect(calls.map((c) => c.cmd)).not.toContain("delete_crate");

    // Go back deletes nothing.
    await userEvent.click(within(dialog).getByRole("button", { name: tx("crates:delete.cancel") }));
    expect(screen.queryByRole("alertdialog")).toBeNull();
    expect(calls.map((c) => c.cmd)).not.toContain("delete_crate");
  });

  it("says an empty folder is empty", async () => {
    backend([folderWith(1, "Nothing yet")]);
    renderScreen();
    const dialog = await askToDelete("Nothing yet");
    expect(await within(dialog).findByText(tx("crates:delete.holdsNothing"))).toBeInTheDocument();
    expect(dialog).not.toHaveTextContent(tx("crates:delete.holds"));
  });

  it("deletes the folder with what is in it as one step, and undoes it in one step", async () => {
    const calls = backend(residency());
    renderScreen();
    const dialog = await askToDelete("Friday residency");
    const confirm = within(dialog).getByRole("button", { name: tx("crates:delete.confirm") });
    await waitFor(() => expect(confirm).toBeEnabled());
    await userEvent.click(confirm);

    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.deletedFolder"));
    await waitFor(() => expect(screen.queryByRole("button", { name: /Warmup/ })).toBeNull());
    expect(screen.queryByRole("button", { name: /Peak time/ })).toBeNull();
    // The crate outside the folder stays.
    expect(screen.getByRole("button", { name: /Loose/ })).toBeInTheDocument();
    expect(calls.filter((c) => c.cmd === "delete_crate")).toEqual([
      { cmd: "delete_crate", args: { id: 1 } },
    ]);

    await userEvent.click(await readyUndo());
    expect(await screen.findByRole("button", { name: /Peak time/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Warmup/ })).toBeInTheDocument();
  });
});

describe("moving to a folder", () => {
  async function openMove(name: RegExp, label: string) {
    await userEvent.click(await screen.findByRole("button", { name }));
    await userEvent.click(await button("crates:move.button"));
    return screen.findByLabelText(tx("crates:move.label", { name: label }));
  }

  it("offers the top level and every folder, but not the folder itself or one inside it", async () => {
    backend([...residency(), folderWith(6, "Archive")]);
    renderScreen();
    const choice = await openMove(/^Friday residency$/, "Friday residency");
    expect(within(choice).getAllByRole("option").map((o) => o.textContent)).toEqual([
      tx("crates:move.top"),
      "Archive",
    ]);

    await userEvent.click(await button("crates:move.cancel"));
    const forCrate = await openMove(/Peak time/, "Peak time");
    expect(within(forCrate).getAllByRole("option").map((o) => o.textContent)).toEqual([
      tx("crates:move.top"),
      "Friday residency",
      "Friday residency › Late",
      "Archive",
    ]);
    // It starts on the folder the crate is in.
    expect(forCrate).toHaveValue("3");
  });

  it("moves a crate into a folder, and Undo puts it back", async () => {
    const calls = backend([...residency(), folderWith(6, "Archive")]);
    renderScreen();
    const choice = await openMove(/Loose/, "Loose");
    await userEvent.selectOptions(choice, "Archive");
    await userEvent.click(await button("crates:move.confirm"));

    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.moved"));
    expect(calls.filter((c) => c.cmd === "move_crate")).toEqual([
      { cmd: "move_crate", args: { id: 5, folder: 6 } },
    ]);
    await waitFor(() =>
      expect(screen.getByLabelText(tx("crates:path.label"))).toHaveTextContent(
        `${tx("crates:path.root")} › Archive › Loose`,
      ),
    );

    await userEvent.click(await readyUndo());
    await waitFor(() =>
      expect(screen.getByLabelText(tx("crates:path.label"))).toHaveTextContent(
        `${tx("crates:path.root")} › Loose`,
      ),
    );
  });

  it("moves a folder to the top level", async () => {
    const calls = backend(residency());
    renderScreen();
    const choice = await openMove(/^Late$/, "Late");
    await userEvent.selectOptions(choice, tx("crates:move.top"));
    await userEvent.click(await button("crates:move.confirm"));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("crates:done.moved"));
    expect(calls.filter((c) => c.cmd === "move_crate")).toEqual([
      { cmd: "move_crate", args: { id: 3, folder: null } },
    ]);
  });

  it("offers no Undo when the crate is where it was put already", async () => {
    const calls = backend(residency());
    renderScreen();
    await openMove(/Loose/, "Loose");
    await userEvent.click(await button("crates:move.confirm"));
    await waitFor(() => expect(calls.filter((c) => c.cmd === "move_crate")).toHaveLength(1));
    await waitFor(() =>
      expect(screen.queryByLabelText(tx("crates:move.label", { name: "Loose" }))).toBeNull(),
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows why a move was refused and keeps the form open", async () => {
    backend([...residency(), folderWith(6, "Archive"), crateWith(7, "loose", [], 6)]);
    renderScreen();
    const choice = await openMove(/^Loose/, "Loose");
    await userEvent.selectOptions(choice, "Archive");
    await userEvent.click(await button("crates:move.confirm"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("crates:error.nameTaken", { name: "loose" }),
    );
    expect(screen.getByLabelText(tx("crates:move.label", { name: "Loose" }))).toBeInTheDocument();
  });
});

import { QueryClientProvider, useMutation } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LibraryTrack, OperationDetails, OperationInfo, UndoRefusal } from "../bindings";
import "../i18n";
import { LibraryScreen } from "../library/LibraryScreen";
import { tx } from "../test/tx";
import { UndoButton } from "./UndoButton";

const NO_DETAILS: OperationDetails = { name: null, from: null, tracks: null };

function operation(id: number, kind: string, details: Partial<OperationDetails> = {}): OperationInfo {
  return { id, kind, details: { ...NO_DETAILS, ...details } };
}

/** One step of the history, as the Rust side would answer about it. */
type Step = {
  operation: OperationInfo;
  /** Refused, and known ahead of time. */
  refusal?: UndoRefusal;
  /** Refused, but only found when Undo is used (a very large operation). */
  refusedOnUse?: UndoRefusal;
  /** What undoing it does to the rest of the fake backend. */
  onUndone?: () => void;
};

/**
 * Stands in for the Rust side's history: `steps` oldest first, so the last
 * one is the next to undo. Undo takes back only the newest, only if it's the
 * one asked for, and never skips a refused one. `more` answers other commands.
 */
function history(steps: Step[], more: (cmd: string, args: Record<string, unknown>) => unknown = () => undefined) {
  const undone: number[] = [];
  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      const top = steps.at(-1);
      if (cmd === "next_undo_operation") {
        return { operation: top?.operation ?? null, refusal: top?.refusal ?? null };
      }
      if (cmd === "undo_last_operation") {
        const only = args.operationId as number | null;
        if (top === undefined || (only !== null && only !== top.operation.id)) {
          return { status: "nothingToUndo" };
        }
        const reason = top.refusal ?? top.refusedOnUse;
        if (reason !== undefined) {
          return { status: "refused", operation: top.operation, reason, conflicts: [] };
        }
        steps.pop();
        undone.push(top.operation.id);
        top.onUndone?.();
        return { status: "undone", operation: top.operation };
      }
      const answer = more(cmd, args);
      if (answer === undefined) throw new Error(`unexpected command ${cmd}`);
      return answer;
    },
    { shouldMockEvents: true },
  );
  return { steps, undone };
}

/** A button that does some other action, as any screen's would. */
function OtherAction({ run }: { run: () => void }) {
  const action = useMutation({
    mutationFn: () => {
      run();
      return Promise.resolve(null);
    },
  });
  return (
    <button type="button" onClick={() => action.mutate()}>
      other action
    </button>
  );
}

function renderBar(extra: React.ReactNode = null) {
  const client = createQueryClient();
  const view = render(
    <QueryClientProvider client={client}>
      <UndoButton />
      <input aria-label="a text field" />
      <textarea aria-label="a longer text field" />
      <div contentEditable suppressContentEditableWarning aria-label="an editable text" role="textbox">
        <span data-testid="inside-editable">text</span>
      </div>
      {extra}
    </QueryClientProvider>,
  );
  return { ...view, client };
}

/** The top bar's Undo, once it reads `name`. */
const undoButton = (name: string) => screen.findByRole("button", { name });

const CRATE = operation(1, "create_crate", { name: "House" });
const ADD = operation(2, "promote");
const TO_CRATE = operation(3, "add_to_crate", { name: "House", tracks: 3 });
const LABEL = {
  crate: () => tx("shell:undo.action.createCrate", { name: "House" }),
  add: () => tx("shell:undo.action.addToLibrary"),
  toCrate: () => tx("shell:undo.action.addToCrate", { count: 3, name: "House" }),
};

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the top bar's Undo", () => {
  it("is greyed out, and says nothing is left, when there is nothing to undo", async () => {
    history([]);
    renderBar();
    const button = await undoButton(tx("shell:undo.button"));
    await waitFor(() => expect(button).toHaveAttribute("title", tx("shell:undo.nothing")));
    expect(button).toBeDisabled();
    expect(screen.getByText(tx("shell:undo.shortcut"))).toBeInTheDocument();
  });

  it.each([
    [operation(1, "promote"), "shell:undo.action.addToLibrary", {}],
    [operation(1, "remove_from_library"), "shell:undo.action.removeFromLibrary", {}],
    [operation(1, "add_rekordbox_tracks", { tracks: 1 }), "shell:undo.action.addFromRekordbox", { count: 1 }],
    [operation(1, "add_rekordbox_tracks", { tracks: 240 }), "shell:undo.action.addFromRekordbox", { count: 240 }],
    [operation(1, "create_crate", { name: "House" }), "shell:undo.action.createCrate", { name: "House" }],
    [
      operation(1, "rename_crate", { name: "Techno", from: "House" }),
      "shell:undo.action.renameCrate",
      { from: "House", name: "Techno" },
    ],
    [operation(1, "delete_crate", { name: "House" }), "shell:undo.action.deleteCrate", { name: "House" }],
    [
      operation(1, "add_to_crate", { name: "House", tracks: 1 }),
      "shell:undo.action.addToCrate",
      { count: 1, name: "House" },
    ],
    [
      operation(1, "add_to_crate", { name: "House", tracks: 3 }),
      "shell:undo.action.addToCrate",
      { count: 3, name: "House" },
    ],
    [
      operation(1, "remove_from_crate", { name: "House", tracks: 2 }),
      "shell:undo.action.removeFromCrate",
      { count: 2, name: "House" },
    ],
  ] as const)("names the action it will undo: %o", async (next, key, params) => {
    history([{ operation: next }]);
    renderBar();
    expect(await undoButton(tx(key, params))).toBeEnabled();
  });

  it.each([
    operation(1, "something_newer"),
    // Recorded by an older version, without the details its wording needs.
    operation(1, "delete_crate"),
    operation(1, "add_to_crate", { name: "House" }),
    operation(1, "rename_crate", { name: "Techno" }),
    operation(1, "add_rekordbox_tracks"),
  ])("names an action it has no wording for plainly, and still undoes it: %o", async (next) => {
    const backend = history([{ operation: next }]);
    renderBar();
    await userEvent.click(await undoButton(tx("shell:undo.action.other")));
    await waitFor(() => expect(backend.undone).toEqual([1]));
  });

  it("walks back one action per press, newest first, naming the next one each time", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }, { operation: TO_CRATE }]);
    renderBar();
    await userEvent.click(await undoButton(LABEL.toCrate()));
    await userEvent.click(await undoButton(LABEL.add()));
    await userEvent.click(await undoButton(LABEL.crate()));
    expect(await undoButton(tx("shell:undo.button"))).toBeDisabled();
    expect(backend.undone).toEqual([3, 2, 1]);
  });

  it("is greyed out with the reason beside it when the next step is known to be refused", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD, refusal: { code: "sentSince" } }]);
    renderBar();
    const button = await undoButton(LABEL.add());
    expect(button).toBeDisabled();
    expect(screen.getByRole("status")).toHaveTextContent(tx("shell:undo.refused.sentSince"));
    // Nothing is undone, and the older step isn't offered instead.
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    await userEvent.click(button);
    expect(backend.undone).toEqual([]);
    expect(screen.queryByRole("button", { name: LABEL.crate() })).toBeNull();
  });

  it.each([
    [{ code: "sentSince" }, "shell:undo.refused.sentSince", {}],
    [{ code: "sourceGone" }, "shell:undo.refused.sourceGone", {}],
    [{ code: "changedSince" }, "shell:undo.refused.changedSince", {}],
    [{ code: "crateNameTaken", name: "House" }, "shell:undo.refused.crateNameTaken", { name: "House" }],
  ] as const)("gives each refusal its own reason: %o", async (refusal, key, params) => {
    history([{ operation: ADD, refusal }]);
    renderBar();
    expect(await screen.findByRole("status")).toHaveTextContent(tx(key, params));
  });

  it("greys out and gives the reason once a press finds the step is refused", async () => {
    const backend = history([
      { operation: CRATE },
      { operation: ADD, refusedOnUse: { code: "changedSince" } },
    ]);
    renderBar();
    const button = await undoButton(LABEL.add());
    expect(button).toBeEnabled();
    await userEvent.click(button);
    expect(await screen.findByRole("status")).toHaveTextContent(tx("shell:undo.refused.changedSince"));
    expect(button).toBeDisabled();
    expect(button).toHaveAccessibleName(LABEL.add());
    expect(backend.undone).toEqual([]);
  });

  it("shows the error, not a refusal, when the undo itself fails", async () => {
    mockIPC(
      (cmd) => {
        if (cmd === "next_undo_operation") return { operation: ADD, refusal: null };
        if (cmd === "undo_last_operation") throw { kind: "database", params: {} };
        throw new Error(`unexpected command ${cmd}`);
      },
      { shouldMockEvents: true },
    );
    renderBar();
    await userEvent.click(await undoButton(LABEL.add()));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:database"));
  });
});

describe("Ctrl+Z", () => {
  it("undoes the next action, like the button", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }]);
    renderBar();
    await undoButton(LABEL.add());
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    expect(backend.undone).toEqual([2]);
  });

  it("in a text field is left to the field, and undoes no action", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }]);
    renderBar();
    await undoButton(LABEL.add());
    const field = screen.getByRole("textbox", { name: "a text field" });
    field.focus();
    // Not cancelled: the field's own undo of typing still runs.
    expect(fireEvent.keyDown(field, { key: "z", ctrlKey: true })).toBe(true);
    // The next Ctrl+Z outside the field takes back the newest action, which
    // shows the one in the field took back none.
    expect(fireEvent.keyDown(document.body, { key: "z", ctrlKey: true })).toBe(false);
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    expect(backend.undone).toEqual([2]);
  });

  it("is left alone in every kind of text field", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }]);
    renderBar();
    await undoButton(LABEL.add());
    for (const field of [
      screen.getByRole("textbox", { name: "a longer text field" }),
      screen.getByRole("textbox", { name: "an editable text" }),
      screen.getByTestId("inside-editable"),
    ]) {
      expect(fireEvent.keyDown(field, { key: "z", ctrlKey: true })).toBe(true);
    }
    // A checkbox or a button is no text field: there it undoes.
    fireEvent.keyDown(screen.getByRole("button", { name: LABEL.add() }), { key: "z", ctrlKey: true });
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    expect(backend.undone).toEqual([2]);
  });

  it("does nothing while a dialog is waiting for an answer", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }]);
    const view = renderBar(
      <div role="alertdialog" aria-label="a question">
        <button type="button">go back</button>
      </div>,
    );
    await undoButton(LABEL.add());
    const inDialog = screen.getByRole("button", { name: "go back" });
    inDialog.focus();
    expect(fireEvent.keyDown(inDialog, { key: "z", ctrlKey: true })).toBe(true);

    // Once the dialog has gone, Ctrl+Z takes back the newest action: the
    // press in the dialog took back none.
    view.rerender(
      <QueryClientProvider client={view.client}>
        <UndoButton />
      </QueryClientProvider>,
    );
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    expect(backend.undone).toEqual([2]);
  });

  it("counts once while the keys are held down, and ignores other key combinations", async () => {
    const backend = history([{ operation: CRATE }, { operation: ADD }]);
    renderBar();
    await undoButton(LABEL.add());
    fireEvent.keyDown(document.body, { key: "z" });
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, shiftKey: true });
    fireEvent.keyDown(document.body, { key: "y", ctrlKey: true });
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, repeat: true });
    fireEvent.keyDown(document.body, { key: "z", ctrlKey: true, repeat: true });
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    expect(backend.undone).toEqual([2]);
  });
});

describe("keeping the top bar's Undo current", () => {
  it("names the new action after any action is done", async () => {
    const backend = history([{ operation: CRATE }]);
    renderBar(<OtherAction run={() => backend.steps.push({ operation: ADD })} />);
    await undoButton(LABEL.crate());
    await userEvent.click(screen.getByRole("button", { name: "other action" }));
    expect(await undoButton(LABEL.add())).toBeEnabled();
  });

  it("asks again when a background task ends, since a send can make the next step refused", async () => {
    const backend = history([{ operation: ADD }]);
    renderBar();
    expect(await undoButton(LABEL.add())).toBeEnabled();
    backend.steps[0].refusal = { code: "sentSince" };
    await act(() =>
      emit("job-updates", [{ seq: 1, id: 1, kind: "export", status: "done", progress: null, priority: 0 }]),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(tx("shell:undo.refused.sentSince"));
    expect(screen.getByRole("button", { name: LABEL.add() })).toBeDisabled();
  });
});

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
 * The Library screen under the top bar's Undo, on a fake Rust side whose
 * removals are steps of the history.
 */
function libraryUnderTheBar(initial: LibraryTrack[], earlier: Step[] = []) {
  let tracks = initial;
  let nextId = 10;
  const backend = history(earlier, (cmd, args) => {
    if (cmd === "list_crates") return [];
    if (cmd === "library_tracks") return tracks;
    if (cmd === "remove_library_track") {
      const removed = tracks.find((t) => t.id === args.id);
      if (removed === undefined) throw { kind: "libraryTrackNotFound", params: {} };
      tracks = tracks.filter((t) => t !== removed);
      backend.steps.push({
        operation: operation(nextId++, "remove_from_library"),
        onUndone: () => {
          tracks = [...tracks, removed].sort((a, b) => a.id - b.id);
        },
      });
      return null;
    }
    return undefined;
  });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <UndoButton />
      <LibraryScreen />
    </QueryClientProvider>,
  );
  return backend;
}

async function rows() {
  const table = await screen.findByRole("table");
  return within(table).getAllByRole("row").slice(1);
}

async function removeFirstTrack() {
  const [first] = await rows();
  await userEvent.click(
    within(first).getByRole("button", { name: tx("library:remove.buttonFor", { title: "Synthetic Tune 1" }) }),
  );
  await userEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", { name: tx("library:remove.confirm") }),
  );
}

describe("the top bar's Undo and the Undo offered right after an action", () => {
  it("the top bar's Undo puts a removed track back in the list without a restart", async () => {
    const backend = libraryUnderTheBar([track(1), track(2)]);
    await removeFirstTrack();
    await waitFor(async () => expect(await rows()).toHaveLength(1));

    await userEvent.click(await undoButton(tx("shell:undo.action.removeFromLibrary")));
    await waitFor(async () => expect(await rows()).toHaveLength(2));
    expect(backend.undone).toEqual([10]);
  });

  it("once the top bar has undone the action, the Undo offered after it is gone", async () => {
    const backend = libraryUnderTheBar([track(1), track(2)], [{ operation: CRATE }]);
    await removeFirstTrack();
    const offered = await screen.findByRole("button", { name: tx("library:remove.undo") });
    expect(offered).toBeEnabled();

    await userEvent.click(await undoButton(tx("shell:undo.action.removeFromLibrary")));
    // The top bar moves on to the step before; the after-action Undo, which
    // was for the removal, is no longer offered for it.
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: tx("library:remove.undo") })).toBeNull(),
    );
    expect(backend.undone).toEqual([10]);
  });

  it("after the Undo offered after an action is used, the top bar names the step before", async () => {
    const backend = libraryUnderTheBar([track(1), track(2)], [{ operation: CRATE }]);
    await removeFirstTrack();
    await undoButton(tx("shell:undo.action.removeFromLibrary"));

    await userEvent.click(await screen.findByRole("button", { name: tx("library:remove.undo") }));
    expect(await screen.findByText(tx("library:remove.undone"))).toBeInTheDocument();
    expect(await undoButton(LABEL.crate())).toBeEnabled();
    await waitFor(async () => expect(await rows()).toHaveLength(2));
    // It undid the removal and nothing older.
    expect(backend.undone).toEqual([10]);
  });

  it("the Undo offered after an action never takes back a different, newer action", async () => {
    const backend = libraryUnderTheBar([track(1), track(2)]);
    await removeFirstTrack();
    const offered = await screen.findByRole("button", { name: tx("library:remove.undo") });
    // It is for operation 10. Something else is done before it's pressed,
    // and the press lands before the screen has heard.
    await waitFor(() => expect(backend.steps.at(-1)?.operation.id).toBe(10));
    await undoButton(tx("shell:undo.action.removeFromLibrary"));
    backend.steps.push({ operation: operation(11, "create_crate", { name: "Later" }) });
    await userEvent.click(offered);
    // Nothing is undone, and the offer goes away: it was for the removal.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: tx("library:remove.undo") })).toBeNull(),
    );
    expect(await undoButton(tx("shell:undo.action.createCrate", { name: "Later" }))).toBeEnabled();
    expect(backend.undone).toEqual([]);
  });
});

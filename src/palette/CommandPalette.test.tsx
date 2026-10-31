import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { type ReviewKey, useReviewKeys } from "../review/useReviewKeys";
import type { OperationInfo, TableQuery } from "../bindings";
import { pageOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { useThemeStore } from "../theme/themeStore";
import { type PaletteAction, useRegisterActions } from "./registry";

const OPERATION: OperationInfo = {
  id: 7,
  kind: "promote",
  details: { name: null, from: null, tracks: null, newToLibrary: null, newCrate: null },
};

/** Stands in for the Rust side, and remembers which commands were asked. */
function fakeBackend({
  library = 2,
  undoable = true,
  libraryBroken = false,
}: { library?: number; undoable?: boolean; libraryBroken?: boolean } = {}) {
  const backend = { calls: [] as string[], undone: [] as number[], undoable };
  mockIPC(
    (cmd, rawArgs) => {
      backend.calls.push(cmd);
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      if (cmd === "library_summary" && libraryBroken) throw { kind: "database", params: {} };
      if (cmd === "library_summary") return { tracks: library, changeStamp: library };
      if (cmd === "next_undo_operation") {
        return { operation: backend.undoable ? OPERATION : null, refusal: null };
      }
      if (cmd === "undo_last_operation") {
        backend.undone.push(args.operationId as number);
        backend.undoable = false;
        return { status: "undone", operation: OPERATION };
      }
      if (cmd === "send_state") {
        return {
          revision: 0,
          filePath: String.raw`C:\data\send.xml`,
          exportPath: null,
          preflight: null,
          step: null,
          workingOn: null,
          failure: null,
          sent: null,
        };
      }
      if (cmd === "track_table_page") {
        return pageOf([tableRow(1)], args.query as TableQuery);
      }
      if (cmd === "rekordbox_offer") {
        return { toAdd: 0, alreadyInLibrary: 0, waitingInMissing: 0, waitingForConfirmation: 0 };
      }
      if (cmd === "rekordbox_xml_source") {
        return { path: null, watch: false, lastRead: null, lastFailure: null, newerExport: null, exportFolder: null };
      }
      if (cmd === "list_crates" || cmd === "music_folders") return [];
      if (cmd === "read_online_only_files") return false;
      if (cmd === "key_notation") return "camelot";
      if (cmd === "table_columns") return null;
      // The shell's own commands (Activity) aren't this test's business.
      throw { kind: "internal", params: {} };
    },
    { shouldMockEvents: true },
  );
  return backend;
}

afterEach(() => {
  cleanup();
  clearMocks();
  useThemeStore.getState().setTheme("dark");
});

const goTo = (stage: string) => tx("palette:goTo", { name: tx(`shell:stages.${stage}`) });
const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Opens the palette with Ctrl+K and returns its text box. */
async function openPalette() {
  await userEvent.keyboard("{Control>}k{/Control}");
  return screen.findByRole("combobox", { name: tx("palette:input.label") });
}

const optionNames = () =>
  within(screen.getByRole("listbox", { name: tx("palette:list.label") }))
    .queryAllByRole("option")
    .map((option) => option.textContent);

describe("the command palette", () => {
  it("opens on Ctrl+K and closes on Escape, giving the focus back to where it was", async () => {
    fakeBackend();
    renderApp("/overview");
    const link = await screen.findByRole("link", { name: tx("shell:stages.review") });
    link.focus();

    const box = await openPalette();
    expect(screen.getByRole("dialog", { name: tx("palette:title") })).toBeInTheDocument();
    expect(box).toHaveFocus();

    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
    expect(link).toHaveFocus();
  });

  it("filters the list as you type, and says when nothing matches", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    expect(optionNames()).toContain(goTo("review"));
    expect(optionNames()).toContain(goTo("library"));

    await userEvent.keyboard(tx("shell:stages.review"));
    expect(optionNames()).toEqual([goTo("review")]);

    await userEvent.clear(screen.getByRole("combobox"));
    await userEvent.keyboard("zzzz");
    expect(optionNames()).toEqual([]);
    const dialog = screen.getByRole("dialog", { name: tx("palette:title") });
    expect(within(dialog).getByRole("status")).toHaveTextContent(tx("palette:empty"));
  });

  it("runs the highlighted action with Enter, through the screen's own route", async () => {
    fakeBackend();
    const { router } = renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    await userEvent.keyboard(tx("shell:stages.library"));
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(router.state.location.pathname).toBe("/library"));
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
  });

  it("lists Open Settings, always, and runs it by opening the Settings screen", async () => {
    fakeBackend();
    const { router } = renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    expect(optionNames()).toContain(tx("palette:openSettings"));
    await userEvent.keyboard(tx("palette:openSettings"));
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(router.state.location.pathname).toBe("/settings"));
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
  });

  it("runs Undo through the top bar's handler, and shows its shortcut", async () => {
    const backend = fakeBackend();
    renderApp("/overview");
    const topBarUndo = await screen.findByRole("button", { name: tx("shell:undo.action.addToLibrary") });
    await waitFor(() => expect(topBarUndo).toBeEnabled());
    topBarUndo.focus();

    await openPalette();
    const option = screen.getByRole("option", {
      name: new RegExp(escapeRegExp(tx("shell:undo.action.addToLibrary"))),
    });
    expect(within(option).getByText(tx("shell:undo.shortcut"))).toBeInTheDocument();
    await userEvent.click(option);

    await waitFor(() => expect(backend.undone).toEqual([OPERATION.id]));
    expect(topBarUndo).toHaveFocus();
  });

  it("moves the highlight with the arrow keys and tells the box which row it is", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const box = await openPalette();

    const selected = () =>
      screen.getAllByRole("option").filter((option) => option.getAttribute("aria-selected") === "true");
    const options = screen.getAllByRole("option");
    expect(selected()).toEqual([options[0]]);
    expect(box).toHaveAttribute("aria-activedescendant", options[0]?.id);

    await userEvent.keyboard("{ArrowDown}");
    expect(selected()).toEqual([options[1]]);
    expect(box).toHaveAttribute("aria-activedescendant", options[1]?.id);

    await userEvent.keyboard("{ArrowUp}{ArrowUp}");
    expect(selected()).toEqual([options.at(-1)]);
    expect(box).toHaveFocus();
  });

  it("lists only the actions that can run now", async () => {
    const backend = fakeBackend({ library: 0, undoable: false });
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await waitFor(() => expect(backend.calls).toContain("library_summary"));
    await waitFor(() => expect(backend.calls).toContain("next_undo_operation"));
    await openPalette();

    const names = optionNames();
    // An empty Library has nothing to send; with nothing done there is nothing to undo.
    expect(names).not.toContain(tx("send:open"));
    expect(names.some((name) => name?.includes(tx("shell:undo.button")))).toBe(false);
    // The theme in use isn't offered, the other is.
    expect(names).not.toContain(tx("palette:theme", { name: tx("theme:dark") }));
    expect(names).toContain(tx("palette:theme", { name: tx("theme:light") }));
  });

  it("opens from inside a text box and leaves what was typed there alone", async () => {
    fakeBackend();
    renderApp("/all-music");
    const search = await screen.findByRole("searchbox", { name: tx("allMusic:search") });
    await userEvent.click(search);
    await userEvent.keyboard("tune");
    expect(search).toHaveFocus();

    await openPalette();
    expect(screen.getByRole("dialog", { name: tx("palette:title") })).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    expect(search).toHaveFocus();
    expect(search).toHaveValue("tune");
  });

  it("puts the cursor in the All music search box from another screen", async () => {
    fakeBackend();
    const { router } = renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    await userEvent.keyboard(tx("palette:searchAllMusic"));
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(router.state.location.pathname).toBe("/all-music"));
    const search = await screen.findByRole("searchbox", { name: tx("allMusic:search") });
    await waitFor(() => expect(search).toHaveFocus());
  });

  it("blocks the other shortcuts while it is open", async () => {
    const backend = fakeBackend();
    const reviewKeys: ReviewKey[] = [];
    function ReviewKeysProbe() {
      useReviewKeys((key) => {
        reviewKeys.push(key);
        return true;
      });
      return null;
    }
    render(<ReviewKeysProbe />);
    renderApp("/overview");
    const topBarUndo = await screen.findByRole("button", { name: tx("shell:undo.action.addToLibrary") });
    await waitFor(() => expect(topBarUndo).toBeEnabled());
    await openPalette();

    // fireEvent returns false when a handler took the key (preventDefault),
    // and Undo takes it at once when it runs, so this is exact and needs no waiting.
    expect(fireEvent.keyDown(document.body, { key: "z", ctrlKey: true })).toBe(true);
    expect(fireEvent.keyDown(document.body, { key: "1" })).toBe(true);
    expect(reviewKeys).toEqual([]);

    await userEvent.keyboard("{Escape}");
    expect(fireEvent.keyDown(document.body, { key: "1" })).toBe(false);
    expect(reviewKeys).toEqual([1]);
    expect(fireEvent.keyDown(document.body, { key: "z", ctrlKey: true })).toBe(false);
    await waitFor(() => expect(backend.undone).toEqual([OPERATION.id]));
  });

  it("keeps the focus in the box when the empty message is clicked, so Escape still closes it", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const box = await openPalette();
    await userEvent.keyboard("zzzz");
    await userEvent.click(screen.getByText(tx("palette:empty")));
    expect(box).toHaveFocus();

    await userEvent.keyboard("{Escape}");
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
  });

  it("closes on a click outside, on a second Ctrl+K, and keeps the focus in the box on Tab", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const box = await openPalette();
    await userEvent.tab();
    expect(box).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(box).toHaveFocus();

    await userEvent.keyboard("{Control>}k{/Control}");
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();

    await openPalette();
    // The backdrop covers the window, so that is what a click outside lands on.
    await userEvent.click(screen.getByRole("dialog", { name: tx("palette:title") }).parentElement!);
    expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
  });

  it("jumps to the first and last row with Home and End", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    const options = screen.getAllByRole("option");
    const selected = () => screen.getAllByRole("option").filter((o) => o.getAttribute("aria-selected") === "true");

    await userEvent.keyboard("{End}");
    expect(selected()).toEqual([options.at(-1)]);
    await userEvent.keyboard("{Home}");
    expect(selected()).toEqual([options[0]]);
  });

  it("switches the theme through the theme store, as the theme switch does", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await openPalette();
    await userEvent.keyboard(tx("palette:theme", { name: tx("theme:light") }));
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(useThemeStore.getState().theme).toBe("light"));
  });

  it("does not open over a question waiting for an answer", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const question = document.createElement("div");
    question.setAttribute("role", "alertdialog");
    document.body.append(question);
    try {
      await userEvent.keyboard("{Control>}k{/Control}");
      expect(screen.queryByRole("dialog", { name: tx("palette:title") })).toBeNull();
    } finally {
      question.remove();
    }
  });

  it("doesn't offer Send to rekordbox while the Overview's Send button is greyed out", async () => {
    const backend = fakeBackend({ libraryBroken: true });
    renderApp("/overview");
    // The Library can't be read: the button is greyed out, and so is the palette's Send.
    expect(await screen.findByRole("button", { name: tx("send:open") })).toBeDisabled();
    await waitFor(() => expect(backend.calls).toContain("library_summary"));
    await openPalette();
    expect(optionNames()).not.toContain(tx("send:open"));
    expect(optionNames()).toContain(goTo("review"));
  });

  it("opens the send review for Send to rekordbox and sends nothing", async () => {
    const backend = fakeBackend();
    const { router } = renderApp("/library");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    await waitFor(() => expect(backend.calls).toContain("library_summary"));
    await openPalette();
    await userEvent.keyboard(tx("send:open"));
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
    expect(await screen.findByRole("heading", { level: 2, name: tx("send:title") })).toBeInTheDocument();
    expect(backend.calls).not.toContain("prepare_send");
    expect(backend.calls).not.toContain("write_send");
  });

  it("lists actions other parts of the app register, and runs their latest handler", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const ran: string[] = [];
    function Source({ tag }: { tag: string }) {
      const actions: PaletteAction[] = [
        { id: "extra", label: "Synthetic Extra", shortcut: "J", run: () => ran.push(tag) },
      ];
      useRegisterActions("test", actions);
      return null;
    }
    const source = render(<Source tag="first" />);
    source.rerender(<Source tag="second" />);

    await openPalette();
    await userEvent.keyboard("synthetic ex");
    expect(optionNames()).toEqual(["Synthetic ExtraJ"]);
    await userEvent.keyboard("{Enter}");
    expect(ran).toEqual(["second"]);

    source.unmount();
    await openPalette();
    await userEvent.keyboard("synthetic ex");
    expect(optionNames()).toEqual([]);
  });

  it("lists an action once when two sources register the same id, the first source's", async () => {
    fakeBackend();
    renderApp("/overview");
    await screen.findByRole("link", { name: tx("shell:stages.review") });
    const ran: string[] = [];
    function Source({ source, tag }: { source: string; tag: string }) {
      useRegisterActions(source, [
        { id: "twice", label: "Synthetic Twice " + tag, run: () => ran.push(tag) },
      ]);
      return null;
    }
    render(
      <>
        <Source source="b-later" tag="second" />
        <Source source="a-first" tag="first" />
      </>,
    );

    await openPalette();
    await userEvent.keyboard("synthetic twice");
    expect(optionNames()).toEqual(["Synthetic Twice first"]);
    await userEvent.keyboard("{Enter}");
    expect(ran).toEqual(["first"]);
  });
});

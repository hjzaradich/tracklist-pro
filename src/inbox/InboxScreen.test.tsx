import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks } from "@tauri-apps/api/mocks";
import { createMemoryHistory, createRootRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import "../i18n";
import { usePlayer } from "../player/playerStore";
import { Sidebar } from "../shell/Sidebar";
import { inboxBackend, inboxFolder, inboxItem, labeled } from "../test/inboxBackend";
import { tx } from "../test/tx";
import { InboxScreen } from "./InboxScreen";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

afterEach(() => {
  cleanup();
  clearMocks();
  dialog.open.mockReset();
});

const FOLDER = inboxFolder(1, String.raw`C:\Users\dj\Downloads`);

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <InboxScreen />
    </QueryClientProvider>,
  );
}

/** The sidebar and the screen together, on a router of their own. */
function renderWithSidebar() {
  const rootRoute = createRootRoute({
    component: () => (
      <>
        <Sidebar />
        <InboxScreen />
      </>
    ),
  });
  const router = createRouter({ routeTree: rootRoute, history: createMemoryHistory({ initialEntries: ["/"] }) });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

/** The row of the file with this name. */
const rowOf = async (name: string) =>
  (await screen.findByText(name)).closest("li") as HTMLElement;

const hasButton = (el: HTMLElement, key: string) =>
  within(el).queryByRole("button", { name: tx(key) }) !== null;

describe("the Inbox list", () => {
  it("groups files by the batch they arrived in, newest batch first, and says what each batch is", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [
        inboxItem(1, { batchId: 1, arrivedAt: "2026-11-01T10:00:00.000Z", fileName: "old loose.mp3" }),
        inboxItem(2, { batchId: 2, batchKind: "folder", batchLabel: "Pool Pack 12", arrivedAt: "2026-11-02T10:00:00.000Z", fileName: "pack tune.mp3" }),
      ],
    });
    renderScreen();

    const headings = (await screen.findAllByRole("heading", { level: 2 })).map((h) => h.textContent);
    expect(headings.slice(0, 2)).toEqual(["Pool Pack 12", tx("inbox:batch.loose")]);
    const pack = screen.getByRole("region", { name: "Pool Pack 12" });
    expect(within(pack).getByText("pack tune.mp3")).toBeInTheDocument();
    expect(within(pack).queryByText("old loose.mp3")).toBeNull();
  });

  it("keeps files already in the Library in a collapsed group at the bottom, with one Clear", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [labeled(1, "new_to_you"), labeled(2, "already_have", { fileName: "dupe.mp3" }), labeled(3, "already_have", { fileName: "dupe two.mp3" })],
    });
    renderScreen();

    const group = await screen.findByRole("region", { name: tx("inbox:already.title", { count: 2 }) });
    expect(screen.queryByText("dupe.mp3")).toBeNull();
    // It is the last group of files: only the Inbox folders section follows it.
    const regions = screen.getAllByRole("region");
    expect(regions.at(-2)).toBe(group);
    expect(regions.at(-1)).toBe(screen.getByRole("region", { name: tx("inbox:folders.title") }));

    const user = userEvent.setup();
    await user.click(within(group).getByRole("button", { name: tx("inbox:action.clearAll") }));
    await waitFor(() => expect(backend.called("inbox_clear")).toHaveLength(1));
    expect(backend.called("inbox_clear")[0]!.args.itemIds).toEqual([2, 3]);
  });

  it("clears one file of the Already have group on its own", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [labeled(2, "already_have", { fileName: "dupe.mp3" }), labeled(3, "already_have", { fileName: "dupe two.mp3" })],
    });
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("inbox:already.show") }));
    const row = await rowOf("dupe two.mp3");
    expect(hasButton(row, "inbox:action.add")).toBe(false);
    await user.click(within(row).getByRole("button", { name: tx("inbox:action.clear") }));
    await waitFor(() => expect(backend.called("inbox_clear")[0]?.args.itemIds).toEqual([3]));
  });

  it.each([
    ["new_to_you", ["add", "notNow", "hide"]],
    ["other_cut", ["add", "notNow", "hide"]],
    ["rework", ["add", "notNow", "hide"]],
    ["better_file", ["notNow", "hide"]],
  ] as const)("offers a %s file the actions that fit it", async (label, offered) => {
    inboxBackend({ folders: [FOLDER], items: [labeled(1, label, { fileName: "the file.mp3" })] });
    renderScreen();
    const row = await rowOf("the file.mp3");
    for (const action of ["add", "notNow", "hide", "clear"]) {
      const shown = hasButton(row, `inbox:action.${action}`);
      expect(shown, action).toBe((offered as readonly string[]).includes(action));
    }
  });

  it("shows a file still being checked as checking, with no actions", async () => {
    inboxBackend({ folders: [FOLDER], items: [labeled(1, "checking", { fileName: "wait.mp3" })] });
    renderScreen();
    const row = await rowOf("wait.mp3");
    expect(within(row).getByText(tx("inbox:label.checking"))).toBeInTheDocument();
    expect(within(row).queryAllByRole("button").map((b) => b.getAttribute("data-play-for") ?? "action")).not.toContain("action");
  });

  it("gives the reason beside the label, worded from the structured reason", async () => {
    inboxBackend({
      folders: [FOLDER],
      items: [
        labeled(1, "other_cut", {
          fileName: "cut.mp3",
          reason: { key: "inbox:reason.otherCut", params: { label: "radio" }, source: "fact" },
        }),
      ],
    });
    renderScreen();
    const row = await rowOf("cut.mp3");
    expect(within(row).getByText(tx("inbox:reason.otherCut_labeled", { label: "radio" }))).toBeInTheDocument();
  });

  it("filters by label with a chip, with the counts on the chips", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [labeled(1, "new_to_you", { fileName: "fresh.mp3" }), labeled(2, "rework", { fileName: "flip.mp3" }), labeled(3, "rework", { fileName: "flip two.mp3" })],
    });
    renderScreen();
    const user = userEvent.setup();
    const chip = await screen.findByRole("button", { name: `${tx("inbox:label.rework")} 2` });
    expect(screen.getByRole("button", { name: `${tx("inbox:filter.all")} 3` })).toHaveAttribute("aria-pressed", "true");

    await user.click(chip);
    await waitFor(() => expect(screen.queryByText("fresh.mp3")).toBeNull());
    expect(screen.getByText("flip.mp3")).toBeInTheDocument();
    expect(chip).toHaveAttribute("aria-pressed", "true");
    expect(backend.called("inbox_items").some((c) => (c.args.filter as { label: string | null }).label === "rework")).toBe(true);

    await user.click(screen.getByRole("button", { name: `${tx("inbox:filter.all")} 3` }));
    expect(await screen.findByText("fresh.mp3")).toBeInTheDocument();
  });
});

describe("Inbox actions", () => {
  it("adds one file to the Library and takes it off the list", async () => {
    const backend = inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "keep.mp3" }), labeled(2, "new_to_you", { fileName: "other.mp3" })] });
    renderScreen();
    const user = userEvent.setup();
    await user.click(within(await rowOf("keep.mp3")).getByRole("button", { name: tx("inbox:action.add") }));
    await waitFor(() => expect(screen.queryByText("keep.mp3")).toBeNull());
    expect(backend.called("inbox_add")[0]!.args).toEqual({ itemIds: [1], confirmLowBitrate: false });
    expect(screen.getByText("other.mp3")).toBeInTheDocument();
  });

  it("shows why an action was refused, beside the file", async () => {
    inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "gone.mp3" })], refused: { 1: "fileMissing" } });
    renderScreen();
    const user = userEvent.setup();
    const row = await rowOf("gone.mp3");
    await user.click(within(row).getByRole("button", { name: tx("inbox:action.add") }));
    expect(await within(row).findByRole("alert")).toHaveTextContent(tx("inbox:refusal.fileMissing"));
    expect(screen.getByText("gone.mp3")).toBeInTheDocument();
  });

  it("asks before adding a low-bitrate file, and adds it once the user agrees", async () => {
    const backend = inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "thin.mp3" })], lowBitrate: { 1: 96 } });
    renderScreen();
    const user = userEvent.setup();
    const row = await rowOf("thin.mp3");
    await user.click(within(row).getByRole("button", { name: tx("inbox:action.add") }));
    expect(await within(row).findByRole("alert")).toHaveTextContent(tx("inbox:confirm.lowBitrate", { kbps: 96 }));
    await user.click(within(row).getByRole("button", { name: tx("inbox:action.addAnyway") }));
    await waitFor(() => expect(screen.queryByText("thin.mp3")).toBeNull());
    expect(backend.called("inbox_add").map((c) => c.args.confirmLowBitrate)).toEqual([false, true]);
  });

  it("sets a file aside with Not now and hides one with Hide", async () => {
    const backend = inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "later.mp3" }), labeled(2, "new_to_you", { fileName: "never.mp3" })] });
    renderScreen();
    const user = userEvent.setup();
    await user.click(within(await rowOf("later.mp3")).getByRole("button", { name: tx("inbox:action.notNow") }));
    await waitFor(() => expect(screen.queryByText("later.mp3")).toBeNull());
    await user.click(within(await rowOf("never.mp3")).getByRole("button", { name: tx("inbox:action.hide") }));
    await waitFor(() => expect(screen.queryByText("never.mp3")).toBeNull());
    expect(backend.called("inbox_not_now")[0]!.args.itemIds).toEqual([1]);
    expect(backend.called("inbox_hide")[0]!.args.itemIds).toEqual([2]);
  });

  it("adds a whole batch with Add all, then lists the files it left out with their reasons", async () => {
    const backend = inboxBackend({
      folders: [FOLDER],
      items: [
        labeled(1, "new_to_you", { batchId: 7, batchKind: "folder", batchLabel: "Promo", fileName: "easy.mp3" }),
        labeled(2, "new_to_you", { batchId: 7, batchKind: "folder", batchLabel: "Promo", fileName: "tricky.mp3" }),
      ],
      skipOnAddAll: [{ itemId: 2, reason: "inRekordboxAsAnotherFile" }],
    });
    renderScreen();
    const user = userEvent.setup();
    const batch = await screen.findByRole("region", { name: "Promo" });
    await user.click(within(batch).getByRole("button", { name: tx("inbox:action.addAll") }));

    await waitFor(() => expect(screen.queryByText("easy.mp3")).toBeNull());
    expect(backend.called("inbox_add_batch")[0]!.args).toEqual({ batchId: 7, confirmLowBitrate: false });
    expect(await within(batch).findByText(tx("inbox:skipped.title", { count: 1 }))).toBeInTheDocument();
    expect(
      within(batch).getByText(
        tx("inbox:skipped.line", { file: "tricky.mp3", reason: tx("inbox:refusal.inRekordboxAsAnotherFile") }),
      ),
    ).toBeInTheDocument();
  });
});

describe("the Inbox sidebar badge", () => {
  it("counts the new files, and counts again after an action", async () => {
    inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "a.mp3" }), labeled(2, "new_to_you", { fileName: "b.mp3" })] });
    renderWithSidebar();
    const user = userEvent.setup();
    const link = await screen.findByRole("link", { name: (name) => name.startsWith(tx("shell:stages.inbox")) });
    await waitFor(() => expect(within(link).getByTestId("count-badge")).toHaveTextContent("2"));

    await user.click(within(await rowOf("a.mp3")).getByRole("button", { name: tx("inbox:action.hide") }));
    await waitFor(() => expect(within(link).getByTestId("count-badge")).toHaveTextContent("1"));
  });

  it("shows no badge when nothing is new", async () => {
    inboxBackend({ folders: [FOLDER], items: [] });
    renderWithSidebar();
    const link = await screen.findByRole("link", { name: tx("shell:stages.inbox") });
    await screen.findByText(tx("inbox:empty.zero"));
    expect(within(link).queryByTestId("count-badge")).toBeNull();
  });
});

describe("Inbox folders and the empty states", () => {
  it("says what to do when there is no Inbox folder yet", async () => {
    inboxBackend({ items: [] });
    renderScreen();
    expect(await screen.findByText(tx("inbox:empty.noFolders"))).toBeInTheDocument();
    expect(screen.queryByText(tx("inbox:empty.zero"))).toBeNull();
    expect(screen.getByText(tx("inbox:folders.none"))).toBeInTheDocument();
  });

  it("says Inbox zero when there are folders and nothing is new", async () => {
    inboxBackend({ folders: [FOLDER], items: [] });
    renderScreen();
    expect(await screen.findByText(tx("inbox:empty.zero"))).toBeInTheDocument();
    expect(screen.queryByText(tx("inbox:empty.noFolders"))).toBeNull();
  });

  it("adds a folder with the Inbox role through the folder picker", async () => {
    const backend = inboxBackend({ items: [] });
    dialog.open.mockResolvedValue(String.raw`C:\Users\dj\Pool`);
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("inbox:folders.add") }));
    await waitFor(() =>
      expect(backend.called("add_music_folder")[0]!.args).toEqual({ path: String.raw`C:\Users\dj\Pool`, role: "inbox" }),
    );
    expect(await screen.findByText(String.raw`C:\Users\dj\Pool`)).toBeInTheDocument();
  });

  it("removes an Inbox folder", async () => {
    const backend = inboxBackend({ folders: [FOLDER], items: [] });
    renderScreen();
    const user = userEvent.setup();
    await user.click(await screen.findByRole("button", { name: tx("inbox:folders.removeNamed", { path: FOLDER.path }) }));
    await waitFor(() => expect(backend.called("remove_music_folder")[0]!.args).toEqual({ id: 1 }));
    expect(await screen.findByText(tx("inbox:folders.none"))).toBeInTheDocument();
  });

  it("says a folder is still having its first look, and only that folder", async () => {
    inboxBackend({ folders: [FOLDER, inboxFolder(2, String.raw`C:\Users\dj\Old`)], waitingForFirstLook: [1], items: [] });
    renderScreen();
    const waiting = await screen.findByText(FOLDER.path);
    expect(within(waiting.closest("li")!).getByText(tx("inbox:folders.firstLook"))).toBeInTheDocument();
    const other = screen.getByText(String.raw`C:\Users\dj\Old`).closest("li")!;
    expect(within(other).queryByText(tx("inbox:folders.firstLook"))).toBeNull();
  });
});

describe("previews from the Inbox", () => {
  it("start about a third of the way into the file, and only here", async () => {
    inboxBackend({ folders: [FOLDER], items: [labeled(1, "new_to_you", { fileName: "listen.mp3" })] });
    const select = vi.fn();
    usePlayer.setState({ select });
    renderScreen();
    const user = userEvent.setup();
    const row = await rowOf("listen.mp3");
    await user.click(row.querySelector("[data-play-for]")!);
    expect(select).toHaveBeenCalledTimes(1);
    const [file, options] = select.mock.calls[0] as [{ id: number }, { startAtFraction: number }];
    expect(file.id).toBe(1001);
    expect(options.startAtFraction).toBeCloseTo(1 / 3);
  });
});

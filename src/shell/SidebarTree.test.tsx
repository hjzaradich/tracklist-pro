import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { validateCratesSearch } from "../app/router";
import type { Crate } from "../bindings";
import { tx } from "../test/tx";
import { quietAnswer } from "../test/quietCommands";

const crate = (id: number, name: string, parentId: number | null = null): Crate => ({
  id,
  parentId,
  kind: "static",
  name,
  trackCount: 0,
});
const folder = (id: number, name: string, parentId: number | null = null): Crate => ({
  ...crate(id, name, parentId),
  kind: "folder",
});

/** Friday residency > (Warmup, Late > Peak time), and Loose at the top. */
const TREE: Crate[] = [
  folder(1, "Friday residency"),
  crate(2, "Warmup", 1),
  folder(3, "Late", 1),
  crate(4, "Peak time", 3),
  crate(5, "Loose"),
];

function backend(list: Crate[]) {
  const calls: string[] = [];
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "list_crates":
          return list;
        case "crate_tracks":
          return [];
        case "library_summary":
          return { tracks: 0, changeStamp: 0 };
        case "next_undo_operation":
          return { operation: null, refusal: null };
        default:
          return quietAnswer(cmd);
      }
    },
    { shouldMockEvents: true },
  );
  return calls;
}

afterEach(() => {
  cleanup();
  clearMocks();
});

/** A name that starts with this text (a count badge may follow it). */
const startingWith = (text: string) =>
  new RegExp(`^${text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`);

const tree = () => screen.findByRole("list", { name: tx("shell:sidebar.treeLabel") });

describe("the crate tree in the sidebar", () => {
  it("sits under the Crates stage and lists folders with their crates, nested, in order", async () => {
    backend(TREE);
    renderApp("/overview");
    const list = await tree();
    const nav = screen.getByRole("navigation", { name: tx("shell:stages.label") });
    const stage = within(nav).getByRole("link", { name: startingWith(tx("shell:stages.crates")) });
    // The tree comes after the Crates stage link, which comes after the other stages.
    expect(stage.compareDocumentPosition(list) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(within(list).getAllByRole("link").map((link) => link.textContent)).toEqual([
      "Friday residency",
      "Warmup",
      "Late",
      "Peak time",
      "Loose",
    ]);
    // Nested: Warmup and Late are inside the folder's item, Peak time inside Late's.
    const folderItem = within(list).getByRole("link", { name: "Friday residency" }).closest("li");
    expect(within(folderItem as HTMLElement).getAllByRole("link")).toHaveLength(4);
    const lateItem = within(list).getByRole("link", { name: "Late" }).closest("li");
    expect(within(lateItem as HTMLElement).getAllByRole("link")).toHaveLength(2);
  });

  it("counts only crates in the Crates badge, not folders", async () => {
    backend(TREE);
    renderApp("/overview");
    await tree();
    const link = screen.getByRole("link", { name: startingWith(tx("shell:stages.crates")) });
    expect(within(link).getByTestId("count-badge")).toHaveTextContent("3");
  });

  it("opens a crate on the Crates screen, marks it as the current one, and keeps the choice in the address", async () => {
    backend(TREE);
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: "Peak time" }));
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Peak time");
    expect(router.state.location.pathname).toBe("/crates");
    expect(router.state.location.search).toEqual({ crate: 4 });
    expect(screen.getByRole("link", { name: "Peak time" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("link", { name: "Loose" })).not.toHaveAttribute("aria-current");
    // Its folder is open, and the page says where rekordbox shows it.
    expect(screen.getByLabelText(tx("crates:path.label"))).toHaveTextContent(
      `${tx("crates:path.root")} › Friday residency › Late › Peak time`,
    );
  });

  it("opens straight on a crate named in the address", async () => {
    backend(TREE);
    renderApp("/crates?crate=2");
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Warmup");
    expect(await tree()).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Warmup" })).toHaveAttribute("aria-current", "page");
  });

  it("drops a question about one crate when another is chosen from the sidebar", async () => {
    backend(TREE);
    renderApp("/crates?crate=2");
    await userEvent.click(await screen.findByRole("button", { name: tx("crates:rename.button") }));
    expect(await screen.findByLabelText(tx("crates:rename.label"))).toHaveValue("Warmup");
    await userEvent.click(within(await tree()).getByRole("link", { name: "Loose" }));
    expect(await screen.findByRole("heading", { level: 2 })).toHaveTextContent("Loose");
    expect(screen.queryByLabelText(tx("crates:rename.label"))).toBeNull();
  });

  it("closes and opens a folder, and keeps open the one holding the chosen crate", async () => {
    backend(TREE);
    renderApp("/overview");
    const list = await tree();
    await userEvent.click(
      within(list).getByRole("button", { name: tx("shell:sidebar.collapse", { name: "Late" }) }),
    );
    expect(within(list).queryByRole("link", { name: "Peak time" })).toBeNull();
    expect(within(list).getByRole("link", { name: "Late" })).toBeInTheDocument();
    await userEvent.click(
      within(list).getByRole("button", { name: tx("shell:sidebar.expand", { name: "Late" }) }),
    );
    expect(within(list).getByRole("link", { name: "Peak time" })).toBeInTheDocument();

    // Close Late, then choose Peak time on the screen: Late opens by itself,
    // so the choice is never hidden.
    await userEvent.click(
      within(list).getByRole("button", { name: tx("shell:sidebar.collapse", { name: "Late" }) }),
    );
    await userEvent.click(await screen.findByRole("link", { name: startingWith(tx("shell:stages.crates")) }));
    await userEvent.click(await screen.findByRole("button", { name: /Peak time/ }));
    expect(await within(list).findByRole("link", { name: "Peak time" })).toHaveAttribute(
      "aria-current",
      "page",
    );
  });

  it("says there are no crates, and draws no tree", async () => {
    backend([]);
    renderApp("/overview");
    expect(await screen.findByText(tx("shell:sidebar.noCrates"))).toBeInTheDocument();
    expect(screen.queryByRole("list", { name: tx("shell:sidebar.treeLabel") })).toBeNull();
  });

  it("shows nothing chosen, with no error, when the address names a crate that is gone", async () => {
    backend(TREE);
    renderApp("/crates?crate=999");
    expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("heading", { level: 2 })).toBeNull();
    const list = await tree();
    expect(within(list).queryAllByRole("link", { current: "page" })).toHaveLength(0);
  });

  it("shows nothing chosen, with no error, when the address holds no usable id", async () => {
    for (const search of ["crate=abc", "crate=-4", "crate=1.5", "crate=0", "crate="]) {
      backend(TREE);
      const { unmount } = renderApp(`/crates?${search}`);
      expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
      expect(screen.queryByRole("alert")).toBeNull();
      unmount();
      clearMocks();
    }
  });

  it("goes to nothing chosen when the chosen crate is deleted", async () => {
    let list = TREE;
    mockIPC(
      (cmd, args) => {
        if (cmd === "list_crates") return list;
        if (cmd === "crate_tracks") return [];
        if (cmd === "delete_crate") {
          list = list.filter((c) => c.id !== (args as { id: number }).id);
          return null;
        }
        if (cmd === "next_undo_operation") return { operation: null, refusal: null };
        return quietAnswer(cmd);
      },
      { shouldMockEvents: true },
    );
    const { router } = renderApp("/crates?crate=5");
    await userEvent.click(await screen.findByRole("button", { name: tx("crates:delete.button") }));
    await userEvent.click(
      within(await screen.findByRole("alertdialog")).getByRole("button", {
        name: tx("crates:delete.confirm"),
      }),
    );
    expect(await screen.findByText(tx("crates:select"))).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.search).toEqual({}));
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("the address of a chosen crate", () => {
  it("takes a whole number above zero and nothing else", () => {
    expect(validateCratesSearch({ crate: 7 })).toEqual({ crate: 7 });
    for (const crate of [0, -1, 1.5, NaN, Infinity, "7", null, [], {}, true, undefined]) {
      expect(validateCratesSearch({ crate })).toEqual({});
    }
    expect(validateCratesSearch({})).toEqual({});
  });
});

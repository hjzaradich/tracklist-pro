import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks } from "@tauri-apps/api/mocks";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import "../i18n";
import { SettingsScreen } from "../settings/SettingsScreen";
import { Sidebar } from "../shell/Sidebar";
import { owned, wishItem, wishlistBackend } from "../test/wishlistBackend";
import { tx } from "../test/tx";
import { storeLink } from "./linkTemplates";
import { WishlistScreen } from "./WishlistScreen";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

afterEach(() => {
  cleanup();
  clearMocks();
  dialog.open.mockReset();
  vi.unstubAllGlobals();
});

/** The sidebar, the screen and Settings on a router of their own. */
function renderApp(path = "/wishlists") {
  const root = createRootRoute({
    component: () => (
      <>
        <Sidebar />
        <Outlet />
      </>
    ),
  });
  const routeTree = root.addChildren([
    createRoute({ getParentRoute: () => root, path: "/wishlists", component: WishlistScreen }),
    createRoute({ getParentRoute: () => root, path: "/settings", component: SettingsScreen }),
  ]);
  const router = createRouter({ routeTree, history: createMemoryHistory({ initialEntries: [path] }) });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <WishlistScreen />
    </QueryClientProvider>,
  );
}

const LIST = { id: 1, name: "List One", source: "paste" as const };

/** The row of the track with this title (its name comes before its reason, which may repeat the title). */
const rowOf = async (title: string) =>
  (await screen.findAllByText(new RegExp(title)))[0].closest("li") as HTMLElement;

describe("the Wishlists screen while the feature is off", () => {
  it("says it is turned off, links to Settings and asks the backend for no lists", async () => {
    const backend = wishlistBackend({ on: false });
    renderApp();
    expect(await screen.findByText(tx("wishlist:off.message"))).toBeInTheDocument();
    expect(screen.getByRole("link", { name: tx("wishlist:off.settings") })).toHaveAttribute("href", "/settings");
    expect(backend.called("wishlists_list")).toHaveLength(0);
    expect(screen.queryByRole("button", { name: tx("wishlist:import.makeFromPaste") })).toBeNull();
  });
});

describe("the Wishlists screen", () => {
  it("makes a wishlist from pasted text, opens it, and lists the lines it could not take", async () => {
    const backend = wishlistBackend({ ownedTitles: ["Title Two"] });
    renderScreen();
    const user = userEvent.setup();

    await user.type(await screen.findByLabelText(tx("wishlist:import.name")), "List Two");
    await user.click(screen.getByLabelText(tx("wishlist:import.paste")));
    await user.paste("Artist One - Title One\nArtist Two - Title Two\nnot a track line");
    await user.click(screen.getByRole("button", { name: tx("wishlist:import.makeFromPaste") }));

    expect(await screen.findByText(tx("wishlist:import.added", { count: 2 }))).toBeInTheDocument();
    expect(backend.called("wishlist_create_from_paste")[0].args).toEqual({
      name: "List Two",
      pasted: "Artist One - Title One\nArtist Two - Title Two\nnot a track line",
    });
    expect(screen.getByText(tx("wishlist:import.skippedTitle", { count: 1 }))).toBeInTheDocument();
    expect(
      screen.getByText(
        tx("wishlist:import.skippedLine", {
          line: 3,
          text: "not a track line",
          reason: tx("wishlist:skip.noSeparator"),
        }),
      ),
    ).toBeInTheDocument();

    // The new list opens, each track with its result and the reason for it.
    const missing = await rowOf("Title One");
    expect(within(missing).getByText(tx("wishlist:kind.missing"))).toBeInTheDocument();
    expect(within(missing).getByText(tx("wishlist:reason.missing"))).toBeInTheDocument();
    const have = await rowOf("Title Two");
    expect(within(have).getByText(tx("wishlist:kind.owned"))).toBeInTheDocument();
    expect(
      within(have).getByText(tx("wishlist:reason.ownedLibrary", { have: "Title Two" })),
    ).toBeInTheDocument();
  });

  it("makes a wishlist from a picked CSV file, named after the file when the name is empty", async () => {
    const backend = wishlistBackend({ csvRows: [{ artist: "Artist Nine", title: "Title Nine" }] });
    dialog.open.mockResolvedValue(String.raw`C:\dj\wanted tracks.csv`);
    renderScreen();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: tx("wishlist:import.chooseCsv") }));

    expect(await screen.findByText(tx("wishlist:import.added", { count: 1 }))).toBeInTheDocument();
    expect(backend.called("wishlist_create_from_csv")[0].args).toEqual({
      name: "wanted tracks",
      path: String.raw`C:\dj\wanted tracks.csv`,
    });
    expect(dialog.open.mock.calls[0][0].filters[0].extensions).toEqual(["csv"]);
    expect(await rowOf("Title Nine")).toBeInTheDocument();
  });

  it("does nothing when the file pick is cancelled, and says why an import was refused", async () => {
    const backend = wishlistBackend({ refuse: "nameEmpty" });
    dialog.open.mockResolvedValueOnce(null);
    renderScreen();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: tx("wishlist:import.chooseCsv") }));
    expect(backend.called("wishlist_create_from_csv")).toHaveLength(0);

    await user.click(screen.getByRole("button", { name: tx("wishlist:import.makeFromPaste") }));
    expect(await screen.findByText(tx("wishlist:refusal.nameEmpty"))).toBeInTheDocument();
  });

  it("marks a track bought, keeps a note on it, removes it, and deletes the list", async () => {
    const backend = wishlistBackend({
      lists: [{ ...LIST, items: [wishItem(1, { title: "Title One" }), wishItem(2, { title: "Title Two" })] }],
    });
    renderScreen();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: new RegExp(LIST.name) }));
    const row = await rowOf("Title One");
    await user.click(within(row).getByRole("checkbox", { name: tx("wishlist:item.bought") }));
    await waitFor(() =>
      expect(backend.called("wishlist_set_bought")[0].args).toEqual({ itemId: 1, bought: true }),
    );

    await user.type(within(row).getByLabelText(tx("wishlist:item.notes")), "check the other store");
    await user.tab();
    await waitFor(() =>
      expect(backend.called("wishlist_set_notes")[0].args).toEqual({
        itemId: 1,
        notes: "check the other store",
      }),
    );

    await user.click(within(await rowOf("Title Two")).getByRole("button", { name: tx("wishlist:item.remove") }));
    await waitFor(() => expect(backend.called("wishlist_remove_item")[0].args).toEqual({ itemId: 2 }));
    await waitFor(() => expect(screen.queryByText(/Title Two/)).toBeNull());

    await user.click(screen.getByRole("button", { name: tx("wishlist:lists.delete") }));
    await waitFor(() => expect(backend.called("wishlist_delete")[0].args).toEqual({ wishlistId: 1 }));
    expect(await screen.findByText(tx("wishlist:lists.none"))).toBeInTheDocument();
  });

  it("does not save a note that did not change", async () => {
    const backend = wishlistBackend({ lists: [{ ...LIST, items: [wishItem(1, { title: "Title One", notes: "kept" })] }] });
    renderScreen();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: new RegExp(LIST.name) }));
    const row = await rowOf("Title One");
    await user.click(within(row).getByLabelText(tx("wishlist:item.notes")));
    await user.tab();
    expect(backend.called("wishlist_set_notes")).toHaveLength(0);
  });
});

describe("store links", () => {
  it("show each store's search address as text, copy it on request and open nothing", async () => {
    wishlistBackend({
      lists: [
        {
          ...LIST,
          items: [
            wishItem(1, { artist: "Artist One", title: "Title One", version: "Dub Mix" }),
            wishItem(2, { title: "Title Two", result: owned("Title Two") }),
          ],
        },
      ],
    });
    const open = vi.fn();
    vi.stubGlobal("open", open);
    renderScreen();
    const user = userEvent.setup();

    await user.click(await screen.findByRole("button", { name: new RegExp(LIST.name) }));
    const row = await rowOf("Title One");
    await user.click(within(row).getByText(tx("wishlist:links.title")));

    const track = { artist: "Artist One", title: "Title One", version: "Dub Mix" };
    const beatport = within(row).getByText(tx("wishlist:store.beatport")).closest("li") as HTMLElement;
    expect(within(beatport).getByText(storeLink("beatport", track))).toBeInTheDocument();
    for (const store of ["bandcamp", "traxsource", "juno"] as const) {
      expect(within(row).getByText(storeLink(store, track))).toBeInTheDocument();
    }
    // Links are text, not anchors: the app can't open a page.
    expect(within(row).queryByRole("link")).toBeNull();

    await user.click(within(beatport).getByRole("button", { name: tx("wishlist:links.copy") }));
    expect(await navigator.clipboard.readText()).toBe(storeLink("beatport", track));
    expect(within(beatport).getByText(tx("wishlist:links.copied"))).toBeInTheDocument();
    expect(open).not.toHaveBeenCalled();

    // A track you own needs no store.
    expect(within(await rowOf("Title Two")).queryByText(tx("wishlist:links.title"))).toBeNull();
  });

  it("say so when the copy fails", async () => {
    wishlistBackend({ lists: [{ ...LIST, items: [wishItem(1, { title: "Title One" })] }] });
    renderScreen();
    const user = userEvent.setup();
    // After setup: userEvent installs its own clipboard on setup.
    vi.stubGlobal("navigator", {
      ...navigator,
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });

    await user.click(await screen.findByRole("button", { name: new RegExp(LIST.name) }));
    const row = await rowOf("Title One");
    await user.click(within(row).getByText(tx("wishlist:links.title")));
    const beatport = within(row).getByText(tx("wishlist:store.beatport")).closest("li") as HTMLElement;
    await user.click(within(beatport).getByRole("button", { name: tx("wishlist:links.copy") }));
    expect(await within(beatport).findByText(tx("wishlist:links.failed"))).toBeInTheDocument();
  });
});

describe("the switch in Settings", () => {
  it("turns the feature on and off, and the sidebar entry follows", async () => {
    const backend = wishlistBackend({ on: false });
    const root = createRootRoute({
      component: () => (
        <>
          <Sidebar />
          <SettingsScreen />
        </>
      ),
    });
    const router = createRouter({ routeTree: root, history: createMemoryHistory({ initialEntries: ["/"] }) });
    render(
      <QueryClientProvider client={createQueryClient()}>
        <RouterProvider router={router} />
      </QueryClientProvider>,
    );
    const user = userEvent.setup();
    const box = await screen.findByRole("checkbox", { name: tx("wishlist:feature.label") });
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
    expect(screen.queryByRole("link", { name: tx("wishlist:title") })).toBeNull();

    await user.click(box);
    expect(await screen.findByRole("link", { name: tx("wishlist:title") })).toHaveAttribute("href", "/wishlists");
    expect(backend.called("set_feature_enabled")[0].args).toEqual({ feature: "wishlist", on: true });
    expect(box).toBeChecked();

    await user.click(box);
    await waitFor(() => expect(screen.queryByRole("link", { name: tx("wishlist:title") })).toBeNull());
    expect(backend.called("set_feature_enabled")[1].args).toEqual({ feature: "wishlist", on: false });
  });
});

import { createMemoryHistory, createRootRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { renderApp } from "../app/testApp";
import type { Crate } from "../bindings";
import { DEFAULT_THEME, syncThemeToDocument, useThemeStore } from "../theme/themeStore";
import { Sidebar } from "./Sidebar";
import { tx } from "../test/tx";

const WORKFLOW_ORDER = ["overview", "review", "allMusic", "library", "crates"].map((id) =>
  tx(`shell:stages.${id}`),
);

/** A link whose name starts with this text (a count badge may follow it). */
const startingWith = (text: string) =>
  new RegExp(`^${text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}`);

function stageLinks() {
  const nav = screen.getByRole("navigation", { name: tx("shell:stages.label") });
  return within(nav).getAllByRole("link");
}

function currentStages() {
  return stageLinks()
    .filter((link) => link.getAttribute("aria-current") === "page")
    .map((link) => link.textContent);
}

/** Stands in for the Rust side as far as the sidebar goes: the crates. */
function crates(names: string[]) {
  const list: Crate[] = names.map((name, n) => ({ id: n + 1, name, trackCount: 0 }));
  mockIPC(
    (cmd) => {
      if (cmd === "list_crates") return list;
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
}

/** The sidebar alone, on a router of its own. */
function renderSidebar(counts?: Parameters<typeof Sidebar>[0]["counts"]) {
  const rootRoute = createRootRoute({ component: () => <Sidebar counts={counts} /> });
  const router = createRouter({
    routeTree: rootRoute,
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <RouterProvider router={router} />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("layout shell", () => {
  it("has every zone: top bar, sidebar, center, Details panel and player", async () => {
    crates([]);
    renderApp("/overview");
    const topBar = await screen.findByRole("banner");
    expect(within(topBar).getByRole("search", { name: tx("shell:search.label") })).toBeInTheDocument();
    expect(within(topBar).getByRole("status", { name: tx("activity:label") })).toHaveTextContent(
      tx("activity:idle"),
    );
    expect(await screen.findByText(tx("shell:sidebar.noCrates"))).toBeInTheDocument();
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: tx("shell:details.label") })).toHaveTextContent(
      tx("shell:details.empty"),
    );
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent(tx("shell:player.empty"));
  });

  it("shows the command palette shortcut in the search slot", async () => {
    renderApp("/overview");
    const search = await screen.findByRole("search", { name: tx("shell:search.label") });
    expect(within(search).getByRole("searchbox", { name: tx("shell:search.label") })).toHaveAttribute(
      "placeholder",
      tx("shell:search.placeholder"),
    );
    expect(search).toHaveTextContent(tx("shell:search.shortcut"));
  });
});

describe("sidebar", () => {
  it("lists the stages in workflow order", async () => {
    renderApp("/overview");
    await screen.findByRole("navigation", { name: tx("shell:stages.label") });
    expect(stageLinks().map((link) => link.textContent)).toEqual(WORKFLOW_ORDER);
  });

  it("marks only the current stage with aria-current", async () => {
    renderApp("/all-music");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.allMusic") });
    expect(currentStages()).toEqual([tx("shell:stages.allMusic")]);
  });

  it("navigates to each stage when its link is clicked", async () => {
    const user = userEvent.setup();
    const { router } = renderApp("/overview");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") });

    for (const name of [...WORKFLOW_ORDER].reverse()) {
      await user.click(screen.getByRole("link", { name }));
      expect(await screen.findByRole("heading", { level: 1, name })).toBeInTheDocument();
      expect(currentStages()).toEqual([name]);
    }
    expect(router.state.location.pathname).toBe("/overview");
  });

  it("can be used from the keyboard: Tab to a stage, Enter to open it", async () => {
    const user = userEvent.setup();
    renderApp("/overview");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") });

    const review = screen.getByRole("link", { name: tx("shell:stages.review") });
    review.focus();
    await user.tab();
    expect(screen.getByRole("link", { name: tx("shell:stages.allMusic") })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.allMusic") })).toBeInTheDocument();
  });

  it("shows a count badge only for stages given a count", async () => {
    crates([]);
    renderSidebar({ review: 1234, crates: 0 });

    const review = await screen.findByRole("link", { name: startingWith(tx("shell:stages.review")) });
    expect(within(review).getByTestId("count-badge")).toHaveTextContent("1,234");
    expect(within(screen.getByRole("link", { name: startingWith(tx("shell:stages.crates")) })).getByTestId("count-badge")).toHaveTextContent("0");
    expect(within(screen.getByRole("link", { name: tx("shell:stages.library") })).queryByTestId("count-badge")).toBeNull();
  });

  it("says there are no crates only when there are none", async () => {
    crates([]);
    renderSidebar();
    expect(await screen.findByText(tx("shell:sidebar.noCrates"))).toBeInTheDocument();
    expect(
      within(screen.getByRole("link", { name: tx("shell:stages.crates") })).queryByTestId("count-badge"),
    ).toBeNull();
  });

  it("shows how many crates there are, and no longer says there are none", async () => {
    crates(["Friday", "Warm up"]);
    renderSidebar();
    const link = await screen.findByRole("link", { name: startingWith(tx("shell:stages.crates")) });
    expect(await within(link).findByTestId("count-badge")).toHaveTextContent("2");
    expect(screen.queryByText(tx("shell:sidebar.noCrates"))).toBeNull();
  });

  it("says nothing about crates until it knows", async () => {
    mockIPC(() => new Promise(() => {}), { shouldMockEvents: true });
    renderSidebar();
    await screen.findByRole("link", { name: tx("shell:stages.crates") });
    expect(screen.queryByText(tx("shell:sidebar.noCrates"))).toBeNull();
    expect(screen.queryByTestId("count-badge")).toBeNull();
  });
});

describe("themes", () => {
  let unsync: () => void;

  beforeEach(() => {
    useThemeStore.setState({ theme: DEFAULT_THEME });
    unsync = syncThemeToDocument();
  });

  afterEach(() => {
    unsync();
    delete document.documentElement.dataset.theme;
  });

  it.each(["dark", "light"] as const)("the shell renders in the %s theme", async (theme) => {
    useThemeStore.setState({ theme });
    renderApp("/library");
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.library") })).toBeInTheDocument();
    expect(document.documentElement.dataset.theme).toBe(theme);
    expect(screen.getByRole("radio", { name: theme === "dark" ? tx("theme:dark") : tx("theme:light") })).toBeChecked();
    expect(screen.getByRole("navigation", { name: tx("shell:stages.label") })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: tx("shell:details.label") })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toBeInTheDocument();
  });

  it("switches theme from the top bar without leaving the current stage", async () => {
    const user = userEvent.setup();
    renderApp("/review");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.review") });

    await user.click(within(screen.getByRole("banner")).getByRole("radio", { name: tx("theme:light") }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("heading", { level: 1, name: tx("shell:stages.review") })).toBeInTheDocument();
    expect(currentStages()).toEqual([tx("shell:stages.review")]);
  });
});

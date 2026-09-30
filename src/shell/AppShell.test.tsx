import { createMemoryHistory, createRootRoute, createRouter, RouterProvider } from "@tanstack/react-router";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { DEFAULT_THEME, syncThemeToDocument, useThemeStore } from "../theme/themeStore";
import { Sidebar } from "./Sidebar";

const WORKFLOW_ORDER = ["Overview", "Review", "All music", "Library", "Crates"];

function stageLinks() {
  const nav = screen.getByRole("navigation", { name: "Stages" });
  return within(nav).getAllByRole("link");
}

function currentStages() {
  return stageLinks()
    .filter((link) => link.getAttribute("aria-current") === "page")
    .map((link) => link.textContent);
}

describe("layout shell", () => {
  it("has every zone: top bar, sidebar, center, Details panel and player", async () => {
    renderApp("/overview");
    const topBar = await screen.findByRole("banner");
    expect(within(topBar).getByRole("search", { name: "Search" })).toBeInTheDocument();
    expect(within(topBar).getByRole("status", { name: "Activity" })).toHaveTextContent(
      "No background tasks",
    );
    expect(screen.getByRole("navigation", { name: "Stages" })).toHaveTextContent("No crates");
    expect(screen.getByRole("main")).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Details" })).toHaveTextContent(
      "Select a track",
    );
    expect(screen.getByRole("region", { name: "Player" })).toHaveTextContent("Nothing playing");
  });

  it("shows the command palette shortcut in the search slot", async () => {
    renderApp("/overview");
    const search = await screen.findByRole("search", { name: "Search" });
    expect(within(search).getByRole("searchbox", { name: "Search" })).toHaveAttribute(
      "placeholder",
      "Search or type a command",
    );
    expect(search).toHaveTextContent("Ctrl K");
  });
});

describe("sidebar", () => {
  it("lists the stages in workflow order", async () => {
    renderApp("/overview");
    await screen.findByRole("navigation", { name: "Stages" });
    expect(stageLinks().map((link) => link.textContent)).toEqual(WORKFLOW_ORDER);
  });

  it("marks only the current stage with aria-current", async () => {
    renderApp("/all-music");
    await screen.findByRole("heading", { level: 1, name: "All music" });
    expect(currentStages()).toEqual(["All music"]);
  });

  it("navigates to each stage when its link is clicked", async () => {
    const user = userEvent.setup();
    const { router } = renderApp("/overview");
    await screen.findByRole("heading", { level: 1, name: "Overview" });

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
    await screen.findByRole("heading", { level: 1, name: "Overview" });

    const review = screen.getByRole("link", { name: "Review" });
    review.focus();
    await user.tab();
    expect(screen.getByRole("link", { name: "All music" })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(await screen.findByRole("heading", { level: 1, name: "All music" })).toBeInTheDocument();
  });

  it("shows a count badge only for stages given a count", async () => {
    const rootRoute = createRootRoute({
      component: () => <Sidebar counts={{ review: 1234, crates: 0 }} />,
    });
    const router = createRouter({
      routeTree: rootRoute,
      history: createMemoryHistory({ initialEntries: ["/"] }),
    });
    render(<RouterProvider router={router} />);

    const review = await screen.findByRole("link", { name: /Review/ });
    expect(within(review).getByTestId("count-badge")).toHaveTextContent("1,234");
    expect(within(screen.getByRole("link", { name: /Crates/ })).getByTestId("count-badge")).toHaveTextContent("0");
    expect(within(screen.getByRole("link", { name: "Library" })).queryByTestId("count-badge")).toBeNull();
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
    expect(await screen.findByRole("heading", { level: 1, name: "Library" })).toBeInTheDocument();
    expect(document.documentElement.dataset.theme).toBe(theme);
    expect(screen.getByRole("radio", { name: theme === "dark" ? "Dark" : "Light" })).toBeChecked();
    expect(screen.getByRole("navigation", { name: "Stages" })).toBeInTheDocument();
    expect(screen.getByRole("complementary", { name: "Details" })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Player" })).toBeInTheDocument();
  });

  it("switches theme from the top bar without leaving the current stage", async () => {
    const user = userEvent.setup();
    renderApp("/review");
    await screen.findByRole("heading", { level: 1, name: "Review" });

    await user.click(within(screen.getByRole("banner")).getByRole("radio", { name: "Light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("heading", { level: 1, name: "Review" })).toBeInTheDocument();
    expect(currentStages()).toEqual(["Review"]);
  });
});

import { useQuery } from "@tanstack/react-query";
import {
  createMemoryHistory,
  createRootRoute,
  createRoute,
  createRouter,
  Outlet,
} from "@tanstack/react-router";
import { render, screen, within } from "@testing-library/react";
import { useContext } from "react";
import { I18nContext, useTranslation } from "react-i18next";
import { describe, expect, it } from "vitest";
import { create } from "zustand";
import i18n from "../i18n";
import { AppProviders } from "./AppProviders";
import { createQueryClient } from "./queryClient";
import { renderApp } from "./testApp";

/**
 * A routed component that needs every provider: it reads i18n from React
 * context (not the global fallback react-i18next would otherwise use), and it
 * runs a query. Rendered through <AppProviders>, so removing either provider
 * from AppProviders fails a test below.
 */
function ProvidersProbe() {
  const context = useContext(I18nContext);
  const { t } = useTranslation("common");
  const { data } = useQuery({ queryKey: ["probe"], queryFn: async () => "query-result" });
  return (
    <p data-testid="probe" data-i18n-from-context={String(context?.i18n === i18n)}>
      {t("appName")} {data}
    </p>
  );
}

function renderProbe() {
  const rootRoute = createRootRoute({ component: Outlet });
  const probeRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: "/",
    component: ProvidersProbe,
  });
  const router = createRouter({
    routeTree: rootRoute.addChildren([probeRoute]),
    history: createMemoryHistory({ initialEntries: ["/"] }),
  });
  return render(<AppProviders router={router} queryClient={createQueryClient()} />);
}

describe("app providers", () => {
  it("render the app's routes inside the layout shell", async () => {
    renderApp("/overview");
    expect(await screen.findByRole("heading", { name: "Overview" })).toBeInTheDocument();
    expect(screen.getByRole("navigation", { name: "Stages" })).toBeInTheDocument();
  });

  it("keep the theme switch reachable from the top bar", async () => {
    renderApp("/");
    const topBar = await screen.findByRole("banner");
    expect(within(topBar).getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();
  });

  it("give routed components query results through TanStack Query", async () => {
    renderProbe();
    expect(await screen.findByText(/query-result/)).toBeInTheDocument();
  });

  it("give routed components the app's i18n instance and its translations", async () => {
    renderProbe();
    const probe = await screen.findByTestId("probe");
    expect(probe).toHaveAttribute("data-i18n-from-context", "true");
    // The translated value, not the key "appName".
    expect(probe).toHaveTextContent("tracklist-pro");
  });
});

describe("TanStack Query client", () => {
  it("does not refetch when the window regains focus or retry failed queries", () => {
    const defaults = createQueryClient().getDefaultOptions().queries;
    expect(defaults?.refetchOnWindowFocus).toBe(false);
    expect(defaults?.retry).toBe(false);
  });
});

describe("Zustand", () => {
  it("shares UI state between components without a provider", () => {
    const useCounter = create<{ count: number }>(() => ({ count: 3 }));
    function Count() {
      return <p>{useCounter((state) => state.count)}</p>;
    }
    render(
      <>
        <Count />
        <Count />
      </>,
    );
    expect(screen.getAllByText("3")).toHaveLength(2);
  });
});

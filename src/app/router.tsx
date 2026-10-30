import type { QueryClient } from "@tanstack/react-query";
import {
  createRootRouteWithContext,
  createRoute,
  createRouter,
  redirect,
  useNavigate,
  type RouterHistory,
} from "@tanstack/react-router";
import { AfterSendScreen } from "../afterSend/AfterSendScreen";
import { AllMusicScreen } from "../allMusic/AllMusicScreen";
import { CratesScreen } from "../crates/CratesScreen";
import { launchScreen, type RouterContext } from "../launch/launchContext";
import { LaunchWatcher } from "../launch/LaunchWatcher";
import { isCrateId } from "../launch/savedScreen";
import { LibraryScreen } from "../library/LibraryScreen";
import { OverviewScreen } from "../overview/OverviewScreen";
import { ReviewScreen } from "../review/ReviewScreen";
import { AppShell } from "../shell/AppShell";

// Code-based routes (no file-based route generation). Every screen renders
// inside the layout shell. Stage paths match STAGES in src/shell/stages.ts.
const rootRoute = createRootRouteWithContext<RouterContext>()({
  component: Root,
});

function Root() {
  return (
    <>
      <AppShell />
      <LaunchWatcher />
    </>
  );
}

// The app opens where the user left off, and on Overview on a first run (1.13,
// "Launch"; src/launch). Overview is also where it moves to when a scan or a
// read has found something new.
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: async ({ context }) => {
    const { path, crate } = await launchScreen(context);
    if (path === "/crates" && crate !== undefined) throw redirect({ to: "/crates", search: { crate } });
    throw redirect({ to: path });
  },
});

const overviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/overview",
  component: OverviewScreen,
});

const reviewRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/review",
  component: ReviewScreen,
});

const allMusicRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/all-music",
  component: AllMusicScreen,
});

const libraryRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/library",
  component: LibraryScreen,
});

/**
 * `/crates?crate=ID` chooses a crate or folder, for the screen and the
 * sidebar's tree alike. An id that isn't a whole number above zero is no
 * choice at all, and one that names nothing (a crate since deleted) shows
 * nothing chosen: neither is an error.
 */
export function validateCratesSearch(search: Record<string, unknown>): { crate?: number } {
  const id = search.crate;
  return isCrateId(id) ? { crate: id } : {};
}

function CratesRoute() {
  const { crate } = cratesRoute.useSearch();
  const navigate = useNavigate();
  return (
    <CratesScreen
      chosen={crate ?? null}
      onChoose={(id) =>
        void navigate({ to: "/crates", search: id === null ? {} : { crate: id }, replace: true })
      }
    />
  );
}

const cratesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/crates",
  validateSearch: validateCratesSearch,
  component: CratesRoute,
});

// Not a stage and not in the sidebar: the after-send lists on their own (1aF-2).
const afterSendRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/after-send",
  component: AfterSendScreen,
});

// One route per line, with a trailing comma, so lanes adding routes each add
// a line and don't conflict.
const routeTree = rootRoute.addChildren([
  indexRoute,
  overviewRoute,
  reviewRoute,
  allMusicRoute,
  libraryRoute,
  cratesRoute,
  afterSendRoute,
]);

/**
 * Tests pass a memory history; the app uses the browser's. The query client
 * lets the launch check that a remembered crate still exists.
 */
export function createAppRouter(options: { history?: RouterHistory; queryClient?: QueryClient } = {}) {
  return createRouter({
    routeTree,
    history: options.history,
    context: { launch: { reopened: false }, queryClient: options.queryClient },
  });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}

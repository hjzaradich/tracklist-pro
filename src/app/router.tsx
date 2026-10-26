import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  useNavigate,
  type RouterHistory,
} from "@tanstack/react-router";
import { AfterSendScreen } from "../afterSend/AfterSendScreen";
import { AllMusicScreen } from "../allMusic/AllMusicScreen";
import { CratesScreen } from "../crates/CratesScreen";
import { LibraryScreen } from "../library/LibraryScreen";
import { OverviewScreen } from "../overview/OverviewScreen";
import { ReviewScreen } from "../review/ReviewScreen";
import { AppShell } from "../shell/AppShell";
import { HOME_STAGE_PATH } from "../shell/stages";

// Code-based routes (no file-based route generation). Every screen renders
// inside the layout shell. Stage paths match STAGES in src/shell/stages.ts.
const rootRoute = createRootRoute({
  component: AppShell,
});

// The app opens on Overview; "reopen where you left off" comes later (1.13).
const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  beforeLoad: () => {
    throw redirect({ to: HOME_STAGE_PATH });
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
  return typeof id === "number" && Number.isSafeInteger(id) && id > 0 ? { crate: id } : {};
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

/** Tests pass a memory history; the app uses the browser's. */
export function createAppRouter(options: { history?: RouterHistory } = {}) {
  return createRouter({ routeTree, history: options.history });
}

declare module "@tanstack/react-router" {
  interface Register {
    router: ReturnType<typeof createAppRouter>;
  }
}

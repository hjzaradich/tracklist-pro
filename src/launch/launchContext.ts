import type { QueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands } from "../bindings";
import { CRATES_LIST_QUERY_KEY } from "../crates/useCrates";
import { HOME_STAGE_PATH } from "../shell/stages";
import { readSavedScreen } from "./savedScreen";

/** What the router knows about this launch. */
export interface LaunchContext {
  /** Set when the app opened on a remembered screen other than Overview. */
  reopened: boolean;
}

export interface RouterContext {
  launch: LaunchContext;
  /** Lets the launch check that a remembered crate still exists; without it the crate is trusted. */
  queryClient?: QueryClient;
}

export const DEFAULT_SCREEN = { path: HOME_STAGE_PATH };

/**
 * Where the app opens: the screen the user was last on, else the default
 * (Overview, as on a first run). A remembered crate that was deleted since,
 * or that can't be checked, opens the default screen, quietly.
 */
export async function launchScreen(context: RouterContext): Promise<{ path: string; crate?: number }> {
  const saved = readSavedScreen();
  if (saved === null) return DEFAULT_SCREEN;
  if (saved.crate !== undefined && context.queryClient !== undefined) {
    try {
      const crates = await context.queryClient.fetchQuery({
        queryKey: CRATES_LIST_QUERY_KEY,
        queryFn: () => unwrap(commands.listCrates()),
      });
      if (!crates.some((crate) => crate.id === saved.crate)) return DEFAULT_SCREEN;
    } catch {
      return DEFAULT_SCREEN;
    }
  }
  context.launch.reopened = saved.path !== DEFAULT_SCREEN.path;
  return saved;
}

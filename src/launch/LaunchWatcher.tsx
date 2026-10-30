import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { useRouter, type AnyRouter } from "@tanstack/react-router";
import { useEffect } from "react";
import { unwrap } from "../api/errors";
import { commands, type Overview, type ReviewCounts } from "../bindings";
import { LIBRARY_SUMMARY_QUERY_KEY } from "../library/useLibrarySummary";
import { COUNTING_INTERVAL_MS, isCounting, OVERVIEW_QUERY_KEY } from "../overview/useOverview";
import { REVIEW_COUNTS_QUERY_KEY } from "../review/useReview";
import type { LaunchContext } from "./launchContext";
import { screenAt, writeSavedScreen, type SavedScreen } from "./savedScreen";
import { forgetSeen, hasNews, lowest, newsNumbers, readSeen, writeSeen, type Seen } from "./seenNews";

/** How long a move waits for the next one before the screen is remembered. */
export const SCREEN_SAVE_DELAY_MS = 400;

/** How long after launch the app keeps looking for news (a scan can take a while). */
export const NEWS_WINDOW_MS = 120_000;

/**
 * Everything the app does about launch (ROADMAP 1.13) once it is running: it
 * remembers the screen as the user moves, opens on Overview when a scan or
 * read has found something new, and keeps the record of what the user has
 * seen. Draws nothing. Call it once, in the shell.
 */
export function LaunchWatcher() {
  const router = useRouter();
  const queryClient = useQueryClient();
  useEffect(() => rememberScreen(router), [router]);
  useEffect(() => keepWhatWasSeen(router, queryClient), [router, queryClient]);
  useEffect(() => openOnNews(router, queryClient), [router, queryClient]);
  return null;
}

function currentScreen(router: AnyRouter): SavedScreen | null {
  const { pathname, search } = router.state.location;
  return screenAt(pathname, search as Record<string, unknown>);
}

/** Remembers the screen, a moment after the user stops moving. Returns the stop function. */
export function rememberScreen(router: AnyRouter): () => void {
  let pending: SavedScreen | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    clearTimeout(timer);
    if (pending !== null) writeSavedScreen(pending);
    pending = null;
  };
  const stopListening = router.subscribe("onResolved", () => {
    const screen = currentScreen(router);
    if (screen === null) return;
    pending = screen;
    clearTimeout(timer);
    timer = setTimeout(flush, SCREEN_SAVE_DELAY_MS);
  });
  // Leaving the page writes the last move at once, where the webview says so;
  // if a close gives no word, the move of the last moments is lost and the app
  // reopens one screen back.
  window.addEventListener("pagehide", flush);
  return () => {
    stopListening();
    window.removeEventListener("pagehide", flush);
    flush();
  };
}

interface Numbers {
  numbers: Seen;
  /** Background work can still change them. */
  counting: boolean;
}

/** The numbers for the news, from what the cache holds; `null` until all are in. */
function cachedNumbers(queryClient: QueryClient): Numbers | null {
  const review = queryClient.getQueryData<ReviewCounts>(REVIEW_COUNTS_QUERY_KEY);
  const latest = queryClient
    .getQueryCache()
    .findAll({ queryKey: OVERVIEW_QUERY_KEY })
    .filter((query) => query.state.data !== undefined)
    .sort((a, b) => b.state.dataUpdatedAt - a.state.dataUpdatedAt)[0];
  const overview = latest?.state.data as Overview | undefined;
  if (review === undefined || overview === undefined) return null;
  return { numbers: newsNumbers(overview, review), counting: isCounting(overview) };
}

/**
 * Keeps what the user has seen. With the Overview on screen and every number
 * in and none still being counted, they have seen all of it. Anywhere else
 * a number can only lower what is kept: the user worked it down, and when
 * it grows again that is news. Without a record the first complete numbers
 * start one. The record is only good while the Library holds the tracks it
 * held then (see `seenNews.ts`): an empty Library drops it, and numbers
 * from a Library of another size are not compared with it (the launch drops
 * it; on the Overview it is replaced). Returns the stop function.
 */
export function keepWhatWasSeen(router: AnyRouter, queryClient: QueryClient): () => void {
  const note = () => {
    const found = cachedNumbers(queryClient);
    if (found === null || found.counting) return;
    const seen = readSeen();
    const onOverview = router.state.location.pathname === "/overview";
    if (seen === null || (onOverview && seen.tracks !== found.numbers.tracks)) {
      writeSeen(found.numbers);
      return;
    }
    if (seen.tracks !== found.numbers.tracks) return;
    const next = onOverview ? found.numbers : lowest(seen, found.numbers);
    if (differs(seen, next)) writeSeen(next);
  };
  const stopCache = queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || event.action.type !== "success") return;
    const key = event.query.queryKey;
    if (key[0] === OVERVIEW_QUERY_KEY[0] || key[0] === REVIEW_COUNTS_QUERY_KEY[0]) note();
    // A Library with nothing in it ("Start fresh", or every track removed) has no numbers to compare.
    const emptied =
      key.length === LIBRARY_SUMMARY_QUERY_KEY.length &&
      key.every((part: unknown, i: number) => part === LIBRARY_SUMMARY_QUERY_KEY[i]) &&
      (event.query.state.data as { tracks?: number } | undefined)?.tracks === 0;
    if (emptied) forgetSeen();
  });
  const stopRouter = router.subscribe("onResolved", note);
  return () => {
    stopCache();
    stopRouter();
  };
}

function differs(a: Seen, b: Seen): boolean {
  return JSON.stringify(a) !== JSON.stringify(b);
}

async function fetchNumbers(queryClient: QueryClient): Promise<Numbers | null> {
  const library = await queryClient.fetchQuery({
    queryKey: LIBRARY_SUMMARY_QUERY_KEY,
    queryFn: () => unwrap(commands.librarySummary()),
  });
  if (library.tracks === 0) return null;
  const [overview, review] = await Promise.all([
    queryClient.fetchQuery({
      queryKey: [...OVERVIEW_QUERY_KEY, library.tracks],
      queryFn: (): Promise<Overview> => unwrap(commands.overview()),
    }),
    queryClient.fetchQuery({
      queryKey: REVIEW_COUNTS_QUERY_KEY,
      queryFn: (): Promise<ReviewCounts> => unwrap(commands.reviewCounts()),
    }),
  ]);
  return { numbers: newsNumbers(overview, review), counting: isCounting(overview) };
}

/**
 * When the app reopened on a remembered screen and a scan or read has found
 * something new (see `seenNews.ts`), moves to Overview, as long as the user
 * has done nothing yet. The first navigation, key or pointer press after
 * launch ends the watch for good (going away and coming back included, and an
 * open dialog or edit, which takes input). While numbers are still being
 * counted it looks again, for {@link NEWS_WINDOW_MS}. A number it can't read
 * is no news. Returns the stop function.
 */
export function openOnNews(router: AnyRouter, queryClient: QueryClient): () => void {
  const { launch } = router.options.context as { launch: LaunchContext };
  if (!launch.reopened) return () => {};
  const seen = readSeen();
  if (seen === null) return () => {};
  const giveUpAt = Date.now() + NEWS_WINDOW_MS;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const stop = () => {
    stopped = true;
    clearTimeout(timer);
    window.removeEventListener("keydown", stop, true);
    window.removeEventListener("pointerdown", stop, true);
    stopNavigation();
  };
  window.addEventListener("keydown", stop, true);
  window.addEventListener("pointerdown", stop, true);
  const stopNavigation = router.subscribe("onBeforeNavigate", (event) => {
    if (event.toLocation.href !== event.fromLocation?.href) stop();
  });

  const look = async () => {
    try {
      const found = await fetchNumbers(queryClient);
      if (stopped) return;
      // An empty Library, or one of another size than when the record was made, has nothing to compare.
      if (found === null || found.numbers.tracks !== seen.tracks) {
        // Start a record from these numbers if they are complete; else the first complete ones will.
        if (found !== null && found.numbers.tracks > 0 && !found.counting) writeSeen(found.numbers);
        else forgetSeen();
        stop();
      } else if (hasNews(found.numbers, seen)) {
        stop();
        await router.navigate({ to: "/overview", replace: true });
      } else if (found.counting && Date.now() < giveUpAt) {
        timer = setTimeout(() => void look(), COUNTING_INTERVAL_MS);
      } else {
        stop();
      }
    } catch {
      // Not read: nothing is known to be new.
      stop();
    }
  };
  void look();
  return stop;
}

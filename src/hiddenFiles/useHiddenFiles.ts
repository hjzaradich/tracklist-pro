import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ALL_MUSIC_QUERY_KEY } from "../allMusic/useAllMusic";
import { unwrap } from "../api/errors";
import { commands, type HiddenPage, type TableSort } from "../bindings";
import { WINDOW } from "../trackTable/useTrackRows";

/**
 * The Hidden filter's lists (1cB-5) and the "N hidden files match" count
 * (1cB-6) keep their queries under All music's key, so whatever reloads All
 * music (a track added, a job ending, Undo) reloads them too.
 */
export const HIDDEN_FILES_QUERY_KEY = [...ALL_MUSIC_QUERY_KEY, "hidden"] as const;

/** A search text, as the backend takes it: blank is no search. */
const searchOf = (search: string) => (search.trim() === "" ? null : search);

/**
 * The hidden files, a window at a time like the track table's: the first
 * window carries the count, and "Show more" asks for the next. Nothing is
 * ever fetched for the whole list.
 */
export function useHiddenFiles(search: string, sort: TableSort | null) {
  const query = useInfiniteQuery({
    queryKey: [...HIDDEN_FILES_QUERY_KEY, "page", searchOf(search), sort],
    initialPageParam: 0,
    queryFn: ({ pageParam }): Promise<HiddenPage> =>
      unwrap(
        commands.hiddenFilesPage({
          search: searchOf(search),
          sort,
          offset: pageParam,
          limit: WINDOW,
          withTotal: pageParam === 0,
        }),
      ),
    getNextPageParam: (last, all) => {
      const loaded = all.reduce((n, page) => n + page.rows.length, 0);
      const total = all[0]?.total ?? 0;
      return last.rows.length > 0 && loaded < total ? loaded : undefined;
    },
  });
  const pages = query.data?.pages ?? [];
  return {
    total: pages[0]?.total ?? undefined,
    rows: pages.flatMap((page) => page.rows),
    behind: pages[0]?.behind ?? false,
    hasMore: query.hasNextPage,
    loadMore: () => void query.fetchNextPage(),
    loadingMore: query.isFetchingNextPage,
    error: query.error,
  };
}

/** "Show again": the file is shown in the lists again; Undo (top bar) hides it as it was. */
export function useShowAgain() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (fileId: number) => unwrap(commands.showFileAgain(fileId)),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ALL_MUSIC_QUERY_KEY }),
  });
}

/** How many hidden files `search` matches that the results don't show already. */
export function useHiddenMatches(search: string) {
  const text = searchOf(search);
  return useQuery({
    queryKey: [...HIDDEN_FILES_QUERY_KEY, "matches", text],
    queryFn: () => unwrap(commands.hiddenFilesMatching(text ?? "")),
    enabled: text !== null,
  });
}

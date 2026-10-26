import { keepPreviousData, useQueries, useQuery } from "@tanstack/react-query";
import { useCallback, useMemo, useState } from "react";
import { unwrap } from "../api/errors";
import { commands, type TablePage, type TableQuery, type TableRow } from "../bindings";

/**
 * How many rows one request asks for. A window is one indexed query on the
 * backend, so this is small enough to come back at once and large enough
 * that a flick of the wheel is a request or two.
 */
export const WINDOW = 200;

/** Rows fetched ahead of the ones on screen, before and after. */
const AHEAD = { before: 50, after: 100 };

/** How long a window nobody is looking at is kept, in milliseconds. */
const KEEP_MS = 30_000;

/** What the table is asked for, without the window. */
export type RowsQuery = Omit<TableQuery, "offset" | "limit" | "withTotal">;

/** The windows of rows the table needs, and what it knows of them. */
export interface TrackRows {
  /** How many rows there are in all; the last count seen while a new query's first window is on its way. */
  total: number | undefined;
  /** The row at `index`, if its window has arrived. */
  getRow: (index: number) => TableRow | undefined;
  /** Tell it which rows are on screen: it fetches their windows and the ones beside them. */
  setRange: (first: number, last: number) => void;
  /** The first window of the query is still on its way. */
  loading: boolean;
  /** The index job hasn't caught up with some change. */
  behind: boolean;
  error: unknown;
}

/**
 * The rows of `query`, a window at a time, as the table scrolls
 * (ROADMAP 1.11 "The track table"). Nothing is ever fetched for the whole
 * list: only the windows around the rows on screen, each its own query under
 * `keyPrefix`, so the lists' existing invalidations (a track added to the
 * Library, a job ending) fetch the windows on screen again.
 */
export function useTrackRows(keyPrefix: readonly unknown[], query: RowsQuery): TrackRows {
  const [range, setRangeState] = useState({ first: 0, last: 0 });
  const setRange = useCallback((first: number, last: number) => {
    setRangeState((now) => (now.first === first && now.last === last ? now : { first, last }));
  }, []);

  const fetchWindow = (window: number): Promise<TablePage> =>
    unwrap(
      commands.trackTablePage({
        ...query,
        offset: window * WINDOW,
        limit: WINDOW,
        // The count costs about as much as a window, so only the first
        // window asks for it.
        withTotal: window === 0,
      }),
    );
  const keyOf = (window: number) => [...keyPrefix, "window", query, window] as const;

  // The first window carries the count. It keeps the last count while a new
  // query's own is on its way, so the scrollbar doesn't jump; its rows are
  // the old query's, and aren't shown.
  const head = useQuery({
    queryKey: keyOf(0),
    queryFn: () => fetchWindow(0),
    placeholderData: keepPreviousData,
    gcTime: KEEP_MS,
  });

  const others = useMemo(() => {
    const first = Math.max(0, Math.floor((range.first - AHEAD.before) / WINDOW));
    const last = Math.floor((range.last + AHEAD.after) / WINDOW);
    const wanted: number[] = [];
    for (let w = Math.max(1, first); w <= last; w++) wanted.push(w);
    return wanted;
  }, [range]);
  const rest = useQueries({
    queries: others.map((window) => ({
      queryKey: keyOf(window),
      queryFn: () => fetchWindow(window),
      gcTime: KEEP_MS,
    })),
  });

  const pages = new Map<number, TablePage>();
  others.forEach((window, i) => {
    const data = rest[i]?.data;
    if (data !== undefined) pages.set(window, data);
  });
  if (head.data !== undefined && !head.isPlaceholderData) pages.set(0, head.data);

  return {
    total: head.data?.total ?? undefined,
    getRow: (index) => pages.get(Math.floor(index / WINDOW))?.rows[index % WINDOW],
    setRange,
    loading: head.data === undefined || head.isPlaceholderData,
    behind: head.data?.behind ?? false,
    error: head.error ?? rest.find((r) => r.error != null)?.error ?? null,
  };
}

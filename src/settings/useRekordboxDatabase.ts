import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useState } from "react";
import { unwrap } from "../api/errors";
import { commands, type RekordboxDatabase } from "../bindings";

// The optional reader for rekordbox's library database (2C-R1, ROADMAP 2.7).
// Off by default. The key the user types is kept only in the backend's memory,
// for this run of the app, and is never sent back.

export const DATABASE_FEATURE_QUERY_KEY = ["setting", "feature", "rekordboxDatabase"] as const;
export const DATABASE_QUERY_KEY = ["rekordbox", "database"] as const;
/** How often the reader's state is asked again while a read is under way. */
export const READING_INTERVAL_MS = 1_000;

/** Whether the user turned the reader on. */
export function useDatabaseFeature() {
  return useQuery({
    queryKey: DATABASE_FEATURE_QUERY_KEY,
    queryFn: () => unwrap(commands.featureEnabled("rekordboxDatabase")),
  });
}

/** Turns the reader on or off. Turning it off also forgets the key. */
export function useSetDatabaseFeature() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (on: boolean) => {
      await unwrap(commands.setFeatureEnabled("rekordboxDatabase", on));
      if (!on) await unwrap(commands.setRekordboxDatabaseKey(""));
      return on;
    },
    onSuccess: (on) => queryClient.setQueryData(DATABASE_FEATURE_QUERY_KEY, on),
    onSettled: () => queryClient.invalidateQueries({ queryKey: DATABASE_QUERY_KEY }),
  });
}

/**
 * The reader's state, only asked for while it is on. While a read begun at
 * `readingSince` (the state's {@link databaseReadMarker} then) hasn't ended,
 * it's asked every {@link READING_INTERVAL_MS}.
 */
export function useDatabaseState(on: boolean, readingSince: string | null) {
  return useQuery({
    queryKey: DATABASE_QUERY_KEY,
    queryFn: () => unwrap(commands.rekordboxDatabase()),
    enabled: on,
    refetchInterval: (query) =>
      readingSince !== null && databaseReadMarker(query.state.data) === readingSince
        ? READING_INTERVAL_MS
        : false,
  });
}

/**
 * Keeps a typed key for this run of the app. Deliberately not a mutation: a
 * mutation's cache holds on to what it was called with, and the key must not
 * outlive the call anywhere in the frontend.
 */
export function useSetDatabaseKey() {
  const queryClient = useQueryClient();
  const [isPending, setPending] = useState(false);
  const send = useCallback(
    async (key: string) => {
      setPending(true);
      try {
        return await unwrap(commands.setRekordboxDatabaseKey(key));
      } finally {
        setPending(false);
        void queryClient.invalidateQueries({ queryKey: DATABASE_QUERY_KEY });
      }
    },
    [queryClient],
  );
  return { send, isPending };
}

/** Reads the database at `path`, as a job, unless the backend refuses. */
export function useReadDatabase() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => unwrap(commands.readRekordboxDatabase(path)),
    onSettled: () => queryClient.invalidateQueries({ queryKey: DATABASE_QUERY_KEY }),
  });
}

/** What changes when a read ends, either way: the last read's time or the failure count. */
export function databaseReadMarker(state: RekordboxDatabase | undefined): string {
  return `${state?.lastRead?.readAt ?? ""}|${state?.failures ?? 0}`;
}

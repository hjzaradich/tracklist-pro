import { useCallback, useEffect } from "react";
import { create } from "zustand";
import type { TableRow } from "../bindings";

/**
 * The track the Details panel shows (ROADMAP 1.1, Layout): the selected row
 * of the track table on screen. A table reports its selection here
 * ({@link useReportSelection}); the panel reads it. `tables` is how many
 * tables are on screen, so the shell draws the panel only beside one.
 */
export const useSelectedTrack = create<{
  row: TableRow | null;
  tables: number;
  select: (row: TableRow | null) => void;
  tableMounted: () => void;
  tableUnmounted: () => void;
}>((set) => ({
  row: null,
  tables: 0,
  select: (row) => set({ row }),
  tableMounted: () => set((state) => ({ tables: state.tables + 1 })),
  tableUnmounted: () => set((state) => ({ tables: Math.max(0, state.tables - 1), row: null })),
}));

/** Whether a track table is on screen, so the Details panel has a track to show. */
export function useTableOnScreen(): boolean {
  return useSelectedTrack((state) => state.tables > 0);
}

/**
 * For a track table: reports the selected row to the Details panel, and
 * calls `onSelectionChange` as well. The table's selection goes when it does.
 */
export function useReportSelection(onSelectionChange?: (row: TableRow | null) => void) {
  const select = useSelectedTrack((state) => state.select);
  useEffect(() => {
    const { tableMounted, tableUnmounted } = useSelectedTrack.getState();
    tableMounted();
    return tableUnmounted;
  }, []);
  return useCallback(
    (row: TableRow | null) => {
      select(row);
      onSelectionChange?.(row);
    },
    [select, onSelectionChange],
  );
}

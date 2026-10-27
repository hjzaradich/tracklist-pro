import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { unwrap } from "../api/errors";
import { commands, type TableColumns } from "../bindings";
import { resolveColumns, storedFrom } from "./columnChoice";
import type { ColumnId, TableScreen } from "./columns";

// The columns a screen shows are the user's choice, kept in the backend's
// `ui_setting` table (1cA-5). It is not Library data: no operation, no undo.

const queryKey = (screen: TableScreen) => ["setting", "table_columns", screen] as const;

/** What a screen's table shows, and how the column editor changes it. */
export interface TableColumnsChoice {
  /** The columns to draw, in order: the user's choice, else the screen's default. */
  columns: readonly ColumnId[];
  /** The choice has been read (or couldn't be): drawing before that would flash the default. */
  ready: boolean;
  /** Show these columns from now on. */
  change: (columns: readonly ColumnId[]) => void;
  /** Go back to the screen's default columns. */
  reset: () => void;
}

/**
 * The columns the `screen` shows. Every crate shares the one choice of the
 * "crate" screen. A choice that can't be read is no choice, so the default
 * shows; one that can't be saved is read again, so the screen shows what is
 * kept.
 */
export function useTableColumns(screen: TableScreen): TableColumnsChoice {
  const queryClient = useQueryClient();
  const key = queryKey(screen);
  const choice = useQuery({
    queryKey: key,
    queryFn: () => unwrap(commands.tableColumns(screen)),
  });
  const columns = useMemo(() => resolveColumns(screen, choice.data), [screen, choice.data]);

  const save = useMutation({
    mutationFn: (next: TableColumns | null) => unwrap(commands.setTableColumns(screen, next)),
    // The table follows at once; the backend catches up.
    onMutate: (next) => queryClient.setQueryData(key, next),
    onError: () => queryClient.invalidateQueries({ queryKey: key }),
  });

  return {
    columns,
    ready: !choice.isPending,
    change: (next) => save.mutate(storedFrom(next)),
    reset: () => save.mutate(null),
  };
}

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../../api/errors";
import { commands, type SmartRule, type TableQuery } from "../../bindings";
import { CRATES_QUERY_KEY, invalidateCrates, type NewIn } from "../useCrates";

/** Every smart crate query starts with this, under the crate queries. */
export const SMART_QUERY_KEY = [...CRATES_QUERY_KEY, "smart"] as const;

/** A smart crate's stored rule; `null` when it doesn't read (it then matches nothing). */
export function useSmartRule(id: number) {
  return useQuery({
    queryKey: [...SMART_QUERY_KEY, "rule", id] as const,
    queryFn: (): Promise<SmartRule | null> => unwrap(commands.smartCrateRule(id)),
  });
}

/**
 * The live count: how many tracks the crate would hold with `rule` (its
 * always and never lists included). A window of no rows, with the total.
 */
export function useRuleCount(id: number, rule: SmartRule) {
  const query: TableQuery = {
    source: { kind: "smart", id, rule },
    search: null,
    filters: [],
    sort: null,
    offset: 0,
    limit: 0,
    withTotal: true,
  };
  return useQuery({
    queryKey: [...SMART_QUERY_KEY, "count", id, JSON.stringify(rule)] as const,
    queryFn: async (): Promise<number> => (await unwrap(commands.trackTablePage(query))).total ?? 0,
    placeholderData: (previous) => previous,
  });
}

/** Makes a smart crate with `rule`; resolves to its id. */
export function useCreateSmartCrate() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, parent, rule }: NewIn & { rule: SmartRule }) =>
      unwrap(commands.createSmartCrate(name, parent, rule)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

/** Saves a smart crate's rule; resolves to the operation, or `null` when unchanged. */
export function useSetSmartRule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, rule }: { id: number; rule: SmartRule }) => unwrap(commands.setSmartCrateRule(id, rule)),
    onSettled: () => invalidateCrates(queryClient),
  });
}

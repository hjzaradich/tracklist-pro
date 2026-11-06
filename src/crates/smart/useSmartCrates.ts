import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
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

/** How long the draft rule rests before the live count asks about it. */
export const COUNT_DELAY_MS = 300;

/** `value`, once it has stayed the same for `delay` ms. */
function useRested<T>(value: T, delay: number): T {
  const [rested, setRested] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setRested(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return rested;
}

/** A window of no rows, with the total: the count of a smart crate (`rule` null: as saved). */
function countQuery(id: number, rule: SmartRule | null): TableQuery {
  return {
    source: { kind: "smart", id, rule },
    search: null,
    filters: [],
    sort: null,
    offset: 0,
    limit: 0,
    withTotal: true,
  };
}

/**
 * The live count: how many tracks the crate would hold with `rule` (its
 * always and never lists included). It asks once the rule has rested, so
 * typing "126" is one question, not three.
 */
export function useRuleCount(id: number, rule: SmartRule) {
  const rested = useRested(JSON.stringify(rule), COUNT_DELAY_MS);
  return useQuery({
    queryKey: [...SMART_QUERY_KEY, "count", id, rested] as const,
    queryFn: async (): Promise<number> =>
      (await unwrap(commands.trackTablePage(countQuery(id, JSON.parse(rested) as SmartRule)))).total ?? 0,
    placeholderData: (previous) => previous,
  });
}

/**
 * A smart crate's saved count, for the crate list. The list itself carries
 * none (counting is a scan of the Library per smart crate), so each row
 * asks for its own.
 */
export function useSmartCount(id: number) {
  return useQuery({
    queryKey: [...SMART_QUERY_KEY, "saved-count", id] as const,
    queryFn: async (): Promise<number> =>
      (await unwrap(commands.trackTablePage(countQuery(id, null)))).total ?? 0,
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

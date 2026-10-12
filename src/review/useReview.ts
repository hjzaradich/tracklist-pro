import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import {
  commands,
  type ReviewComparison,
  type ReviewCounts,
  type ReviewGroup,
  type ReviewItemId,
  type ReviewPage,
} from "../bindings";

/** Invalidate these when background work ends (see useReloadListsWhenJobsEnd). */
export const REVIEW_COUNTS_QUERY_KEY = ["reviewCounts"] as const;
export const REVIEW_ITEMS_QUERY_KEY = ["reviewItems"] as const;
export const REVIEW_ITEM_QUERY_KEY = ["reviewItem"] as const;
export const REVIEW_QUERY_KEYS = [
  REVIEW_COUNTS_QUERY_KEY,
  REVIEW_ITEMS_QUERY_KEY,
  REVIEW_ITEM_QUERY_KEY,
] as const;

/** How many items a group loads at a time. */
export const PAGE_SIZE = 100;

/** The groups, in the order the queue shows them. */
export const GROUPS = ["needsCloserLook", "clearBestFile", "qualityProblems"] as const satisfies readonly ReviewGroup[];

/** One string for an item's id: its React key, and its query key. */
export function itemKey(id: ReviewItemId): string {
  return `${id.kind}:${id.a}:${id.b ?? ""}`;
}

/** How many items wait in each group (1bD-1). `total` is "how many need a decision". */
export function useReviewCounts() {
  return useQuery({
    queryKey: REVIEW_COUNTS_QUERY_KEY,
    queryFn: (): Promise<ReviewCounts> => unwrap(commands.reviewCounts()),
  });
}

/** A group's items, a page at a time, oldest first. */
export function useReviewItems(group: ReviewGroup, enabled: boolean) {
  return useInfiniteQuery({
    queryKey: [...REVIEW_ITEMS_QUERY_KEY, group],
    enabled,
    initialPageParam: 0,
    queryFn: ({ pageParam }): Promise<ReviewPage> =>
      unwrap(commands.reviewItems(group, pageParam, PAGE_SIZE)),
    getNextPageParam: (last, pages) => {
      const loaded = pages.reduce((n, page) => n + page.items.length, 0);
      return loaded < last.total && last.items.length > 0 ? loaded : undefined;
    },
  });
}

/** The comparison for one item: `null` once it is no longer in the queue. */
export function useReviewItem(id: ReviewItemId | null) {
  return useQuery({
    queryKey: [...REVIEW_ITEM_QUERY_KEY, id === null ? null : itemKey(id)],
    enabled: id !== null,
    queryFn: (): Promise<ReviewComparison | null> =>
      id === null ? Promise.resolve(null) : unwrap(commands.reviewItem(id)),
  });
}

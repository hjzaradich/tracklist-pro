import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useWaitingOn } from "../activity/useWaitingOn";
import { errorMessage } from "../api/errors";
import type { ReviewItemId } from "../bindings";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { AcceptedLine } from "./AcceptedLine";
import { MissingList } from "./MissingList";
import { ReviewComparison } from "./ReviewComparison";
import { ReviewQueue, type QueueGroup } from "./ReviewQueue";
import { useAddMissingFolder } from "./useAddMissingFolder";
import { useMissingTracks } from "./useMissingTracks";
import {
  GROUPS,
  itemKey,
  REVIEW_QUERY_KEYS,
  useReviewCounts,
  useReviewItem,
  useReviewItems,
} from "./useReview";
import { useAcceptAll, useAcceptAllItems } from "./useReviewActions";
import styles from "./ReviewScreen.module.css";

/** The background work that decides which rekordbox tracks have no file. */
const DECIDES_MISSING = ["read_rekordbox", "relink"] as const;

/**
 * The item after `current` for auto-advance (1bE-2): the next one in the same
 * group, else the first one of the next group that has any. `null` when there
 * is none (the screen then shows the first item left, or the empty state).
 */
export function itemAfter(groups: QueueGroup[], current: ReviewItemId | null): ReviewItemId | null {
  if (current === null) return null;
  const key = itemKey(current);
  const at = groups.findIndex((g) => g.items.some((item) => itemKey(item.id) === key));
  if (at < 0) return null;
  const inGroup = groups[at]!.items;
  const next = inGroup[inGroup.findIndex((item) => itemKey(item.id) === key) + 1];
  if (next) return next.id;
  return groups.slice(at + 1).find((g) => g.items.length > 0)?.items[0]?.id ?? null;
}

/**
 * Review (1bD): the queue of decisions on the left, grouped, and the
 * comparison of the selected item on the right, then the Missing list as its
 * own section (nothing to compare there: it has no file). The actions for an
 * item are under its comparison (`ReviewActions`), and "Accept all N" is at
 * the "Clearly best file" heading. After "Accept all", a line at the top says
 * what it switched (`AcceptedLine`). After a decision the next item loads
 * (`itemAfter`).
 */
export function ReviewScreen() {
  const { t } = useTranslation("review");
  const queryClient = useQueryClient();
  const counts = useReviewCounts();
  const needs = useReviewItems("needsCloserLook", (counts.data?.needsCloserLook ?? 0) > 0);
  const clear = useReviewItems("clearBestFile", (counts.data?.clearBestFile ?? 0) > 0);
  const quality = useReviewItems("qualityProblems", (counts.data?.qualityProblems ?? 0) > 0);
  const lists = { needsCloserLook: needs, clearBestFile: clear, qualityProblems: quality };
  // "Accept all N": N is how many items it would switch, and a click sends
  // exactly those items.
  const acceptItems = useAcceptAllItems((counts.data?.clearBestFile ?? 0) > 0);
  const accept = useAcceptAll();

  const groups: QueueGroup[] = GROUPS.flatMap((group) => {
    const count = counts.data?.[group] ?? 0;
    const list = lists[group];
    if (count === 0) return [];
    return [
      {
        group,
        count,
        items: list.data?.pages.flatMap((page) => page.items) ?? [],
        hasMore: list.hasNextPage,
        loadingMore: list.isFetchingNextPage,
        onLoadMore: () => void list.fetchNextPage(),
        acceptAll:
          group === "clearBestFile"
            ? {
                count: acceptItems.data?.length ?? 0,
                pending: accept.isPending,
                onAccept: () => accept.mutate(acceptItems.data ?? []),
              }
            : undefined,
      },
    ];
  });

  // The selected item, if it is still loaded; else the first one.
  const [chosen, setChosen] = useState<ReviewItemId | null>(null);
  const loaded = groups.flatMap((g) => g.items);
  const selected: ReviewItemId | null =
    loaded.find((item) => chosen !== null && itemKey(item.id) === itemKey(chosen))?.id ??
    loaded[0]?.id ??
    null;
  const comparison = useReviewItem(selected);
  // "More details" stays on from one item to the next.
  const [details, setDetails] = useState(false);
  // Where to go when the selected item is decided: the next one, worked out
  // from the queue as it is now.
  const after = itemAfter(groups, selected);

  // An item answered between two reads is gone: ask for the queue again.
  const gone = comparison.data === null && comparison.isSuccess;
  useEffect(() => {
    if (!gone) return;
    for (const queryKey of REVIEW_QUERY_KEYS) void queryClient.invalidateQueries({ queryKey });
  }, [gone, queryClient]);

  const missing = useMissingTracks();
  const add = useAddMissingFolder();
  const failure = missing.error ?? add.error;
  const waiting = useWaitingOn(DECIDES_MISSING);
  const problem = counts.error ?? needs.error ?? clear.error ?? quality.error;

  return (
    <StageScreen stage="review">
      <div className={styles.content}>
        <AcceptedLine />
        {problem ? (
          <p role="alert" className={styles.error}>
            {errorMessage(problem)}
          </p>
        ) : counts.data === undefined ? null : counts.data.total === 0 ? (
          <EmptyState>{t("queue.empty")}</EmptyState>
        ) : (
          <div className={styles.panes}>
            <div className={styles.left}>
              {accept.error && (
                <p role="alert" className={styles.error}>
                  {errorMessage(accept.error)}
                </p>
              )}
              <ReviewQueue groups={groups} selected={selected} onSelect={setChosen} />
            </div>
            <div className={styles.right}>
              {comparison.isError ? (
                <p role="alert" className={styles.error}>
                  {errorMessage(comparison.error)}
                </p>
              ) : comparison.data ? (
                <ReviewComparison
                  comparison={comparison.data}
                  details={details}
                  onDetails={setDetails}
                  onDecided={() => setChosen(after)}
                />
              ) : selected === null ? (
                <EmptyState>{t("compare.none")}</EmptyState>
              ) : null}
            </div>
          </div>
        )}
        <MissingList
          list={missing.data}
          problem={failure ? errorMessage(failure) : undefined}
          adding={add.isPending}
          waiting={waiting}
          onAddFolder={(folder) => add.mutate(folder)}
        />
      </div>
    </StageScreen>
  );
}

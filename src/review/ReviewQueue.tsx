import { useId, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import type { ReviewGroup, ReviewItem, ReviewItemId, ReviewTrackName } from "../bindings";
import { itemKey } from "./useReview";
import styles from "./ReviewQueue.module.css";

/** A track as the queue and the comparison name it: "Artist - Title". */
export function trackLabel(t: TFunction<"review">, track: ReviewTrackName): string {
  const title = track.title ?? t("item.untitled");
  return track.artist === null ? title : t("item.trackName", { artist: track.artist, title });
}

/** What the queue shows of one group. */
export interface QueueGroup {
  group: ReviewGroup;
  /** How many items the group has in all. */
  count: number;
  /** The items loaded so far. */
  items: ReviewItem[];
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  /**
   * "Accept all N" for the group, if it has one ("Clearly best file"): how many
   * items it would switch, and what to do on a click. Left out, or with a
   * count of zero, there is no button.
   */
  acceptAll?: { count: number; pending: boolean; onAccept: () => void };
}

/**
 * The queue (1bD-1): one list per non-empty group, each with its heading and
 * how many are left. Each list is one tab stop (the selected item, else the
 * first); Up, Down, Home and End move through it and select as they go. The
 * 1bE keyboard (Enter, number keys, S) joins here. A group's heading carries
 * its "Accept all N" button, when it has one.
 */
export function ReviewQueue({
  groups,
  selected,
  onSelect,
}: {
  groups: QueueGroup[];
  selected: ReviewItemId | null;
  onSelect: (id: ReviewItemId) => void;
}) {
  return (
    <div className={styles.queue}>
      {groups.map((group) => (
        <GroupList key={group.group} group={group} selected={selected} onSelect={onSelect} />
      ))}
    </div>
  );
}

function GroupList({
  group,
  selected,
  onSelect,
}: {
  group: QueueGroup;
  selected: ReviewItemId | null;
  onSelect: (id: ReviewItemId) => void;
}) {
  const { t } = useTranslation("review");
  const headingId = useId();
  const selectedKey = selected === null ? null : itemKey(selected);
  const inGroup = group.items.some((item) => itemKey(item.id) === selectedKey);

  function move(event: KeyboardEvent<HTMLUListElement>) {
    const keys: Record<string, (at: number, last: number) => number> = {
      ArrowDown: (at, last) => Math.min(at + 1, last),
      ArrowUp: (at) => Math.max(at - 1, 0),
      Home: () => 0,
      End: (_, last) => last,
    };
    const step = keys[event.key];
    if (!step) return;
    const options = Array.from(event.currentTarget.querySelectorAll<HTMLElement>('[role="option"]'));
    const at = options.indexOf(document.activeElement as HTMLElement);
    const to = step(Math.max(at, 0), options.length - 1);
    event.preventDefault();
    options[to]?.focus();
    const item = group.items[to];
    if (item) onSelect(item.id);
  }

  return (
    <section className={styles.group} aria-labelledby={headingId}>
      <div className={styles.heading}>
        <h2 id={headingId} className={styles.title}>
          {t(`group.${group.group}`)}
        </h2>
        <span className={styles.count}>{t("queue.left", { count: group.count })}</span>
        {group.acceptAll !== undefined && group.acceptAll.count > 0 && (
          <button
            type="button"
            className={styles.more}
            disabled={group.acceptAll.pending}
            onClick={group.acceptAll.onAccept}
          >
            {t("queue.acceptAll", { count: group.acceptAll.count })}
          </button>
        )}
      </div>
      {/* The list is a listbox, named by its group's heading: one tab stop,
          arrow keys inside. */}
      <ul role="listbox" aria-labelledby={headingId} className={styles.list} onKeyDown={move}>
        {group.items.map((item, n) => {
          const isSelected = itemKey(item.id) === selectedKey;
          return (
            <li
              key={itemKey(item.id)}
              role="option"
              aria-selected={isSelected}
              tabIndex={isSelected || (!inGroup && n === 0) ? 0 : -1}
              className={styles.item}
              onClick={() => onSelect(item.id)}
            >
              <ItemName item={item} />
              <span className={styles.kind}>{t(`kind.${item.kind}`)}</span>
            </li>
          );
        })}
      </ul>
      {group.hasMore && (
        <button
          type="button"
          className={styles.more}
          disabled={group.loadingMore}
          onClick={group.onLoadMore}
        >
          {t("queue.more")}
        </button>
      )}
    </section>
  );
}

/** Each track on a line of its own, then "and N more" if the item has more tracks. */
function ItemName({ item }: { item: ReviewItem }) {
  const { t } = useTranslation("review");
  // The backend names the first few tracks and counts the rest.
  const extra = item.moreTracks;
  return (
    <span className={styles.names}>
      {item.tracks.map((track, n) => (
        <span key={n} className={styles.track}>
          {trackLabel(t, track)}
        </span>
      ))}
      {extra > 0 && <span className={styles.track}>{t("item.moreTracks", { count: extra })}</span>}
    </span>
  );
}

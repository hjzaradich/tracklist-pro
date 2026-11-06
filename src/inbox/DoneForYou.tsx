import { useTranslation } from "react-i18next";
import type { DoneForYou as Entry } from "../bindings";
import { reasonText, refusalText } from "./inboxText";
import styles from "./inbox.module.css";

/** The wish as the list names it: the title, with the version when it has one. */
function wishTitle(entry: Entry): string {
  return entry.wishVersion === null ? entry.wishTitle : `${entry.wishTitle} (${entry.wishVersion})`;
}

/**
 * The Inbox's "Done for you" group (ROADMAP 2.5): new files that were on a
 * wishlist. Each says which wishlist item it matched and how; one added for
 * you says so (the top bar's Undo takes the last one back), one left to you
 * says why. Nothing shows while there is nothing to show (or wishlists are off).
 */
export function DoneForYou({ entries }: { entries: readonly Entry[] }) {
  const { t } = useTranslation("inbox");
  if (entries.length === 0) return null;
  return (
    <section className={styles.batch} aria-labelledby="inbox-done-for-you">
      <div className={styles.batchHead}>
        <h2 id="inbox-done-for-you" className={styles.heading}>
          {t("doneForYou.title", { count: entries.length })}
        </h2>
      </div>
      <ul className={styles.list}>
        {entries.map((entry) => (
          <li key={entry.inboxItemId} className={styles.row} data-added={entry.added}>
            <div className={styles.rowMain}>
              <span className={styles.name}>{entry.fileName}</span>
              <span className={styles.label}>
                {entry.added
                  ? t("doneForYou.added")
                  : entry.refusal !== null
                    ? refusalText(t, entry.refusal)
                    : t("doneForYou.leftToYou")}
              </span>
              <span className={styles.reason}>
                {t("doneForYou.wish", {
                  wishlist: entry.wishlistName,
                  artist: entry.wishArtist,
                  title: wishTitle(entry),
                })}
              </span>
              <span className={styles.reason}>{reasonText(t, entry.reason)}</span>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

import { useTranslation } from "react-i18next";
import type { Overview } from "../bindings";
import { LibraryDoor } from "./LibraryDoor";
import styles from "./overview.module.css";

const MINUTE_MS = 60_000;
const HOUR_MINUTES = 60;

/**
 * The four numbers (ROADMAP 1.13): the Library's tracks and total length,
 * how many are in crates, how many rekordbox has analyzed, and when it was
 * last sent. The track count comes from the Library's own list, so it shows
 * before the rest, which wait for the Overview command. A number that
 * background work can still change says so.
 *
 * Every number that counts tracks is a door (1cC-4): the track count opens the
 * Library, "in crates" and "analyzed" open it with the chip that holds those
 * tracks. The length and the date of the last send count no tracks, so they
 * open nothing.
 */
export function FourNumbers({
  libraryTracks,
  overview,
}: {
  /** `undefined` if the Library can't be read. */
  libraryTracks: number | undefined;
  overview: Overview | undefined;
}) {
  const { t, i18n } = useTranslation("overview");
  const number = new Intl.NumberFormat(i18n.language);
  const counting = overview?.counting;

  const minutes = overview ? Math.round(overview.totalLengthMs / MINUTE_MS) : 0;
  const length =
    minutes === 0
      ? null
      : minutes < HOUR_MINUTES
        ? t("numbers.lengthMinutes", { minutes })
        : t("numbers.length", {
            hours: Math.floor(minutes / HOUR_MINUTES),
            minutes: minutes % HOUR_MINUTES,
          });

  const stillCounting = (yes: boolean | undefined) =>
    yes ? <span className={styles.muted}>{t("todo.lineCounting")}</span> : null;

  return (
    <ul className={styles.numbers}>
      {libraryTracks !== undefined && (
        <li className={styles.number}>
          <LibraryDoor className={styles.numberDoor}>
            <span className={styles.value}>{number.format(libraryTracks)}</span>
            <span className={styles.caption}>{t("numbers.tracks", { count: libraryTracks })}</span>
            {length !== null && <span className={styles.caption}>{length}</span>}
            {stillCounting(counting?.files)}
          </LibraryDoor>
        </li>
      )}
      {overview && (
        <>
          <li className={styles.number}>
            <LibraryDoor filter={{ kind: "inCrate" }} className={styles.numberDoor}>
              <span className={styles.value}>{number.format(overview.inCrates)}</span>
              <span className={styles.caption}>{t("numbers.inCrates")}</span>
            </LibraryDoor>
          </li>
          <li className={styles.number}>
            {overview.rekordboxRead ? (
              <LibraryDoor filter={{ kind: "analyzedByRekordbox" }} className={styles.numberDoor}>
                <span className={styles.value}>{number.format(overview.analyzedByRekordbox)}</span>
                <span className={styles.caption}>{t("numbers.analyzed")}</span>
                {stillCounting(counting?.rekordbox)}
              </LibraryDoor>
            ) : (
              // Before rekordbox has been read, "none analyzed" would say nothing,
              // and there are no rows behind it: not a door.
              <>
                <span className={styles.value}>–</span>
                <span className={styles.caption}>{t("numbers.analyzedUnread")}</span>
                {stillCounting(counting?.rekordbox)}
              </>
            )}
          </li>
          <li className={styles.number}>
            {overview.lastSentMs === null ? (
              <span className={styles.value}>{t("numbers.neverSent")}</span>
            ) : (
              <>
                <span className={styles.value}>
                  {new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium" }).format(
                    new Date(overview.lastSentMs),
                  )}
                </span>
                <span className={styles.caption}>{t("numbers.lastSent")}</span>
              </>
            )}
          </li>
        </>
      )}
    </ul>
  );
}

import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { Overview, SendState } from "../bindings";
import sendStyles from "../send/SendChecklist.module.css";
import styles from "./overview.module.css";

/**
 * The Overview's header (ROADMAP 1.13): the Library's size, when it was last
 * sent, and the Overview's one filled button, which opens the send review
 * and never sends by itself (1.9).
 *
 * There's nothing to count the changes a send would carry without reading
 * rekordbox's export, which the review does, so the button carries no
 * number. It's greyed out, with the reason beside it, only when the
 * Library can't be read. A review whose last read found nothing to send, or
 * refused the send, is said under the button, with when the export was
 * read; the button stays, because the next read may find otherwise.
 */
export function OverviewHeader({
  libraryTracks,
  overview,
  send,
  onSend,
}: {
  /** `undefined` if the Library can't be read. */
  libraryTracks: number | undefined;
  overview: Overview | undefined;
  send: SendState | undefined;
  onSend: () => void;
}) {
  const { t, i18n } = useTranslation("overview");
  const { t: tSend } = useTranslation("send");
  const reasonId = useId();
  const unreadable = libraryTracks === undefined;

  const when = (date: Date) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(date);

  const preflight = send?.preflight ?? null;
  const lastCheck =
    preflight === null
      ? null
      : preflight.refusal !== null
        ? tSend(`review.refused.${preflight.refusal.reason}`, {
            path: preflight.refusal.path.join(" / "),
          })
        : preflight.nothingToSend
          ? tSend("review.nothing")
          : null;

  return (
    <header className={styles.header}>
      <div className={styles.facts}>
        {libraryTracks !== undefined && (
          <p className={styles.size}>{t("header.library", { count: libraryTracks })}</p>
        )}
        {overview && (
          <p className={styles.muted}>
            {overview.lastSentMs === null
              ? t("header.neverSent")
              : t("header.lastSent", { when: when(new Date(overview.lastSentMs)) })}
          </p>
        )}
        {unreadable && (
          <p id={reasonId} className={styles.reason}>
            {t("header.unreadable")}
          </p>
        )}
        {lastCheck !== null && preflight !== null && (
          <p className={styles.muted}>
            {t("header.lastCheck", {
              what: lastCheck,
              when: when(new Date(preflight.export.readAt)),
            })}
          </p>
        )}
      </div>
      <button
        type="button"
        className={sendStyles.primary}
        disabled={unreadable}
        aria-describedby={unreadable ? reasonId : undefined}
        onClick={onSend}
      >
        {tSend("open")}
      </button>
    </header>
  );
}

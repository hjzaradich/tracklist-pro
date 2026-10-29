import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { Overview, SendState } from "../bindings";
import sendStyles from "../send/SendChecklist.module.css";
import { sendAvailable } from "../send/sendAvailable";
import { useLibraryChangedAt } from "../send/useLibraryChangedAt";
import { FourNumbers } from "./FourNumbers";
import styles from "./overview.module.css";

/**
 * The Overview's header (ROADMAP 1.13): the four numbers and the
 * Overview's one filled button, which opens the send review and never sends
 * by itself (1.9).
 *
 * There's nothing to count the changes a send would carry without reading
 * rekordbox's export, which the review does, so the button carries no
 * number and no "send due" (ROADMAP 1.13). It's greyed out, with the reason
 * beside it, only when the Library can't be read (the command palette's Send
 * is unavailable then too: {@link sendAvailable}). A review whose last read
 * found nothing to send, or refused the send, is said beside the button,
 * with when the export was read; the button stays, because the next read may
 * find otherwise. Once the Library has changed since that read, it isn't said
 * any more: it describes a Library that is gone.
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
  const unreadable = !sendAvailable(libraryTracks);
  const libraryChangedAt = useLibraryChangedAt();

  const when = (date: Date) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(date);

  const preflight = send?.preflight ?? null;
  const readAt = preflight === null ? null : new Date(preflight.export.readAt);
  // A read from before the Library last changed says nothing about it now.
  const current = readAt !== null && !(libraryChangedAt > readAt.getTime());
  const lastCheck =
    preflight === null || !current
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
      <FourNumbers libraryTracks={libraryTracks} overview={overview} />
      <div className={styles.sendRow}>
        <div className={styles.facts}>
          {unreadable && (
            <p id={reasonId} className={styles.reason}>
              {t("header.unreadable")}
            </p>
          )}
          {lastCheck !== null && readAt !== null && (
            <>
              <p>{lastCheck}</p>
              <p className={styles.muted}>{t("header.exportRead", { when: when(readAt) })}</p>
            </>
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
      </div>
    </header>
  );
}

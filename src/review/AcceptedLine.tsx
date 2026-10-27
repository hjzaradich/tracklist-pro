import { useQuery } from "@tanstack/react-query";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage, unwrap } from "../api/errors";
import { commands, type SwitchedView } from "../bindings";
import { useAcceptedStore } from "./acceptedStore";
import { trackLabel } from "./ReviewQueue";
import comparisonStyles from "./ReviewComparison.module.css";
import styles from "./ReviewScreen.module.css";

/**
 * The line at the top of Review after "Accept all" (1bE-3; owner decision,
 * 2026-10-13): how many tracks were switched to the copy on disk, with View
 * (each track with its old and new file), kept until it is dismissed or the
 * next "Accept all" replaces it. It has no Undo of its own (owner decision,
 * 2026-10-27: Undo lives in the top bar, and Ctrl+Z); the line goes when the
 * top bar's Undo or Ctrl+Z takes that operation back (`useKeepNextUndoCurrent`
 * tells the store).
 */
export function AcceptedLine() {
  const { t } = useTranslation("review");
  const accepted = useAcceptedStore((state) => state.accepted);
  const dismiss = useAcceptedStore((state) => state.dismiss);
  const textId = useId();
  const [viewing, setViewing] = useState<number | null>(null);
  const operationId = accepted?.operationId ?? null;
  const open = operationId !== null && viewing === operationId;
  const view = useQuery({
    queryKey: ["reviewSwitchedView", operationId],
    enabled: open,
    queryFn: (): Promise<SwitchedView[]> =>
      unwrap(
        commands.reviewSwitchedView(
          (accepted?.switched ?? []).map((s) => ({
            recordingId: s.recordingId,
            fromFileId: s.fromFileId,
            toFileId: s.toFileId,
          })),
        ),
      ),
  });
  if (accepted === null) return null;

  return (
    <section className={styles.accepted} aria-labelledby={textId}>
      <p id={textId} role="status" className={styles.acceptedText}>
        {t("accepted.line", { count: accepted.switched.length })}
      </p>
      <button
        type="button"
        className={comparisonStyles.button}
        aria-expanded={open}
        onClick={() => setViewing(open ? null : accepted.operationId)}
      >
        {open ? t("accepted.hide") : t("accepted.view")}
      </button>
      <button type="button" className={comparisonStyles.button} onClick={dismiss}>
        {t("accepted.dismiss")}
      </button>
      {open && view.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(view.error)}
        </p>
      )}
      {open && view.data !== undefined && (
        <table className={`${comparisonStyles.table} ${styles.acceptedTable}`} aria-label={t("accepted.list")}>
          <thead>
            <tr>
              <th scope="col">{t("accepted.col.track")}</th>
              <th scope="col">{t("accepted.col.from")}</th>
              <th scope="col">{t("accepted.col.to")}</th>
            </tr>
          </thead>
          <tbody>
            {view.data.map((switched) => (
              <tr key={switched.recordingId}>
                <td>{trackLabel(t, switched.track)}</td>
                <td>{switched.fromName}</td>
                <td>{switched.toName}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}

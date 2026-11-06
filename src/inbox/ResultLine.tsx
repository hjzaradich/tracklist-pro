import { useTranslation } from "react-i18next";
import type { Result } from "./inboxText";
import { refusalText } from "./inboxText";
import styles from "./inbox.module.css";

/**
 * What an action that did not simply work came to: why it was refused, the
 * go-ahead it needs (a low bitrate), or, after "Add all", the files it left
 * out with their reasons.
 */
export function ResultLine({
  result,
  names,
  onAddAnyway,
  addAnywayDisabled,
}: {
  result: Result;
  names: ReadonlyMap<number, string>;
  onAddAnyway: () => void;
  addAnywayDisabled: boolean;
}) {
  const { t } = useTranslation("inbox");
  if (result.outcome === "refused") {
    return (
      <p role="alert" className={styles.problem}>
        {refusalText(t, result.reason)}
      </p>
    );
  }
  if (result.outcome === "needsConfirmation") {
    return (
      <p role="alert" className={styles.problem}>
        <span>{t("confirm.lowBitrate", { kbps: result.warning.kbps })}</span>{" "}
        <button type="button" className={styles.action} disabled={addAnywayDisabled} onClick={onAddAnyway}>
          {t("action.addAnyway")}
        </button>
      </p>
    );
  }
  if (result.skipped.length === 0) return null;
  return (
    <div className={styles.skipped}>
      <p>{t("skipped.title", { count: result.skipped.length })}</p>
      <ul>
        {result.skipped.map((s) => (
          <li key={s.itemId}>
            {t("skipped.line", { file: names.get(s.itemId) ?? "", reason: refusalText(t, s.reason) })}
          </li>
        ))}
      </ul>
    </div>
  );
}

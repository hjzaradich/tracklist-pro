import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import styles from "../firstRun/firstRun.module.css";
import { useSetUpdateChecks, useUpdateStatus } from "./useUpdates";

/**
 * "Check for updates?" (ROADMAP 4.5): asked once, on the Overview, saying
 * what it sends. Yes or No is stored and the step never shows again, whatever
 * the Library holds. Settings has the switch afterwards.
 */
export function UpdateQuestion() {
  const { t } = useTranslation("updates");
  const titleId = useId();
  const status = useUpdateStatus();
  const answer = useSetUpdateChecks();
  // Until the answer is known, or once there is one, nothing shows.
  if (status.data === undefined || (status.data.asked && !answer.isPending)) return null;
  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("ask.title")}
      </h2>
      <p className={styles.muted}>{t("ask.help")}</p>
      <div className={styles.actions}>
        <button type="button" className={styles.button} disabled={answer.isPending} onClick={() => answer.mutate(true)}>
          {t("ask.yes")}
        </button>
        <button type="button" className={styles.button} disabled={answer.isPending} onClick={() => answer.mutate(false)}>
          {t("ask.no")}
        </button>
      </div>
      {answer.isError && (
        <p role="alert" className={styles.problem}>
          {errorMessage(answer.error)}
        </p>
      )}
    </section>
  );
}

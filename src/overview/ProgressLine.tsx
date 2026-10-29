import { useTranslation } from "react-i18next";
import type { Overview } from "../bindings";
import styles from "./overview.module.css";

/** What's been prepared this week (ROADMAP 1.13): tracks added, decisions made. */
export function ProgressLine({ overview }: { overview: Overview }) {
  const { t } = useTranslation("overview");
  return (
    <p className={styles.progress}>
      {t("progress.line", {
        added: t("progress.added", { count: overview.addedThisWeek }),
        decisions: t("progress.decisions", { count: overview.decisionsThisWeek }),
      })}
    </p>
  );
}

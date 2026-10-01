import { Link } from "@tanstack/react-router";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import styles from "./firstRun.module.css";

/**
 * Start fresh (1aE-4): the Library is simply empty, and tracks are added
 * from All music. Nothing is stored about the choice.
 */
export function StartFresh() {
  const { t } = useTranslation("firstRun");
  const titleId = useId();
  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("fresh.title")}
      </h2>
      <p className={styles.muted}>{t("fresh.help")}</p>
      <div className={styles.actions}>
        <Link to="/all-music" className={styles.button}>
          {t("fresh.action")}
        </Link>
      </div>
    </section>
  );
}

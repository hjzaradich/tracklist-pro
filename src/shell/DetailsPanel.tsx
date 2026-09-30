import { useTranslation } from "react-i18next";
import styles from "./DetailsPanel.module.css";

/** The Details panel on the right: the selected track's info. Empty for now. */
export function DetailsPanel({ className = "" }: { className?: string }) {
  const { t } = useTranslation("shell");

  return (
    <aside aria-label={t("details.label")} className={`${styles.details} ${className}`}>
      <p className={styles.empty}>{t("details.empty")}</p>
    </aside>
  );
}

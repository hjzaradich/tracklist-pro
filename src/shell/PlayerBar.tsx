import { useTranslation } from "react-i18next";
import styles from "./PlayerBar.module.css";

/** The preview player along the bottom (1.11). No playback yet. */
export function PlayerBar({ className = "" }: { className?: string }) {
  const { t } = useTranslation("shell");

  return (
    <section aria-label={t("player.label")} className={`${styles.player} ${className}`}>
      <p className={styles.empty}>{t("player.empty")}</p>
    </section>
  );
}

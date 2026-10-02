import { useTranslation } from "react-i18next";
import { Activity } from "../activity/Activity";
import { ThemeSwitch } from "../theme/ThemeSwitch";
import styles from "./TopBar.module.css";
import { UndoButton } from "./UndoButton";

/**
 * App name, the search / command palette slot (1.11, Ctrl+K), Undo (1bA-12),
 * the Activity status for background jobs (0E-3) and the theme switch. The
 * search slot is an inert placeholder for now.
 */
export function TopBar({ className = "" }: { className?: string }) {
  const { t } = useTranslation(["shell", "common"]);

  return (
    <header className={`${styles.bar} ${className}`}>
      <span className={styles.appName}>{t("common:appName")}</span>
      <div role="search" aria-label={t("search.label")} className={styles.search}>
        <input
          type="search"
          className={styles.searchInput}
          aria-label={t("search.label")}
          placeholder={t("search.placeholder")}
          disabled
        />
        <kbd className={styles.shortcut}>{t("search.shortcut")}</kbd>
      </div>
      <div className={styles.end}>
        <UndoButton />
        <Activity />
        <ThemeSwitch />
      </div>
    </header>
  );
}

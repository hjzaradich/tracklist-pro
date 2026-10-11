import { useTranslation } from "react-i18next";
import { Activity } from "../activity/Activity";
import { ThemeSwitch } from "../theme/ThemeSwitch";
import styles from "./TopBar.module.css";
import { UndoButton } from "./UndoButton";
import { ZONES } from "./zones";

/**
 * App name, the search / command palette slot (1.11, Ctrl+K), Undo (1bA-12),
 * the Activity status for background jobs (0E-3) and the theme switch. The
 * search slot is drawn only with `search` (see `zones.ts`): until the
 * search exists it would be a box that does nothing. Its place in the bar
 * stays, so the rest doesn't move.
 */
export function TopBar({
  className = "",
  search = ZONES.search,
}: {
  className?: string;
  search?: boolean;
}) {
  const { t } = useTranslation(["shell", "common"]);

  return (
    <header className={`${styles.bar} ${className}`}>
      <span className={styles.appName}>{t("common:appName")}</span>
      {search ? (
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
      ) : (
        // The bar's middle column, left empty.
        <span />
      )}
      <div className={styles.end}>
        <UndoButton />
        <Activity />
        <ThemeSwitch />
      </div>
    </header>
  );
}

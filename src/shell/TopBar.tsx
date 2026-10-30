import { Link } from "@tanstack/react-router";
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
        <SettingsLink />
        <ThemeSwitch />
      </div>
    </header>
  );
}

/**
 * The gear that opens Settings (1cC-2). Icon only, with its name as the
 * accessible name and the hover text. On `/settings` it carries the active
 * mark (the router's `aria-current`), since no stage is selected there.
 */
function SettingsLink() {
  const { t } = useTranslation("shell");
  return (
    <Link to="/settings" className={styles.gear} aria-label={t("settings.button")} title={t("settings.button")}>
      <svg viewBox="0 0 16 16" aria-hidden="true" focusable="false" className={styles.gearIcon}>
        <path
          fillRule="evenodd"
          d="M13.04 6.36L14.89 6.77L14.89 9.23L13.04 9.64L12.72 10.41L13.74 12.00L12.00 13.74L10.41 12.72L9.64 13.04L9.23 14.89L6.77 14.89L6.36 13.04L5.59 12.72L4.00 13.74L2.26 12.00L3.28 10.41L2.96 9.64L1.11 9.23L1.11 6.77L2.96 6.36L3.28 5.59L2.26 4.00L4.00 2.26L5.59 3.28L6.36 2.96L6.77 1.11L9.23 1.11L9.64 2.96L10.41 3.28L12.00 2.26L13.74 4.00L12.72 5.59ZM8 5.6a2.4 2.4 0 1 0 0 4.8a2.4 2.4 0 1 0 0-4.8Z"
        />
      </svg>
    </Link>
  );
}

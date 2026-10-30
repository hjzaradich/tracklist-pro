import { useId } from "react";
import { useTranslation } from "react-i18next";
import stage from "../shell/StageScreen.module.css";
import { EnergyStarsRow } from "./EnergyStarsRow";
import styles from "./settings.module.css";

/**
 * Settings, at `/settings` (1cC-2): a screen, not a stage, reached from the
 * gear in the top bar and the palette's "Open Settings". One panel per group
 * of settings, each row applying at once: no Save, no filled button of its own.
 */
export function SettingsScreen() {
  const { t } = useTranslation("settings");
  const rekordboxId = useId();
  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        <div className={styles.column}>
          <section className={styles.panel} aria-labelledby={rekordboxId}>
            <h2 id={rekordboxId} className={styles.panelTitle}>
              {t("rekordbox.title")}
            </h2>
            <EnergyStarsRow />
          </section>
        </div>
      </div>
    </div>
  );
}

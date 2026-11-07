import { Link } from "@tanstack/react-router";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import stage from "../shell/StageScreen.module.css";
import { AboutPanel } from "./AboutPanel";
import { BundlePanel } from "./BundlePanel";
import { EnergyStarsRow } from "./EnergyStarsRow";
import { LocalKeyRow } from "./LocalKeyRow";
import { RekordboxDatabaseRow } from "./RekordboxDatabaseRow";
import { WishlistFeatureRow } from "./WishlistFeatureRow";
import styles from "./settings.module.css";

/**
 * Settings, at `/settings` (1cC-2): a screen, not a stage, reached from the
 * gear in the top bar and the palette's "Open Settings". One panel per group
 * of settings, each row applying at once: no Save, no filled button of its own.
 */
export function SettingsScreen() {
  const { t } = useTranslation("settings");
  const { t: tg } = useTranslation("genre");
  const rekordboxId = useId();
  const { t: tw } = useTranslation("wishlist");
  const featuresId = useId();
  const genresId = useId();
  const { t: td } = useTranslation("diagnostics");
  const diagnosticsId = useId();
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
            <RekordboxDatabaseRow />
          </section>
          <section className={styles.panel} aria-labelledby={featuresId}>
            <h2 id={featuresId} className={styles.panelTitle}>
              {tw("feature.panel")}
            </h2>
            <WishlistFeatureRow />
            <LocalKeyRow />
          </section>
          <section className={styles.panel} aria-labelledby={genresId}>
            <h2 id={genresId} className={styles.panelTitle}>
              {tg("settingsPanel.title")}
            </h2>
            <Link to="/genres">{tg("unmapped.openGenres")}</Link>
          </section>
          <section className={styles.panel} aria-labelledby={diagnosticsId}>
            <h2 id={diagnosticsId} className={styles.panelTitle}>
              {td("panel")}
            </h2>
            <Link to="/diagnostics">{td("open")}</Link>
          </section>
          <BundlePanel />
          <AboutPanel />
        </div>
      </div>
    </div>
  );
}

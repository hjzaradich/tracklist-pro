import { useTranslation } from "react-i18next";
import { THEMES, useThemeStore } from "./themeStore";
import styles from "./ThemeSwitch.module.css";

/** A two-way segmented switch: Dark | Light. */
export function ThemeSwitch() {
  const { t } = useTranslation("theme");
  const theme = useThemeStore((state) => state.theme);
  const setTheme = useThemeStore((state) => state.setTheme);

  return (
    <div role="radiogroup" aria-label={t("label")} className={styles.group}>
      <span className={styles.label} aria-hidden="true">
        {t("label")}
      </span>
      {THEMES.map((option) => (
        <button
          key={option}
          type="button"
          role="radio"
          aria-checked={theme === option}
          className={styles.option}
          onClick={() => setTheme(option)}
        >
          {t(option)}
        </button>
      ))}
    </div>
  );
}

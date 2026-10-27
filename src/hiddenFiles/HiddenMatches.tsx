import { useTranslation } from "react-i18next";
import styles from "./HiddenFiles.module.css";
import { useHiddenMatches } from "./useHiddenFiles";

/**
 * "N hidden files match · Show them" (1cB-6; ROADMAP 1.11): under a search,
 * how many hidden files match that the results don't show already (a hidden
 * file whose track is in the results is not counted). "Show them" opens the
 * Hidden filter with the same search. Nothing at all when none match.
 */
export function HiddenMatches({ search, onShow }: { search: string; onShow: () => void }) {
  const { t } = useTranslation("hiddenFiles");
  const matches = useHiddenMatches(search);
  const count = matches.data?.count ?? 0;
  if (count === 0) return null;
  return (
    <p className={styles.matches} role="status">
      <span>{t("matches", { count })}</span>
      <span aria-hidden="true">·</span>
      <button type="button" className={styles.link} onClick={onShow}>
        {t("showThem")}
      </button>
    </p>
  );
}

import { useTranslation } from "react-i18next";
import styles from "../shell/StageScreen.module.css";
import { AfterSendPanel } from "./AfterSendLists";

/**
 * The after-send lists on a page of their own, at `/after-send` (not in the
 * sidebar). The send checklist mounts {@link AfterSendPanel} in its last
 * step; this is for building and checking the lists without it.
 */
export function AfterSendScreen() {
  const { t } = useTranslation("afterSend");
  return (
    <div className={styles.screen}>
      <div className={styles.header}>
        <h1 className={styles.title}>{t("page")}</h1>
      </div>
      <div className={styles.body}>
        <AfterSendPanel />
      </div>
    </div>
  );
}

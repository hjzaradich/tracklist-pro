import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import styles from "../firstRun/firstRun.module.css";
import { useStartupUpdateCheck, useUpdateStatus } from "./useUpdates";

/** One line on the Overview when a newer version exists; the address to copy is in Settings → About. */
export function UpdateNotice() {
  const { t } = useTranslation("updates");
  useStartupUpdateCheck();
  const available = useUpdateStatus().data?.available;
  if (!available) return null;
  return (
    <p className={styles.muted} role="status">
      {t("notice.line", { version: available.version })} <Link to="/settings">{t("notice.open")}</Link>
    </p>
  );
}

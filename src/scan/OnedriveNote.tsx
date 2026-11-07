import { useTranslation } from "react-i18next";

/**
 * Next to a music folder in OneDrive: OneDrive can keep its files online
 * only or remove local copies, and what to do about it. Text only; the app
 * changes nothing in OneDrive.
 */
export function OnedriveNote({ className }: { className?: string }) {
  const { t } = useTranslation("musicFolderStatus");
  return (
    <span className={className} data-note="onedrive">
      {t("onedrive")} {t("onedriveHelp")}
    </span>
  );
}

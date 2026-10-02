import { useId } from "react";
import { useTranslation } from "react-i18next";
import { useReadOnlineOnlyFiles, useSetReadOnlineOnlyFiles } from "./useReadOnlineOnlyFiles";
import styles from "./ReadOnlineOnlySwitch.module.css";

/** The opt-in to reading OneDrive online-only files, shown in the Music folders panel. */
export function ReadOnlineOnlySwitch() {
  const { t } = useTranslation("musicFolderStatus");
  const on = useReadOnlineOnlyFiles();
  const setOn = useSetReadOnlineOnlyFiles();
  const helpId = useId();
  return (
    <div className={styles.root}>
      <label className={styles.label}>
        <input
          type="checkbox"
          checked={on}
          disabled={setOn.isPending}
          aria-describedby={helpId}
          onChange={(event) => setOn.mutate(event.target.checked)}
        />
        {t("readOnlineOnly")}
      </label>
      <p id={helpId} className={styles.help}>
        {t("readOnlineOnlyHelp")}
      </p>
    </div>
  );
}

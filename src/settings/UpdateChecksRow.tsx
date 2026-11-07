import { useTranslation } from "react-i18next";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";
import { useCheckForUpdatesNow, useSetUpdateChecks, useStartupUpdateCheck, useUpdateStatus } from "../update/useUpdates";

/**
 * "Update checks" under Online services (ROADMAP 4.5): off until turned on (or
 * answered yes on the first run). The help says what is sent. "Check now" asks
 * at once; the result shows here and, for a newer version, in About.
 */
export function UpdateChecksRow() {
  const { t } = useTranslation("updates");
  useStartupUpdateCheck();
  const status = useUpdateStatus().data;
  const save = useSetUpdateChecks();
  const checkNow = useCheckForUpdatesNow();
  const known = status !== undefined;
  const on = status?.enabled === true;
  const last =
    status?.checkedAt == null
      ? t("settings.notYet")
      : status.available
        ? t("settings.newer")
        : t("settings.upToDate", { when: new Date(status.checkedAt).toLocaleString() });
  const checked = save.isPending ? save.variables === true : on;
  return (
    <SwitchRow
      label={t("settings.label")}
      help={known ? t("settings.help") : undefined}
      checked={checked}
      pending={!known || save.isPending}
      error={save.isError ? t("settings.error") : null}
      onChange={(next) => save.mutate(next)}
    >
      {on && (
        <div className={styles.reader}>
          <div className={styles.actions}>
            <button type="button" className={styles.button} disabled={checkNow.isPending} onClick={() => checkNow.mutate()}>
              {t("settings.checkNow")}
            </button>
          </div>
          <p role="status" className={styles.status}>
            {checkNow.isPending ? t("settings.checking") : last}
          </p>
        </div>
      )}
    </SwitchRow>
  );
}

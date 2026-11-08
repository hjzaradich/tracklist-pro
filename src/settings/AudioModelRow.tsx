import { useTranslation } from "react-i18next";
import type { AudioModelStatus } from "../bindings";
import { useAudioModelStatus, useSetAudioModel } from "../audioModel/useAudioModel";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";

/**
 * "Audio model" under Optional features (ROADMAP 3.8): off until turned on.
 * While on, the model listens to Library tracks in the background; the status
 * line says how far it has got. Turning it off stops the listening and keeps
 * what it heard.
 */
export function AudioModelRow() {
  const { t } = useTranslation("audioModel");
  const status = useAudioModelStatus().data;
  const save = useSetAudioModel();
  const known = status !== undefined;
  const on = status?.enabled === true;
  const checked = save.isPending ? save.variables === true : on;
  return (
    <SwitchRow
      label={t("switch.label")}
      help={known ? t("switch.help") : undefined}
      checked={checked}
      pending={!known || save.isPending}
      error={save.isError ? t("switch.error") : null}
      onChange={(next) => save.mutate(next)}
    >
      {on && (
        <div className={styles.reader}>
          <StatusLines status={status} />
        </div>
      )}
    </SwitchRow>
  );
}

function StatusLines({ status }: { status: AudioModelStatus }) {
  const { t } = useTranslation("audioModel");
  const left = status.fingerprinted + status.failed < status.total;
  return (
    <>
      <p role="status" className={styles.status}>
        {status.total === 0
          ? t("switch.noTracks")
          : t("switch.progress", { done: status.fingerprinted, count: status.total })}
      </p>
      {status.failed > 0 && <p className={styles.status}>{t("switch.failed", { count: status.failed })}</p>}
      {status.total > 0 && (
        <p className={styles.status}>
          {status.running ? t("switch.running") : left ? t("switch.paused") : t("switch.finished")}
        </p>
      )}
    </>
  );
}

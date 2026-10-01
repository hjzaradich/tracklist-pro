import { open } from "@tauri-apps/plugin-dialog";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { MusicFolderList } from "../scan/MusicFolderList";
import { useSetMusicFolderWatch } from "../scan/useMusicFolderWatch";
import { useMusicFolders } from "../scan/useMusicFolders";
import styles from "./firstRun.module.css";
import { useAddMusicFolder } from "./useAddMusicFolder";

/**
 * Step one of starting a Library (1aE-1): pick the music folders. The list,
 * the watcher toggle and the commands are the existing ones; this adds the
 * folder picker. An added folder is scanned at once.
 */
export function MusicFoldersStep() {
  const { t } = useTranslation("firstRun");
  const titleId = useId();
  const folders = useMusicFolders();
  const add = useAddMusicFolder();
  const setWatch = useSetMusicFolderWatch();
  const failure = folders.error ?? add.error ?? setWatch.error;

  const choose = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (typeof picked === "string") add.mutate(picked);
  };

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("folders.title")}
      </h2>
      <p className={styles.muted}>{t("folders.help")}</p>
      {folders.data && (
        <MusicFolderList
          folders={folders.data}
          onWatchChange={(id, watch) => setWatch.mutate({ id, watch })}
        />
      )}
      {failure && (
        <p role="alert" className={styles.problem}>
          {errorMessage(failure)}
        </p>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.button}
          disabled={add.isPending}
          onClick={() => void choose()}
        >
          {t("folders.add")}
        </button>
      </div>
    </section>
  );
}

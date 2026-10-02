import { open } from "@tauri-apps/plugin-dialog";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { MusicFolderList } from "../scan/MusicFolderList";
import { ReadOnlineOnlySwitch } from "../scan/ReadOnlineOnlySwitch";
import { useSetMusicFolderWatch } from "../scan/useMusicFolderWatch";
import { useMusicFolders } from "../scan/useMusicFolders";
import styles from "./firstRun.module.css";
import { useAddMusicFolder, useScanMusicFolder } from "./useAddMusicFolder";

/**
 * The Music folders panel: step one of starting a Library (1aE-1), and
 * the place the folders are managed from then on (the Overview keeps it
 * below the Library's own content). Each folder has its notes, its
 * watcher toggle and "Scan again"; below them, the online-only opt-in and
 * the folder picker. An added folder is scanned at once.
 */
export function MusicFoldersStep() {
  const { t } = useTranslation("firstRun");
  const titleId = useId();
  const folders = useMusicFolders();
  const add = useAddMusicFolder();
  const setWatch = useSetMusicFolderWatch();
  const scan = useScanMusicFolder();
  const failure = folders.error ?? add.error ?? setWatch.error ?? scan.error;

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
          onScanAgain={(id) => scan.mutate(id)}
        />
      )}
      <ReadOnlineOnlySwitch />
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

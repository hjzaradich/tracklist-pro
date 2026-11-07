import { open } from "@tauri-apps/plugin-dialog";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { InboxFolder, MusicFolder } from "../bindings";
import { OnedriveNote } from "../scan/OnedriveNote";
import { useOnedriveFolders } from "../scan/useOnedriveFolders";
import { useAddInboxFolder, useRemoveMusicFolder } from "./useInbox";
import styles from "./inbox.module.css";

/**
 * The folders new music is watched in: each with its path and, until its
 * first look is taken, "Still taking a first look"; a button to remove it
 * (its files are never touched) and one to add another through the usual
 * folder picker.
 */
export function InboxFolders({
  folders,
  inbox,
}: {
  folders: MusicFolder[];
  inbox: InboxFolder[];
}) {
  const { t } = useTranslation("inbox");
  const titleId = useId();
  const add = useAddInboxFolder();
  const remove = useRemoveMusicFolder();
  const onedrive = useOnedriveFolders(folders);
  const failure = add.error ?? remove.error;
  const waiting = new Set(inbox.filter((f) => f.waitingForFirstLook).map((f) => f.musicFolderId));

  const choose = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (typeof picked === "string") add.mutate(picked);
  };

  return (
    <section className={styles.folders} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.heading}>
        {t("folders.title")}
      </h2>
      <p className={styles.muted}>{t("folders.help")}</p>
      {folders.length === 0 ? (
        <p className={styles.muted}>{t("folders.none")}</p>
      ) : (
        <ul className={styles.folderList}>
          {folders.map((folder) => (
            <li key={folder.id} className={styles.folder}>
              <span className={styles.name}>{folder.path}</span>
              {onedrive.has(folder.id) && <OnedriveNote className={styles.muted} />}
              {waiting.has(folder.id) && <span className={styles.muted}>{t("folders.firstLook")}</span>}
              <button
                type="button"
                className={styles.action}
                title={t("folders.removeNamed", { path: folder.path })}
                aria-label={t("folders.removeNamed", { path: folder.path })}
                disabled={remove.isPending}
                onClick={() => remove.mutate(folder.id)}
              >
                {t("folders.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      {failure && (
        <p role="alert" className={styles.problem}>
          {errorMessage(failure)}
        </p>
      )}
      <div>
        <button type="button" className={styles.action} disabled={add.isPending} onClick={() => void choose()}>
          {t("folders.add")}
        </button>
      </div>
    </section>
  );
}

import { open } from "@tauri-apps/plugin-dialog";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { MusicFolder } from "../bindings";
import { useAddInboxFolder } from "../inbox/useInbox";
import { useSetMusicFolderWatch } from "../scan/useMusicFolderWatch";
import { useMusicFolders } from "../scan/useMusicFolders";
import styles from "./firstRun.module.css";

/**
 * "Where do you put new music?" (ROADMAP 4.2): asks for the Inbox folder.
 * Nothing is assumed: no folder is suggested, and watching it for new files
 * is a separate switch that starts off. "Skip" leaves it for later (the
 * Inbox screen adds folders too). With an Inbox folder already, the step
 * shows it instead of asking. The folder's files are only read.
 */
export function InboxStep() {
  const { t } = useTranslation("firstRun");
  const titleId = useId();
  const [skipped, setSkipped] = useState(false);
  const folders = useMusicFolders();
  const add = useAddInboxFolder();
  const setWatch = useSetMusicFolderWatch();
  const inbox = folders.data?.filter((folder) => folder.role === "inbox") ?? [];
  const failure = add.error ?? setWatch.error;

  // Until the folders have loaded, an Inbox folder may already exist.
  if (folders.data === undefined || (skipped && inbox.length === 0))
    return null;

  const choose = async () => {
    const picked = await open({ multiple: false, directory: true });
    if (typeof picked === "string") add.mutate(picked);
  };

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("inbox.title")}
      </h2>
      {inbox.length === 0 ? (
        <>
          <p className={styles.muted}>{t("inbox.help")}</p>
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.button}
              disabled={add.isPending}
              onClick={() => void choose()}
            >
              {t("inbox.choose")}
            </button>
            <button
              type="button"
              className={styles.button}
              onClick={() => setSkipped(true)}
            >
              {t("inbox.skip")}
            </button>
          </div>
        </>
      ) : (
        <ul className={styles.summary}>
          {inbox.map((folder) => (
            <InboxFolderRow
              key={folder.id}
              folder={folder}
              saving={setWatch.isPending}
              onWatch={(watch) => setWatch.mutate({ id: folder.id, watch })}
            />
          ))}
        </ul>
      )}
      {failure && (
        <p role="alert" className={styles.problem}>
          {errorMessage(failure)}
        </p>
      )}
    </section>
  );
}

function InboxFolderRow({
  folder,
  saving,
  onWatch,
}: {
  folder: MusicFolder;
  saving: boolean;
  onWatch: (watch: boolean) => void;
}) {
  const { t } = useTranslation("firstRun");
  const helpId = useId();
  return (
    <li>
      <p>{folder.path}</p>
      <label>
        <input
          type="checkbox"
          checked={folder.watch}
          disabled={saving}
          aria-describedby={helpId}
          onChange={(event) => onWatch(event.target.checked)}
        />
        {t("inbox.watch")}
      </label>
      <p id={helpId} className={styles.muted}>
        {t("inbox.watchHelp")}
      </p>
    </li>
  );
}

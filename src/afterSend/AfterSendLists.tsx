import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { AfterSendLists as Lists, ManualRemoval, StalePlaylist } from "../bindings";
import styles from "./AfterSendLists.module.css";
import { useAfterSendLists } from "./useAfterSendLists";

/** Where a stale playlist or folder sits in rekordbox, from `Crates` or `Playlists` down. */
function pathOf(stale: StalePlaylist): string {
  return stale.path.join(" > ");
}

function fileName(path: string): string {
  return path.slice(path.lastIndexOf("\\") + 1);
}

function StaleItem({ stale }: { stale: StalePlaylist }) {
  const { t } = useTranslation("afterSend");
  const path = pathOf(stale);
  return (
    <li className={styles.item}>
      <span className={styles.name}>{path}</span>
      {stale.kind === "folder" && (
        <span className={styles.detail}>
          {stale.playlistsInside === 0
            ? t("stale.emptyFolder")
            : t("stale.folderInside", { count: stale.playlistsInside })}
        </span>
      )}
    </li>
  );
}

function RemovalItem({ removal }: { removal: ManualRemoval }) {
  const { t } = useTranslation("afterSend");
  const title = removal.title ?? (removal.path !== null ? fileName(removal.path) : null);
  const name =
    title === null
      ? t("removals.unnamed")
      : removal.artist
        ? t("removals.trackName", { artist: removal.artist, title })
        : title;
  return (
    <li className={styles.item}>
      <span className={styles.name}>{name}</span>
      {removal.path !== null && (
        <span className={styles.path} title={t("removals.sentTo")}>
          {removal.path}
        </span>
      )}
    </li>
  );
}

/**
 * The two lists shown after a send (ROADMAP 1.9 rules 6 and 7): playlists
 * rekordbox still has that the app no longer sends, and tracks removed from
 * the Library that rekordbox still holds. An import can remove neither, so
 * the user does it by hand. While `lists` is still loading only the headings
 * show; `problem` is a failed load's message.
 */
export function AfterSendLists({ lists, problem }: { lists: Lists | undefined; problem?: string }) {
  const { t } = useTranslation("afterSend");
  const staleTitle = useId();
  const removalsTitle = useId();
  return (
    <div className={styles.lists}>
      {problem !== undefined && (
        <p role="alert" className={styles.problem}>
          {problem}
        </p>
      )}
      <section className={styles.section} aria-labelledby={staleTitle}>
        <h2 id={staleTitle} className={styles.title}>
          {t("stale.title")}
        </h2>
        <p className={styles.hint}>{t("stale.hint")}</p>
        {lists && !lists.playlistsChecked && <p className={styles.empty}>{t("stale.unchecked")}</p>}
        {lists?.playlistsChecked && lists.stalePlaylists.length === 0 && (
          <p className={styles.empty}>{t("stale.empty")}</p>
        )}
        {lists && lists.stalePlaylists.length > 0 && (
          <ul className={styles.items}>
            {lists.stalePlaylists.map((stale) => (
              <StaleItem key={stale.path.join("\u0000")} stale={stale} />
            ))}
          </ul>
        )}
      </section>
      <section className={styles.section} aria-labelledby={removalsTitle}>
        <h2 id={removalsTitle} className={styles.title}>
          {t("removals.title")}
        </h2>
        <p className={styles.hint}>{t("removals.hint")}</p>
        {lists?.manualRemovals.length === 0 && <p className={styles.empty}>{t("removals.empty")}</p>}
        {lists && lists.manualRemovals.length > 0 && (
          <ul className={styles.items}>
            {lists.manualRemovals.map((removal) => (
              <RemovalItem key={removal.recordingId} removal={removal} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

/**
 * {@link AfterSendLists} with its own data: mount this in the send
 * checklist's last step, or on its own.
 */
export function AfterSendPanel() {
  const lists = useAfterSendLists();
  return (
    <AfterSendLists
      lists={lists.data}
      problem={lists.error ? errorMessage(lists.error) : undefined}
    />
  );
}

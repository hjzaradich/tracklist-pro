import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { MissingList as Missing } from "../bindings";
import styles from "./MissingList.module.css";

/** Called when the user asks to add a group's folder as a music folder. */
export type OnAddFolder = (folder: string) => void;

/**
 * The Missing list (1aD-5): rekordbox tracks with no file, grouped by the
 * folder they were last in. A group whose folder exists now and isn't in a
 * music folder offers to add it. While `list` is still loading only the
 * heading shows; `problem` is a failure's message (the load's, or an
 * add's), and `adding` holds the buttons while an add runs.
 */
export function MissingList({
  list,
  problem,
  adding = false,
  onAddFolder,
}: {
  list: Missing | undefined;
  problem?: string;
  adding?: boolean;
  onAddFolder?: OnAddFolder;
}) {
  const { t } = useTranslation("review");
  const titleId = useId();
  const folderIdPrefix = useId();
  return (
    <section className={styles.section} aria-labelledby={titleId}>
      <h2 id={titleId} className={styles.title}>
        {t("missing.title")}
      </h2>
      {problem !== undefined && (
        <p role="alert" className={styles.problem}>
          {problem}
        </p>
      )}
      {list?.groups.length === 0 && <p className={styles.empty}>{t("missing.empty")}</p>}
      {list?.groups.map((group, n) => {
        const folderId = `${folderIdPrefix}-${n}`;
        return (
          <div key={group.folder ?? ""} className={styles.group}>
            <div className={styles.heading}>
              <span id={folderId} className={styles.folder}>
                {group.folder ?? t("missing.unknownFolder")}
              </span>
              <span className={styles.count}>
                {t("missing.tracksCount", { count: group.tracks.length })}
              </span>
              {group.canAdd && group.folder !== null && onAddFolder && (
                <button
                  type="button"
                  aria-describedby={folderId}
                  disabled={adding}
                  onClick={() => onAddFolder(group.folder as string)}
                >
                  {t("missing.addFolder")}
                </button>
              )}
            </div>
            <ul className={styles.tracks}>
              {group.tracks.map((track) => (
                <li key={track.id} className={styles.track}>
                  <span>
                    {track.artist === ""
                      ? track.title
                      : t("missing.trackName", { artist: track.artist, title: track.title })}
                  </span>
                  {track.lastKnownPath !== null && (
                    <span className={styles.path} title={t("missing.lastKnown")}>
                      {track.lastKnownPath}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        );
      })}
    </section>
  );
}

import { useTranslation } from "react-i18next";
import type { MissingList as Missing } from "../bindings";
import styles from "./MissingList.module.css";

/** Called when the user asks to add a group's folder as a music folder. */
export type OnAddFolder = (folder: string) => void;

/**
 * The Missing list (1aD-5): rekordbox tracks with no file, grouped by the
 * folder they were last in. A group whose folder exists now and isn't in a
 * music folder offers to add it.
 */
export function MissingList({
  list,
  onAddFolder,
}: {
  list: Missing;
  onAddFolder?: OnAddFolder;
}) {
  const { t } = useTranslation("review");
  return (
    <section className={styles.section} aria-labelledby="missing-title">
      <h2 id="missing-title" className={styles.title}>
        {t("missing.title")}
      </h2>
      {list.groups.length === 0 ? (
        <p className={styles.empty}>{t("missing.empty")}</p>
      ) : (
        list.groups.map((group) => (
          <div key={group.folder ?? ""} className={styles.group}>
            <div className={styles.heading}>
              <span className={styles.folder}>{group.folder ?? t("missing.unknownFolder")}</span>
              <span className={styles.count}>
                ({t("missing.tracks", { count: group.tracks.length })})
              </span>
              {group.canAdd && group.folder !== null && onAddFolder && (
                <button type="button" onClick={() => onAddFolder(group.folder as string)}>
                  {t("missing.addFolder")}
                </button>
              )}
            </div>
            <ul className={styles.tracks}>
              {group.tracks.map((track) => (
                <li key={track.id} className={styles.track}>
                  <span>
                    {track.artist === "" ? track.title : `${track.artist} - ${track.title}`}
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
        ))
      )}
    </section>
  );
}

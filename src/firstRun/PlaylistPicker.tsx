import { useId, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import type { PlaylistChoice } from "../bindings";
import styles from "./firstRun.module.css";
import type { PlaylistPath } from "./useRekordboxOffer";

/** One line of the tree: a folder's name, or a playlist to tick. */
type Row =
  | { kind: "folder"; depth: number; name: string; key: string }
  | { kind: "playlist"; depth: number; playlist: PlaylistChoice; key: string };

/** A path as one string, for keys and for comparing. */
export function pathKey(path: PlaylistPath): string {
  return JSON.stringify(path);
}

/**
 * The playlists as tree lines: each folder is named once, above what it
 * holds. The backend lists playlists by folder, so a folder's playlists
 * arrive together.
 */
export function treeRows(playlists: PlaylistChoice[]): Row[] {
  const rows: Row[] = [];
  let open: string[] = [];
  for (const playlist of playlists) {
    const folders = playlist.path.slice(0, -1);
    let shared = 0;
    while (shared < open.length && shared < folders.length && open[shared] === folders[shared]) {
      shared += 1;
    }
    for (let depth = shared; depth < folders.length; depth += 1) {
      rows.push({
        kind: "folder",
        depth,
        name: folders[depth],
        key: `folder:${pathKey(folders.slice(0, depth + 1))}`,
      });
    }
    open = folders;
    rows.push({
      kind: "playlist",
      depth: folders.length,
      playlist,
      key: `playlist:${pathKey(playlist.path)}`,
    });
  }
  return rows;
}

/**
 * The rekordbox playlist tree with a tick box per playlist (1aE-3). The
 * pick narrows the offer to the tracks in the ticked playlists; it lives in
 * the caller's state only and is never stored.
 */
export function PlaylistPicker({
  playlists,
  chosen,
  onChange,
}: {
  playlists: PlaylistChoice[];
  chosen: PlaylistPath[];
  onChange: (chosen: PlaylistPath[]) => void;
}) {
  const { t } = useTranslation("rekordboxOffer");
  const titleId = useId();
  const chosenKeys = new Set(chosen.map(pathKey));

  const toggle = (path: PlaylistPath, on: boolean) => {
    const key = pathKey(path);
    onChange(on ? [...chosen, path] : chosen.filter((other) => pathKey(other) !== key));
  };

  return (
    <div className={styles.picker} role="group" aria-labelledby={titleId}>
      <h3 id={titleId} className={styles.subtitle}>
        {t("playlists.title")}
      </h3>
      <p className={styles.muted}>{t("playlists.note")}</p>
      {playlists.length === 0 ? (
        <p className={styles.muted}>{t("playlists.empty")}</p>
      ) : (
        <ul className={styles.tree}>
          {treeRows(playlists).map((row) => (
            <li
              key={row.key}
              className={styles.treeRow}
              style={{ "--depth": row.depth } as CSSProperties}
            >
              {row.kind === "folder" ? (
                <span className={styles.folder}>{row.name}</span>
              ) : (
                <label className={styles.playlist}>
                  <input
                    type="checkbox"
                    checked={chosenKeys.has(pathKey(row.playlist.path))}
                    onChange={(event) => toggle(row.playlist.path, event.target.checked)}
                  />
                  <span>{row.playlist.path[row.playlist.path.length - 1]}</span>
                  <span className={styles.count}>
                    {t("playlists.tracks", { count: row.playlist.tracks })}
                  </span>
                </label>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

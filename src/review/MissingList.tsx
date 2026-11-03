import { useMutation } from "@tanstack/react-query";
import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage, unwrap } from "../api/errors";
import { commands, type MissingGroup, type MissingList as Missing, type MissingTrack } from "../bindings";
import styles from "./MissingList.module.css";

/** Called when the user asks to add a group's folder as a music folder. */
export type OnAddFolder = (folder: string) => void;

/** How many more tracks "Show more" loads (the backend answers at most 200). */
const MORE = 100;

/**
 * The Missing list (1aD-5): rekordbox tracks with no file, grouped by the
 * folder they were last in. A group comes with its first tracks, and "Show
 * more" loads the rest of them a page at a time (X3); its count is every
 * track in it, and its "add folder" is about the folder, not the rows loaded. A group whose folder exists now and isn't in a
 * music folder offers to add it. While `list` is still loading only the
 * heading shows; `problem` is a failure's message (the load's, or an
 * add's), and `adding` holds the buttons while an add runs.
 */
export function MissingList({
  list,
  problem,
  adding = false,
  waiting,
  onAddFolder,
}: {
  list: Missing | undefined;
  problem?: string;
  adding?: boolean;
  /** Shown in place of "no missing tracks" while that isn't known yet. */
  waiting?: string | null;
  onAddFolder?: OnAddFolder;
}) {
  const { t } = useTranslation("review");
  const titleId = useId();
  const folderIdPrefix = useId();
  // One line (the heading and how many tracks) until it is opened. It is
  // open when there is nothing missing (its one line says so) or when
  // something went wrong, so the message isn't hidden.
  const empty = list?.groups.length === 0;
  return (
    <details className={styles.section} aria-labelledby={titleId} open={empty || problem !== undefined}>
      <summary className={styles.summary}>
        <h2 id={titleId} className={styles.title}>
          {t("missing.title")}
        </h2>
        {list !== undefined && list.total > 0 && (
          <span className={styles.count}>{t("missing.tracksCount", { count: list.total })}</span>
        )}
      </summary>
      {problem !== undefined && (
        <p role="alert" className={styles.problem}>
          {problem}
        </p>
      )}
      {list?.groups.length === 0 && (
        <p className={styles.empty}>{waiting ?? t("missing.empty")}</p>
      )}
      {list?.groups.map((group, n) => {
        const folderId = `${folderIdPrefix}-${n}`;
        return (
          <div key={group.folder ?? ""} className={styles.group}>
            <div className={styles.heading}>
              <span id={folderId} className={styles.folder}>
                {group.folder ?? t("missing.unknownFolder")}
              </span>
              <span className={styles.count}>
                {t("missing.tracksCount", { count: group.trackIds.length })}
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
            <GroupTracks group={group} />
          </div>
        );
      })}
    </details>
  );
}

/** A group's tracks: those it came with, then each page "Show more" fetched. */
function GroupTracks({ group }: { group: MissingGroup }) {
  const { t } = useTranslation("review");
  const [fetched, setFetched] = useState<MissingTrack[]>([]);
  // Asked for but not returned: matched since, so there is nothing to show.
  const [gone, setGone] = useState<number[]>([]);
  // What the list holds now wins: a fetched track that was matched since
  // (it is gone from `trackIds`) or that the list now carries is dropped.
  const ids = new Set(group.trackIds);
  const shown = [...group.tracks];
  const have = new Set(shown.map((track) => track.id));
  for (const track of fetched) {
    if (ids.has(track.id) && !have.has(track.id)) shown.push(track);
    have.add(track.id);
  }
  for (const id of gone) have.add(id);
  const next = group.trackIds.filter((id) => !have.has(id)).slice(0, MORE);
  const more = useMutation({
    mutationFn: () => unwrap(commands.missingTrackPage(next)),
    onSuccess: (tracks) => {
      setFetched((before) => [...before, ...tracks]);
      const returned = new Set(tracks.map((track) => track.id));
      setGone((before) => [...before, ...next.filter((id) => !returned.has(id))]);
    },
  });
  const left = group.trackIds.filter((id) => !have.has(id)).length;
  return (
    <>
      <ul className={styles.tracks}>
        {shown.map((track) => (
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
      {more.error && (
        <p role="alert" className={styles.problem}>
          {errorMessage(more.error)}
        </p>
      )}
      {left > 0 && (
        <button type="button" className={styles.more} disabled={more.isPending} onClick={() => more.mutate()}>
          {t("missing.showMore")}
        </button>
      )}
    </>
  );
}

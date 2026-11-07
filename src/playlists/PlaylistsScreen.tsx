import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Playlist, TableRow } from "../bindings";
import { NameForm } from "../crates/CrateForms";
import styles from "../crates/CratesScreen.module.css";
import { PATH_SEPARATOR } from "../crates/tree";
import { EmptyState } from "../shell/StageScreen";
import stage from "../shell/StageScreen.module.css";
import { useWhenUndone } from "../shell/undoneStore";
import { shownName } from "../trackTable/columns";
import { TrackTableView } from "../trackTable/TrackTableView";
import { RekordboxPicker } from "./RekordboxPicker";
import {
  PLAYLISTS_QUERY_KEY,
  useCreatePlaylist,
  useDeletePlaylist,
  useMoveInPlaylist,
  usePlaylists,
  useRemoveFromPlaylist,
  useRenamePlaylist,
  useSavePlaylistAsCrate,
} from "./usePlaylists";

/**
 * `/playlists?playlist=ID`: an id that isn't a whole number above zero is no
 * choice at all (and one that names nothing shows nothing chosen).
 */
export function validatePlaylistsSearch(search: Record<string, unknown>): { playlist?: number } {
  const id = search.playlist;
  return typeof id === "number" && Number.isSafeInteger(id) && id > 0 ? { playlist: id } : {};
}

type Asking = "new" | "rename" | "delete" | "saveAsCrate" | "fromRekordbox" | null;

/**
 * The Playlists screen (3C-P1b; ROADMAP 3.5): the playlists, and the chosen
 * one's tracks in their order. Unlike a crate's, the table has a `#` column
 * and its rows can be dragged to another place (while nothing sorts or
 * narrows the list). A playlist copied from rekordbox says where it came
 * from. Every change is one step for the top bar's Undo.
 */
export function PlaylistsScreen({
  chosen,
  onChoose,
}: {
  chosen: number | null;
  onChoose: (id: number | null) => void;
}) {
  const { t } = useTranslation("playlists");
  const playlists = usePlaylists();
  const [asking, setAsking] = useState<Asking>(null);
  // What the strip above the screen says about the last change.
  const [done, setDone] = useState<string | null>(null);
  const create = useCreatePlaylist();
  const rename = useRenamePlaylist();
  const remove = useDeletePlaylist();
  const take = useRemoveFromPlaylist();
  const move = useMoveInPlaylist();
  const saveAsCrate = useSavePlaylistAsCrate();

  const list = playlists.data ?? [];
  const current = list.find((playlist) => playlist.id === chosen) ?? null;
  // A question about one playlist is dropped when another is chosen.
  const [askedAbout, setAskedAbout] = useState<number | null>(null);
  if ((current?.id ?? null) !== askedAbout) {
    setAskedAbout(current?.id ?? null);
    setAsking(null);
  }
  useWhenUndone(() => setDone(null));

  const mutations = [create, rename, remove, take, move, saveAsCrate];
  const clearErrors = () => mutations.forEach((m) => m.reset());
  const failure = mutations.find((m) => m.isError);
  const ask = (what: Asking) => {
    clearErrors();
    setAsking(what);
  };
  const finished = (message: string) => {
    setDone(message);
    setAsking(null);
  };

  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        <div className={styles.content}>
          {done !== null && (
            <p role="status" className={styles.status}>
              {done}
            </p>
          )}
          {failure !== undefined && (
            <p role="alert" className={styles.error}>
              {errorMessage(failure.error)}
            </p>
          )}
          {playlists.isError ? (
            <p role="alert" className={styles.error}>
              {errorMessage(playlists.error)}
            </p>
          ) : playlists.data === undefined ? null : (
            <div className={styles.columns}>
              <div className={styles.side}>
                <div className={styles.actions}>
                  <button type="button" className={styles.button} onClick={() => ask("new")}>
                    {t("new.button")}
                  </button>
                  <button type="button" className={styles.button} onClick={() => ask("fromRekordbox")}>
                    {t("fromRekordbox.button")}
                  </button>
                </div>
                {asking === "new" && (
                  <NameForm
                    key="new"
                    label={t("new.label")}
                    confirm={t("new.confirm")}
                    cancel={t("new.cancel")}
                    initial=""
                    pending={create.isPending}
                    onSubmit={(name) => {
                      clearErrors();
                      create.mutate(name, {
                        onSuccess: (id) => {
                          onChoose(id);
                          finished(t("done.created", { name: name.trim() }));
                        },
                      });
                    }}
                    onCancel={() => ask(null)}
                  />
                )}
                {asking === "fromRekordbox" && (
                  <RekordboxPicker
                    onDone={(copied, name) => {
                      onChoose(copied.playlistId);
                      finished(
                        copied.notInLibrary > 0
                          ? t("done.copiedLeftOut", { name, count: copied.added, left: copied.notInLibrary })
                          : t("done.copied", { name, count: copied.added }),
                      );
                    }}
                    onCancel={() => ask(null)}
                  />
                )}
                {list.length === 0 ? (
                  <EmptyState>{t("empty")}</EmptyState>
                ) : (
                  <ul className={styles.list} aria-label={t("list.label")}>
                    {list.map((playlist) => (
                      <li key={playlist.id}>
                        <button
                          type="button"
                          className={styles.crate}
                          aria-pressed={playlist.id === current?.id}
                          onClick={() => {
                            clearErrors();
                            setAsking(null);
                            onChoose(playlist.id);
                          }}
                        >
                          <span className={styles.name}>{playlist.name}</span>
                          <span className={styles.count}>{t("trackCount", { count: playlist.trackCount })}</span>
                        </button>
                      </li>
                    ))}
                  </ul>
                )}
              </div>
              <div className={styles.main}>
                {current === null ? (
                  list.length > 0 && <EmptyState>{t("select")}</EmptyState>
                ) : (
                  <PlaylistView
                    playlist={current}
                    asking={asking}
                    pending={rename.isPending || remove.isPending || saveAsCrate.isPending}
                    onAsk={ask}
                    onRename={(name) => {
                      clearErrors();
                      rename.mutate(
                        { id: current.id, name },
                        {
                          // The name it has: nothing is recorded, so nothing is said.
                          onSuccess: (operation) =>
                            operation === null
                              ? setAsking(null)
                              : finished(t("done.renamed", { name: name.trim() })),
                        },
                      );
                    }}
                    onDelete={() => {
                      clearErrors();
                      remove.mutate(current.id, {
                        onSuccess: () => {
                          onChoose(null);
                          finished(t("done.deleted", { name: current.name }));
                        },
                      });
                    }}
                    onSaveAsCrate={(name) => {
                      clearErrors();
                      saveAsCrate.mutate(
                        { id: current.id, name },
                        { onSuccess: () => finished(t("done.savedAsCrate", { name: name.trim() })) },
                      );
                    }}
                    onRemoveTrack={(row) => {
                      if (row.entryId === null) return;
                      clearErrors();
                      take.mutate(
                        { id: current.id, entries: [row.entryId] },
                        {
                          // An entry that was gone already records nothing.
                          onSuccess: (result) => {
                            if (result.operationId !== null) finished(t("done.removed", { title: shownName(row) }));
                          },
                        },
                      );
                    }}
                    onMoveRows={(rows, before) => {
                      clearErrors();
                      move.mutate(
                        {
                          id: current.id,
                          entries: rows.flatMap((row) => (row.entryId === null ? [] : [row.entryId])),
                          before: before?.entryId ?? null,
                        },
                        {
                          // Dropped where they were: nothing is recorded, so nothing is said.
                          onSuccess: (result) => {
                            if (result.operationId !== null) finished(t("done.moved", { count: result.changed }));
                          },
                        },
                      );
                    }}
                  />
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function PlaylistView({
  playlist,
  asking,
  pending,
  onAsk,
  onRename,
  onDelete,
  onSaveAsCrate,
  onRemoveTrack,
  onMoveRows,
}: {
  playlist: Playlist;
  asking: Asking;
  pending: boolean;
  onAsk: (what: Asking) => void;
  onRename: (name: string) => void;
  onDelete: () => void;
  onSaveAsCrate: (name: string) => void;
  onRemoveTrack: (row: TableRow) => void;
  onMoveRows: (rows: readonly TableRow[], before: TableRow | null) => void;
}) {
  const { t } = useTranslation("playlists");
  return (
    <>
      <div className={styles.header}>
        <div className={styles.titles}>
          <h2 className={styles.crateName}>{playlist.name}</h2>
          {playlist.sourcePath !== null && (
            <p className={styles.path}>{t("copiedFrom", { path: playlist.sourcePath.join(PATH_SEPARATOR) })}</p>
          )}
        </div>
        <div className={styles.actions}>
          <button type="button" className={styles.button} onClick={() => onAsk("saveAsCrate")}>
            {t("saveAsCrate.button")}
          </button>
          <button type="button" className={styles.button} onClick={() => onAsk("rename")}>
            {t("rename.button")}
          </button>
          <button type="button" className={styles.button} onClick={() => onAsk("delete")}>
            {t("delete.button")}
          </button>
        </div>
      </div>
      {asking === "rename" && (
        <NameForm
          key={`rename-${playlist.id}`}
          label={t("rename.label")}
          confirm={t("rename.confirm")}
          cancel={t("rename.cancel")}
          initial={playlist.name}
          pending={pending}
          onSubmit={onRename}
          onCancel={() => onAsk(null)}
        />
      )}
      {asking === "saveAsCrate" && (
        <NameForm
          key={`save-${playlist.id}`}
          label={t("saveAsCrate.label")}
          confirm={t("saveAsCrate.confirm")}
          cancel={t("saveAsCrate.cancel")}
          initial={playlist.name}
          pending={pending}
          onSubmit={onSaveAsCrate}
          onCancel={() => onAsk(null)}
        />
      )}
      {asking === "delete" && (
        <div role="alertdialog" aria-label={t("delete.title", { name: playlist.name })} className={styles.confirm}>
          <p className={styles.confirmTitle}>{t("delete.title", { name: playlist.name })}</p>
          <p className={styles.confirmDetail}>{t("delete.detail")}</p>
          <div className={styles.actions}>
            <button type="button" className={styles.primary} disabled={pending} onClick={onDelete}>
              {t("delete.confirm")}
            </button>
            <button type="button" className={styles.button} onClick={() => onAsk(null)}>
              {t("delete.cancel")}
            </button>
          </div>
        </div>
      )}
      {/* A playlist's table starts afresh (its own sort and filters) for another playlist. */}
      <TrackTableView
        key={playlist.id}
        screen="playlist"
        source={{ kind: "playlist", id: playlist.id }}
        keyPrefix={[...PLAYLISTS_QUERY_KEY, "table", playlist.id]}
        count={(total) => t("trackCount", { count: total })}
        empty={<EmptyState>{t("noTracks")}</EmptyState>}
        actionsWidth={96}
        onMoveRows={onMoveRows}
        actions={(row) =>
          row.entryId !== null && (
            <button
              type="button"
              className={styles.button}
              aria-label={t("removeTrack.buttonFor", { title: shownName(row) })}
              onClick={() => onRemoveTrack(row)}
            >
              {t("removeTrack.button")}
            </button>
          )
        }
      />
    </>
  );
}

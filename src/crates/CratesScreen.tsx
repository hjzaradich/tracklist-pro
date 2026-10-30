import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate, FolderContents } from "../bindings";
import { libraryRow, type LibraryRow } from "../library/libraryRow";
import { shownTitle } from "../library/RemoveFromLibrary";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { useWhenUndone } from "../shell/undoneStore";
import { TrackTableView } from "../trackTable/TrackTableView";
import { ConfirmDelete, MoveForm, NameForm } from "./CrateForms";
import { CrateNotes, CrateSummaryLine } from "./CrateInfo";
import styles from "./CratesScreen.module.css";
import { buildTree, PATH_SEPARATOR, pathOf, type TreeNode } from "./tree";
import {
  CRATES_QUERY_KEY,
  useCrates,
  useCreateCrate,
  useCreateFolder,
  useDeleteCrate,
  useFolderContents,
  useMoveCrate,
  useRemoveFromCrate,
  useRenameCrate,
} from "./useCrates";
import { useSetCrateNotes } from "./useCrateSummary";

/** What the strip above the screen says about the last change. */
type Done =
  | "created"
  | "createdFolder"
  | "renamed"
  | "renamedFolder"
  | "deleted"
  | "deletedFolder"
  | "moved"
  | "removed"
  | "notesSaved"
  | "notesCleared";

/**
 * What the screen is asking for, if anything. The first four make something
 * new: at the top of the tree, or inside the chosen folder.
 */
type Asking =
  | "new"
  | "newFolder"
  | "newInside"
  | "newFolderInside"
  | "rename"
  | "move"
  | "delete"
  | null;

/**
 * The Crates screen (1aG-8, 1cA-11): the tree of folders and crates, and the
 * chosen crate's tracks. Which one is chosen is the page's address
 * (`/crates?crate=ID`), so the sidebar's tree can choose too; an id that
 * isn't there is no choice. A crate shows its note and a summary line of
 * its tracks (1cB-15), and its table can be sorted by fit (1cB-16). Every change can be undone, from the top bar or
 * with Ctrl+Z; this screen offers no Undo of its own (owner decision, 2026-10-27).
 */
export function CratesScreen({
  chosen,
  onChoose,
}: {
  chosen: number | null;
  onChoose: (id: number | null) => void;
}) {
  const { t } = useTranslation("crates");
  const crates = useCrates();
  const [asking, setAsking] = useState<Asking>(null);
  const [done, setDone] = useState<Done | null>(null);
  const create = useCreateCrate();
  const createFolder = useCreateFolder();
  const rename = useRenameCrate();
  const remove = useDeleteCrate();
  const move = useMoveCrate();
  const take = useRemoveFromCrate();
  const setNotes = useSetCrateNotes();

  const list = crates.data ?? [];
  // The crate or folder the right side shows: the chosen one, if it exists.
  const current = list.find((crate) => crate.id === chosen) ?? null;
  // A question about one crate or folder is dropped when another is chosen,
  // whether from this list or from the sidebar's tree.
  const [askedAbout, setAskedAbout] = useState<number | null>(null);
  if ((current?.id ?? null) !== askedAbout) {
    setAskedAbout(current?.id ?? null);
    setAsking(null);
  }
  const contents = useFolderContents(
    asking === "delete" && current?.kind === "folder" ? current.id : null,
  );

  // Undone from the top bar: the line describes a change that no longer stands.
  useWhenUndone(() => setDone(null));

  const finished = (what: Done) => {
    setDone(what);
    setAsking(null);
  };
  // Each new attempt, and each Go back, clears what failed before, so the
  // error on screen is always the newest one.
  const clearErrors = () => {
    create.reset();
    createFolder.reset();
    rename.reset();
    remove.reset();
    move.reset();
    take.reset();
    setNotes.reset();
  };
  const failure = [create, createFolder, rename, remove, move, take, setNotes].find((m) => m.isError);
  const ask = (what: Asking) => {
    clearErrors();
    setAsking(what);
  };
  // Where a new crate or folder goes.
  const parent = (inside: boolean) => (inside && current?.kind === "folder" ? current.id : null);

  return (
    <StageScreen stage="crates">
      <div className={styles.content}>
        {done !== null && (
          <p role="status" className={styles.status}>
            {t(`done.${done}`)}
          </p>
        )}
        {failure !== undefined && (
          <p role="alert" className={styles.error}>
            {errorMessage(failure.error)}
          </p>
        )}
        {crates.isError ? (
          <p role="alert" className={styles.error}>
            {errorMessage(crates.error)}
          </p>
        ) : crates.data === undefined ? null : (
          <div className={styles.columns}>
            <div className={styles.side}>
              <div className={styles.actions}>
                <button type="button" className={styles.button} onClick={() => ask("new")}>
                  {t("new.button")}
                </button>
                <button type="button" className={styles.button} onClick={() => ask("newFolder")}>
                  {t("newFolder.button")}
                </button>
              </div>
              {(asking === "new" || asking === "newInside") && (
                <NameForm
                  key={asking}
                  label={
                    asking === "newInside"
                      ? t("newInside.label", { name: current?.name ?? "" })
                      : t("new.label")
                  }
                  confirm={t("new.confirm")}
                  cancel={t("new.cancel")}
                  initial=""
                  pending={create.isPending}
                  onSubmit={(name) => {
                    clearErrors();
                    create.mutate(
                      { name, parent: parent(asking === "newInside") },
                      {
                        onSuccess: (id) => {
                          onChoose(id);
                          finished("created");
                        },
                      },
                    );
                  }}
                  onCancel={() => ask(null)}
                />
              )}
              {(asking === "newFolder" || asking === "newFolderInside") && (
                <NameForm
                  key={asking}
                  label={
                    asking === "newFolderInside"
                      ? t("newFolderInside.label", { name: current?.name ?? "" })
                      : t("newFolder.label")
                  }
                  confirm={t("newFolder.confirm")}
                  cancel={t("newFolder.cancel")}
                  initial=""
                  pending={createFolder.isPending}
                  onSubmit={(name) => {
                    clearErrors();
                    createFolder.mutate(
                      { name, parent: parent(asking === "newFolderInside") },
                      {
                        onSuccess: (id) => {
                          onChoose(id);
                          finished("createdFolder");
                        },
                      },
                    );
                  }}
                  onCancel={() => ask(null)}
                />
              )}
              {list.length === 0 ? (
                <EmptyState>{t("empty")}</EmptyState>
              ) : (
                <ul className={styles.list} aria-label={t("list.label")}>
                  <TreeRows
                    nodes={buildTree(list)}
                    chosen={current?.id ?? null}
                    onChoose={(id) => {
                      clearErrors();
                      setAsking(null);
                      onChoose(id);
                    }}
                  />
                </ul>
              )}
            </div>
            <div className={styles.main}>
              {current === null ? (
                list.length > 0 && <EmptyState>{t("select")}</EmptyState>
              ) : (
                <CrateView
                  crate={current}
                  all={list}
                  asking={asking}
                  renaming={rename.isPending}
                  deleting={remove.isPending}
                  moving={move.isPending}
                  contents={contents.data}
                  contentsError={contents.error}
                  onAsk={ask}
                  onRename={(name) => {
                    clearErrors();
                    rename.mutate(
                      { id: current.id, name },
                      {
                        // Nothing is recorded when the name is the one it has:
                        // nothing changed, so there is nothing to say.
                        onSuccess: (operation) =>
                          operation === null
                            ? setAsking(null)
                            : finished(current.kind === "folder" ? "renamedFolder" : "renamed"),
                      },
                    );
                  }}
                  onMove={(folder) => {
                    clearErrors();
                    move.mutate(
                      { id: current.id, folder },
                      {
                        // Moved to where it is: nothing is recorded.
                        onSuccess: (operation) =>
                          operation === null ? setAsking(null) : finished("moved"),
                      },
                    );
                  }}
                  onDelete={() => {
                    clearErrors();
                    remove.mutate(current.id, {
                      onSuccess: () => {
                        onChoose(null);
                        finished(current.kind === "folder" ? "deletedFolder" : "deleted");
                      },
                    });
                  }}
                  onSaveNotes={(notes) => {
                    clearErrors();
                    setNotes.mutate(
                      { id: current.id, notes },
                      {
                        // The note it has: nothing is recorded, so nothing is said.
                        onSuccess: (operation) => {
                          if (operation !== null) finished(notes.trim() === "" ? "notesCleared" : "notesSaved");
                        },
                      },
                    );
                  }}
                  onRemoveTrack={(track) => {
                    clearErrors();
                    take.mutate(
                      { id: current.id, tracks: [track.id] },
                      {
                        // A track that was gone already records nothing.
                        onSuccess: (result) => {
                          if (result.operationId !== null) finished("removed");
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
    </StageScreen>
  );
}

/** The tree's rows: a folder, then what is in it, nested one level in. */
function TreeRows({
  nodes,
  chosen,
  onChoose,
}: {
  nodes: TreeNode[];
  chosen: number | null;
  onChoose: (id: number) => void;
}) {
  const { t } = useTranslation("crates");
  return nodes.map(({ crate, children }) => (
    <li key={crate.id}>
      <button
        type="button"
        className={`${styles.crate} ${crate.kind === "folder" ? styles.folder : ""}`}
        aria-pressed={crate.id === chosen}
        onClick={() => onChoose(crate.id)}
      >
        <span className={styles.name}>{crate.name}</span>
        {crate.kind === "static" && (
          <span className={styles.count}>{t("trackCount", { count: crate.trackCount })}</span>
        )}
      </button>
      {children.length > 0 && (
        <ul className={`${styles.list} ${styles.nested}`}>
          <TreeRows nodes={children} chosen={chosen} onChoose={onChoose} />
        </ul>
      )}
    </li>
  ));
}

function CrateView({
  crate,
  all,
  asking,
  renaming,
  deleting,
  moving,
  contents,
  contentsError,
  onAsk,
  onRename,
  onMove,
  onDelete,
  onSaveNotes,
  onRemoveTrack,
}: {
  crate: Crate;
  all: Crate[];
  asking: Asking;
  renaming: boolean;
  deleting: boolean;
  moving: boolean;
  contents: FolderContents | undefined;
  contentsError: unknown;
  onAsk: (what: Asking) => void;
  onRename: (name: string) => void;
  onMove: (folder: number | null) => void;
  onDelete: () => void;
  onSaveNotes: (notes: string) => void;
  onRemoveTrack: (track: LibraryRow) => void;
}) {
  const { t } = useTranslation("crates");
  const folder = crate.kind === "folder";
  return (
    <>
      <div className={styles.header}>
        <div className={styles.titles}>
          <h2 className={styles.crateName}>{crate.name}</h2>
          {/* A folder holds crates, so it has no notes or summary (ROADMAP 1.14). */}
          {!folder && <CrateNotes key={crate.id} id={crate.id} onSave={onSaveNotes} />}
          {/* Where rekordbox will show it after a send (ROADMAP 1.9 and 1.14). */}
          <p className={styles.path} aria-label={t("path.label")}>
            {[t("path.root"), ...pathOf(all, crate.id)].join(PATH_SEPARATOR)}
          </p>
          {!folder && <CrateSummaryLine id={crate.id} />}
        </div>
        <div className={styles.actions}>
          {folder && (
            <>
              <button type="button" className={styles.button} onClick={() => onAsk("newInside")}>
                {t("newInside.button")}
              </button>
              <button type="button" className={styles.button} onClick={() => onAsk("newFolderInside")}>
                {t("newFolderInside.button")}
              </button>
            </>
          )}
          <button type="button" className={styles.button} onClick={() => onAsk("move")}>
            {t("move.button")}
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
          key={`rename-${crate.id}`}
          label={t("rename.label")}
          confirm={t("rename.confirm")}
          cancel={t("rename.cancel")}
          initial={crate.name}
          pending={renaming}
          onSubmit={onRename}
          onCancel={() => onAsk(null)}
        />
      )}
      {asking === "move" && (
        <MoveForm
          key={`move-${crate.id}`}
          crate={crate}
          all={all}
          pending={moving}
          onSubmit={onMove}
          onCancel={() => onAsk(null)}
        />
      )}
      {asking === "delete" && (
        <ConfirmDelete
          key={`delete-${crate.id}`}
          crate={crate}
          contents={contents}
          contentsError={contentsError}
          pending={deleting}
          onConfirm={onDelete}
          onCancel={() => onAsk(null)}
        />
      )}
      {/* A crate's table starts afresh (its own sort and filters) for another crate. */}
      {folder ? (
        <EmptyState>
          {all.some((c) => c.parentId === crate.id) ? t("folder.select") : t("folder.empty")}
        </EmptyState>
      ) : (
        <TrackTableView
          key={crate.id}
          screen="crate"
          source={{ kind: "crate", id: crate.id }}
          keyPrefix={[...CRATES_QUERY_KEY, "table", crate.id]}
          count={(total) => t("trackCount", { count: total })}
          empty={<EmptyState>{t("noTracks")}</EmptyState>}
          actionsWidth={96}
          actions={(row) => {
            const track = libraryRow(row);
            return (
              track && (
                <button
                  type="button"
                  className={styles.button}
                  aria-label={t("removeTrack.buttonFor", { title: shownTitle(track) })}
                  onClick={() => onRemoveTrack(track)}
                >
                  {t("removeTrack.button")}
                </button>
              )
            );
          }}
        />
      )}
    </>
  );
}

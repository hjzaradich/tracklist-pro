import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Crate, LibraryTrack } from "../bindings";
import { LibraryTrackList } from "../library/LibraryTrackList";
import { shownTitle } from "../library/RemoveFromLibrary";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { useUndoAfterAction } from "../shell/useUndo";
import styles from "./CratesScreen.module.css";
import {
  useCrates,
  useCrateTracks,
  useCreateCrate,
  useDeleteCrate,
  useRemoveFromCrate,
  useRenameCrate,
} from "./useCrates";

/** What the strip above the screen says about the last change. */
type Done = "created" | "renamed" | "deleted" | "removed";

/** What the screen is asking for, if anything. */
type Asking = "new" | "rename" | "delete" | null;

/**
 * The Crates screen (1aG-8): the crates with their track counts, and the
 * chosen crate's tracks. Folders, notes, summaries and drag and drop wait
 * for Phase 1c (ROADMAP 1.14). Every change can be undone.
 */
export function CratesScreen() {
  const { t } = useTranslation("crates");
  const crates = useCrates();
  const [chosen, choose] = useState<number | null>(null);
  const [asking, setAsking] = useState<Asking>(null);
  const [done, setDone] = useState<Done | null>(null);
  const create = useCreateCrate();
  const rename = useRenameCrate();
  const remove = useDeleteCrate();
  const take = useRemoveFromCrate();
  const { arm, offered, undo } = useUndoAfterAction();

  const list = crates.data ?? [];
  // The crate the right side shows: the chosen one, if it still exists.
  const current = list.find((crate) => crate.id === chosen) ?? null;
  const tracks = useCrateTracks(current?.id ?? null);

  const finished = (what: Done) => {
    undo.reset();
    arm();
    setDone(what);
    setAsking(null);
  };
  // Each new attempt, and each Go back, clears what failed before, so the
  // error on screen is always the newest one.
  const clearErrors = () => {
    create.reset();
    rename.reset();
    remove.reset();
    take.reset();
    undo.reset();
  };
  const failure = [create, rename, remove, take, undo].find((m) => m.isError);

  return (
    <StageScreen stage="crates">
      <div className={styles.content}>
        {done !== null && (
          <p role="status" className={styles.status}>
            {t(`done.${done}`)}
            {/* Gone once the change isn't the next step to undo any more. */}
            {offered && (
              <button
                type="button"
                className={styles.button}
                disabled={undo.isPending}
                onClick={() => {
                  clearErrors();
                  undo.mutate(undefined, { onSuccess: () => setDone(null) });
                }}
              >
                {t("undo")}
              </button>
            )}
          </p>
        )}
        {done === null && undo.data?.status === "undone" && (
          <p role="status" className={styles.status}>
            {t("undone")}
          </p>
        )}
        {undo.data !== undefined && undo.data.status !== "undone" && (
          <p role="alert" className={styles.error}>
            {t("undoRefused")}
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
                <button
                  type="button"
                  className={styles.button}
                  onClick={() => {
                    clearErrors();
                    setAsking("new");
                  }}
                >
                  {t("new.button")}
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
                        choose(id);
                        finished("created");
                      },
                    });
                  }}
                  onCancel={() => {
                    clearErrors();
                    setAsking(null);
                  }}
                />
              )}
              {list.length === 0 ? (
                <EmptyState>{t("empty")}</EmptyState>
              ) : (
                <ul className={styles.list} aria-label={t("list.label")}>
                  {list.map((crate) => (
                    <li key={crate.id}>
                      <button
                        type="button"
                        className={styles.crate}
                        aria-pressed={crate.id === current?.id}
                        onClick={() => {
                          clearErrors();
                          setAsking(null);
                          choose(crate.id);
                        }}
                      >
                        <span className={styles.name}>{crate.name}</span>
                        <span className={styles.count}>
                          {t("trackCount", { count: crate.trackCount })}
                        </span>
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
                <CrateView
                  crate={current}
                  tracks={tracks.data}
                  tracksError={tracks.isError ? errorMessage(tracks.error) : null}
                  asking={asking}
                  renaming={rename.isPending}
                  deleting={remove.isPending}
                  onAsk={(what) => {
                    clearErrors();
                    setAsking(what);
                  }}
                  onRename={(name) => {
                    clearErrors();
                    rename.mutate(
                      { id: current.id, name },
                      {
                        // Nothing is recorded when the name is the one it has,
                        // and Undo would then take back an earlier operation.
                        onSuccess: (operation) =>
                          operation === null ? setAsking(null) : finished("renamed"),
                      },
                    );
                  }}
                  onDelete={() => {
                    clearErrors();
                    remove.mutate(current.id, {
                      onSuccess: () => {
                        choose(null);
                        finished("deleted");
                      },
                    });
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

function CrateView({
  crate,
  tracks,
  tracksError,
  asking,
  renaming,
  deleting,
  onAsk,
  onRename,
  onDelete,
  onRemoveTrack,
}: {
  crate: Crate;
  tracks: LibraryTrack[] | undefined;
  tracksError: string | null;
  asking: Asking;
  renaming: boolean;
  deleting: boolean;
  onAsk: (what: Asking) => void;
  onRename: (name: string) => void;
  onDelete: () => void;
  onRemoveTrack: (track: LibraryTrack) => void;
}) {
  const { t } = useTranslation("crates");
  return (
    <>
      <div className={styles.header}>
        <h2 className={styles.crateName}>{crate.name}</h2>
        <div className={styles.actions}>
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
      {asking === "delete" && (
        <ConfirmDelete
          key={`delete-${crate.id}`}
          crate={crate}
          pending={deleting}
          onConfirm={onDelete}
          onCancel={() => onAsk(null)}
        />
      )}
      {tracksError !== null ? (
        <p role="alert" className={styles.error}>
          {tracksError}
        </p>
      ) : tracks === undefined ? null : tracks.length === 0 ? (
        <EmptyState>{t("noTracks")}</EmptyState>
      ) : (
        <div className={styles.scroll}>
          <LibraryTrackList
            tracks={tracks}
            actions={(track) => (
              <button
                type="button"
                className={styles.button}
                aria-label={t("removeTrack.buttonFor", { title: shownTitle(track) })}
                onClick={() => onRemoveTrack(track)}
              >
                {t("removeTrack.button")}
              </button>
            )}
          />
        </div>
      )}
    </>
  );
}

/** One line to type a crate's name in. Enter confirms, Escape goes back. */
function NameForm({
  label,
  confirm,
  cancel,
  initial,
  pending,
  onSubmit,
  onCancel,
}: {
  label: string;
  confirm: string;
  cancel: string;
  initial: string;
  pending: boolean;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial);
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  useEffect(() => input.current?.focus(), []);
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        onSubmit(name);
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <label htmlFor={id} className={styles.label}>
        {label}
      </label>
      <input
        ref={input}
        id={id}
        className={styles.input}
        value={name}
        onChange={(event) => setName(event.target.value)}
      />
      <div className={styles.actions}>
        <button type="submit" className={styles.primary} disabled={pending}>
          {confirm}
        </button>
        <button type="button" className={styles.button} onClick={onCancel}>
          {cancel}
        </button>
      </div>
    </form>
  );
}

/** Asks before a crate is deleted. Nothing happens until the user confirms. */
function ConfirmDelete({
  crate,
  pending,
  onConfirm,
  onCancel,
}: {
  crate: Crate;
  pending: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t } = useTranslation("crates");
  // The dialog takes focus, on the safe choice.
  const cancel = useRef<HTMLButtonElement>(null);
  useEffect(() => cancel.current?.focus(), []);
  const titleId = useId();
  const detailId = useId();
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={detailId}
      className={styles.confirm}
      onKeyDown={(event) => {
        if (event.key === "Escape") onCancel();
      }}
    >
      <p id={titleId} className={styles.confirmTitle}>
        {t("delete.title", { name: crate.name })}
      </p>
      <p id={detailId} className={styles.confirmDetail}>
        {t("delete.detail")}
      </p>
      <div className={styles.actions}>
        <button type="button" className={styles.primary} disabled={pending} onClick={onConfirm}>
          {t("delete.confirm")}
        </button>
        <button ref={cancel} type="button" className={styles.button} onClick={onCancel}>
          {t("delete.cancel")}
        </button>
      </div>
    </div>
  );
}

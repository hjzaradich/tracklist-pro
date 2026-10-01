import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { AddSummary } from "../bindings";
import styles from "./firstRun.module.css";
import { PlaylistPicker } from "./PlaylistPicker";
import {
  useAddRekordboxTracks,
  useRekordboxOffer,
  useReloadOfferWhenJobsEnd,
  useRekordboxPlaylists,
  useUndoAddRekordboxTracks,
  type PlaylistPath,
} from "./useRekordboxOffer";

/** What an undo of the add came to. */
type Undone = "undone" | "refused";

/**
 * The offer to add rekordbox tracks to the Library (1aE-2, 1aE-3; ROADMAP
 * 1.3): "N rekordbox tracks aren't in your Library", with the action that
 * adds them. Nothing is added until the user asks. The same offer serves
 * the first run (`firstRun`: titled "Start from rekordbox") and every later
 * rekordbox read.
 *
 * It shows only while there's something to add or a summary to read (or
 * the offer couldn't be read, which is said). The
 * offer can be narrowed to chosen playlists; the pick is this component's
 * state and is gone when it closes. After an add, the summary says what
 * happened and offers one undo for the whole batch.
 */
export function RekordboxOffer({ firstRun = false }: { firstRun?: boolean }) {
  const { t } = useTranslation("rekordboxOffer");
  const { t: tFirstRun } = useTranslation("firstRun");
  const titleId = useId();
  // null: the whole collection. An array: only the tracks in these playlists.
  const [chosen, setChosen] = useState<PlaylistPath[] | null>(null);
  const [summary, setSummary] = useState<AddSummary | null>(null);
  const [undone, setUndone] = useState<Undone | null>(null);

  useReloadOfferWhenJobsEnd();
  const whole = useRekordboxOffer(null);
  const narrowed = useRekordboxOffer(chosen);
  const playlists = useRekordboxPlaylists(chosen !== null);
  const add = useAddRekordboxTracks();
  const undo = useUndoAddRekordboxTracks();

  const title = (
    <h2 id={titleId} className={styles.title}>
      {firstRun ? tFirstRun("fromRekordbox.title") : t("title")}
    </h2>
  );

  if (summary !== null) {
    const operationId = summary.operationId;
    return (
      <section className={styles.panel} aria-labelledby={titleId}>
        {title}
        {undone === "undone" ? (
          <p role="status">{t("summary.undone")}</p>
        ) : (
          <ul className={styles.summary} aria-label={t("title")}>
            <li>{t("summary.added", { count: summary.added })}</li>
            {summary.alreadyInLibrary > 0 && (
              <li>{t("summary.alreadyInLibrary", { count: summary.alreadyInLibrary })}</li>
            )}
            {summary.waitingInMissing > 0 && (
              <li>{t("summary.waitingInMissing", { count: summary.waitingInMissing })}</li>
            )}
            {summary.waitingForConfirmation > 0 && (
              <li>
                {t("summary.waitingForConfirmation", { count: summary.waitingForConfirmation })}
              </li>
            )}
          </ul>
        )}
        {undone === "refused" && (
          <p role="alert" className={styles.problem}>
            {t("summary.cantUndo")}
          </p>
        )}
        {undo.isError && (
          <p role="alert" className={styles.problem}>
            {errorMessage(undo.error)}
          </p>
        )}
        <div className={styles.actions}>
          {operationId !== null && undone === null && (
            <button
              type="button"
              className={styles.button}
              disabled={undo.isPending}
              aria-busy={undo.isPending}
              onClick={() =>
                undo.mutate(operationId, {
                  onSuccess: (outcome) =>
                    setUndone(outcome.status === "undone" ? "undone" : "refused"),
                })
              }
            >
              {t("summary.undo")}
            </button>
          )}
          <button
            type="button"
            className={styles.button}
            onClick={() => {
              setSummary(null);
              setUndone(null);
              setChosen(null);
              undo.reset();
            }}
          >
            {t("summary.done")}
          </button>
        </div>
      </section>
    );
  }

  // A failure is said, never shown as "nothing to add".
  if (whole.isError) {
    return (
      <section className={styles.panel} aria-labelledby={titleId}>
        {title}
        <p role="alert" className={styles.problem}>
          {errorMessage(whole.error)}
        </p>
      </section>
    );
  }
  // Shown only when the count is above zero.
  if (whole.data === undefined || whole.data.toAdd === 0) return null;
  const pickFailure = chosen === null ? null : (playlists.error ?? narrowed.error);

  const count = chosen === null ? whole.data.toAdd : narrowed.data?.toAdd;

  return (
    <section className={styles.panel} aria-labelledby={titleId}>
      {title}
      {firstRun && <p className={styles.muted}>{tFirstRun("fromRekordbox.help")}</p>}
      {chosen !== null && playlists.data !== undefined && (
        <PlaylistPicker playlists={playlists.data} chosen={chosen} onChange={setChosen} />
      )}
      {pickFailure && (
        <p role="alert" className={styles.problem}>
          {errorMessage(pickFailure)}
        </p>
      )}
      {count !== undefined && (chosen === null || playlists.data !== undefined) && (
        <p role="status">
          {chosen === null
            ? t("count", { count })
            : chosen.length === 0
              ? t("noPlaylistsChosen")
              : count === 0
                ? t("noneInPlaylists")
                : t("countInPlaylists", { count })}
        </p>
      )}
      {add.isError && (
        <p role="alert" className={styles.problem}>
          {errorMessage(add.error)}
        </p>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.primary}
          disabled={add.isPending || count === undefined || count === 0}
          aria-busy={add.isPending}
          onClick={() => add.mutate(chosen, { onSuccess: setSummary })}
        >
          {t("add", { count: count ?? 0 })}
        </button>
        <button
          type="button"
          className={styles.button}
          disabled={add.isPending}
          onClick={() => setChosen(chosen === null ? [] : null)}
        >
          {chosen === null ? t("choosePlaylists") : t("wholeCollection")}
        </button>
      </div>
    </section>
  );
}

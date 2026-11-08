import { useQueryClient } from "@tanstack/react-query";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { reloadLists } from "../app/lists";
import type { Guess, PanelState, TableRow, YourGenre } from "../bindings";
import { useSelectedTrack } from "../shell/selectedTrack";
import { useSetGenre } from "../trackTable/useSetGenre";
import { useAudioGuesses, useAudioModelStatus, useSoundsLike, useYourGenres } from "./useAudioModel";
import styles from "./AudioModelPanel.module.css";

/** The keys of the labels the model can guess; a Rust test checks the backend sends only these. */
type LabelKey = keyof typeof import("../locales/en/audioModel.json")["label"];

/**
 * "Audio model" in the Details panel (ROADMAP 3.8): the model's genre and mood
 * guesses for the selected track and the Library tracks that sound like it.
 * Shown only while the switch is on. It is the model's opinion on its own:
 * nothing here is applied to a track by itself, and it feeds no other
 * suggestion. A Library track's genre guesses each have "Use" (owner
 * decision 2026-11-08), which sets the track's genre to the guess's name as
 * the user's own edit: one Undo, sent to rekordbox like any edit. A guess
 * that is already the track's genre has none.
 *
 * Give it a `key` per track, so one track's answer doesn't stay under the next.
 */
export function AudioModelPanel({ row }: { row: TableRow }) {
  const status = useAudioModelStatus();
  if (status.data?.enabled !== true) return null;
  return <Panel row={row} listened={status.data.fingerprinted} total={status.data.total} />;
}

function Panel({ row, listened, total }: { row: TableRow; listened: number; total: number }) {
  const { t } = useTranslation("audioModel");
  const headingId = useId();
  const guesses = useAudioGuesses(row.recordingId);
  const ready = guesses.data?.state === "ready";
  return (
    <section aria-labelledby={headingId} className={styles.section}>
      <h3 id={headingId} className={styles.heading}>
        {t("title")}
      </h3>
      <p className={styles.quiet}>{t("opinion")}</p>
      {guesses.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(guesses.error)}
        </p>
      )}
      {guesses.isPending && <p className={styles.quiet}>{t("loading")}</p>}
      {guesses.data !== undefined && !ready && (
        <p className={styles.quiet}>{stateText(guesses.data.state, listened, total, t)}</p>
      )}
      {guesses.data !== undefined && ready && (
        <>
          <dl className={styles.guesses}>
            <GenreGuesses row={row} guesses={guesses.data.genres} />
            <GuessLine name={t("mood")} guesses={guesses.data.mood === null ? [] : [guesses.data.mood]} none={t("noMood")} />
          </dl>
          <YourGenres row={row} />
          <SoundsLike row={row} />
        </>
      )}
    </section>
  );
}

type T = ReturnType<typeof useTranslation<"audioModel">>["t"];

/** What a state other than "ready" says; "off" never reaches the screen. */
function stateText(state: PanelState, listened: number, total: number, t: T): string {
  switch (state) {
    case "notListenedYet":
      return t("state.notListened", { done: listened, count: total });
    case "couldntBeRead":
      return t("state.couldntRead");
    case "notInLibrary":
      return t("state.notInLibrary");
    default:
      return "";
  }
}

function GuessLine({ name, guesses, none }: { name: string; guesses: Guess[]; none: string }) {
  const { t } = useTranslation("audioModel");
  return (
    <div className={styles.guessLine}>
      <dt className={styles.name}>{name}</dt>
      <dd className={styles.value}>
        {guesses.length === 0
          ? none
          : guesses
              .map((guess) => t("guess", { label: t(`label.${guess.key as LabelKey}`), percent: guess.percent }))
              .join(t("guessSeparator"))}
      </dd>
    </div>
  );
}

/**
 * The genre guesses, each with "Use" for a Library track: it sets the track's
 * genre to the guess's name as shown (useSetGenre). A guess that is already
 * the track's genre has no "Use".
 */
function GenreGuesses({ row, guesses }: { row: TableRow; guesses: Guess[] }) {
  const { t } = useTranslation("audioModel");
  const setGenre = useSetGenre();
  const libraryTrackId = row.libraryTrackId;
  return (
    <div className={styles.guessLine}>
      <dt className={styles.name}>{t("genre")}</dt>
      <dd className={styles.value}>
        {guesses.length === 0 ? (
          t("noGenre")
        ) : (
          <ul className={styles.genreGuesses}>
            {guesses.map((guess) => {
              const name = t(`label.${guess.key as LabelKey}`);
              const usable = libraryTrackId !== null && row.genre?.trim() !== name.trim();
              return (
                <li key={guess.key} className={styles.genreGuess}>
                  <span>{t("guess", { label: name, percent: guess.percent })}</span>
                  {usable && (
                    <button
                      type="button"
                      className={styles.use}
                      aria-label={t("useGenre", { genre: name })}
                      disabled={setGenre.isPending}
                      onClick={() => setGenre.mutate({ libraryTrackId, name })}
                    >
                      {t("use")}
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        )}
        {setGenre.isError && (
          <p role="alert" className={styles.error}>
            {errorMessage(setGenre.error)}
          </p>
        )}
        <p role="status" className={styles.quiet}>
          {setGenre.isSuccess && setGenre.data !== null ? t("used", { genre: setGenre.variables.name }) : ""}
        </p>
      </dd>
    </div>
  );
}

/**
 * "Your genres": up to 3 of your own genres the model hears in the track,
 * learned from the genres you gave your other tracks. The percent is how sure
 * the model is, not a chance. Each has "Use" under the same rules as the
 * generic guesses (Library tracks only, none for the genre the track has).
 * After a Use the guesses are asked for again.
 */
function YourGenres({ row }: { row: TableRow }) {
  const { t } = useTranslation("audioModel");
  const headingId = useId();
  const queryClient = useQueryClient();
  const genres = useYourGenres(row.recordingId);
  const setGenre = useSetGenre();
  const libraryTrackId = row.libraryTrackId;
  const use = (name: string) => {
    if (libraryTrackId === null) return;
    setGenre.mutate({ libraryTrackId, name }, { onSuccess: () => void reloadLists(queryClient, "audioModel") });
  };
  return (
    <div>
      <h4 id={headingId} className={styles.subheading}>
        {t("yourGenres.title")}
      </h4>
      <p className={styles.quiet}>{t("yourGenres.opinion")}</p>
      {genres.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(genres.error)}
        </p>
      )}
      {genres.isPending && <p className={styles.quiet}>{t("loading")}</p>}
      {genres.data !== undefined && genres.data.length === 0 && <p className={styles.quiet}>{t("yourGenres.none")}</p>}
      {genres.data !== undefined && genres.data.length > 0 && (
        <ul aria-labelledby={headingId} className={styles.genreGuesses}>
          {genres.data.map((genre: YourGenre) => {
            const usable = libraryTrackId !== null && row.genre?.trim() !== genre.name.trim();
            return (
              <li key={genre.node} className={styles.genreGuess}>
                <span>{t("guess", { label: genre.name, percent: genre.percent })}</span>
                {usable && (
                  <button
                    type="button"
                    className={styles.use}
                    aria-label={t("useGenre", { genre: genre.name })}
                    disabled={setGenre.isPending}
                    onClick={() => use(genre.name)}
                  >
                    {t("use")}
                  </button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      {setGenre.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(setGenre.error)}
        </p>
      )}
      <p role="status" className={styles.quiet}>
        {setGenre.isSuccess && setGenre.data !== null ? t("used", { genre: setGenre.variables.name }) : ""}
      </p>
    </div>
  );
}

function SoundsLike({ row }: { row: TableRow }) {
  const { t } = useTranslation("audioModel");
  const headingId = useId();
  const select = useSelectedTrack((state) => state.select);
  const similar = useSoundsLike(row.recordingId, true);
  return (
    <div>
      <h4 id={headingId} className={styles.subheading}>
        {t("similar.title")}
      </h4>
      {similar.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(similar.error)}
        </p>
      )}
      {similar.isPending && <p className={styles.quiet}>{t("loading")}</p>}
      {similar.data !== undefined &&
        (similar.data.state !== "ready" || similar.data.tracks.length === 0 ? (
          <p className={styles.quiet}>{t("similar.empty")}</p>
        ) : (
          <ol aria-labelledby={headingId} className={styles.results}>
            {similar.data.tracks.map(({ row: result, percent }) => (
              <li key={result.recordingId}>
                <button type="button" className={styles.track} onClick={() => select(result)}>
                  <span className={styles.trackName}>
                    {result.artist !== null && result.artist !== "" && result.title !== null && result.title !== ""
                      ? t("similar.track", { artist: result.artist, title: result.title })
                      : (result.title ?? result.artist)}
                  </span>
                  <span className={styles.percent}>{t("similar.percent", { percent })}</span>
                </button>
              </li>
            ))}
          </ol>
        ))}
    </div>
  );
}

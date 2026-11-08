import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { Guess, PanelState, TableRow } from "../bindings";
import { useSelectedTrack } from "../shell/selectedTrack";
import { useAudioGuesses, useAudioModelStatus, useSoundsLike } from "./useAudioModel";
import styles from "./AudioModelPanel.module.css";

/** The keys of the labels the model can guess; a Rust test checks the backend sends only these. */
type LabelKey = keyof typeof import("../locales/en/audioModel.json")["label"];

/**
 * "Audio model" in the Details panel (ROADMAP 3.8): the model's genre and mood
 * guesses for the selected track and the Library tracks that sound like it.
 * Shown only while the switch is on. It is the model's opinion on its own:
 * review only, nothing here is applied to a track, and it feeds no other
 * suggestion.
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
            <GuessLine name={t("genre")} guesses={guesses.data.genres} none={t("noGenre")} />
            <GuessLine name={t("mood")} guesses={guesses.data.mood === null ? [] : [guesses.data.mood]} none={t("noMood")} />
          </dl>
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

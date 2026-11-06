import { Link } from "@tanstack/react-router";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { MapSpelling } from "./MapSpelling";
import { useGenreEdits, useGenreTree, useUnmappedGenres } from "./useGenre";
import styles from "./genre.module.css";

/**
 * Review's "unmapped genre" group (3.2): the Library tracks with no genre
 * come first, then the genre spellings no rule maps to a node, most tracks
 * first. Picking a genre makes the spelling an alias of it (exact text),
 * which empties it from the queue at once. Nothing shows when everything is
 * mapped and every track has a genre.
 */
export function UnmappedGenres() {
  const { t } = useTranslation("genre");
  const titleId = useId();
  const unmapped = useUnmappedGenres();
  const tree = useGenreTree();
  const edits = useGenreEdits();
  const data = unmapped.data;
  if (!data || (data.noGenre === 0 && data.spellings.length === 0)) return null;
  const problem = unmapped.error ?? edits.addAlias.error;
  return (
    <details className={styles.section} aria-labelledby={titleId}>
      <summary>
        <h2
          id={titleId}
          className={styles.sectionTitle}
          style={{ display: "inline" }}
        >
          {t("unmapped.title")}
        </h2>{" "}
        <span className={styles.muted}>
          {t("unmapped.summary", {
            noGenre: data.noGenre,
            spellings: data.spellings.length,
          })}
        </span>
      </summary>
      {problem && (
        <p role="alert" className={styles.error}>
          {errorMessage(problem)}
        </p>
      )}
      <p>
        <Link to="/genres">{t("unmapped.openGenres")}</Link>
      </p>
      {data.noGenre > 0 && (
        <>
          <h3 className={styles.sectionTitle}>
            {t("unmapped.noGenre", { n: data.noGenre })}
          </h3>
          <ul className={styles.list}>
            {data.noGenreSample.map((track) => (
              <li key={track.recordingId} className={styles.line}>
                <span className={styles.spelling}>{track.title}</span>
                <span className={styles.muted}>{track.artist}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {data.spellings.length > 0 && (
        <>
          <h3 className={styles.sectionTitle}>{t("unmapped.spellings")}</h3>
          <ul className={styles.list}>
            {data.spellings.map((s) => (
              <MapSpelling
                key={s.spelling}
                spelling={s}
                nodes={tree.data?.nodes ?? []}
                busy={edits.addAlias.isPending}
                onMap={(genreId) =>
                  edits.addAlias.mutate({
                    genreId,
                    alias: s.spelling,
                    kind: "exact",
                  })
                }
              />
            ))}
          </ul>
        </>
      )}
    </details>
  );
}

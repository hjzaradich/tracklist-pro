import { useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import stage from "../shell/StageScreen.module.css";
import { MapSpelling } from "./MapSpelling";
import { NodeDetails } from "./NodeDetails";
import { rows } from "./tree";
import { useGenreEdits, useGenreTree, useSpellings } from "./useGenre";
import styles from "./genre.module.css";

/**
 * Genres, at `/genres` (3.2): a screen, not a stage, reached from Review's
 * unmapped-genre group. The tree on the left, the chosen genre's details
 * (rename, move, add below, aliases) beside it, and every genre spelling the
 * Library uses with where it goes and which rule sent it there. A track
 * keeps its own spelling; the send writes rekordbox's spelling as it is.
 */
export function GenreScreen() {
  const { t } = useTranslation("genre");
  const treeId = useId();
  const spellingsId = useId();
  const tree = useGenreTree();
  const spellings = useSpellings();
  const edits = useGenreEdits();
  const [chosen, setChosen] = useState<number | null>(null);
  const [branch, setBranch] = useState("");
  const nodes = tree.data?.nodes ?? [];
  const selected = nodes.find((n) => n.id === chosen) ?? null;
  const problem =
    tree.error ?? spellings.error ?? edits.loadSeed.error ?? edits.add.error;

  return (
    <div className={stage.screen}>
      <div className={stage.header}>
        <h1 className={stage.title}>{t("title")}</h1>
      </div>
      <div className={stage.body}>
        <div className={styles.column}>
          {problem && (
            <p role="alert" className={styles.error}>
              {errorMessage(problem)}
            </p>
          )}
          <section className={styles.panel} aria-labelledby={treeId}>
            <h2 id={treeId} className={styles.panelTitle}>
              {t("tree.title")}
            </h2>
            <form
              className={styles.form}
              onSubmit={(e) => {
                e.preventDefault();
                edits.add.mutate(
                  { name: branch, parent: null },
                  { onSuccess: () => setBranch("") },
                );
              }}
            >
              <input
                aria-label={t("tree.branchLabel")}
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
              />
              <button type="submit" disabled={branch.trim() === ""}>
                {t("tree.addBranch")}
              </button>
              {tree.data !== undefined && nodes.length === 0 && (
                <button
                  type="button"
                  disabled={edits.loadSeed.isPending}
                  onClick={() => edits.loadSeed.mutate()}
                >
                  {t("tree.loadSeed")}
                </button>
              )}
            </form>
            {tree.data !== undefined && nodes.length === 0 && (
              <p className={styles.muted}>{t("tree.empty")}</p>
            )}
            <div className={styles.split}>
              <ul className={styles.tree}>
                {rows(nodes).map(({ node, depth }) => (
                  <li
                    key={node.id}
                    style={{ paddingInlineStart: `${depth * 1.25}rem` }}
                  >
                    <button
                      type="button"
                      className={styles.node}
                      aria-current={node.id === selected?.id}
                      onClick={() => setChosen(node.id)}
                    >
                      <span>{node.name}</span>
                      <span className={styles.muted}>
                        {t("spelling.tracks", { n: node.tracks })}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
              {selected !== null && (
                <NodeDetails
                  key={selected.id}
                  node={selected}
                  nodes={nodes}
                  aliases={(tree.data?.aliases ?? []).filter(
                    (a) => a.genreId === selected.id,
                  )}
                />
              )}
            </div>
          </section>
          <section className={styles.panel} aria-labelledby={spellingsId}>
            <h2 id={spellingsId} className={styles.panelTitle}>
              {t("spellings.title")}
            </h2>
            {spellings.data?.length === 0 && (
              <p className={styles.muted}>{t("spellings.none")}</p>
            )}
            <ul className={styles.list}>
              {spellings.data?.map((s) => (
                <MapSpelling
                  key={s.spelling}
                  spelling={s}
                  nodes={nodes}
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
          </section>
        </div>
      </div>
    </div>
  );
}

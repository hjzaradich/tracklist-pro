import { useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { AliasKind, GenreAlias, GenreNode } from "../bindings";
import { NodePicker } from "./NodePicker";
import { withDescendants } from "./tree";
import { useGenreEdits } from "./useGenre";
import styles from "./genre.module.css";

/**
 * The chosen genre: rename it, move it, add a genre below it, and its
 * aliases. Every change is one undoable operation (Undo in the top bar).
 */
export function NodeDetails({
  node,
  nodes,
  aliases,
}: {
  node: GenreNode;
  nodes: readonly GenreNode[];
  aliases: readonly GenreAlias[];
}) {
  const { t } = useTranslation("genre");
  const edits = useGenreEdits();
  const [name, setName] = useState(node.name);
  const [child, setChild] = useState("");
  const [alias, setAlias] = useState("");
  const [kind, setKind] = useState<AliasKind>("folded");
  const failure =
    edits.rename.error ??
    edits.move.error ??
    edits.add.error ??
    edits.addAlias.error ??
    edits.removeAlias.error;

  const submit = (event: FormEvent, run: () => void) => {
    event.preventDefault();
    run();
  };

  return (
    <div className={styles.details}>
      <h3 className={styles.sectionTitle}>{node.name}</h3>
      {failure && (
        <p role="alert" className={styles.error}>
          {errorMessage(failure)}
        </p>
      )}
      <form
        className={styles.form}
        onSubmit={(e) =>
          submit(e, () => edits.rename.mutate({ id: node.id, name }))
        }
      >
        <input
          aria-label={t("details.renameLabel")}
          value={name}
          onChange={(e) => setName(e.target.value)}
        />
        <button
          type="submit"
          disabled={name.trim() === "" || name.trim() === node.name}
        >
          {t("details.rename")}
        </button>
      </form>
      <div className={styles.form}>
        <NodePicker
          nodes={nodes}
          value={node.parentId}
          none={t("details.top")}
          label={t("details.moveLabel")}
          skip={withDescendants(nodes, node.id)}
          onChange={(parent) => edits.move.mutate({ id: node.id, parent })}
        />
      </div>
      <form
        className={styles.form}
        onSubmit={(e) =>
          submit(e, () =>
            edits.add.mutate(
              { name: child, parent: node.id },
              { onSuccess: () => setChild("") },
            ),
          )
        }
      >
        <input
          aria-label={t("details.childLabel")}
          value={child}
          onChange={(e) => setChild(e.target.value)}
        />
        <button type="submit" disabled={child.trim() === ""}>
          {t("details.addChild")}
        </button>
      </form>
      <h4 className={styles.sectionTitle}>{t("aliases.title")}</h4>
      {aliases.length === 0 ? (
        <p className={styles.muted}>{t("aliases.none")}</p>
      ) : (
        <ul className={styles.list}>
          {aliases.map((a) => (
            <li key={a.id} className={styles.line}>
              <span className={styles.spelling}>{a.alias}</span>
              <span className={styles.muted}>
                {t(`aliases.kind.${a.kind}`)}
              </span>
              <button
                type="button"
                aria-label={t("aliases.removeLabel", { alias: a.alias })}
                onClick={() => edits.removeAlias.mutate(a.id)}
              >
                {t("aliases.remove")}
              </button>
            </li>
          ))}
        </ul>
      )}
      <form
        className={styles.form}
        onSubmit={(e) =>
          submit(e, () =>
            edits.addAlias.mutate(
              { genreId: node.id, alias, kind },
              { onSuccess: () => setAlias("") },
            ),
          )
        }
      >
        <input
          aria-label={t("aliases.addLabel")}
          value={alias}
          onChange={(e) => setAlias(e.target.value)}
        />
        <select
          aria-label={t("aliases.kindLabel")}
          value={kind}
          onChange={(e) => setKind(e.target.value as AliasKind)}
        >
          <option value="folded">{t("aliases.kind.folded")}</option>
          <option value="exact">{t("aliases.kind.exact")}</option>
        </select>
        <button type="submit" disabled={alias.trim() === ""}>
          {t("aliases.add")}
        </button>
      </form>
    </div>
  );
}

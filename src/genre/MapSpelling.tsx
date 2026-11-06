import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { GenreNode, Spelling } from "../bindings";
import { NodePicker } from "./NodePicker";
import styles from "./genre.module.css";

/**
 * One genre spelling as the Library has it. A mapped one says where it goes
 * and which rule did it; an unmapped one offers to make it an alias of a
 * genre the user picks (case and accents are ignored).
 */
export function MapSpelling({
  spelling,
  nodes,
  onMap,
  busy = false,
}: {
  spelling: Spelling;
  nodes: readonly GenreNode[];
  onMap: (node: number) => void;
  busy?: boolean;
}) {
  const { t } = useTranslation("genre");
  const [chosen, setChosen] = useState<number | null>(null);
  const target = nodes.find((n) => n.id === spelling.resolution?.node);
  return (
    <li className={styles.line}>
      <span className={styles.spelling}>{spelling.spelling}</span>
      <span className={styles.muted}>
        {t("spelling.tracks", { n: spelling.tracks })}
      </span>
      {spelling.resolution !== null && target !== undefined ? (
        <span>
          {t("spelling.goesTo", {
            name: target.name,
            rule: t(`rule.${spelling.resolution.rule}`),
          })}
        </span>
      ) : (
        <>
          <NodePicker
            nodes={nodes}
            value={chosen}
            onChange={setChosen}
            label={t("spelling.pickLabel", { spelling: spelling.spelling })}
          />
          <button
            type="button"
            disabled={chosen === null || busy}
            onClick={() => chosen !== null && onMap(chosen)}
          >
            {t("spelling.map")}
          </button>
        </>
      )}
    </li>
  );
}

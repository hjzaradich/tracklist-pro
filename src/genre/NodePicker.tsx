import { useTranslation } from "react-i18next";
import type { GenreNode } from "../bindings";
import { rows } from "./tree";

/** A choice among the genres, shown as the tree (indented). `null` is no choice (or the top). */
export function NodePicker({
  nodes,
  value,
  onChange,
  label,
  none,
  skip,
}: {
  nodes: readonly GenreNode[];
  value: number | null;
  onChange: (id: number | null) => void;
  label: string;
  /** The text of the "no genre" choice; without it the picker has none. */
  none?: string;
  /** Genres not offered (a node can't go below itself). */
  skip?: ReadonlySet<number>;
}) {
  const { t } = useTranslation("genre");
  return (
    <select
      aria-label={label}
      value={value ?? ""}
      onChange={(e) =>
        onChange(e.target.value === "" ? null : Number(e.target.value))
      }
    >
      {none !== undefined && <option value="">{none}</option>}
      {none === undefined && value === null && (
        <option value="">{t("picker.choose")}</option>
      )}
      {rows(nodes)
        .filter((row) => !skip?.has(row.node.id))
        .map((row) => (
          <option key={row.node.id} value={row.node.id}>
            {" ".repeat(row.depth)}
            {row.node.name}
          </option>
        ))}
    </select>
  );
}

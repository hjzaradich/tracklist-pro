import { useTranslation } from "react-i18next";
import type { KeyAgreement, KeyComparison } from "../bindings";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";
import { useLocalKeyComparison, useLocalKeyFeature, useSetLocalKeyFeature } from "./useLocalKey";

/**
 * "Local key estimates (test)": the owner's switch for the app's own key
 * (3C-E2, ROADMAP 3.7). Off until turned on; while on, tracks with no key from
 * rekordbox or a file tag show the estimate, and a table counts how it agrees
 * with rekordbox on the tracks that have both. Numbers only, no track names.
 */
export function LocalKeyRow() {
  const { t } = useTranslation("estimate");
  const stored = useLocalKeyFeature();
  const save = useSetLocalKeyFeature();
  const known = stored.data !== undefined;
  const on = stored.data === true;
  const checked = save.isPending ? save.variables === true : on;
  const comparison = useLocalKeyComparison(on && !save.isPending);
  return (
    <SwitchRow
      label={t("localKey.label")}
      help={known ? t(on ? "localKey.helpOn" : "localKey.helpOff") : undefined}
      checked={checked}
      pending={!known || save.isPending}
      error={save.isError ? t("localKey.error") : null}
      onChange={(next) => save.mutate(next)}
    >
      {on && (
        <div className={styles.reader}>
          {comparison.isError && (
            <p role="alert" className={styles.status}>
              {t("localKey.compare.loadError")}
            </p>
          )}
          {comparison.data != null && <AgreementTable comparison={comparison.data} />}
        </div>
      )}
    </SwitchRow>
  );
}

const COLUMNS = ["exact", "relative", "oneStep", "parallel", "other"] as const;

function AgreementTable({ comparison }: { comparison: KeyComparison }) {
  const { t } = useTranslation("estimate");
  if (comparison.overall.tracks === 0) {
    return <p className={styles.status}>{t("localKey.compare.empty")}</p>;
  }
  const rows: { name: string; counts: KeyAgreement }[] = [
    { name: t("localKey.compare.all"), counts: comparison.overall },
    ...comparison.genres.map((g) => ({ name: g.genre ?? t("localKey.compare.noGenre"), counts: g.counts })),
  ];
  return (
    <table className={styles.agreement}>
      <caption>{t("localKey.compare.title")}</caption>
      <thead>
        <tr>
          <th scope="col">{t("localKey.compare.genre")}</th>
          <th scope="col">{t("localKey.compare.tracks")}</th>
          {COLUMNS.map((column) => (
            <th key={column} scope="col">
              {t(`localKey.compare.${column}`)}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {rows.map((row, index) => (
          // Genre names are unique; the totals row comes first.
          <tr key={index === 0 ? "" : `genre:${row.name}`}>
            <th scope="row">{row.name}</th>
            <td>{row.counts.tracks}</td>
            {COLUMNS.map((column) => (
              <td key={column}>
                {t("localKey.compare.cell", {
                  value: row.counts[column],
                  percent: Math.round((100 * row.counts[column]) / row.counts.tracks),
                })}
              </td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

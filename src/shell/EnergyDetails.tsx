import { useId, useRef } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { TableRow } from "../bindings";
import { EnergyButtons } from "../trackTable/EnergyButtons";
import { useClearEnergy, useUserEnergyCount } from "../trackTable/useClearEnergy";
import { useSetEnergy } from "../trackTable/useSetEnergy";
import { useSelectedTrack } from "./selectedTrack";
import styles from "./TrackDetails.module.css";

/**
 * The Details panel's Energy section (1cD): the ten buttons for the selected
 * tracks, the track's energy when it is not the user's own, and the keys that
 * do the same.
 *
 * - One track with the user's own energy: its button is pressed.
 * - One track with another source's energy (Mixed In Key, the estimate, the
 *   audio model): no button is pressed and a line says what it is and whose;
 *   a pick is the user's word, which then wins.
 * - No energy: no button is pressed and the line says so.
 * - Several tracks: the line says how many, no button is pressed (rows off
 *   screen are not loaded, so there is no summary of theirs), and a pick is
 *   for all of them.
 *
 * "Clear" takes the user's own energy away (so the next source shows again);
 * it is offered only when the track has one, or, with several selected, when
 * any of them has (asked of the backend: rows off screen are not loaded).
 *
 * The row is read again after a pick, as the lists are.
 */
export function EnergyDetails({ row }: { row: TableRow }) {
  const { t } = useTranslation("shell");
  const headingId = useId();
  const section = useRef<HTMLElement>(null);
  const selected = useSelectedTrack((state) => state.ids);
  const give = useSetEnergy();
  const clear = useClearEnergy();
  const many = selected.length > 1;
  const ownAmongMany = useUserEnergyCount(many ? selected : []);
  const tracks = many ? selected : [row.recordingId];
  // A pick only ever changes what is selected: when the focused row has been
  // taken out of the selection (Ctrl+click) or nothing is selected, there is
  // no single track the keys would change, so there is nothing to offer.
  if (!many && !selected.includes(row.recordingId)) return null;
  const { energy, energySource } = row;
  const canClear = many ? ownAmongMany > 0 : energySource === "user";

  let line: string;
  if (many) line = t("details.energy.many", { count: selected.length });
  else if (energy === null) line = t("details.energy.none");
  else if (energySource === null || energySource === "user") line = "";
  else line = t("details.energy.from", { energy, source: t(`details.energy.source.${energySource}`) });

  return (
    <section ref={section} aria-labelledby={headingId} className={styles.energy}>
      <h3 id={headingId} className={styles.heading}>
        {t("details.energy.title")}
      </h3>
      {line !== "" && <p className={styles.quiet}>{line}</p>}
      <EnergyButtons
        current={!many && energySource === "user" ? energy : null}
        onPick={(picked) => give.mutate({ recordingIds: tracks, energy: picked })}
      />
      <p className={styles.quiet}>{t("details.energy.keys")}</p>
      {canClear && (
        <button type="button" className={styles.withdraw} onClick={() =>
            clear.mutate(
              { recordingIds: tracks },
              // The button goes when the user's energy does: keep the keyboard's place.
              { onSuccess: () => section.current?.querySelector("button")?.focus() },
            )
          }>
          {t("details.energy.clear")}
        </button>
      )}
      {(give.isError || clear.isError) && (
        <p role="alert" className={styles.error}>
          {errorMessage(give.isError ? give.error : clear.error)}
        </p>
      )}
    </section>
  );
}

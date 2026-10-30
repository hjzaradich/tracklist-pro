import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { WouldChange } from "../bindings";
import { SwitchRow } from "./SwitchRow";
import styles from "./settings.module.css";
import {
  useEnergyAsStars,
  useEnergyStarsWouldChange,
  useSetEnergyAsStars,
} from "./useEnergyAsStars";

/**
 * "Send energy as the star rating" (1cC-2, ROADMAP 1.9). Turning it on first
 * counts, then asks, inline under the row; nothing changes until "Turn on".
 * Turning it off never asks. A failure puts the row back as it was.
 */
export function EnergyStarsRow() {
  const { t } = useTranslation("settings");
  const stored = useEnergyAsStars();
  // Until the setting has been read, the box is not "off": it is disabled and
  // shows neither state, so a click can never turn on what is already on.
  const known = stored.status === "ready";
  const on = stored.status === "ready" && stored.on;
  const save = useSetEnergyAsStars();
  const count = useEnergyStarsWouldChange();
  const [asking, setAsking] = useState<WouldChange | null>(null);
  // Said under the switch right after it is turned off, until the user leaves.
  const [justOff, setJustOff] = useState(false);
  const [failed, setFailed] = useState<"save" | "count" | null>(null);
  const input = useRef<HTMLInputElement>(null);

  const counting = count.isPending;
  const pending = !known || save.isPending || counting;
  // While a change is saved the box shows where it is going; a failure puts it back.
  const checked = save.isPending ? save.variables === true : on;

  // A box that doesn't know its state shows neither: a dash, not an empty box.
  useEffect(() => {
    if (input.current) input.current.indeterminate = !known;
  });

  const turnOn = () => {
    if (!known) return;
    setFailed(null);
    setJustOff(false);
    count.mutate(undefined, {
      onSuccess: (answer) => setAsking(answer),
      onError: () => setFailed("count"),
    });
  };

  const turnOff = () => {
    if (!known) return;
    setFailed(null);
    save.mutate(false, {
      onSuccess: () => setJustOff(true),
      onError: () => setFailed("save"),
    });
  };

  // The panel held the focus; when it closes the switch gets it back, once
  // the switch is usable again (a box that is still saving can't take it).
  const refocus = useRef(false);
  useEffect(() => {
    if (refocus.current && !pending) {
      refocus.current = false;
      input.current?.focus();
    }
  });
  const close = () => {
    refocus.current = true;
    setAsking(null);
  };

  const confirm = () => {
    setFailed(null);
    save.mutate(true, {
      onSuccess: close,
      onError: () => {
        setFailed("save");
        close();
      },
    });
  };

  return (
    <SwitchRow
      label={t("energyStars.label")}
      help={known ? t(checked ? "energyStars.helpOn" : "energyStars.helpOff") : undefined}
      notes={justOff && !checked ? [t("energyStars.offNote")] : []}
      checked={checked}
      pending={pending}
      error={
        stored.status === "error"
          ? errorMessage(stored.error)
          : failed === "count"
            ? t("energyStars.countError")
            : failed === "save"
              ? t("energyStars.error")
              : null
      }
      onChange={(next) => (next ? turnOn() : turnOff())}
      inputRef={input}
    >
      {counting && (
        <p role="status" className={styles.help}>
          {t("energyStars.counting")}
        </p>
      )}
      {/* The count is a snapshot from when this opened; it names the read it
          comes from, and the send review gives the exact figure. */}
      {asking !== null && (
        <ConfirmTurnOn
          answer={asking}
          saving={save.isPending}
          onConfirm={confirm}
          onCancel={close}
        />
      )}
    </SwitchRow>
  );
}

/**
 * Asks before the setting turns on: how many tracks' stars in rekordbox would
 * change, from the newest read (its date shown), or that nothing has been read
 * yet. An in-flow alertdialog like the Library's remove: the focus starts on
 * the safe choice, Esc closes it, and "Turn on" is the one filled button.
 */
function ConfirmTurnOn({
  answer,
  saving,
  onConfirm,
  onCancel,
}: {
  answer: WouldChange;
  saving: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const { t, i18n } = useTranslation("settings");
  const back = useRef<HTMLButtonElement>(null);
  useEffect(() => back.current?.focus(), []);
  const titleId = useId();
  const countId = useId();
  const detailId = useId();
  const when = (readAt: string) =>
    new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
      new Date(readAt),
    );
  return (
    <div
      role="alertdialog"
      aria-labelledby={titleId}
      aria-describedby={`${countId} ${detailId}`}
      className={styles.confirm}
      onKeyDown={(event) => {
        // Not while the write runs: the panel stays until the result lands.
        if (event.key === "Escape" && !saving) onCancel();
      }}
    >
      <p id={titleId} className={styles.confirmTitle}>
        {t("energyStars.confirmTitle")}
      </p>
      <p id={countId} className={styles.confirmDetail}>
        {answer.result === "counted"
          ? t("energyStars.changes", { count: answer.tracks, when: when(answer.readAt) })
          : t("energyStars.noRead")}
      </p>
      <p id={detailId} className={styles.confirmDetail}>
        {t("energyStars.confirmDetail")}
      </p>
      <div className={styles.actions}>
        <button type="button" className={styles.primary} disabled={saving} onClick={onConfirm}>
          {t("energyStars.confirm")}
        </button>
        <button ref={back} type="button" className={styles.button} onClick={onCancel}>
          {t("energyStars.back")}
        </button>
      </div>
    </div>
  );
}

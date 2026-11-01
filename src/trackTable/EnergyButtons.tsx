import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import styles from "./EnergyButtons.module.css";

/** The energies there are, 1 to 10 (ROADMAP 1.1, "Energy"). */
export const ENERGIES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10] as const;

/** How many buttons are in a row of the grid (5 by 2). */
const PER_ROW = 5;

/**
 * The ten energy buttons (1cD): a group of 1 to 10 in a grid of two rows, shared
 * by the Details panel and the Energy column's popover. `current` is the
 * energy to show as pressed (the user's own; none when it isn't theirs or
 * when several tracks are selected), and `onPick` is told the one chosen.
 *
 * The group is one tab stop (the pressed button, else `start`, else 1); the
 * arrow keys move between the buttons (Left and Right one, Up and Down a
 * row), Home and End go to 1 and 10, and Enter or Space pick, as any button.
 * `autoFocus` puts the focus on that button when the group appears.
 */
export function EnergyButtons({
  current,
  start,
  onPick,
  autoFocus = false,
}: {
  current: number | null;
  /** Where the focus starts when no button is pressed. */
  start?: number | null;
  onPick: (energy: number) => void;
  autoFocus?: boolean;
}) {
  const { t } = useTranslation("shell");
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const [at, setAt] = useState<number>(current ?? start ?? 1);

  useEffect(() => {
    if (autoFocus) buttons.current[at - 1]?.focus();
    // Only when the group appears.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const go = (energy: number) => {
    if (energy < 1 || energy > ENERGIES.length) return;
    setAt(energy);
    buttons.current[energy - 1]?.focus();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    const steps: Record<string, number> = {
      ArrowRight: at + 1,
      ArrowLeft: at - 1,
      ArrowDown: at + PER_ROW,
      ArrowUp: at - PER_ROW,
      Home: 1,
      End: ENERGIES.length,
    };
    const target = steps[event.key];
    if (target === undefined) return;
    event.preventDefault();
    go(target);
  };

  return (
    <div role="group" aria-label={t("details.energy.title")} className={styles.group} onKeyDown={onKeyDown}>
      {ENERGIES.map((energy) => (
        <button
          key={energy}
          ref={(element) => {
            buttons.current[energy - 1] = element;
          }}
          type="button"
          className={styles.button}
          aria-pressed={current === energy}
          aria-label={t("details.energy.set", { energy })}
          tabIndex={energy === at ? 0 : -1}
          onFocus={() => setAt(energy)}
          onClick={() => onPick(energy)}
        >
          {energy}
        </button>
      ))}
    </div>
  );
}

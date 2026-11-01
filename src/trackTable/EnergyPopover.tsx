import { useLayoutEffect, useRef, type FocusEvent, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { EnergyButtons } from "./EnergyButtons";
import styles from "./EnergyPopover.module.css";

/** The gap between the cell and the popover, and the room kept from the window's edge (pixels). */
const GAP = 2;
const MARGIN = 4;

/** The digit keys and the energy each gives: 1 to 9, and 0 for 10. */
function energyOfKey(key: string): number | null {
  if (key === "0") return 10;
  return /^[1-9]$/.test(key) ? Number(key) : null;
}

/**
 * The energy buttons in a small window under a cell of the Energy column
 * (1cD), above it when there is no room. It is a dialog, so the table's keys
 * and the command palette stand aside while it is open (`dialogIsOpen`), and
 * it handles the digits itself: 1 to 9 and 0 pick. Esc, a click or Tab
 * elsewhere closes it without picking and the focus goes back to the cell
 * (`anchor`). A pick closes it too.
 *
 * `count` is how many tracks the pick applies to; with more than one the
 * window says so. `current` is the energy to show as pressed and `start` where
 * the focus begins when none is. It is drawn in a portal, so a click inside
 * it is not a click on a row.
 */
export function EnergyPopover({
  anchor,
  count,
  current,
  start,
  onPick,
  onClose,
}: {
  anchor: HTMLElement;
  count: number;
  current: number | null;
  start: number | null;
  onPick: (energy: number) => void;
  /** `restoreFocus` is whether the focus goes back to the cell. */
  onClose: (restoreFocus: boolean) => void;
}) {
  const { t } = useTranslation("trackTable");
  const { t: tShell } = useTranslation("shell");
  const box = useRef<HTMLDivElement>(null);
  const from = anchor.getBoundingClientRect();

  // Under the cell, or above it when it would not fit; kept inside the window sideways.
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const own = element.getBoundingClientRect();
    const below = from.bottom + GAP;
    const fitsBelow = below + own.height <= window.innerHeight - MARGIN;
    const top = fitsBelow ? below : Math.max(MARGIN, from.top - GAP - own.height);
    const left = Math.max(MARGIN, Math.min(from.right - own.width, window.innerWidth - own.width - MARGIN));
    element.style.top = `${top}px`;
    element.style.left = `${left}px`;
  });

  const pick = (energy: number) => {
    onPick(energy);
    onClose(true);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      // The table's own Escape (select none) must not hear it as well.
      event.preventDefault();
      onClose(true);
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      onClose(true);
      return;
    }
    if (event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) return;
    const energy = energyOfKey(event.key);
    if (energy === null) return;
    event.preventDefault();
    pick(energy);
  };
  // The focus left the window: for a place inside it, or the cell (a click on
  // the cell closes it itself), nothing; anywhere else the window closes.
  const onBlur = (event: FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget;
    if (next instanceof Node && (event.currentTarget.contains(next) || anchor.contains(next))) return;
    // Focus goes back to the cell only when it went nowhere (Esc and Tab return
    // it themselves); a click on another control keeps it there.
    onClose(next === null);
  };

  return createPortal(
    <div
      ref={box}
      role="dialog"
      aria-label={t("energyPopover.label")}
      tabIndex={-1}
      className={styles.popover}
      style={{ top: from.bottom + GAP, left: from.right }}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
    >
      {count > 1 && <p className={styles.count}>{tShell("details.energy.many", { count })}</p>}
      <EnergyButtons current={current} start={start} onPick={pick} autoFocus />
    </div>,
    document.body,
  );
}

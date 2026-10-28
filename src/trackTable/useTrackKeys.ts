import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { dialogIsOpen, isTextField } from "../keys/typing";
import { type PaletteAction, useRegisterActions } from "../palette/registry";
import { usePlayer } from "../player/playerStore";

/** What the keys do, which the track table that mounts the hook provides. */
export interface TrackKeyHandlers {
  /** Moves the focus one row; with `extend`, selects the rows from the anchor to it. */
  move: (step: 1 | -1, extend: boolean) => void;
  /** Selects every track in the list, those not on screen too. */
  selectAll: () => void;
  /** Leaves no track selected. */
  clear: () => void;
  /** How many tracks are selected. */
  selected: number;
  /** Gives the selected tracks an energy from 1 to 10. Without it the energy keys are unavailable. */
  setEnergy?: (energy: number) => void;
}

/** The energy each key gives: 1 to 9, and 0 for 10 (ROADMAP 1.11). */
const ENERGY_KEYS = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"] as const;

interface Binding {
  action: PaletteAction;
  matches: (event: KeyboardEvent) => boolean;
  /** A held key keeps acting (moving down a list); the others act once. */
  repeats: boolean;
}

/** A key pressed with exactly these modifiers. */
const press =
  (key: string, modifiers: { shift?: boolean; ctrl?: boolean } = {}) =>
  (event: KeyboardEvent) =>
    event.key.toLowerCase() === key &&
    event.shiftKey === (modifiers.shift ?? false) &&
    event.ctrlKey === (modifiers.ctrl ?? false) &&
    !event.altKey &&
    !event.metaKey;

/** Whether the key is going to a control that takes Space itself (a button, a link). */
function isControl(target: EventTarget | null): boolean {
  return (
    target instanceof HTMLElement &&
    target.closest(
      "button, a[href], summary, input, select, textarea, [role='button'], [role='checkbox'], [role='switch'], [role='tab'], [role='menuitem']",
    ) !== null
  );
}

/**
 * The track table's hotkeys (1cB-8; ROADMAP 1.11): J and K move the focus
 * down and up (Shift extends the selection), Ctrl+A selects all, Escape
 * clears, 1 to 0 give the selected tracks an energy, Space plays or pauses
 * and T tags.
 *
 * Every key is an action in the palette's registry (`useRegisterActions`),
 * shown there with its shortcut, so the palette runs the very handler the key
 * runs. An action that can't run now is not listed and its key does nothing:
 * Clear and the energy keys need a selected track, Space needs a file in the
 * player, and T waits for tags (Phase 3, ROADMAP 3.1).
 *
 * The keys stand aside while the focus is in a text field, while a dialog is
 * open (the palette is one), and for a key another handler already used. Space
 * is also left to a button or link that has the focus.
 */
export function useTrackKeys({ move, selectAll, clear, selected, setEnergy }: TrackKeyHandlers) {
  const { t } = useTranslation("trackTable");
  const playing = usePlayer((state) => state.file !== null);
  const toggle = usePlayer((state) => state.toggle);

  const bindings: Binding[] = [
    {
      action: { id: "tracks:next", label: t("keys.next"), shortcut: t("keys.shortcut.next"), run: () => move(1, false) },
      matches: press("j"),
      repeats: true,
    },
    {
      action: { id: "tracks:previous", label: t("keys.previous"), shortcut: t("keys.shortcut.previous"), run: () => move(-1, false) },
      matches: press("k"),
      repeats: true,
    },
    {
      action: { id: "tracks:extendNext", label: t("keys.extendNext"), shortcut: t("keys.shortcut.extendNext"), run: () => move(1, true) },
      matches: press("j", { shift: true }),
      repeats: true,
    },
    {
      action: { id: "tracks:extendPrevious", label: t("keys.extendPrevious"), shortcut: t("keys.shortcut.extendPrevious"), run: () => move(-1, true) },
      matches: press("k", { shift: true }),
      repeats: true,
    },
    {
      action: { id: "tracks:selectAll", label: t("keys.selectAll"), shortcut: t("keys.shortcut.selectAll"), run: selectAll },
      matches: press("a", { ctrl: true }),
      repeats: false,
    },
    {
      action: {
        id: "tracks:clearSelection",
        label: t("keys.clearSelection"),
        shortcut: t("keys.shortcut.clearSelection"),
        available: selected > 0,
        run: clear,
      },
      matches: press("escape"),
      repeats: false,
    },
    ...ENERGY_KEYS.map((key, index): Binding => ({
      action: {
        id: `tracks:energy${index + 1}`,
        label: t("keys.energy", { energy: index + 1 }),
        shortcut: key,
        available: setEnergy !== undefined && selected > 0,
        run: () => setEnergy?.(index + 1),
      },
      matches: press(key),
      repeats: false,
    })),
    {
      action: {
        id: "tracks:playPause",
        label: t("keys.playPause"),
        shortcut: t("keys.shortcut.playPause"),
        available: playing,
        run: toggle,
      },
      matches: press(" "),
      repeats: false,
    },
    {
      // Tags arrive with the custom vocabulary (ROADMAP 3.1): nothing to tag with yet.
      action: { id: "tracks:tag", label: t("keys.tag"), shortcut: t("keys.shortcut.tag"), available: false, run: () => {} },
      matches: press("t"),
      repeats: false,
    },
  ];

  useRegisterActions(
    "tracks",
    bindings.map((binding) => binding.action),
  );

  const latest = useRef(bindings);
  useEffect(() => {
    latest.current = bindings;
  });
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;
      if (isTextField(event.target) || event.target instanceof HTMLSelectElement || dialogIsOpen()) return;
      const hit = latest.current.find((binding) => binding.matches(event));
      if (hit === undefined || hit.action.available === false) return;
      if (event.key === " " && isControl(event.target)) return;
      event.preventDefault();
      if (event.repeat && !hit.repeats) return;
      hit.action.run();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}

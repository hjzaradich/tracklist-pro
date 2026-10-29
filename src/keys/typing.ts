// When a shortcut stands aside: the keyboard belongs to the text being typed,
// or to a dialog waiting for an answer. Ctrl+Z (`useUndoShortcut`) and the
// track hotkeys (`useTrackKeys`, 1cB-8) both ask here.

/** Whether typing happens in `target`: a text box or editable text. */
export function isTextField(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLInputElement) {
    return ![
      "button",
      "checkbox",
      "color",
      "file",
      "image",
      "radio",
      "range",
      "reset",
      "submit",
    ].includes(target.type);
  }
  return target.closest('[contenteditable]:not([contenteditable="false"])') !== null;
}

/** A dialog that is waiting for an answer: it has the keyboard. The command palette is one. */
export const OPEN_DIALOG = '[role="alertdialog"], [role="dialog"], dialog[open]';

/** Whether a dialog (the command palette included) is open now. */
export function dialogIsOpen(): boolean {
  return document.querySelector(OPEN_DIALOG) !== null;
}

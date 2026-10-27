import { screen, waitFor, within } from "@testing-library/react";
import { expect } from "vitest";
import { UndoButton } from "../shell/UndoButton";
import { tx } from "./tx";

// Undo lives in the top bar only (owner decision, 2026-10-27): no screen
// offers an Undo of its own after an action. A screen's test renders the top
// bar's Undo beside it (`<TopBarUndo />`) and checks that this is the only
// one.

const TOP_BAR = "top-bar-undo";

/** The top bar's Undo, marked so a test can tell it from anything else on the screen. */
export function TopBarUndo() {
  return (
    <div data-testid={TOP_BAR}>
      <UndoButton />
    </div>
  );
}

/** The name a button is found by: its label, or its text. */
const nameOf = (button: HTMLElement) => button.getAttribute("aria-label") ?? button.textContent ?? "";

/**
 * Waits for the top bar's Undo to offer a step, then checks that no other
 * button on screen is an Undo: the only Undo is the top bar's, and it is the
 * one that undoes the action. Returns it.
 */
export async function expectOnlyTheTopBarUndo(): Promise<HTMLElement> {
  const bar = screen.getByTestId(TOP_BAR);
  const topBarUndo = within(bar).getByRole("button");
  await waitFor(() => expect(topBarUndo).toBeEnabled());
  const undoWord = tx("shell:undo.button");
  const others = screen
    .getAllByRole("button")
    .filter((button) => !bar.contains(button) && nameOf(button).includes(undoWord));
  expect(others.map(nameOf)).toEqual([]);
  return topBarUndo;
}

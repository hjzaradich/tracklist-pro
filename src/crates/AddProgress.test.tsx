import { clearMocks } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import { crate, crateBackend, libraryRows, outsideRow } from "../test/crateBackend";
import { tx } from "../test/tx";
import { PROGRESS_EVERY_MS, SHOW_PROGRESS_AFTER_MS } from "./useAddProgress";

// Progress of an add to a crate (1cE-P2): shown only while the add is pending
// and has taken a while, in the dialog or, for a drop on the sidebar, in the
// line over the track table. Time is faked, so nothing here waits.

const ROW = 28;
const CRATES = [crate(1, "Warmup"), crate(2, "Loose")];

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
  // Time passes by itself as well, so what waits for it (waitFor, the app)
  // still moves; a test jumps ahead with `later`.
  vi.useFakeTimers({ shouldAdvanceTime: true });
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
  clearMocks();
  document.body.innerHTML = "";
});

const later = (ms: number) => act(() => vi.advanceTimersByTimeAsync(ms));

const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const rowAt = (index: number) => {
  const found = rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === index);
  if (!found) throw new Error(`row ${index} is not drawn`);
  return found;
};

async function library() {
  const backend = crateBackend({ rows: [...libraryRows(1), outsideRow(1), outsideRow(2)], crates: CRATES });
  renderApp("/library");
  await screen.findByRole("grid");
  await waitFor(() => expect(rows().length).toBeGreaterThan(2));
  return backend;
}

const asked = (backend: Awaited<ReturnType<typeof library>>) => backend.called("crate_add_progress").length;
const checking = (percent: number) => tx("crates:addProgress.checking", { percent });
const adding = (crate: string, percent: number) => tx("crates:addProgress.adding", { crate, percent });
// The report line goes on with notes (also added to the Library), so match its start.
const added = (count: number, name: string) => {
  const start = tx("crates:addSelection.added", { count, crate: name });
  return (content: string) => content.startsWith(start);
};

async function startTheAddInTheDialog(backend: Awaited<ReturnType<typeof library>>) {
  fireEvent.click(rowAt(1));
  fireEvent.click(rowAt(2), { ctrlKey: true });
  await userEvent.keyboard("c");
  await screen.findByRole("dialog", { name: tx("crates:addDialog.title") });
  await userEvent.type(screen.getByRole("combobox", { name: tx("crates:addDialog.search") }), "loose");
  const release = backend.hold("add_recordings_to_crate");
  await userEvent.keyboard("{Enter}");
  await waitFor(() => expect(backend.called("add_recordings_to_crate")).toHaveLength(1));
  return release;
}

describe("progress of an add to a crate", () => {
  it("the dialog shows the phase and percent only once the add has taken a while, and follows it", async () => {
    const backend = await library();
    const release = await startTheAddInTheDialog(backend);

    // Not yet: a quick add would show nothing, and nothing is asked.
    await later(SHOW_PROGRESS_AFTER_MS / 2);
    expect(asked(backend)).toBe(0);
    expect(screen.queryByText(checking(0))).toBeNull();

    backend.state.addProgress = { phase: "checking", done: 50, total: 200 };
    await later(SHOW_PROGRESS_AFTER_MS);
    const dialog = screen.getByRole("dialog");
    expect(await within(dialog).findByText(checking(25))).toBeInTheDocument();

    backend.state.addProgress = { phase: "adding", done: 100, total: 200 };
    await later(PROGRESS_EVERY_MS);
    expect(await within(dialog).findByText(adding("Loose", 50))).toBeInTheDocument();

    // It settles: the dialog closes, the line goes, and nothing is asked again.
    release();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText(added(2, "Loose"))).toBeInTheDocument();
    const before = asked(backend);
    await later(PROGRESS_EVERY_MS * 5);
    expect(asked(backend)).toBe(before);
    expect(screen.queryByText(adding("Loose", 50))).toBeNull();
  });

  it("a quick add shows no progress and never asks for it", async () => {
    const backend = await library();
    const release = await startTheAddInTheDialog(backend);
    await later(SHOW_PROGRESS_AFTER_MS / 4);
    release();

    expect(await screen.findByText(added(2, "Loose"))).toBeInTheDocument();
    await later(SHOW_PROGRESS_AFTER_MS * 3);
    expect(asked(backend)).toBe(0);
    expect(screen.queryByText(checking(0))).toBeNull();
  });

  it("a drop on the sidebar shows the progress in the report line's place, then the report", async () => {
    const backend = await library();
    fireEvent.click(rowAt(1));
    const target = (await screen.findByRole("link", { name: "Warmup" })).parentElement as HTMLElement;
    const release = backend.hold("add_recordings_to_crate");
    fireEvent.pointerDown(rowAt(1), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(document.body, { clientX: 320, clientY: 120, pointerId: 1 });
    fireEvent.pointerEnter(target, { pointerId: 1 });
    fireEvent.pointerUp(target, { pointerId: 1 });
    await waitFor(() => expect(backend.called("add_recordings_to_crate")).toHaveLength(1));

    await later(SHOW_PROGRESS_AFTER_MS / 2);
    expect(asked(backend)).toBe(0);

    backend.state.addProgress = { phase: "adding", done: 30, total: 100 };
    await later(SHOW_PROGRESS_AFTER_MS);
    expect(await screen.findByText(adding("Warmup", 30))).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).toBeNull();

    release();
    expect(await screen.findByText(added(1, "Warmup"))).toBeInTheDocument();
    expect(screen.queryByText(adding("Warmup", 30))).toBeNull();
  });
});

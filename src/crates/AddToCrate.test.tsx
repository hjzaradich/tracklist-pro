import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import { crate, crateBackend, folder, libraryRows, outsideRow } from "../test/crateBackend";
import { tx } from "../test/tx";

// Adding the selected tracks to a crate (1cB-13; ROADMAP 1.14, "Adding"):
// dragged onto a crate in the sidebar, or with the C key's dialog. The whole
// app is rendered, since the drop lands in the sidebar and the line saying
// what happened is over the track table.

const ROW = 28;

/** Friday residency > (Warmup, Late > Peak time); Loose and the folder Sets at the top. */
const CRATES = [
  folder(1, "Friday residency"),
  crate(2, "Warmup", 1),
  folder(3, "Late", 1),
  crate(4, "Peak time", 3),
  crate(5, "Loose"),
  folder(6, "Sets"),
];

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  document.body.innerHTML = "";
});

const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const rowAt = (index: number) => {
  const found = rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === index);
  if (!found) throw new Error(`row ${index} is not drawn`);
  return found;
};
const clickRow = (index: number, modifiers: { ctrlKey?: boolean } = {}) => fireEvent.click(rowAt(index), modifiers);

async function library(options: Parameters<typeof crateBackend>[0]) {
  const backend = crateBackend(options);
  renderApp("/library");
  await screen.findByRole("grid");
  await waitFor(() => expect(rows().length).toBeGreaterThan(2));
  return backend;
}

/** The sidebar's row for a crate or folder. */
const treeRow = async (name: string) => (await screen.findByRole("link", { name })).parentElement as HTMLElement;

/** Holds a row, moves the pointer onto a place in the sidebar and lets go there. */
async function dragRowTo(index: number, name: string, release = true) {
  const target = await treeRow(name);
  fireEvent.pointerDown(rowAt(index), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
  fireEvent.pointerMove(document.body, { clientX: 320, clientY: 120, pointerId: 1 });
  fireEvent.pointerEnter(target, { pointerId: 1 });
  if (release) fireEvent.pointerUp(target, { pointerId: 1 });
  return target;
}

const added = (count: number, name: string) => tx("crates:addSelection.added", { count, crate: name });

describe("dragging tracks onto a crate", () => {
  it("drag adds the selected tracks to the crate in one operation, and one Undo takes them all out", async () => {
    const backend = await library({ rows: libraryRows(6), crates: CRATES });
    clickRow(1);
    clickRow(3, { ctrlKey: true });
    clickRow(4, { ctrlKey: true });
    await dragRowTo(3, "Warmup");

    expect(await screen.findByText(added(3, "Warmup"))).toBeInTheDocument();
    expect(backend.called("add_recordings_to_crate")).toHaveLength(1);
    expect(backend.called("add_recordings_to_crate")[0]?.args).toEqual({ id: 2, recordings: [101, 103, 104] });
    expect(backend.state.history).toHaveLength(1);

    // Undo is in the top bar only, and one Undo takes all three out.
    const undo = await screen.findByRole("button", { name: tx("shell:undo.action.addToCrate", { count: 3, name: "Warmup" }) });
    await userEvent.click(undo);
    await waitFor(() => expect(backend.called("undo_last_operation")).toHaveLength(1));
    expect(backend.state.members.get(2)?.size).toBe(0);
    await waitFor(() => expect(screen.queryByText(added(3, "Warmup"))).toBeNull());
  });

  it("a row that is not selected is the one carried, and it is selected", async () => {
    const backend = await library({ rows: libraryRows(6), crates: CRATES });
    clickRow(1);
    await dragRowTo(4, "Loose");
    expect(await screen.findByText(added(1, "Loose"))).toBeInTheDocument();
    expect(backend.called("add_recordings_to_crate")[0]?.args).toEqual({ id: 5, recordings: [104] });
    expect(rowAt(4)).toHaveAttribute("aria-selected", "true");
    expect(rowAt(1)).toHaveAttribute("aria-selected", "false");
  });

  it("the crate under the pointer is the drop target, and a folder is not", async () => {
    await library({ rows: libraryRows(4), crates: CRATES });
    const warmup = await dragRowTo(0, "Warmup", false);
    expect(warmup).toHaveAttribute("data-drop", "target");
    const sets = await treeRow("Sets");
    fireEvent.pointerLeave(warmup, { pointerId: 1 });
    fireEvent.pointerEnter(sets, { pointerId: 1 });
    expect(warmup).not.toHaveAttribute("data-drop");
    expect(sets).toHaveAttribute("data-drop", "refused");
  });

  it("a drop on a folder is refused and adds nothing", async () => {
    const backend = await library({ rows: libraryRows(4), crates: CRATES });
    await dragRowTo(2, "Friday residency");
    expect(await screen.findByText(tx("crates:addSelection.folder"))).toBeInTheDocument();
    expect(backend.called("add_recordings_to_crate")).toHaveLength(0);
    expect(backend.state.history).toHaveLength(0);
  });

  it("a drop on a stage, or after Escape, adds nothing", async () => {
    const backend = await library({ rows: libraryRows(4), crates: CRATES });
    const stage = screen.getByRole("link", { name: (name) => name.startsWith(tx("shell:stages.overview")) });
    fireEvent.pointerDown(rowAt(0), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(document.body, { clientX: 320, clientY: 120, pointerId: 1 });
    fireEvent.pointerUp(stage, { pointerId: 1 });

    const loose = await treeRow("Loose");
    fireEvent.pointerDown(rowAt(1), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(document.body, { clientX: 320, clientY: 120, pointerId: 1 });
    fireEvent.pointerEnter(loose, { pointerId: 1 });
    fireEvent.keyDown(document.body, { key: "Escape" });
    fireEvent.pointerUp(loose, { pointerId: 1 });
    expect(backend.called("add_recordings_to_crate")).toHaveLength(0);
    expect(loose).not.toHaveAttribute("data-drop");
  });

  it("a click that moves less than a drag selects the row and starts no drag", async () => {
    const backend = await library({ rows: libraryRows(4), crates: CRATES });
    const loose = await treeRow("Loose");
    fireEvent.pointerDown(rowAt(2), { button: 0, clientX: 300, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(document.body, { clientX: 302, clientY: 101, pointerId: 1 });
    fireEvent.pointerEnter(loose, { pointerId: 1 });
    fireEvent.pointerUp(loose, { pointerId: 1 });
    fireEvent.click(rowAt(2));
    expect(rowAt(2)).toHaveAttribute("aria-selected", "true");
    expect(backend.called("add_recordings_to_crate")).toHaveLength(0);
  });
});

describe("Add to crate or playlist", () => {
  const dialog = () => screen.findByRole("dialog", { name: tx("crates:addDialog.title") });
  const box = () => screen.getByRole("combobox", { name: tx("crates:addDialog.search") });
  const optionNames = () =>
    within(screen.getByRole("listbox", { name: tx("crates:list.label") }))
      .queryAllByRole("option")
      .map((option) => option.textContent);

  it("C opens the dialog, search filters the crates, and Enter adds to the highlighted one", async () => {
    const backend = await library({ rows: libraryRows(6), crates: CRATES });
    clickRow(2);
    clickRow(3, { ctrlKey: true });
    await userEvent.keyboard("c");
    await dialog();
    // Crates are named by their place; folders are not offered.
    expect(optionNames()).toEqual(["Friday residency › Warmup", "Friday residency › Late › Peak time", "Loose"]);

    await userEvent.type(box(), "pea");
    expect(optionNames()[0]).toBe("Friday residency › Late › Peak time");
    await userEvent.keyboard("{Enter}");

    expect(await screen.findByText(added(2, "Peak time"))).toBeInTheDocument();
    expect(backend.called("add_recordings_to_crate")[0]?.args).toEqual({ id: 4, recordings: [102, 103] });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.state.history).toHaveLength(1);
  });

  it("the arrow keys choose another crate, and Go back adds nothing", async () => {
    const backend = await library({ rows: libraryRows(3), crates: CRATES });
    clickRow(0);
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.keyboard("{ArrowDown}{ArrowDown}{Enter}");
    expect(await screen.findByText(added(1, "Loose"))).toBeInTheDocument();

    clickRow(1);
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.click(screen.getByRole("button", { name: tx("crates:addDialog.cancel") }));
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(backend.called("add_recordings_to_crate")).toHaveLength(1);
  });

  it("the palette lists it with its key once tracks are selected, and runs the same dialog", async () => {
    await library({ rows: libraryRows(3), crates: CRATES });
    await userEvent.keyboard("{Control>}k{/Control}");
    const palette = await screen.findByRole("combobox", { name: tx("palette:input.label") });
    await userEvent.type(palette, tx("trackTable:keys.addToCrate"));
    expect(screen.queryByRole("option", { name: (name) => name.includes(tx("trackTable:keys.addToCrate")) })).toBeNull();
    await userEvent.keyboard("{Escape}");

    clickRow(1);
    await userEvent.keyboard("{Control>}k{/Control}");
    await userEvent.type(await screen.findByRole("combobox", { name: tx("palette:input.label") }), tx("trackTable:keys.addToCrate"));
    const option = await screen.findByRole("option", { name: (name) => name.includes(tx("trackTable:keys.addToCrate")) });
    expect(within(option).getByText(tx("trackTable:keys.shortcut.addToCrate"))).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    await dialog();
  });

  it("a new crate from the dialog is made at the top from the typed name, and gets the tracks", async () => {
    const backend = await library({ rows: libraryRows(4), crates: CRATES });
    clickRow(0);
    clickRow(1, { ctrlKey: true });
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.type(box(), "  Closing  ");
    expect(optionNames().at(-1)).toBe(tx("crates:addDialog.create", { name: "Closing" }));
    await userEvent.keyboard("{Enter}");

    expect(await screen.findByText(added(2, "Closing"))).toBeInTheDocument();
    expect(backend.called("create_crate")[0]?.args).toEqual({ name: "Closing", parent: null });
    const made = backend.state.crates.find((c) => c.name === "Closing");
    expect(made?.parentId).toBeNull();
    expect(backend.called("add_recordings_to_crate")[0]?.args).toEqual({ id: made?.id, recordings: [100, 101] });
  });

  it("a new crate follows the sibling-name rules: a name a crate at the top has is that crate, a folder's name is refused", async () => {
    const backend = await library({ rows: libraryRows(3), crates: CRATES });
    clickRow(0);
    await userEvent.keyboard("c");
    await dialog();
    // The same name as a crate at the top (letter case ignored): no second crate is offered.
    await userEvent.type(box(), "LOOSE");
    expect(optionNames()).toEqual(["Loose"]);
    await userEvent.clear(box());

    // The name of a folder at the top is taken too: the refusal is shown and nothing is made or added.
    await userEvent.type(box(), "sets");
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("crates:error.nameTaken", { name: "sets" }));
    expect(backend.called("add_recordings_to_crate")).toHaveLength(0);
    expect(backend.state.crates).toHaveLength(CRATES.length);
    expect(screen.getByRole("dialog")).toBeInTheDocument();

    // The same name in a folder is fine: "Warmup" is Friday residency's, so one at the top is allowed.
    await userEvent.clear(box());
    await userEvent.type(box(), "Warmup");
    expect(optionNames().at(-1)).toBe(tx("crates:addDialog.create", { name: "Warmup" }));
  });

  it("tracks already in the crate are not added twice and the count says so", async () => {
    const backend = await library({ rows: libraryRows(4), crates: CRATES, members: { 5: [100] } });
    clickRow(0);
    clickRow(1, { ctrlKey: true });
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.type(box(), "loose");
    await userEvent.keyboard("{Enter}");

    const line = `${added(1, "Loose")} ${tx("crates:addSelection.alreadyThere", { count: 1 })}`;
    expect(await screen.findByText(line)).toBeInTheDocument();
    expect([...(backend.state.members.get(5) ?? [])]).toEqual([100, 101]);

    // Adding the same tracks again adds none, records nothing, and says so.
    clickRow(0);
    clickRow(1, { ctrlKey: true });
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.type(box(), "loose");
    await userEvent.keyboard("{Enter}");
    expect(
      await screen.findByText(
        `${added(0, "Loose")} ${tx("crates:addSelection.alreadyThere", { count: 2 })}`,
      ),
    ).toBeInTheDocument();
    expect(backend.state.history).toHaveLength(1);
  });

  it("a track outside the Library is left out and the line says how many", async () => {
    const backend = await library({ rows: [...libraryRows(2), outsideRow(5)], crates: CRATES });
    clickRow(0);
    clickRow(2, { ctrlKey: true });
    await userEvent.keyboard("c");
    await dialog();
    await userEvent.type(box(), "loose");
    await userEvent.keyboard("{Enter}");
    expect(
      await screen.findByText(
        `${added(1, "Loose")} ${tx("crates:addSelection.outsideLibrary", { count: 1 })}`,
      ),
    ).toBeInTheDocument();
    expect([...(backend.state.members.get(5) ?? [])]).toEqual([100]);
  });

  it("the keys do not fire in a text field, and C with nothing selected does nothing", async () => {
    await library({ rows: libraryRows(3), crates: CRATES });
    await userEvent.keyboard("c");
    expect(screen.queryByRole("dialog", { name: tx("crates:addDialog.title") })).toBeNull();

    clickRow(0);
    await userEvent.keyboard("c");
    await dialog();
    // Typed into the box: one dialog, and the letter is text.
    await userEvent.type(box(), "c");
    expect(screen.getAllByRole("dialog")).toHaveLength(1);
    expect(box()).toHaveValue("c");
  });
});

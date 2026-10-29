import { clearMocks } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import { crate, crateBackend, folder, libraryRows, outsideRow } from "../test/crateBackend";
import { tx } from "../test/tx";

// The crates a track is in, in the Details panel (1cB-14; ROADMAP 1.14,
// "Membership"), with "Remove from this crate".

const ROW = 28;
const CRATES = [folder(1, "Friday residency"), crate(2, "Warmup", 1), crate(3, "Loose")];

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  document.body.innerHTML = "";
});

const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const clickRow = (index: number) =>
  fireEvent.click(rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === index) as HTMLElement);
const panel = () => within(screen.getByRole("complementary", { name: tx("shell:details.label") }));

async function library(options: Parameters<typeof crateBackend>[0]) {
  const backend = crateBackend(options);
  renderApp("/library");
  await screen.findByRole("grid");
  await waitFor(() => expect(rows().length).toBeGreaterThan(1));
  return backend;
}

describe("the Details panel's crates", () => {
  it("lists the crates the focused track is in, each a link to the crate, in the order of the tree", async () => {
    await library({ rows: libraryRows(3), crates: CRATES, members: { 2: [100], 3: [100, 101] } });
    clickRow(0);
    const warmup = await panel().findByRole("link", { name: "Friday residency › Warmup" });
    expect(warmup).toHaveAttribute("href", expect.stringContaining("crate=2"));
    const loose = panel().getByRole("link", { name: "Loose" });
    expect(loose).toHaveAttribute("href", expect.stringContaining("crate=3"));
    expect(warmup.compareDocumentPosition(loose) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();

    // Another track: only its own crates.
    clickRow(1);
    expect(await panel().findByRole("link", { name: "Loose" })).toBeInTheDocument();
    expect(panel().queryByRole("link", { name: "Friday residency › Warmup" })).toBeNull();
  });

  it("says a Library track is in no crate, and shows nothing for a track outside the Library", async () => {
    await library({ rows: [...libraryRows(2), outsideRow(5)], crates: CRATES });
    clickRow(0);
    expect(await panel().findByText(tx("shell:details.crates.none"))).toBeInTheDocument();
    clickRow(2);
    await waitFor(() => expect(panel().queryByText(tx("shell:details.crates.none"))).toBeNull());
    expect(panel().queryByRole("heading", { name: tx("shell:details.crates.title") })).toBeNull();
  });

  it("Remove from this crate takes the track out as one operation, and one Undo puts it back", async () => {
    const backend = await library({ rows: libraryRows(2), crates: CRATES, members: { 2: [100], 3: [100] } });
    clickRow(0);
    const remove = (await panel().findAllByRole("button", { name: tx("shell:details.crates.remove") }))[0];
    expect(remove).toBeDefined();
    // The visible text is the same for each crate; the crate it belongs to is its description.
    expect(remove).toHaveAccessibleDescription("Friday residency › Warmup");
    await userEvent.click(remove as HTMLElement);

    expect(await panel().findByText(tx("shell:details.crates.removed", { crate: "Friday residency › Warmup" }))).toBeInTheDocument();
    expect(backend.called("remove_tracks_from_crate")).toHaveLength(1);
    expect(backend.called("remove_tracks_from_crate")[0]?.args).toEqual({ id: 2, tracks: [1] });
    await waitFor(() => expect(panel().queryByRole("link", { name: "Friday residency › Warmup" })).toBeNull());
    expect(panel().getByRole("link", { name: "Loose" })).toBeInTheDocument();
    expect(backend.state.history).toHaveLength(1);

    // Undo is in the top bar only: there is no Undo in the panel.
    expect(panel().queryByRole("button", { name: (name) => name.includes(tx("shell:undo.button")) })).toBeNull();
    await userEvent.click(
      await screen.findByRole("button", { name: tx("shell:undo.action.removeFromCrate", { count: 1, name: "Warmup" }) }),
    );
    expect(await panel().findByRole("link", { name: "Friday residency › Warmup" })).toBeInTheDocument();
    expect(backend.state.members.get(2)?.has(100)).toBe(true);
    await waitFor(() => expect(panel().queryByText(tx("shell:details.crates.removed", { crate: "Friday residency › Warmup" }))).toBeNull());
  });
});

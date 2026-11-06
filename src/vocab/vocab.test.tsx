import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { renderApp } from "../app/testApp";
import type { TableQuery, TagGroup } from "../bindings";
import { libraryRows } from "../test/crateBackend";
import { quietAnswer } from "../test/quietCommands";
import { pageOf } from "../test/trackTableIpc";
import { tx } from "../test/tx";

// The tag vocabulary (3A-T): the Tags screen, the chips in the Details panel,
// and the T key's picker. Every name here is made up.

const ROW = 28;

function tag(id: number, name: string, extra: Partial<TagGroup["tags"][number]> = {}): TagGroup["tags"][number] {
  return { id, groupId: 1, name, colour: null, rekordboxName: null, goneFromRekordbox: false, trackCount: 0, ...extra };
}

/** One group, "Mood", with three tags. */
const MOOD: TagGroup = {
  id: 1,
  name: "Mood",
  colour: null,
  tags: [tag(10, "Sunny", { trackCount: 2 }), tag(11, "Dusky", { rekordboxName: "Dusky", goneFromRekordbox: true }), tag(12, "Calm")],
};

type Call = { cmd: string; args: Record<string, unknown> };

/** A fake Rust side that holds the groups and which tags the tracks have. */
function backend(groups: TagGroup[] = [MOOD]) {
  const calls: Call[] = [];
  const have = new Map<number, Set<number>>();
  const rows = libraryRows(2);
  mockIPC(
    (cmd, rawArgs) => {
      const args = (rawArgs ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      switch (cmd) {
        case "list_tags":
          return groups;
        case "tags_of_track":
          return [...(have.get(args.recordingId as number) ?? [])];
        case "apply_tag":
        case "remove_tag": {
          const set = have.get((args.recordings as number[])[0] as number) ?? new Set<number>();
          for (const recording of args.recordings as number[]) {
            const tags = have.get(recording) ?? new Set<number>();
            if (cmd === "apply_tag") tags.add(args.tag as number);
            else tags.delete(args.tag as number);
            have.set(recording, tags);
          }
          return { changed: set.size, skipped: 0, operationId: calls.length };
        }
        case "import_rekordbox_tags":
          return { added: 3, gone: 1, back: 0, applied: 5, skipped: 0, operationId: 9 };
        case "track_table_page":
          return pageOf(rows, args.query as TableQuery);
        case "track_table_ids":
          return rows.map((row) => row.recordingId);
        case "track_table_position":
          return null;
        case "track_tag_sources":
          return { inRekordbox: false, fields: [], confirmedEntry: null };
        case "crates_of_track":
          return [];
        default:
          return quietAnswer(cmd);
      }
    },
    { shouldMockEvents: true },
  );
  const called = (cmd: string) => calls.filter((call) => call.cmd === cmd);
  return { called, have };
}

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  document.body.innerHTML = "";
});

describe("the Tags screen", () => {
  it("lists the groups and tags, and says in words that a tag is gone from rekordbox", async () => {
    backend();
    renderApp("/tags");
    expect(await screen.findByText("Sunny")).toBeInTheDocument();
    expect(screen.getByText("Mood")).toBeInTheDocument();
    expect(screen.getByText(tx("vocab:trackCount", { count: 2 }))).toBeInTheDocument();
    // Only the tag marked gone has the flag.
    expect(screen.getAllByText(tx("vocab:goneFromRekordbox"))).toHaveLength(1);
  });

  it("copies from rekordbox with the group name from the locale, and tells what it did", async () => {
    const fake = backend([]);
    renderApp("/tags");
    expect(await screen.findByText(tx("vocab:empty"))).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: tx("vocab:import.action") }));
    expect(await screen.findByText(tx("vocab:import.done", { added: 3, gone: 1, back: 0, skipped: 0 }))).toBeInTheDocument();
    expect(fake.called("import_rekordbox_tags")[0]?.args).toEqual({ groupName: tx("vocab:import.group") });
  });

  it("makes a tag in a group from the typed name", async () => {
    const fake = backend();
    renderApp("/tags");
    const box = await screen.findByRole("textbox", { name: tx("vocab:newTag.label", { group: "Mood" }) });
    await userEvent.type(box, "Hazy");
    await userEvent.click(screen.getByRole("button", { name: tx("vocab:newTag.action") }));
    await waitFor(() => expect(fake.called("create_tag")).toHaveLength(1));
    expect(fake.called("create_tag")[0]?.args).toEqual({ group: 1, name: "Hazy" });
  });
});

describe("tagging tracks", () => {
  const rows = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
  const panel = () => within(screen.getByRole("complementary", { name: tx("shell:details.label") }));

  it("the Details panel's chips are pressed while the track has the tag, and a press toggles it", async () => {
    const fake = backend();
    fake.have.set(100, new Set([10]));
    renderApp("/library");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(1));
    fireEvent.click(rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === 0) as HTMLElement);

    const sunny = await panel().findByRole("button", { name: "Sunny" });
    expect(sunny).toHaveAttribute("aria-pressed", "true");
    const calm = panel().getByRole("button", { name: "Calm" });
    expect(calm).toHaveAttribute("aria-pressed", "false");

    await userEvent.click(calm);
    await waitFor(() => expect(panel().getByRole("button", { name: "Calm" })).toHaveAttribute("aria-pressed", "true"));
    expect(fake.called("apply_tag")[0]?.args).toEqual({ tag: 12, recordings: [100] });
    await userEvent.click(sunny);
    await waitFor(() => expect(panel().getByRole("button", { name: "Sunny" })).toHaveAttribute("aria-pressed", "false"));
    expect(fake.called("remove_tag")[0]?.args).toEqual({ tag: 10, recordings: [100] });
  });

  it("T opens the picker for the selected tracks, typing narrows it, and Enter tags them", async () => {
    const fake = backend();
    renderApp("/library");
    await screen.findByRole("grid");
    await waitFor(() => expect(rows().length).toBeGreaterThan(1));
    fireEvent.click(rows().find((row) => Number(row.getAttribute("aria-rowindex")) - 2 === 1) as HTMLElement);

    await userEvent.keyboard("t");
    const box = await screen.findByRole("combobox", { name: tx("vocab:picker.search") });
    expect(within(screen.getByRole("listbox")).getAllByRole("option")).toHaveLength(3);
    await userEvent.type(box, "dus");
    expect(within(screen.getByRole("listbox")).getAllByRole("option").map((o) => o.textContent)).toEqual(["Mood › Dusky"]);
    await userEvent.keyboard("{Enter}");

    await waitFor(() => expect(fake.called("apply_tag")).toHaveLength(1));
    expect(fake.called("apply_tag")[0]?.args).toEqual({ tag: 11, recordings: [101] });
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("T does nothing without a selected track", async () => {
    backend();
    renderApp("/library");
    await screen.findByRole("grid");
    await userEvent.keyboard("t");
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("Show tracks opens the Library filtered by the tag, the chip names it, and its × takes it off", async () => {
    const fake = backend();
    renderApp("/tags");
    await screen.findByText("Sunny");
    await userEvent.click(screen.getByRole("link", { name: tx("vocab:showTracks.for", { name: "Sunny" }) }));
    const chip = tx("trackTable:filters.tagNamed", { tag: "Sunny" });
    expect(await screen.findByText(chip)).toBeInTheDocument();
    const filtersOf = () => fake.called("track_table_page").map((call) => (call.args.query as TableQuery).filters);
    await waitFor(() => expect(filtersOf().at(-1)).toEqual([{ kind: "tag", id: 10 }]));
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:filters.remove", { filter: chip }) }));
    await waitFor(() => expect(screen.queryByText(chip)).toBeNull());
    await waitFor(() => expect(filtersOf().at(-1)).toEqual([]));
  });
});

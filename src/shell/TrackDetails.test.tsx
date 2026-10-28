import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { FieldSources, SourcedValue, TagField, TagSources } from "../bindings";
import "../i18n";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { DetailsPanel } from "./DetailsPanel";
import { useSelectedTrack } from "./selectedTrack";
import { useUndoneCount } from "./undoneStore";

const rekordbox = (value: string): SourcedValue => ({ value, origin: "rekordboxEntry", file: null });
const inFile = (value: string, name: string): SourcedValue => ({
  value,
  origin: "file",
  file: { id: 1, name },
});
const field = (name: TagField, shown: SourcedValue | null, others: SourcedValue[] = []): FieldSources => ({
  field: name,
  shown,
  others,
});
const sources = (fields: FieldSources[], confirmedEntry: number | null = null): TagSources => ({
  inRekordbox: confirmedEntry !== null || fields.some((f) => f.shown?.origin === "rekordboxEntry"),
  fields,
  confirmedEntry,
});

/** What the backend answers, and the withdrawals asked for. */
let answer: TagSources;
let withdrawn: number[] = [];
let refuse = false;

beforeEach(() => {
  withdrawn = [];
  refuse = false;
  answer = sources([]);
  mockIPC((cmd, args) => {
    if (cmd === "track_tag_sources") return answer;
    if (cmd === "review_withdraw_relink") {
      withdrawn.push((args as { entryId: number }).entryId);
      if (refuse) throw { kind: "reviewItemGone", params: {} };
      answer = { ...answer, confirmedEntry: null, inRekordbox: false };
      return { operationId: 41 };
    }
    throw new Error(`unexpected command ${cmd}`);
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  useSelectedTrack.setState({ row: null, tables: 0 });
});

function renderPanel(row = tableRow(1)) {
  useSelectedTrack.setState({ row });
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <DetailsPanel />
    </QueryClientProvider>,
  );
}

const fieldNamed = (name: TagField) =>
  screen.getByText(tx(`shell:details.field.${name}`)).closest("div") as HTMLElement;

describe("the Details panel", () => {
  it("asks to select a track while none is selected", () => {
    useSelectedTrack.setState({ row: null });
    render(
      <QueryClientProvider client={createQueryClient()}>
        <DetailsPanel />
      </QueryClientProvider>,
    );
    expect(screen.getByRole("complementary", { name: tx("shell:details.label") })).toHaveTextContent(
      tx("shell:details.empty"),
    );
  });

  it("shows each field's value and where it comes from", async () => {
    answer = sources([
      field("title", rekordbox("Synthetic Tune")),
      field("genre", inFile("House", "tune.mp3")),
    ]);
    renderPanel();
    const title = await waitFor(() => fieldNamed("title"));
    expect(title).toHaveTextContent("Synthetic Tune");
    expect(title).toHaveTextContent(tx("shell:details.source.rekordbox"));
    const genre = fieldNamed("genre");
    expect(genre).toHaveTextContent("House");
    expect(genre).toHaveTextContent(tx("shell:details.source.file", { file: "tune.mp3" }));
  });

  it("says rekordbox has no value for a field it leaves blank", async () => {
    answer = sources([field("genre", null, [inFile("House", "tune.mp3")])]);
    renderPanel();
    const genre = await waitFor(() => fieldNamed("genre"));
    expect(genre).toHaveTextContent(tx("shell:details.source.blankInRekordbox"));
  });

  it("lists the other values, each naming its file, only while More details is on", async () => {
    const user = userEvent.setup();
    answer = sources([
      field("title", rekordbox("Synthetic Tune"), [
        inFile("Tune (file)", "a.mp3"),
        inFile("Tune (copy)", "b.mp3"),
      ]),
    ]);
    renderPanel();
    const title = await waitFor(() => fieldNamed("title"));
    // Off: they are only counted.
    expect(title).toHaveTextContent(tx("shell:details.source.other", { count: 2 }));
    expect(within(title).queryByText("Tune (file)")).toBeNull();

    const more = screen.getByRole("checkbox", { name: tx("shell:details.moreDetails") });
    expect(more).not.toBeChecked();
    await user.click(more);
    expect(within(title).getByText("Tune (file)")).toBeInTheDocument();
    expect(within(title).getByText("Tune (copy)")).toBeInTheDocument();
    const listed = within(title).getAllByRole("listitem");
    expect(listed[0]).toHaveTextContent(tx("shell:details.source.file", { file: "a.mp3" }));
    expect(listed[1]).toHaveTextContent(tx("shell:details.source.file", { file: "b.mp3" }));
    expect(title).not.toHaveTextContent(tx("shell:details.source.other", { count: 2 }));

    await user.click(more);
    expect(within(title).queryByText("Tune (file)")).toBeNull();
  });

  it("shows a long title and a long path in full", async () => {
    const title = "A very long title ".repeat(12).trim();
    const path = "E:\\Music\\" + "Some deeply nested folder name\\".repeat(8) + "track.mp3";
    renderPanel(
      tableRow(1, {
        title,
        file: { id: 1, path, name: "track.mp3", present: true, driveConnected: true },
      }),
    );
    expect(screen.getByRole("heading", { level: 2 }).textContent).toBe(title);
    expect(screen.getByText(path).textContent).toBe(path);
  });

  it("offers to withdraw a confirmation only when there is one", async () => {
    answer = sources([field("title", rekordbox("Synthetic Tune"))], null);
    renderPanel();
    await waitFor(() => fieldNamed("title"));
    expect(screen.queryByRole("button", { name: tx("shell:details.withdraw.action") })).toBeNull();
    cleanup();

    answer = sources([field("title", rekordbox("Synthetic Tune"))], 12);
    renderPanel();
    expect(await screen.findByRole("button", { name: tx("shell:details.withdraw.action") })).toBeEnabled();
    expect(screen.getByText(tx("shell:details.withdraw.confirmed"))).toBeInTheDocument();
  });

  it("withdraws the confirmation of the entry it names and says so, with no Undo of its own", async () => {
    const user = userEvent.setup();
    answer = sources([field("title", rekordbox("Synthetic Tune"))], 12);
    renderPanel();
    await user.click(await screen.findByRole("button", { name: tx("shell:details.withdraw.action") }));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("shell:details.withdraw.done"));
    expect(withdrawn).toEqual([12]);
    // The panel asks again: nothing is left to withdraw.
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: tx("shell:details.withdraw.action") })).toBeNull(),
    );
    // Undo is in the top bar only.
    expect(screen.queryByRole("button", { name: /undo/i })).toBeNull();
  });

  it("drops the withdrawn line once an Undo took the withdrawal back", async () => {
    const user = userEvent.setup();
    answer = sources([field("title", rekordbox("Synthetic Tune"))], 12);
    renderPanel();
    await user.click(await screen.findByRole("button", { name: tx("shell:details.withdraw.action") }));
    expect(await screen.findByRole("status")).toBeInTheDocument();
    useUndoneCount.getState().undone();
    await waitFor(() => expect(screen.queryByRole("status")).toBeNull());
  });

  it("shows the reason when the withdrawal is refused", async () => {
    const user = userEvent.setup();
    refuse = true;
    answer = sources([field("title", rekordbox("Synthetic Tune"))], 12);
    renderPanel();
    await user.click(await screen.findByRole("button", { name: tx("shell:details.withdraw.action") }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(screen.queryByText(tx("shell:details.withdraw.done"))).toBeNull();
  });
});

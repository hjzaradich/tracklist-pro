import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { ConflictRow, ValueKind } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { dayOf } from "./conflictValues";
import { ReviewScreen } from "./ReviewScreen";

function conflict(
  id: number,
  field: string,
  kind: ValueKind,
  title: string,
  values: { app: string | null; rekordbox: string | null; base: string | null },
  fileName: string | null = null,
): ConflictRow {
  return {
    id,
    track: { libraryTrack: id, title, artist: "Made Up Artist", fileName },
    field,
    kind,
    ...values,
  };
}

const RATINGS = [
  conflict(1, "Rating", "rating", "Synthetic Tune", { app: "153", rekordbox: "204", base: "102" }),
  conflict(2, "Rating", "rating", "Second Tune", { app: "51", rekordbox: "255", base: null }),
  conflict(3, "Rating", "rating", "Third Tune", { app: "0", rekordbox: "102", base: "255" }),
];
const GENRES = [
  conflict(4, "Genre", "text", "Synthetic Tune", { app: "House", rekordbox: "Techno", base: "Disco" }),
];

interface Calls {
  cmd: string;
  args: Record<string, unknown>;
}

/** Stands in for the Rust side: the open conflicts by field, settled as the commands are called. */
function backend(
  initial: ConflictRow[],
  {
    refuse,
    reviewItems = 0,
    hangRows = false,
    crates = {},
  }: {
    refuse?: object;
    reviewItems?: number;
    hangRows?: boolean;
    /** The crate name of each crate-membership field. */
    crates?: Record<string, string>;
  } = {},
) {
  const open = [...initial];
  const calls: Calls[] = [];
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    switch (cmd) {
      case "missing_tracks":
        return { total: 0, groups: [] };
      case "review_counts":
        return {
          needsCloserLook: reviewItems,
          clearBestFile: 0,
          qualityProblems: 0,
          conflicts: open.length,
          total: reviewItems + open.length,
        };
      case "review_items":
        return { group: "needsCloserLook", total: 0, items: [] };
      case "review_conflict_fields": {
        const fields = [...new Set(open.map((row) => row.field))];
        return fields.map((field) => {
          const rows = open.filter((row) => row.field === field);
          return { field, kind: rows[0]!.kind, count: rows.length, crateName: crates[field] ?? null };
        });
      }
      case "review_conflicts": {
        if (hangRows) return new Promise(() => {});
        const { field, offset, limit } = args as { field: string; offset: number; limit: number };
        const rows = open.filter((row) => row.field === field);
        return { field, total: rows.length, items: rows.slice(offset, offset + limit) };
      }
      case "review_resolve_conflict": {
        if (refuse) throw refuse;
        const { id } = args as { id: number };
        const at = open.findIndex((row) => row.id === id);
        open.splice(at, 1);
        return { libraryTrack: id, field: "x", status: "keptApp", value: "" };
      }
      case "review_resolve_conflicts": {
        const { field, expected } = args as { field: string; expected: number };
        const count = open.filter((row) => row.field === field).length;
        if (count !== expected) throw { kind: "conflictsChanged", params: {} };
        for (let at = open.length - 1; at >= 0; at--) {
          if (open[at]!.field === field) open.splice(at, 1);
        }
        return count;
      }
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return { calls, open };
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <ReviewScreen />
    </QueryClientProvider>,
  );
}

/** The row of the conflict about `title`, in the pane. */
async function lineOf(title: string) {
  const name = tx("review:item.trackName", { artist: "Made Up Artist", title });
  let line: HTMLElement | undefined;
  await waitFor(() => {
    const table = screen.getByRole("table", { name: tx("conflicts:label") });
    line = within(table)
      .getAllByRole("row")
      .find((row) => within(row).queryByText(name) !== null);
    expect(line).toBeDefined();
  });
  return line!;
}

const settlements = (calls: Calls[]) =>
  calls.filter((c) => c.cmd === "review_resolve_conflict" || c.cmd === "review_resolve_conflicts");

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the Conflicts group", () => {
  it("lists each field that has conflicts, with how many, and shows the first field's conflicts", async () => {
    backend([...RATINGS, ...GENRES]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    const options = await within(group).findAllByRole("option");
    // By the name the user sees: Genre before Rating.
    expect(options[0]).toHaveTextContent(tx("conflicts:field.Genre"));
    expect(options[0]).toHaveTextContent(tx("conflicts:fieldCount", { count: 1 }));
    expect(options[1]).toHaveTextContent(tx("conflicts:field.Rating"));
    expect(options[1]).toHaveTextContent(tx("conflicts:fieldCount", { count: 3 }));
    expect(within(group).getByText(tx("review:queue.left", { count: 4 }))).toBeInTheDocument();
    // The first field is selected, its conflicts are in the pane.
    expect(options[0]).toHaveAttribute("aria-selected", "true");
    expect(await lineOf("Synthetic Tune")).toHaveTextContent("Techno");
  });

  it("shows no group and no pane while no field has a conflict, and the empty state names what is absent", async () => {
    backend([]);
    renderScreen();
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: tx("conflicts:group") })).toBeNull();
    expect(screen.queryByRole("table", { name: tx("conflicts:label") })).toBeNull();
  });

  it("is the whole queue when no item waits, with no empty state", async () => {
    backend(GENRES, { reviewItems: 0 });
    renderScreen();
    await screen.findByRole("region", { name: tx("conflicts:group") });
    expect(screen.queryByText(tx("review:queue.empty"))).toBeNull();
  });

  it("shows the app's value, rekordbox's and the last send side by side, and that a field was never sent", async () => {
    backend([...RATINGS, ...GENRES]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Rating")));
    const first = await lineOf("Synthetic Tune");
    const cells = within(first).getAllByRole("cell");
    // The track, the app's 3 stars, rekordbox's 4, the last send's 2.
    expect(within(cells[1]!).getByRole("img", { name: tx("conflicts:stars", { count: 3 }) })).toBeVisible();
    expect(within(cells[2]!).getByRole("img", { name: tx("conflicts:stars", { count: 4 }) })).toBeVisible();
    expect(within(cells[3]!).getByRole("img", { name: tx("conflicts:stars", { count: 2 }) })).toBeVisible();
    const never = await lineOf("Second Tune");
    expect(never).toHaveTextContent(tx("conflicts:neverSent"));
    // No rating is said in words, not as an empty cell.
    expect(await lineOf("Third Tune")).toHaveTextContent(tx("conflicts:noRating"));
    expect(
      screen.getAllByRole("columnheader").map((h) => h.textContent),
    ).toEqual([
      tx("conflicts:col.track"),
      tx("conflicts:col.app"),
      tx("conflicts:col.rekordbox"),
      tx("conflicts:col.base"),
      tx("conflicts:col.actions"),
    ]);
  });

  it("shows a colour with its code and a date as the user's language writes it", async () => {
    backend([
      conflict(1, "Colour", "colour", "Colour Tune", { app: "0xFF0000", rekordbox: "", base: null }),
      conflict(2, "DateAdded", "date", "Dated Tune", {
        app: "2026-09-28",
        rekordbox: "2026-09-27",
        base: null,
      }),
    ]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Colour")));
    const colour = await lineOf("Colour Tune");
    expect(colour).toHaveTextContent("0xFF0000");
    expect(colour).toHaveTextContent(tx("conflicts:noValue"));
    await userEvent.click(within(group).getByText(tx("conflicts:field.DateAdded")));
    const dated = await lineOf("Dated Tune");
    expect(dated).toHaveTextContent(dayOf("2026-09-28", "en"));
    expect(dated).toHaveTextContent(dayOf("2026-09-27", "en"));
  });

  it("names a field the app has no word for by rekordbox's own name", async () => {
    backend([conflict(1, "Mix", "text", "Mixed Tune", { app: "a", rekordbox: "b", base: null })]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    expect(await within(group).findByRole("option")).toHaveTextContent("Mix");
  });
});

describe("resolving one conflict", () => {
  it("keeps the app's value, and the conflict leaves the list", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    await waitFor(() => expect(screen.queryByText("Synthetic Tune", { exact: false })).toBeNull());
    expect(settlements(calls)).toEqual([
      { cmd: "review_resolve_conflict", args: { id: 1, choice: { choice: "keepApp" } } },
    ]);
    // The others are still there.
    expect(await lineOf("Second Tune")).toBeInTheDocument();
  });

  it("keeps rekordbox's value", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    const line = await lineOf("Second Tune");
    await userEvent.click(
      within(line).getByRole("button", { name: tx("conflicts:action.keepRekordbox") }),
    );
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]).toEqual({
      cmd: "review_resolve_conflict",
      args: { id: 2, choice: { choice: "keepRekordbox" } },
    });
  });

  it("gives a value by hand of the field's kind: a rating is chosen among the six", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.edit") }));
    const choice = within(line).getByRole("combobox", { name: tx("conflicts:edit.label") });
    // It starts from the app's value.
    expect(choice).toHaveValue("153");
    await userEvent.selectOptions(choice, "255");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.save") }));
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]).toEqual({
      cmd: "review_resolve_conflict",
      args: { id: 1, choice: { choice: "edit", value: "255" } },
    });
  });

  it("refuses a number that is not whole digits: Save is greyed out until it fits", async () => {
    const { calls } = backend([
      conflict(1, "PlayCount", "number", "Counted Tune", { app: "4", rekordbox: "9", base: "2" }),
    ]);
    renderScreen();
    const line = await lineOf("Counted Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.edit") }));
    const field = within(line).getByRole("textbox", { name: tx("conflicts:edit.label") });
    const save = within(line).getByRole("button", { name: tx("conflicts:action.save") });
    expect(field).toHaveValue("4");
    expect(save).toBeEnabled();
    await userEvent.clear(field);
    await userEvent.type(field, "many");
    expect(save).toBeDisabled();
    expect(field).toHaveAttribute("aria-invalid", "true");
    await userEvent.clear(field);
    await userEvent.type(field, "12");
    expect(save).toBeEnabled();
    await userEvent.click(save);
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]!.args).toEqual({ id: 1, choice: { choice: "edit", value: "12" } });
  });

  it("goes back from editing without settling anything", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.edit") }));
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.goBack") }));
    expect(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") })).toBeVisible();
    expect(settlements(calls)).toEqual([]);
  });

  it("shows why a settling was refused, and keeps the row", async () => {
    backend(RATINGS, { refuse: { kind: "conflictBadValue", params: {} } });
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("conflicts:error.badValue"));
    expect(await lineOf("Synthetic Tune")).toBeInTheDocument();
  });

  it("has no filled button, and no Undo of its own: Undo is the top bar's", async () => {
    backend(RATINGS);
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    const names = within(line)
      .getAllByRole("button")
      .map((b) => b.textContent);
    expect(names).toEqual([
      tx("conflicts:action.keepApp"),
      tx("conflicts:action.keepRekordbox"),
      tx("conflicts:action.edit"),
    ]);
    expect(screen.queryByRole("button", { name: tx("shell:undo.button") })).toBeNull();
  });
});

describe("resolving a whole field", () => {
  it("keeps rekordbox's value for every conflict of the field in one call", async () => {
    const { calls } = backend([...RATINGS, ...GENRES]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Rating")));
    await lineOf("Synthetic Tune");
    await userEvent.click(
      screen.getByRole("button", { name: tx("conflicts:action.keepRekordboxAll", { count: 3 }) }),
    );
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]).toEqual({
      cmd: "review_resolve_conflicts",
      args: { field: "Rating", side: expect.stringMatching(/^rekordbox$/), expected: 3 },
    });
    // The field has left the queue; the other field is shown.
    await waitFor(() =>
      expect(within(group).queryByText(tx("conflicts:field.Rating"))).toBeNull(),
    );
    expect(await lineOf("Synthetic Tune")).toHaveTextContent("Techno");
  });

  it("keeps the app's value for every conflict of the field in one call", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    await lineOf("Synthetic Tune");
    await userEvent.click(
      screen.getByRole("button", { name: tx("conflicts:action.keepAppAll", { count: 3 }) }),
    );
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]!.args).toEqual({ field: "Rating", side: "app", expected: 3 });
    // Nothing is left, and the open conflicts were counted: the empty state.
    expect(await screen.findByText(tx("review:queue.empty"))).toBeInTheDocument();
  });

  it("counts all of a field's conflicts on the button, not only the ones loaded", async () => {
    const many = Array.from({ length: 130 }, (_, n) =>
      conflict(n + 1, "Rating", "rating", `Tune ${n + 1}`, { app: "51", rekordbox: "102", base: null }),
    );
    backend(many);
    renderScreen();
    await lineOf("Tune 1");
    expect(
      screen.getByRole("button", { name: tx("conflicts:action.keepAppAll", { count: 130 }) }),
    ).toBeEnabled();
    // 100 are loaded; the rest come on request.
    const more = screen.getByRole("button", { name: tx("review:queue.more") });
    expect(screen.getAllByRole("row")).toHaveLength(101);
    await userEvent.click(more);
    await waitFor(() => expect(screen.getAllByRole("row")).toHaveLength(131));
    expect(screen.queryByRole("button", { name: tx("review:queue.more") })).toBeNull();
  });
});

describe("what each kind of field shows and offers", () => {
  it("shows the file's name under a title or an artist in dispute, and under no other field", async () => {
    backend([
      conflict(1, "Name", "text", "Disputed Title", { app: "a", rekordbox: "b", base: null }, "disputed file.mp3"),
      conflict(2, "Genre", "text", "Other Tune", { app: "a", rekordbox: "b", base: null }, "other file.mp3"),
    ]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Name")));
    expect(await lineOf("Disputed Title")).toHaveTextContent("disputed file.mp3");
    await userEvent.click(within(group).getByText(tx("conflicts:field.Genre")));
    expect(await lineOf("Other Tune")).not.toHaveTextContent("other file.mp3");
  });

  it("offers no Edit for a file location, shows it without the percent-encoding, and still keeps a side", async () => {
    const { calls } = backend([
      conflict(1, "Location", "location", "Moved Tune", {
        app: "file://localhost/D:/Sets/a%20b.mp3",
        rekordbox: "file://localhost/D:/Sets/c.mp3",
        base: null,
      }),
    ]);
    renderScreen();
    const line = await lineOf("Moved Tune");
    expect(within(line).queryByRole("button", { name: tx("conflicts:action.edit") })).toBeNull();
    expect(line).toHaveTextContent("file://localhost/D:/Sets/a b.mp3");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
  });

  it("says no value for 0 in a field where 0 is none, and shows 0 as a count in a play count", async () => {
    backend([
      conflict(1, "Year", "number", "Aged Tune", { app: "0", rekordbox: "2001", base: null }),
      conflict(2, "PlayCount", "number", "Played Tune", { app: "0", rekordbox: "5", base: null }),
    ]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Year")));
    const year = await lineOf("Aged Tune");
    expect(within(year).getAllByRole("cell")[1]).toHaveTextContent(tx("conflicts:noValue"));
    await userEvent.click(within(group).getByText(tx("conflicts:field.PlayCount")));
    const plays = await lineOf("Played Tune");
    expect(within(plays).getAllByRole("cell")[1]).toHaveTextContent(/^0$/);
  });
});

describe("giving a value by hand in a field of its kind", () => {
  it("takes a color from the picker as 0xRRGGBB, or none by its own choice", async () => {
    const { calls } = backend([
      conflict(1, "Colour", "colour", "Colour Tune", { app: "0xFF0000", rekordbox: "0x0000FF", base: null }),
      conflict(2, "Colour", "colour", "Plain Tune", { app: "0xFF0000", rekordbox: "0x0000FF", base: null }),
    ]);
    renderScreen();
    const line = await lineOf("Colour Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.edit") }));
    const picker = within(line).getByLabelText(tx("conflicts:edit.label"));
    expect(picker).toHaveValue("#ff0000");
    fireEvent.change(picker, { target: { value: "#00ff00" } });
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.save") }));
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]!.args).toEqual({ id: 1, choice: { choice: "edit", value: "0x00FF00" } });

    const plain = await lineOf("Plain Tune");
    await userEvent.click(within(plain).getByRole("button", { name: tx("conflicts:action.edit") }));
    await userEvent.click(within(plain).getByRole("checkbox", { name: tx("conflicts:edit.noColor") }));
    await userEvent.click(within(plain).getByRole("button", { name: tx("conflicts:action.save") }));
    await waitFor(() => expect(settlements(calls)).toHaveLength(2));
    expect(settlements(calls)[1]!.args).toEqual({ id: 2, choice: { choice: "edit", value: "" } });
  });

  it("will not save a date until it is whole: a half-typed date reads empty and would clear the field", async () => {
    const { calls } = backend([
      conflict(1, "DateAdded", "date", "Dated Tune", { app: "", rekordbox: "2026-09-27", base: null }),
    ]);
    renderScreen();
    const line = await lineOf("Dated Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.edit") }));
    const save = within(line).getByRole("button", { name: tx("conflicts:action.save") });
    const field = within(line).getByLabelText(tx("conflicts:edit.label"));
    // Empty, as a half-typed date reads.
    expect(save).toBeDisabled();
    fireEvent.change(field, { target: { value: "2026-1" } });
    expect(save).toBeDisabled();
    fireEvent.change(field, { target: { value: "2026-10-01" } });
    expect(save).toBeEnabled();
    await userEvent.click(save);
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]!.args).toEqual({ id: 1, choice: { choice: "edit", value: "2026-10-01" } });
  });
});

describe("refusals and what comes after", () => {
  it("says a conflict was resolved already when it is", async () => {
    backend(RATINGS, { refuse: { kind: "conflictGone", params: {} } });
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("conflicts:error.gone"));
  });

  it("asks again, with the new number, when the conflicts changed since the button was drawn", async () => {
    const { open } = backend(RATINGS);
    renderScreen();
    await lineOf("Synthetic Tune");
    const button = screen.getByRole("button", { name: tx("conflicts:action.keepAppAll", { count: 3 }) });
    // A fourth conflict arrives before the click.
    open.push(conflict(9, "Rating", "rating", "Late Tune", { app: "51", rekordbox: "102", base: null }));
    await userEvent.click(button);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("conflicts:error.changed"));
    // Nothing was settled, and the list now shows the fourth.
    expect(await lineOf("Late Tune")).toBeInTheDocument();
    expect(
      await screen.findByRole("button", { name: tx("conflicts:action.keepAppAll", { count: 4 }) }),
    ).toBeEnabled();
  });

  it("forgets an earlier refusal once a later resolve works", async () => {
    backend([...RATINGS, ...GENRES], { refuse: { kind: "conflictBadValue", params: {} } });
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    await userEvent.click(within(group).getByText(tx("conflicts:field.Rating")));
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    expect(await screen.findByRole("alert")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: tx("conflicts:action.keepRekordboxAll", { count: 3 }) }),
    );
    await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  });

  it("moves the focus to the next row when a conflict leaves the table", async () => {
    backend(RATINGS);
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepApp") }));
    const next = await lineOf("Second Tune");
    await waitFor(() =>
      expect(within(next).getByRole("button", { name: tx("conflicts:action.keepApp") })).toHaveFocus(),
    );
  });

  it("shows no count and no bulk buttons while the first page is not here yet", async () => {
    backend(RATINGS, { hangRows: true });
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    expect(group).toBeInTheDocument();
    await screen.findByRole("table", { name: tx("conflicts:label") });
    expect(screen.queryByText(tx("conflicts:fieldCount", { count: 0 }))).toBeNull();
    expect(screen.queryByRole("button", { name: tx("conflicts:action.keepAppAll", { count: 0 }) })).toBeNull();
    expect(screen.queryByRole("button", { name: tx("conflicts:action.keepRekordboxAll", { count: 0 }) })).toBeNull();
  });
});

describe("the keyboard", () => {
  it("does nothing on Enter, S or a number: a conflict has no suggestion", async () => {
    const { calls } = backend(RATINGS);
    renderScreen();
    await lineOf("Synthetic Tune");
    await userEvent.keyboard("{Enter}s1");
    expect(settlements(calls)).toEqual([]);
  });

  it("moves through the fields with the arrow keys, selecting as it goes", async () => {
    backend([...RATINGS, ...GENRES]);
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    const options = await within(group).findAllByRole("option");
    options[0]!.focus();
    await userEvent.keyboard("{ArrowDown}");
    expect(options[1]).toHaveFocus();
    expect(options[1]).toHaveAttribute("aria-selected", "true");
    expect(await lineOf("Second Tune")).toBeInTheDocument();
    await userEvent.keyboard("{Home}");
    expect(options[0]).toHaveFocus();
    expect(options[0]).toHaveAttribute("aria-selected", "true");
  });
});

describe("a crate membership conflict", () => {
  // A crate named with a space, quotes and an accent, inside a folder: the
  // field is a playlist path as JSON, nothing a screen can show or use as an id.
  const NAME = 'Peak "time" café';
  const FIELD = JSON.stringify(["crates", "gigs", "peak \"time\" café"]);
  const MEMBERSHIP = `playlist:${FIELD}`;
  const CRATE = `Gigs / ${NAME}`;
  const ROWS = [
    conflict(7, MEMBERSHIP, "membership", "Synthetic Tune", { app: "in", rekordbox: "out", base: "in" }),
    conflict(8, MEMBERSHIP, "membership", "Second Tune", { app: "out", rekordbox: "in", base: null }),
  ];

  it("is named by its crate, in the group and above its conflicts, never by its field name", async () => {
    backend([...ROWS, ...GENRES], { crates: { [MEMBERSHIP]: CRATE } });
    renderScreen();
    const group = await screen.findByRole("region", { name: tx("conflicts:group") });
    const label = tx("conflicts:membership.field", { name: CRATE });
    const options = await within(group).findAllByRole("option");
    expect(options.map((o) => o.textContent)).toEqual([
      expect.stringContaining(tx("conflicts:field.Genre")),
      expect.stringContaining(label),
    ]);
    expect(group).not.toHaveTextContent("playlist:");
    await userEvent.click(within(group).getByText(label));
    const line = await lineOf("Synthetic Tune");
    // The values are said in words: the app's In, rekordbox's Not in, the last send's In.
    const cells = within(line).getAllByRole("cell");
    expect(cells[1]).toHaveTextContent(tx("conflicts:membership.in"));
    expect(cells[2]).toHaveTextContent(tx("conflicts:membership.out"));
    expect(cells[3]).toHaveTextContent(tx("conflicts:membership.in"));
    expect(await lineOf("Second Tune")).toHaveTextContent(tx("conflicts:neverSent"));
  });

  it("offers keeping the app's or rekordbox's, never Edit, and settles with them", async () => {
    const { calls } = backend(ROWS, { crates: { [MEMBERSHIP]: CRATE } });
    renderScreen();
    const line = await lineOf("Synthetic Tune");
    expect(within(line).getAllByRole("button").map((b) => b.textContent)).toEqual([
      tx("conflicts:action.keepApp"),
      tx("conflicts:action.keepRekordbox"),
    ]);
    expect(within(line).queryByRole("button", { name: tx("conflicts:action.edit") })).toBeNull();
    await userEvent.click(within(line).getByRole("button", { name: tx("conflicts:action.keepRekordbox") }));
    await waitFor(() => expect(settlements(calls)).toHaveLength(1));
    expect(settlements(calls)[0]?.args).toEqual({ id: 7, choice: { choice: "keepRekordbox" } });
  });

  it("labels its pane by an id that is safe whatever the crate is called", async () => {
    backend(ROWS, { crates: { [MEMBERSHIP]: CRATE } });
    renderScreen();
    await lineOf("Synthetic Tune");
    // The pane is found by its name, which only an id that resolves can give.
    const label = tx("conflicts:membership.field", { name: CRATE });
    const pane = await screen.findByRole("region", { name: (name) => name.startsWith(label) });
    const id = pane.getAttribute("aria-labelledby") ?? "";
    expect(id).not.toMatch(/[\s"'\\]/);
    expect(document.getElementById(id)).toHaveTextContent(CRATE);
  });
});

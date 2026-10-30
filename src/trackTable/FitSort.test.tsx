import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { RowFit, TablePositionQuery, TableQuery, TableRow, TableScreen } from "../bindings";
import "../i18n";
import { listedActions, usePaletteRegistry } from "../palette/registry";
import { pageOf, queryOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";
import { TrackTableView } from "./TrackTableView";

const ROW = 28;
const CRATE = 7;

/** The crate's tracks, in the order they were added: track n has id 100 + n. */
const TRACKS: TableRow[] = [0, 1, 2, 3].map((n) =>
  tableRow(n, { recordingId: 100 + n, title: `Tune ${n}`, bpm: 124, key: "8A" }),
);

const NEIGHBOR: RowFit = {
  dots: 3,
  reason: {
    key: { kind: "neighbor", from: "8A", to: "9A" },
    bpm: { kind: "delta", delta: 1.5, time: "straight" },
  },
};

/** What the backend says each track's fit is, when sorted by fit after some other track. */
let fits: Record<number, RowFit>;
let asked: TableQuery[] = [];
let positionsAsked: TablePositionQuery[] = [];
let saved: unknown[] = [];

/**
 * A crate of four tracks. Sorted by fit, the backend (faked) puts the track
 * asked for first and the others in reverse, each with the fit `fits` has for
 * it; the real order and reasons are tested in Rust.
 */
function serve() {
  mockIPC((cmd, args) => {
    switch (cmd) {
      case "key_notation":
        return "camelot";
      case "table_columns":
        return null;
      case "set_table_columns":
        saved.push(args);
        return null;
      case "track_table_formats":
        return [];
      case "track_table_position": {
        const query = (args as { query: TablePositionQuery }).query;
        positionsAsked.push(query);
        const rows = rowsFor(query);
        const at = rows.findIndex((row) => row.recordingId === query.recordingId);
        return at < 0 ? null : at;
      }
      case "track_table_page": {
        const query = queryOf(args);
        asked.push(query);
        return pageOf(rowsFor(query), query);
      }
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
}

function rowsFor(query: Pick<TableQuery, "source">): TableRow[] {
  const { source } = query;
  if (source.kind !== "crateByFit") return TRACKS;
  const first = TRACKS.find((row) => row.recordingId === source.after);
  const rest = TRACKS.filter((row) => row.recordingId !== source.after).reverse();
  return [first, ...rest]
    .filter((row): row is TableRow => row !== undefined)
    .map((row) => ({
      ...row,
      fit: row.recordingId === source.after ? { dots: 0, reason: null } : (fits[row.recordingId] ?? NEIGHBOR),
    }));
}

beforeEach(() => {
  asked = [];
  positionsAsked = [];
  saved = [];
  fits = {};
  vi.spyOn(HTMLElement.prototype, "clientHeight", "get").mockReturnValue(ROW * 11);
});

afterEach(() => {
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

function renderCrate(screenName: TableScreen = "crate") {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <TrackTableView
        screen={screenName}
        source={screenName === "crate" ? { kind: "crate", id: CRATE } : { kind: "library" }}
        keyPrefix={["test", screenName]}
        count={(total) => `rows: ${total}`}
        empty={<p>Nothing at all</p>}
      />
    </QueryClientProvider>,
  );
}

const drawn = () => within(screen.getByRole("grid")).getAllByRole("row").slice(1);
const lastQuery = () => asked[asked.length - 1];
const sortByFit = () => screen.getByRole("button", { name: tx("trackTable:fit.button") });
const headings = () => screen.getAllByRole("columnheader").map((header) => header.textContent);

async function openCrate() {
  serve();
  renderCrate();
  await screen.findByRole("grid");
  await waitFor(() => expect(drawn()).toHaveLength(TRACKS.length));
}

async function select(index: number) {
  await userEvent.click(drawn()[index]);
  await waitFor(() => expect(sortByFit()).toBeEnabled());
}

const reasonOf = (key: { from: string; to: string }, delta: string) =>
  [
    tx("trackTable:fit.keyChange", key),
    tx("trackTable:fit.bpm", { delta }),
  ].join(tx("trackTable:fit.separator"));

describe("Sort by fit after the selected track", () => {
  it("is greyed out while no track is selected, with the reason beside it", async () => {
    await openCrate();
    expect(sortByFit()).toBeDisabled();
    expect(screen.getByText(tx("trackTable:fit.needsSelection"))).toBeInTheDocument();
    await userEvent.click(sortByFit());
    expect(asked.every((query) => query.source.kind === "crate")).toBe(true);
    // Selecting a track enables it, and the reason goes.
    await select(1);
    expect(screen.queryByText(tx("trackTable:fit.needsSelection"))).toBeNull();
  });

  it("asks for the crate sorted after the selected track and shows the Fit column with dots and the reason", async () => {
    await openCrate();
    expect(headings()).not.toContain(tx("trackTable:columns.fit"));
    await select(2);
    await userEvent.click(sortByFit());

    await waitFor(() =>
      expect(lastQuery().source).toEqual({ kind: "crateByFit", id: CRATE, after: TRACKS[2].recordingId }),
    );
    await waitFor(() => expect(headings()).toContain(tx("trackTable:columns.fit")));
    // The selected track first, saying so; the others with dots and a reason.
    const rows = drawn();
    expect(within(rows[0]).getByText(tx("trackTable:fit.selected"))).toBeInTheDocument();
    expect(within(rows[0]).queryByRole("img")).toBeNull();
    for (const row of rows.slice(1)) {
      expect(within(row).getByText(reasonOf({ from: "8A", to: "9A" }, "+1.5"))).toBeInTheDocument();
      expect(within(row).getByRole("img", { name: tx("trackTable:fit.dots", { count: 3, most: 3 }) })).toBeInTheDocument();
    }
    expect(screen.getByText(tx("trackTable:fit.sortedAfter", { track: "Tune 2" }))).toBeInTheDocument();
  });

  it("lists the selected track first and keeps it selected", async () => {
    await openCrate();
    await select(2);
    positionsAsked = [];
    await userEvent.click(sortByFit());
    await waitFor(() => expect(within(drawn()[0]).getByText("Tune 2")).toBeInTheDocument());
    // The table asked where the selected track went, and it is selected, at the top.
    await waitFor(() => expect(drawn()[0]).toHaveAttribute("aria-selected", "true"));
    expect(positionsAsked.at(-1)).toMatchObject({
      source: { kind: "crateByFit", after: TRACKS[2].recordingId },
      recordingId: TRACKS[2].recordingId,
    });
  });

  it("says plainly what is missing in a reason", async () => {
    await openCrate();
    fits = {
      [TRACKS[0].recordingId]: {
        dots: 0,
        reason: { key: { kind: "noKey" }, bpm: { kind: "noBpm" } },
      },
      [TRACKS[1].recordingId]: {
        dots: 1,
        reason: { key: { kind: "noKey" }, bpm: { kind: "delta", delta: -2, time: "half" } },
      },
      [TRACKS[3].recordingId]: {
        dots: 0,
        reason: { key: { kind: "noTargetKey" }, bpm: { kind: "noTargetBpm" } },
      },
    };
    await select(2);
    await userEvent.click(sortByFit());
    await waitFor(() => expect(headings()).toContain(tx("trackTable:columns.fit")));
    const text = (index: number) => drawn()[index].textContent ?? "";
    const sep = tx("trackTable:fit.separator");
    // The fake backend lists the others in reverse: 3, 1, 0.
    await waitFor(() => expect(text(1)).toContain(tx("trackTable:fit.noTarget")));
    expect(text(2)).toContain(
      [
        tx("trackTable:fit.noKey"),
        tx("trackTable:fit.halfTime", { bpm: tx("trackTable:fit.bpm", { delta: "-2.0" }) }),
      ].join(sep),
    );
    expect(text(3)).toContain([tx("trackTable:fit.noKey"), tx("trackTable:fit.noBpm")].join(sep));
  });

  it("goes back to normal sorting, and the Fit column hides, when any column heading is clicked", async () => {
    await openCrate();
    await select(1);
    await userEvent.click(sortByFit());
    await waitFor(() => expect(headings()).toContain(tx("trackTable:columns.fit")));

    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:columns.artist") }));
    await waitFor(() => expect(headings()).not.toContain(tx("trackTable:columns.fit")));
    expect(lastQuery().source).toEqual({ kind: "crate", id: CRATE });
    expect(lastQuery().sort).toEqual({ column: "artist", descending: false });
    expect(screen.queryByText(tx("trackTable:fit.sortedAfter", { track: "Tune 1" }))).toBeNull();
    // The column headings show the new sort.
    expect(screen.getAllByRole("columnheader").find((h) => h.getAttribute("aria-sort") !== null)).toHaveTextContent(
      tx("trackTable:columns.artist"),
    );
  });

  it("goes back to the crate's own order when the Fit heading itself is clicked", async () => {
    await openCrate();
    await select(1);
    await userEvent.click(sortByFit());
    const fitHeading = await screen.findByRole("button", { name: tx("trackTable:columns.fit") });
    await userEvent.click(fitHeading);
    await waitFor(() => expect(headings()).not.toContain(tx("trackTable:columns.fit")));
    expect(lastQuery().source).toEqual({ kind: "crate", id: CRATE });
    expect(lastQuery().sort).toBeNull();
  });

  it("sorts after another track when the button is used again with another track selected", async () => {
    await openCrate();
    await select(1);
    await userEvent.click(sortByFit());
    await waitFor(() => expect(lastQuery().source).toMatchObject({ after: TRACKS[1].recordingId }));
    // Moving the selection leaves the order as it is.
    const before = asked.length;
    await userEvent.click(drawn()[2]);
    expect(asked.slice(before).every((query) => query.source.kind === "crateByFit")).toBe(true);
    expect(lastQuery().source).toMatchObject({ after: TRACKS[1].recordingId });
    // Using it again sorts after the track selected now.
    await userEvent.click(sortByFit());
    await waitFor(() => expect(lastQuery().source).toMatchObject({ after: TRACKS[2].recordingId }));
  });

  it("keeps the search and the filters when it sorts and when it goes back", async () => {
    serve();
    render(
      <QueryClientProvider client={createQueryClient()}>
        <TrackTableView
          screen="crate"
          source={{ kind: "crate", id: CRATE }}
          keyPrefix={["test", "searched"]}
          search="tune"
          count={(total) => `rows: ${total}`}
          empty={<p>Nothing at all</p>}
        />
      </QueryClientProvider>,
    );
    await screen.findByRole("grid");
    await waitFor(() => expect(drawn()).toHaveLength(TRACKS.length));
    await select(0);
    await userEvent.click(sortByFit());
    await waitFor(() => expect(lastQuery().source.kind).toBe("crateByFit"));
    expect(lastQuery().search).toBe("tune");
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:columns.title") }));
    await waitFor(() => expect(lastQuery().source.kind).toBe("crate"));
    expect(lastQuery().search).toBe("tune");
  });

  it("is a palette command only while a track is selected, and runs the same sort", async () => {
    await openCrate();
    const command = () => listedActions(usePaletteRegistry.getState().sources).find((a) => a.id === "tracks:sortByFit");
    expect(command()).toBeUndefined();
    await select(3);
    await waitFor(() => expect(command()?.label).toBe(tx("trackTable:fit.action")));
    await act(async () => command()?.run());
    await waitFor(() =>
      expect(lastQuery().source).toEqual({ kind: "crateByFit", id: CRATE, after: TRACKS[3].recordingId }),
    );
  });

  it("is not offered on the Library", async () => {
    serve();
    renderCrate("library");
    await screen.findByRole("grid");
    expect(screen.queryByRole("button", { name: tx("trackTable:fit.button") })).toBeNull();
    expect(
      listedActions(usePaletteRegistry.getState().sources).find((a) => a.id === "tracks:sortByFit"),
    ).toBeUndefined();
  });

  it("never makes the Fit column part of the columns the user chose", async () => {
    await openCrate();
    await select(1);
    await userEvent.click(sortByFit());
    await waitFor(() => expect(headings()).toContain(tx("trackTable:columns.fit")));
    await userEvent.click(screen.getByRole("button", { name: tx("trackTable:editor.open") }));
    // The editor lists the columns that can be shown or hidden; Fit isn't one.
    expect(screen.queryByRole("button", { name: tx("trackTable:editor.hide", { column: tx("trackTable:columns.fit") }) })).toBeNull();
    expect(screen.queryByRole("button", { name: tx("trackTable:editor.show", { column: tx("trackTable:columns.fit") }) })).toBeNull();
    expect(JSON.stringify(saved)).not.toContain("fit");
  });
});

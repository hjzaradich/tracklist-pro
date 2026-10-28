import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { AllMusicScreen } from "../allMusic/AllMusicScreen";
import type { Schedule } from "../allMusic/useDebounced";
import { createQueryClient } from "../app/queryClient";
import type { HiddenQuery, HiddenRow } from "../bindings";
import "../i18n";
import { pageOf, tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";

/** Typing has "stopped" at once. */
const AT_ONCE: Schedule = (run) => {
  run();
  return () => {};
};

function hiddenRow(id: number, fields: Partial<HiddenRow> = {}): HiddenRow {
  return {
    fileId: id,
    recordingId: id + 100,
    why: "userHid",
    wayBack: "showAgain",
    track: `Synthetic Tune ${id}`,
    artist: `Made Up Artist ${id}`,
    file: `tune ${id}.mp3`,
    bestFile: null,
    bestFileChosen: false,
    trackInLibrary: false,
    ...fields,
  };
}

/**
 * Stands in for the Rust side: All music holds one track, and `hidden` are
 * the hidden files (a search keeps those whose track holds the text);
 * "Show again" takes a file off the list; `matching` is what the count
 * under a search says.
 */
function fakeBackend(hidden: HiddenRow[], matching = 0) {
  const backend = {
    hidden,
    pages: [] as HiddenQuery[],
    shown: [] as number[],
    chosen: [] as number[],
    failWith: null as { kind: string; params: Record<string, never> } | null,
    pending: false,
    behind: false,
    counted: [] as string[],
  };
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      if (cmd === "key_notation") return "camelot";
      if (cmd === "table_columns") return null;
      if (cmd === "track_table_page") {
        return pageOf([tableRow(1)], (a as { query: Parameters<typeof pageOf>[1] }).query);
      }
      if (cmd === "hidden_files_matching") {
        backend.counted.push(a.text as string);
        return { count: matching, behind: false };
      }
      if (cmd === "hidden_files_page") {
        const query = a.query as HiddenQuery;
        backend.pages.push(query);
        if (backend.pending) return new Promise(() => {});
        const search = (query.search ?? "").toLowerCase();
        const kept = backend.hidden.filter((r) => r.track.toLowerCase().includes(search));
        return {
          total: query.withTotal ? kept.length : null,
          offset: query.offset,
          rows: kept.slice(query.offset, query.offset + query.limit),
          behind: backend.behind,
        };
      }
      if (cmd === "use_this_file_instead") {
        if (backend.failWith) throw backend.failWith;
        backend.chosen.push(a.fileId as number);
        return { operationId: 7 };
      }
      if (cmd === "show_file_again") {
        if (backend.failWith) throw backend.failWith;
        const id = a.fileId as number;
        backend.shown.push(id);
        backend.hidden = backend.hidden.filter((r) => r.fileId !== id);
        return { operationId: 5 };
      }
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return backend;
}

function renderScreen() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <AllMusicScreen schedule={AT_ONCE} />
    </QueryClientProvider>,
  );
}

const hiddenSwitch = () => screen.findByRole("button", { name: tx("hiddenFiles:filter") });

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the Hidden filter in All music", () => {
  it("lists each hidden file with its reason, and a way back only where there is one", async () => {
    fakeBackend([
      hiddenRow(1),
      hiddenRow(2, {
        why: "extraFile",
        wayBack: null,
        track: "Synthetic Tune 2",
        file: "tune 2 (copy).mp3",
        bestFile: "tune 2.flac",
      }),
      hiddenRow(3, { why: "clearedInbox", wayBack: null }),
    ]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());

    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    const [user, extra, cleared] = within(table).getAllByRole("row").slice(1);
    expect(screen.getByText(tx("hiddenFiles:count", { count: 3 }))).toBeInTheDocument();

    expect(user).toHaveTextContent("Synthetic Tune 1");
    expect(user).toHaveTextContent("tune 1.mp3");
    expect(user).toHaveTextContent(tx("hiddenFiles:reason.userHid"));
    expect(within(user).getByRole("button", { name: tx("hiddenFiles:showAgain") })).toBeEnabled();

    // An extra copy names its track and the file chosen as the best.
    expect(extra).toHaveTextContent(
      tx("hiddenFiles:reason.extraFileNamed", { track: "Synthetic Tune 2", best: "tune 2.flac" }),
    );
    expect(within(extra).queryByRole("button")).toBeNull();

    // A cleared Inbox file has no way back until the Inbox exists.
    expect(cleared).toHaveTextContent(tx("hiddenFiles:reason.clearedInbox"));
    expect(within(cleared).queryByRole("button")).toBeNull();
  });

  it("shows a file again when asked, and the file leaves the list", async () => {
    const backend = fakeBackend([hiddenRow(1), hiddenRow(2)]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    const [, second] = within(table).getAllByRole("row").slice(1);

    await userEvent.click(within(second!).getByRole("button", { name: tx("hiddenFiles:showAgain") }));

    await waitFor(() => expect(backend.shown).toEqual([2]));
    await waitFor(() => expect(screen.queryByText("tune 2.mp3")).toBeNull());
    expect(screen.getByText("tune 1.mp3")).toBeInTheDocument();
  });

  it("names what's absent when nothing is hidden", async () => {
    fakeBackend([]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    expect(await screen.findByText(tx("hiddenFiles:empty"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).toBeNull();
  });

  it("says no hidden file matches when the search leaves none", async () => {
    fakeBackend([hiddenRow(1)]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    await userEvent.type(screen.getByRole("searchbox"), "nothing like it");
    expect(await screen.findByText(tx("hiddenFiles:noMatch"))).toBeInTheDocument();
  });

  it("asks for the list a window at a time and shows more on request", async () => {
    const many = Array.from({ length: 205 }, (_, i) => hiddenRow(i + 1));
    const backend = fakeBackend(many);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    expect(within(table).getAllByRole("row")).toHaveLength(1 + 200);
    expect(screen.getByText(tx("hiddenFiles:count", { count: 205 }))).toBeInTheDocument();
    // Only the first window was asked for, and it carried the count.
    expect(backend.pages.map((p) => [p.offset, p.withTotal])).toEqual([[0, true]]);

    await userEvent.click(screen.getByRole("button", { name: tx("hiddenFiles:showMore") }));
    await waitFor(() => expect(within(table).getAllByRole("row")).toHaveLength(1 + 205));
    expect(backend.pages.map((p) => [p.offset, p.withTotal])).toEqual([
      [0, true],
      [200, false],
    ]);
    expect(screen.queryByRole("button", { name: tx("hiddenFiles:showMore") })).toBeNull();
  });

  it("offers Use this file instead on an extra copy outside the Library, and only there", async () => {
    const backend = fakeBackend([
      hiddenRow(1, {
        why: "extraFile",
        wayBack: "useThisFile",
        track: "Synthetic Tune 1",
        file: "tune 1 (copy).mp3",
        bestFile: "tune 1.flac",
      }),
      hiddenRow(2, {
        why: "extraFile",
        wayBack: "useThisFile",
        track: "Synthetic Tune 2",
        file: "tune 2.flac",
        bestFile: "tune 2 (copy).mp3",
        bestFileChosen: true,
      }),
      hiddenRow(3, {
        why: "extraFile",
        wayBack: null,
        track: "Synthetic Tune 3",
        file: "tune 3 (copy).mp3",
        bestFile: "tune 3.flac",
        trackInLibrary: true,
      }),
    ]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    const [outside, chosen, inLibrary] = within(table).getAllByRole("row").slice(1);

    expect(outside).toHaveTextContent(
      tx("hiddenFiles:reason.extraFileNamed", { track: "Synthetic Tune 1", best: "tune 1.flac" }),
    );
    expect(chosen).toHaveTextContent(
      tx("hiddenFiles:reason.extraFileChosen", {
        track: "Synthetic Tune 2",
        best: "tune 2 (copy).mp3",
      }),
    );
    // A Library track's extra copy has the reason and no button.
    expect(inLibrary).toHaveTextContent(
      tx("hiddenFiles:reason.extraFileInLibrary", { track: "Synthetic Tune 3", best: "tune 3.flac" }),
    );
    expect(within(inLibrary!).queryByRole("button")).toBeNull();

    await userEvent.click(
      within(outside!).getByRole("button", { name: tx("hiddenFiles:useThisFile") }),
    );
    await waitFor(() => expect(backend.chosen).toEqual([1]));
  });

  it("says why Use this file instead was refused", async () => {
    const backend = fakeBackend([
      hiddenRow(1, { why: "extraFile", wayBack: "useThisFile", bestFile: "tune 1.flac" }),
    ]);
    backend.failWith = { kind: "hiddenChoiceInLibrary", params: {} };
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    await userEvent.click(await screen.findByRole("button", { name: tx("hiddenFiles:useThisFile") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("hiddenFiles:error.inLibrary"));
  });

  it("says why Show again failed, and keeps the file listed", async () => {
    const backend = fakeBackend([hiddenRow(1)]);
    backend.failWith = { kind: "busy", params: {} };
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    await userEvent.click(await screen.findByRole("button", { name: tx("hiddenFiles:showAgain") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:busy"));
    expect(screen.getByText("tune 1.mp3")).toBeInTheDocument();
  });

  it("sorts by a column heading, and reverses it on a second click", async () => {
    const backend = fakeBackend([hiddenRow(1), hiddenRow(2)]);
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    const artist = within(table).getByRole("columnheader", { name: tx("trackTable:columns.artist") });
    expect(artist).not.toHaveAttribute("aria-sort");

    await userEvent.click(within(artist).getByRole("button"));
    await waitFor(() => expect(artist).toHaveAttribute("aria-sort", "ascending"));
    expect(backend.pages.at(-1)?.sort).toEqual({ column: "artist", descending: false });

    await userEvent.click(within(artist).getByRole("button"));
    await waitFor(() => expect(artist).toHaveAttribute("aria-sort", "descending"));
    expect(backend.pages.at(-1)?.sort).toEqual({ column: "artist", descending: true });
  });

  it("says the list is updating while it loads and while the index is behind", async () => {
    const loading = fakeBackend([hiddenRow(1)]);
    loading.pending = true;
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    expect(await screen.findByText(tx("trackTable:updating"))).toBeInTheDocument();
    expect(screen.queryByText(tx("hiddenFiles:empty"))).toBeNull();
    cleanup();
    clearMocks();

    // Nothing listed yet, because the index hasn't caught up: not "No hidden files".
    const behind = fakeBackend([]);
    behind.behind = true;
    renderScreen();
    await userEvent.click(await hiddenSwitch());
    expect(await screen.findByText(tx("trackTable:updating"))).toBeInTheDocument();
    expect(screen.queryByText(tx("hiddenFiles:empty"))).toBeNull();
  });
});

describe("the hidden files that match a search", () => {
  it("says how many hidden files match, and Show them opens the Hidden filter with the same search", async () => {
    const backend = fakeBackend([hiddenRow(1), hiddenRow(2)], 2);
    renderScreen();
    await screen.findByRole("grid");
    await userEvent.type(screen.getByRole("searchbox"), "tune 2");

    const line = await screen.findByRole("status");
    expect(line).toHaveTextContent(tx("hiddenFiles:matches", { count: 2 }));
    expect(backend.counted.at(-1)).toBe("tune 2");

    await userEvent.click(within(line).getByRole("button", { name: tx("hiddenFiles:showThem") }));

    const table = await screen.findByRole("table", { name: tx("hiddenFiles:label") });
    // The same search, applied to the hidden files.
    expect(backend.pages.at(-1)?.search).toBe("tune 2");
    expect(within(table).getAllByRole("row")).toHaveLength(1 + 1);
    expect(within(table).getByText("tune 2.mp3")).toBeInTheDocument();
    // The line is for the track list: the filter is open now.
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("says one hidden file matches in the singular", async () => {
    fakeBackend([hiddenRow(1)], 1);
    renderScreen();
    await screen.findByRole("grid");
    await userEvent.type(screen.getByRole("searchbox"), "tune");
    expect(await screen.findByRole("status")).toHaveTextContent(tx("hiddenFiles:matches", { count: 1 }));
  });

  it("says nothing when no hidden file matches, or when nothing is searched", async () => {
    const backend = fakeBackend([hiddenRow(1)], 0);
    renderScreen();
    await screen.findByRole("grid");
    expect(screen.queryByRole("status")).toBeNull();
    // Nothing searched: nothing is counted.
    expect(backend.counted).toEqual([]);
    await userEvent.type(screen.getByRole("searchbox"), "tune");
    await waitFor(() => expect(backend.counted.at(-1)).toBe("tune"));
    expect(screen.queryByRole("status")).toBeNull();
  });
});

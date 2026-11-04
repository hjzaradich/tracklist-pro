import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { MissingGroup, MissingList as Missing } from "../bindings";
import "../i18n";
import { MissingList as MissingListView } from "./MissingList";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { tx } from "../test/tx";

function group(folder: string | null, fields: Partial<MissingGroup> = {}): MissingGroup {
  const tracks = fields.tracks ?? [
    {
      id: 1,
      title: "Song",
      artist: "Artist",
      lastKnownPath: folder === null ? null : String.raw`${folder}\song.mp3`,
    },
  ];
  return { folder, canAdd: false, trackIds: tracks.map((track) => track.id), ...fields, tracks };
}

function list(groups: MissingGroup[]): Missing {
  return { total: groups.reduce((n, g) => n + g.trackIds.length, 0), groups };
}

/** `n` tracks of one group, ids from `from`. */
function songs(from: number, n: number) {
  return Array.from({ length: n }, (_, i) => ({
    id: from + i,
    title: `Song ${from + i}`,
    artist: "",
    lastKnownPath: null,
  }));
}

function queryClient() {
  return new QueryClient({ defaultOptions: { mutations: { retry: false } } });
}

const client = queryClient();
const invoke = vi.fn();

/** The list inside the query client its "Show more" needs. */
function MissingList(props: React.ComponentProps<typeof MissingListView>) {
  return (
    <QueryClientProvider client={client}>
      <MissingListView {...props} />
    </QueryClientProvider>
  );
}

beforeEach(() => {
  invoke.mockReset();
  mockIPC((cmd, args) => invoke(cmd, args) as unknown);
});

afterEach(() => {
  cleanup();
  clearMocks();
});

/** The line with a folder's name, its count and its button (the summary line also counts). */
function folderRow(folder: string): HTMLElement {
  return screen.getByText(folder).parentElement as HTMLElement;
}

describe("the Missing list", () => {
  it("names what's absent when no track is missing", () => {
    render(<MissingList list={list([])} />);
    expect(screen.getByText(tx("review:missing.empty"))).toBeInTheDocument();
  });

  it("says what's still being worked out instead of claiming no track is missing", () => {
    render(<MissingList list={list([])} waiting={tx("activity:task.relink")} />);
    expect(screen.getByText(tx("activity:task.relink"))).toBeInTheDocument();
    expect(screen.queryByText(tx("review:missing.empty"))).toBeNull();
  });

  it("shows each folder with its track count and the tracks' last known paths", () => {
    render(<MissingList list={list([group(String.raw`D:\Old`)])} />);
    expect(screen.getByText(String.raw`D:\Old`)).toBeInTheDocument();
    expect(within(folderRow(String.raw`D:\Old`)).getByText(tx("review:missing.tracksCount", { count: 1 }))).toBeInTheDocument();
    expect(screen.getByText(tx("review:missing.trackName", { artist: "Artist", title: "Song" }))).toBeInTheDocument();
    expect(screen.getByText(String.raw`D:\Old\song.mp3`)).toBeInTheDocument();
  });

  it("calls a group with no known folder the unknown folder", () => {
    render(<MissingList list={list([group(null)])} />);
    expect(screen.getByText(tx("review:missing.unknownFolder"))).toBeInTheDocument();
  });

  it("offers to add only a folder the list says can be added", async () => {
    const onAddFolder = vi.fn();
    render(
      <MissingList
        list={list([group(String.raw`D:\Here`, { canAdd: true }), group(String.raw`D:\Gone`)])}
        onAddFolder={onAddFolder}
      />,
    );
    const buttons = screen.getAllByRole("button", { name: tx("review:missing.addFolder") });
    expect(buttons).toHaveLength(1);
    await userEvent.click(buttons[0]);
    expect(onAddFolder).toHaveBeenCalledWith(String.raw`D:\Here`);
  });

  it("counts several tracks in a folder in the plural", () => {
    const many = group(String.raw`D:\Old`, {
      tracks: [1, 2, 3].map((id) => ({
        id,
        title: `Song ${id}`,
        artist: "",
        lastKnownPath: null,
      })),
    });
    render(<MissingList list={list([many])} />);
    expect(within(folderRow(String.raw`D:\Old`)).getByText(tx("review:missing.tracksCount", { count: 3 }))).toBeInTheDocument();
  });

  it("shows only the heading while the list loads", () => {
    render(<MissingList list={undefined} />);
    expect(screen.getByRole("heading", { name: tx("review:missing.title") })).toBeInTheDocument();
    expect(screen.queryByText(tx("review:missing.empty"))).toBeNull();
  });

  it("says so when the list or an add failed, instead of looking empty", () => {
    render(<MissingList list={undefined} problem={tx("errors:internal")} />);
    expect(screen.getByRole("alert")).toHaveTextContent(tx("errors:internal"));
  });

  it("holds the add buttons while an add runs", () => {
    render(
      <MissingList
        list={list([group(String.raw`D:\Here`, { canAdd: true })])}
        adding
        onAddFolder={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: tx("review:missing.addFolder") })).toBeDisabled();
  });

  it("is one line, the heading and how many tracks, until it is opened", () => {
    const { container } = render(<MissingList list={list([group(String.raw`D:\Old`), group(String.raw`D:\Older`)])} />);
    const details = container.querySelector("details") as HTMLDetailsElement;
    expect(details.open).toBe(false);
    const summary = details.querySelector("summary") as HTMLElement;
    expect(within(summary).getByRole("heading", { name: tx("review:missing.title") })).toBeInTheDocument();
    expect(within(summary).getByText(tx("review:missing.tracksCount", { count: 2 }))).toBeInTheDocument();
  });

  it("is open when nothing is missing, and when something went wrong", () => {
    const { container, rerender } = render(<MissingList list={list([])} />);
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(true);
    rerender(<MissingList list={list([group(String.raw`D:\Old`)])} problem="It failed" />);
    expect((container.querySelector("details") as HTMLDetailsElement).open).toBe(true);
    expect(screen.getByRole("alert")).toHaveTextContent("It failed");
  });

  describe("with a group too big to load at once", () => {
    const folder = String.raw`D:\Big`;
    const big = () =>
      group(folder, {
        canAdd: true,
        tracks: songs(1, 100),
        trackIds: Array.from({ length: 250 }, (_, i) => i + 1),
      });

    it("counts every track in the group, though only the first page is drawn", () => {
      render(<MissingList list={list([big()])} />);
      expect(within(folderRow(folder)).getByText(tx("review:missing.tracksCount", { count: 250 }))).toBeInTheDocument();
      expect(screen.getAllByRole("listitem")).toHaveLength(100);
    });

    it("still adds the folder, which is about the whole group and not the rows loaded", async () => {
      const onAddFolder = vi.fn();
      render(<MissingList list={list([big()])} onAddFolder={onAddFolder} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.addFolder") }));
      expect(onAddFolder).toHaveBeenCalledOnce();
      expect(onAddFolder).toHaveBeenCalledWith(folder);
    });

    it("loads the next page of ids, in order, when asked to show more", async () => {
      invoke.mockImplementation((_cmd, args: { ids: number[] }) => args.ids.map((id) => songs(id, 1)[0]));
      render(<MissingList list={list([big()])} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(200));
      expect(invoke).toHaveBeenCalledWith("missing_track_page", { ids: Array.from({ length: 100 }, (_, i) => i + 101) });
    });

    it("moves focus to the first new row and announces the page", async () => {
      invoke.mockImplementation((_cmd, args: { ids: number[] }) => args.ids.map((id) => songs(id, 1)[0]));
      render(<MissingList list={list([big()])} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(200));
      expect(screen.getAllByRole("listitem")[100]).toHaveFocus();
      expect(screen.getByRole("status")).toHaveTextContent(tx("review:missing.moreShown", { count: 100 }));
    });

    it("keeps each row in the group's order when the list refetches with a row that comes back", async () => {
      invoke.mockImplementation((_cmd, args: { ids: number[] }) => args.ids.map((id) => songs(id, 1)[0]));
      const ids = Array.from({ length: 250 }, (_, i) => i + 1);
      const { rerender } = render(<MissingList list={list([big()])} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(200));
      // Track 999 sorts first: the list's own rows shift by one, so row 100 is no longer among them.
      const trackIds = [999, ...ids];
      rerender(<MissingList list={list([group(folder, { canAdd: true, tracks: [songs(999, 1)[0], ...songs(1, 99)], trackIds })])} />);
      const titles = screen.getAllByRole("listitem").map((li) => li.textContent);
      expect(titles.slice(0, 3)).toEqual(["Song 999", "Song 1", "Song 2"]);
      expect(titles).toHaveLength(200);
      // Row 100 fell out of the list's first rows; when it comes back it takes its own place.
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(251));
      const after = screen.getAllByRole("listitem").map((li) => li.textContent);
      expect(after.slice(99, 102)).toEqual(["Song 99", "Song 100", "Song 101"]);
    });

    it("stops offering more once every track is shown, and keeps the count", async () => {
      invoke.mockImplementation((_cmd, args: { ids: number[] }) => args.ids.map((id) => songs(id, 1)[0]));
      render(<MissingList list={list([big()])} />);
      for (let page = 0; page < 2; page++) {
        await userEvent.click(await screen.findByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
        await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(200 + page * 50));
      }
      expect(screen.queryByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) })).toBeNull();
      expect(within(folderRow(folder)).getByText(tx("review:missing.tracksCount", { count: 250 }))).toBeInTheDocument();
    });

    it("does not ask again for tracks the last page left out because they were matched since", async () => {
      invoke.mockImplementation((_cmd, args: { ids: number[] }) => args.ids.slice(1).map((id) => songs(id, 1)[0]));
      render(<MissingList list={list([big()])} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      await waitFor(() => expect(screen.getAllByRole("listitem")).toHaveLength(199));
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      expect(invoke).toHaveBeenLastCalledWith("missing_track_page", { ids: Array.from({ length: 100 }, (_, i) => i + 201).slice(0, 50) });
    });

    it("says so when a page fails to load, and offers it again", async () => {
      invoke.mockImplementation(() => {
        throw { kind: "internal", params: {} };
      });
      render(<MissingList list={list([big()])} />);
      await userEvent.click(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) }));
      expect(await screen.findByRole("alert")).toHaveTextContent(tx("errors:internal"));
      expect(screen.getByRole("button", { name: tx("review:missing.showMoreIn", { group: folder }) })).toBeEnabled();
    });
  });
});

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, within } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it } from "vitest";
import { XML_SOURCE_QUERY_KEY } from "../rekordbox/useXmlSource";
import { SEND_STATE_QUERY_KEY } from "../send/useSend";
import { createQueryClient } from "../app/queryClient";
import type { AfterSendLists as Lists, ManualRemoval, StalePlaylist } from "../bindings";
import "../i18n";
import { AfterSendLists, AfterSendPanel } from "./AfterSendLists";
import { tx } from "../test/tx";

function lists(fields: Partial<Lists> = {}): Lists {
  return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [], ...fields };
}

function stale(path: string[], fields: Partial<StalePlaylist> = {}): StalePlaylist {
  return { path, kind: "playlist", playlistsInside: 0, empty: false, ...fields };
}

function removal(fields: Partial<ManualRemoval> = {}): ManualRemoval {
  return {
    recordingId: 1,
    title: "Song",
    artist: "Artist",
    path: String.raw`E:\Music\song.mp3`,
    removedAt: "2026-10-01T10:00:00.000Z",
    ...fields,
  };
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the after-send lists", () => {
  it("names what's absent in each list when there's nothing to do", () => {
    render(<AfterSendLists lists={lists()} />);
    expect(screen.getByText(tx("afterSend:stale.empty"))).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:removals.empty"))).toBeInTheDocument();
  });

  it("says the playlists weren't checked, instead of looking empty, before rekordbox is read", () => {
    render(<AfterSendLists lists={lists({ playlistsChecked: false })} />);
    expect(screen.getByText(tx("afterSend:stale.unchecked"))).toBeInTheDocument();
    expect(screen.queryByText(tx("afterSend:stale.empty"))).toBeNull();
    // The other list doesn't depend on it.
    expect(screen.getByText(tx("afterSend:removals.empty"))).toBeInTheDocument();
  });

  it("shows only the headings while the lists load", () => {
    render(<AfterSendLists lists={undefined} />);
    expect(screen.getByRole("heading", { name: tx("afterSend:stale.title") })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: tx("afterSend:removals.title") })).toBeInTheDocument();
    expect(screen.queryByText(tx("afterSend:stale.empty"))).toBeNull();
    expect(screen.queryByText(tx("afterSend:removals.empty"))).toBeNull();
  });

  it("lists each stale playlist by where it sits in rekordbox", () => {
    render(
      <AfterSendLists
        lists={lists({ stalePlaylists: [stale(["Crates", "House", "Old name"])] })}
      />,
    );
    const section = screen.getByRole("region", { name: tx("afterSend:stale.title") });
    expect(within(section).getByText("Crates > House > Old name")).toBeInTheDocument();
    expect(within(section).queryByText(tx("afterSend:stale.empty"))).toBeNull();
  });

  it("says how many playlists a stale folder holds, and when it's empty", () => {
    render(
      <AfterSendLists
        lists={lists({
          stalePlaylists: [
            stale(["Crates", "One"], { kind: "folder", playlistsInside: 1 }),
            stale(["Crates", "Many"], { kind: "folder", playlistsInside: 3 }),
            stale(["Crates", "Hollow"], { kind: "folder", playlistsInside: 0, empty: true }),
            stale(["Crates", "Subfolders"], { kind: "folder", playlistsInside: 0 }),
          ],
        })}
      />,
    );
    expect(screen.getByText(tx("afterSend:stale.folderInside", { count: 1 }))).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:stale.folderInside", { count: 3 }))).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:stale.emptyFolder"))).toBeInTheDocument();
    // A folder holding only subfolders isn't empty.
    expect(screen.getByText(tx("afterSend:stale.folderNoPlaylists"))).toBeInTheDocument();
  });

  it("lists each removed track with where it was sent", () => {
    render(<AfterSendLists lists={lists({ manualRemovals: [removal()] })} />);
    const section = screen.getByRole("region", { name: tx("afterSend:removals.title") });
    expect(within(section).getByText(tx("afterSend:removals.trackName", { artist: "Artist", title: "Song" }))).toBeInTheDocument();
    expect(within(section).getByText(String.raw`E:\Music\song.mp3`)).toBeInTheDocument();
    expect(within(section).queryByText(tx("afterSend:removals.empty"))).toBeNull();
  });

  it("falls back to the file name, then to a plain label, for a track with no title", () => {
    render(
      <AfterSendLists
        lists={lists({
          manualRemovals: [
            removal({ recordingId: 1, title: null, artist: null }),
            removal({ recordingId: 2, title: null, artist: null, path: null }),
            removal({ recordingId: 3, title: "Solo", artist: null }),
          ],
        })}
      />,
    );
    expect(screen.getByText("song.mp3")).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:removals.unnamed"))).toBeInTheDocument();
    expect(screen.getByText("Solo")).toBeInTheDocument();
  });

  it("says so when the lists couldn't be loaded, instead of looking empty", () => {
    render(<AfterSendLists lists={undefined} problem={tx("errors:internal")} />);
    expect(screen.getByRole("alert")).toHaveTextContent(tx("errors:internal"));
  });
});

describe("the after-send panel", () => {
  it("asks the backend for both lists and shows them", async () => {
    mockIPC((cmd) => {
      if (cmd === "after_send_lists") {
        return lists({
          stalePlaylists: [stale(["Playlists", "Old"])],
          manualRemovals: [removal()],
        });
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    render(
      <QueryClientProvider client={createQueryClient()}>
        <AfterSendPanel />
      </QueryClientProvider>,
    );
    expect(await screen.findByText("Playlists > Old")).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:removals.trackName", { artist: "Artist", title: "Song" }))).toBeInTheDocument();
  });
});

describe("the after-send panel stays current", () => {
  function setup() {
    let answer: Lists = lists({ playlistsChecked: false });
    mockIPC((cmd) => {
      if (cmd === "after_send_lists") {
        return answer;
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    const queryClient = createQueryClient();
    render(
      <QueryClientProvider client={queryClient}>
        <AfterSendPanel />
      </QueryClientProvider>,
    );
    return {
      queryClient,
      answer: (next: Lists) => {
        answer = next;
      },
    };
  }

  it("asks again when the send's state changes, such as after a read or a write", async () => {
    const panel = setup();
    expect(await screen.findByText(tx("afterSend:stale.unchecked"))).toBeInTheDocument();
    // A read finishes: the send's revision moves, and rekordbox is now read.
    panel.answer(lists({ stalePlaylists: [stale(["Crates", "Old name"])] }));
    act(() => {
      panel.queryClient.setQueryData(SEND_STATE_QUERY_KEY, { revision: 1 });
    });
    expect(await screen.findByText("Crates > Old name")).toBeInTheDocument();
    expect(screen.queryByText(tx("afterSend:stale.unchecked"))).toBeNull();

    // The file is written: the revision moves again.
    panel.answer(lists());
    act(() => {
      panel.queryClient.setQueryData(SEND_STATE_QUERY_KEY, { revision: 2 });
    });
    expect(await screen.findByText(tx("afterSend:stale.empty"))).toBeInTheDocument();
  });

  it("asks again when the rekordbox export is read again", async () => {
    const panel = setup();
    await screen.findByText(tx("afterSend:stale.unchecked"));
    panel.answer(lists());
    act(() => {
      panel.queryClient.setQueryData(XML_SOURCE_QUERY_KEY, {
        lastRead: { readAt: "2026-10-01T10:00:00.000Z" },
      });
    });
    expect(await screen.findByText(tx("afterSend:stale.empty"))).toBeInTheDocument();
  });
});

import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { AfterSendLists as Lists, ManualRemoval, StalePlaylist } from "../bindings";
import "../i18n";
import { AfterSendLists, AfterSendPanel } from "./AfterSendLists";

function lists(fields: Partial<Lists> = {}): Lists {
  return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [], ...fields };
}

function stale(path: string[], fields: Partial<StalePlaylist> = {}): StalePlaylist {
  return { path, kind: "playlist", playlistsInside: 0, ...fields };
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
    expect(screen.getByText("No playlists to delete")).toBeInTheDocument();
    expect(screen.getByText("No tracks to remove")).toBeInTheDocument();
  });

  it("says the playlists weren't checked, instead of looking empty, before rekordbox is read", () => {
    render(<AfterSendLists lists={lists({ playlistsChecked: false })} />);
    expect(screen.getByText("Not checked yet (rekordbox hasn't been read)")).toBeInTheDocument();
    expect(screen.queryByText("No playlists to delete")).toBeNull();
    // The other list doesn't depend on it.
    expect(screen.getByText("No tracks to remove")).toBeInTheDocument();
  });

  it("shows only the headings while the lists load", () => {
    render(<AfterSendLists lists={undefined} />);
    expect(screen.getByRole("heading", { name: "Playlists to delete in rekordbox" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Tracks to remove in rekordbox" })).toBeInTheDocument();
    expect(screen.queryByText("No playlists to delete")).toBeNull();
    expect(screen.queryByText("No tracks to remove")).toBeNull();
  });

  it("lists each stale playlist by where it sits in rekordbox", () => {
    render(
      <AfterSendLists
        lists={lists({ stalePlaylists: [stale(["Crates", "House", "Old name"])] })}
      />,
    );
    const section = screen.getByRole("region", { name: "Playlists to delete in rekordbox" });
    expect(within(section).getByText("Crates > House > Old name")).toBeInTheDocument();
    expect(within(section).queryByText("No playlists to delete")).toBeNull();
  });

  it("says how many playlists a stale folder holds, and when it's empty", () => {
    render(
      <AfterSendLists
        lists={lists({
          stalePlaylists: [
            stale(["Crates", "One"], { kind: "folder", playlistsInside: 1 }),
            stale(["Crates", "Many"], { kind: "folder", playlistsInside: 3 }),
            stale(["Crates", "Hollow"], { kind: "folder", playlistsInside: 0 }),
          ],
        })}
      />,
    );
    expect(screen.getByText("(folder with 1 playlist)")).toBeInTheDocument();
    expect(screen.getByText("(folder with 3 playlists)")).toBeInTheDocument();
    expect(screen.getByText("(empty folder)")).toBeInTheDocument();
  });

  it("lists each removed track with where it was sent", () => {
    render(<AfterSendLists lists={lists({ manualRemovals: [removal()] })} />);
    const section = screen.getByRole("region", { name: "Tracks to remove in rekordbox" });
    expect(within(section).getByText("Artist - Song")).toBeInTheDocument();
    expect(within(section).getByText(String.raw`E:\Music\song.mp3`)).toBeInTheDocument();
    expect(within(section).queryByText("No tracks to remove")).toBeNull();
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
    expect(screen.getByText("Unnamed track")).toBeInTheDocument();
    expect(screen.getByText("Solo")).toBeInTheDocument();
  });

  it("says so when the lists couldn't be loaded, instead of looking empty", () => {
    render(<AfterSendLists lists={undefined} problem="Something went wrong." />);
    expect(screen.getByRole("alert")).toHaveTextContent("Something went wrong.");
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
    expect(screen.getByText("Artist - Song")).toBeInTheDocument();
  });
});

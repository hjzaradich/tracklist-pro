import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { useRemoveLibraryTrack } from "../library/useRemoveLibraryTrack";
import { libraryRows } from "../test/crateBackend";
import { playlistBackend } from "../test/playlistBackend";
import { PLAYLISTS_QUERY_KEY, usePlaylists } from "./usePlaylists";

// Removing a Library track takes its entries out of every playlist (the
// backend does it in the same operation), so what the playlists show has to
// be asked for again.

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("removing a track from the Library", () => {
  it("brings the playlist counts and an open playlist's table up to date", async () => {
    playlistBackend({
      rows: libraryRows(3),
      playlists: [
        {
          id: 1,
          name: "Arc",
          origin: "app",
          sourcePath: null,
          entries: [
            [1, 100],
            [2, 101],
            [3, 100],
          ],
        },
      ],
    });
    const queryClient = createQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    );
    const { result } = renderHook(() => ({ list: usePlaylists(), remove: useRemoveLibraryTrack() }), { wrapper });
    await waitFor(() => expect(result.current.list.data?.[0]?.trackCount).toBe(3));
    // An open playlist's table windows live under the playlist's own key.
    const table = [...PLAYLISTS_QUERY_KEY, "table", 1, "window"];
    queryClient.setQueryData(table, { rows: [] });

    // Library track 1 is recording 100: it sits in the playlist twice.
    await act(async () => {
      await result.current.remove.mutateAsync(1);
    });
    await waitFor(() => expect(result.current.list.data?.[0]?.trackCount).toBe(1));
    expect(queryClient.getQueryState(table)?.isInvalidated).toBe(true);
  });
});

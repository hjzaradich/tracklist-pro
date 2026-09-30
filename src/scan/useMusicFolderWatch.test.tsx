import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { MusicFolder } from "../bindings";
import { useSetMusicFolderWatch } from "./useMusicFolderWatch";
import { MUSIC_FOLDERS_QUERY_KEY, useMusicFolders } from "./useMusicFolders";

function folder(id: number, watch: boolean): MusicFolder {
  return {
    id,
    role: "scan",
    watch,
    path: String.raw`E:\Music ` + id,
    online: true,
    volumeLabel: "",
    addedAt: "2026-09-29T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("turning a music folder's watcher on", () => {
  it("stores the choice and shows it on the folder without another load", async () => {
    const calls: [string, unknown][] = [];
    mockIPC((cmd, args) => {
      calls.push([cmd, args]);
      if (cmd === "music_folders") return [folder(1, false), folder(2, false)];
      if (cmd === "set_music_folder_watch") return null;
      throw new Error(`unexpected command ${cmd}`);
    });
    const client = createQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
    const { result } = renderHook(
      () => ({ folders: useMusicFolders(), set: useSetMusicFolderWatch() }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.folders.data).toHaveLength(2));

    await act(() => result.current.set.mutateAsync({ id: 2, watch: true }));
    expect(calls).toContainEqual(["set_music_folder_watch", { id: 2, watch: true }]);
    const shown = client.getQueryData<MusicFolder[]>(MUSIC_FOLDERS_QUERY_KEY);
    expect(shown?.map((f) => f.watch)).toEqual([false, true]);
    // One load: the answer was applied in place.
    expect(calls.filter(([cmd]) => cmd === "music_folders")).toHaveLength(1);
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { JobUpdate, MusicFolder } from "../bindings";
import { useMusicFolders } from "./useMusicFolders";

function folder(fields: Partial<MusicFolder> = {}): MusicFolder {
  return {
    id: 1,
    role: "scan",
    watch: false,
    path: String.raw`E:\Music`,
    online: true,
    volumeLabel: "",
    addedAt: "2026-09-29T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
    ...fields,
  };
}

function scanJob(status: JobUpdate["status"]): JobUpdate {
  return { seq: 1, id: 7, kind: "scan", status, progress: null, priority: 10 };
}

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={createQueryClient()}>{children}</QueryClientProvider>;
}

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the music folders query", () => {
  it("reloads the folders when a scan finishes, not while it runs", async () => {
    let answer = [folder()];
    let asked = 0;
    mockIPC(
      (cmd) => {
        if (cmd !== "music_folders") throw new Error(`unexpected command ${cmd}`);
        asked += 1;
        return answer;
      },
      { shouldMockEvents: true },
    );
    const { result } = renderHook(() => useMusicFolders(), { wrapper });
    await waitFor(() => expect(result.current.data).toEqual(answer));

    await act(() => emit("job-updates", [scanJob("running")]));
    expect(asked).toBe(1);

    answer = [folder({ walkedAt: "2026-09-29T10:05:00.000Z", unreadableFolders: 2 })];
    await act(() => emit("job-updates", [scanJob("done")]));
    await waitFor(() => expect(result.current.data?.[0].unreadableFolders).toBe(2));
  });

  it("reloads the folders when a drive is plugged in or unplugged", async () => {
    let answer = [folder()];
    mockIPC(
      (cmd) => {
        if (cmd !== "music_folders") throw new Error(`unexpected command ${cmd}`);
        return answer;
      },
      { shouldMockEvents: true },
    );
    const { result } = renderHook(() => useMusicFolders(), { wrapper });
    await waitFor(() => expect(result.current.data?.[0].online).toBe(true));

    answer = [folder({ online: false })];
    await act(() => emit("volumes-changed", null));
    await waitFor(() => expect(result.current.data?.[0].online).toBe(false));
  });
});

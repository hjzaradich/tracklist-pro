import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { MusicFoldersStep } from "./MusicFoldersStep";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

function folder(id: number, path: string): MusicFolder {
  return {
    id,
    role: "scan",
    watch: false,
    path,
    online: true,
    volumeLabel: "",
    addedAt: "2026-10-01T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

/** Stands in for the Rust side: the music folders, and every call made. */
function fakeBackend(initial: MusicFolder[] = [], refuse: unknown = null) {
  const backend = {
    folders: initial,
    calls: [] as { cmd: string; args: Record<string, unknown> }[],
  };
  mockIPC(
    (cmd, args) => {
      const a = (args ?? {}) as Record<string, unknown>;
      backend.calls.push({ cmd, args: a });
      if (cmd === "music_folders") return backend.folders;
      if (cmd === "add_music_folder") {
        if (refuse) throw refuse;
        const added = folder(backend.folders.length + 1, a.path as string);
        backend.folders = [...backend.folders, added];
        return added;
      }
      if (cmd === "scan_music_folders") return 42;
      if (cmd === "set_music_folder_watch") {
        backend.folders = backend.folders.map((f) =>
          f.id === a.id ? { ...f, watch: a.watch as boolean } : f,
        );
        return null;
      }
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return backend;
}

function renderStep() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <MusicFoldersStep />
    </QueryClientProvider>,
  );
}

beforeEach(() => dialog.open.mockReset());
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("picking music folders", () => {
  it("names what's absent when there are no music folders", async () => {
    fakeBackend();
    renderStep();
    expect(await screen.findByText("No music folders")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Music folders" })).toBeInTheDocument();
  });

  it("adds the picked folder, scans it and lists it", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(String.raw`E:\Music`);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: "Add folder" }));

    expect(await screen.findByText(String.raw`E:\Music`)).toBeInTheDocument();
    expect(dialog.open).toHaveBeenCalledWith({ multiple: false, directory: true });
    const made = backend.calls.filter((call) => call.cmd !== "music_folders");
    expect(made).toEqual([
      { cmd: "add_music_folder", args: { path: String.raw`E:\Music`, role: null } },
      { cmd: "scan_music_folders", args: { ids: [1] } },
    ]);
  });

  it("adds nothing when the picker is closed without a folder", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(null);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: "Add folder" }));
    await waitFor(() => expect(dialog.open).toHaveBeenCalled());
    expect(backend.calls.every((call) => call.cmd === "music_folders")).toBe(true);
  });

  it("says why a folder couldn't be added, and scans nothing", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`)], {
      kind: "alreadyAdded",
      params: { path: String.raw`E:\Music` },
    });
    dialog.open.mockResolvedValue(String.raw`E:\Music`);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: "Add folder" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      String.raw`Already a music folder (E:\Music)`,
    );
    expect(backend.calls.some((call) => call.cmd === "scan_music_folders")).toBe(false);
  });

  it("turns a folder's watcher on with the existing switch", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`)]);
    renderStep();
    await userEvent.click(await screen.findByRole("checkbox", { name: "Watch for changes" }));
    await waitFor(() => expect(screen.getByRole("checkbox")).toBeChecked());
    expect(backend.calls.at(-1)).toEqual({
      cmd: "set_music_folder_watch",
      args: { id: 1, watch: true },
    });
  });
});

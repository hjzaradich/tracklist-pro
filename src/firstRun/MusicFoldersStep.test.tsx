import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { MusicFoldersStep } from "./MusicFoldersStep";
import { tx } from "../test/tx";

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
    readOnlineOnly: false,
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
      if (cmd === "read_online_only_files") return backend.readOnlineOnly;
      if (cmd === "set_read_online_only_files") {
        backend.readOnlineOnly = a.on as boolean;
        return null;
      }
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

/** The calls that change or start something: not the ones that only ask. */
function acted(backend: ReturnType<typeof fakeBackend>) {
  return backend.calls.filter(
    (call) => call.cmd !== "music_folders" && call.cmd !== "read_online_only_files",
  );
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
    expect(await screen.findByText(tx("musicFolderStatus:empty"))).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: tx("firstRun:folders.title") })).toBeInTheDocument();
  });

  it("adds the picked folder and lists it, leaving the scan to the add itself", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(String.raw`E:\Music`);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: tx("firstRun:folders.add") }));

    expect(await screen.findByText(String.raw`E:\Music`)).toBeInTheDocument();
    expect(dialog.open).toHaveBeenCalledWith({ multiple: false, directory: true });
    // Adding a folder scans it, whichever screen adds it: no second call.
    expect(acted(backend)).toEqual([
      { cmd: "add_music_folder", args: { path: String.raw`E:\Music`, role: null } },
    ]);
  });

  it("adds nothing when the picker is closed without a folder", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(null);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: tx("firstRun:folders.add") }));
    await waitFor(() => expect(dialog.open).toHaveBeenCalled());
    expect(acted(backend)).toEqual([]);
  });

  it("says why a folder couldn't be added, and scans nothing", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`)], {
      kind: "alreadyAdded",
      params: { path: String.raw`E:\Music` },
    });
    dialog.open.mockResolvedValue(String.raw`E:\Music`);
    renderStep();
    await userEvent.click(await screen.findByRole("button", { name: tx("firstRun:folders.add") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("musicFolders:alreadyAdded", { path: String.raw`E:\Music` }),
    );
    expect(backend.calls.some((call) => call.cmd === "scan_music_folders")).toBe(false);
  });

  it("turns a folder's watcher on with the existing switch", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`)]);
    renderStep();
    await userEvent.click(await screen.findByRole("checkbox", { name: tx("musicFolderWatch:watch") }));
    await waitFor(() =>
      expect(screen.getByRole("checkbox", { name: tx("musicFolderWatch:watch") })).toBeChecked(),
    );
    expect(acted(backend)).toEqual([{ cmd: "set_music_folder_watch", args: { id: 1, watch: true } }]);
  });

  it("scans a folder again when asked", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`), folder(2, String.raw`F:\More`)]);
    renderStep();
    const buttons = await screen.findAllByRole("button", { name: tx("musicFolderStatus:scanAgain") });
    expect(buttons).toHaveLength(2);
    await userEvent.click(buttons[1]);
    await waitFor(() =>
      expect(acted(backend)).toEqual([{ cmd: "scan_music_folders", args: { ids: [2] } }]),
    );
  });

  it("greys out Scan again for a folder whose drive isn't connected, beside the note saying so", async () => {
    fakeBackend([{ ...folder(1, String.raw`E:\Music`), online: false }]);
    renderStep();
    expect(
      await screen.findByRole("button", { name: tx("musicFolderStatus:scanAgain") }),
    ).toBeDisabled();
    expect(screen.getByText(tx("musicFolderStatus:offline"))).toBeInTheDocument();
  });

  it("holds the opt-in to reading online-only files", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\Music`)]);
    renderStep();
    const optIn = await screen.findByRole("checkbox", { name: tx("musicFolderStatus:readOnlineOnly") });
    expect(optIn).not.toBeChecked();
    await userEvent.click(optIn);
    await waitFor(() =>
      expect(acted(backend)).toEqual([{ cmd: "set_read_online_only_files", args: { on: true } }]),
    );
  });
});

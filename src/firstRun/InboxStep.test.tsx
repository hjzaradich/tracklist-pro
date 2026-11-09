import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { MusicFolder } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { InboxStep } from "./InboxStep";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

function folder(id: number, path: string, role: "scan" | "inbox"): MusicFolder {
  return {
    id,
    role,
    watch: false,
    path,
    online: true,
    folderFound: true,
    volumeLabel: "",
    addedAt: "2026-10-01T10:00:00.000Z",
    walkedAt: null,
    unreadableFolders: null,
    unreadableFiles: null,
    onlineOnlyFiles: 0,
  };
}

function fakeBackend(initial: MusicFolder[] = []) {
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
        const added = folder(
          backend.folders.length + 1,
          a.path as string,
          a.role as "inbox",
        );
        backend.folders = [...backend.folders, added];
        return added;
      }
      if (cmd === "set_music_folder_watch") {
        backend.folders = backend.folders.map((f) =>
          f.id === a.id ? { ...f, watch: a.watch as boolean } : f,
        );
        return null;
      }
      if (cmd === "inbox") return null;
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return backend;
}

const acted = (backend: ReturnType<typeof fakeBackend>) =>
  backend.calls.filter(
    (call) => call.cmd !== "music_folders" && call.cmd !== "inbox",
  );

function renderStep() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <InboxStep />
    </QueryClientProvider>,
  );
}

beforeEach(() => dialog.open.mockReset());
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the first-run Inbox step", () => {
  it("adds the picked folder as the Inbox and does not watch it unasked", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(String.raw`E:\New`);
    renderStep();
    await userEvent.click(
      await screen.findByRole("button", { name: tx("firstRun:inbox.choose") }),
    );

    const watch = await screen.findByRole("checkbox", {
      name: tx("firstRun:inbox.watch"),
    });
    expect(watch).not.toBeChecked();
    expect(screen.getByText(String.raw`E:\New`)).toBeInTheDocument();
    expect(acted(backend)).toEqual([
      {
        cmd: "add_music_folder",
        args: { path: String.raw`E:\New`, role: "inbox" },
      },
    ]);
  });

  it("watches the folder only after the user turns the switch on", async () => {
    const backend = fakeBackend([folder(1, String.raw`E:\New`, "inbox")]);
    renderStep();
    const watch = await screen.findByRole("checkbox", {
      name: tx("firstRun:inbox.watch"),
    });
    await userEvent.click(watch);
    await waitFor(() =>
      expect(
        screen.getByRole("checkbox", { name: tx("firstRun:inbox.watch") }),
      ).toBeChecked(),
    );
    expect(acted(backend)).toEqual([
      { cmd: "set_music_folder_watch", args: { id: 1, watch: true } },
    ]);
  });

  it("tells the two watch switches apart when there are two Inbox folders", async () => {
    fakeBackend([
      folder(1, String.raw`E:New`, "inbox"),
      folder(2, String.raw`F:More`, "inbox"),
    ]);
    renderStep();
    const switches = await screen.findAllByRole("checkbox", {
      name: tx("firstRun:inbox.watch"),
    });
    expect(switches).toHaveLength(2);
    const ids = switches.map((box) => box.getAttribute("aria-describedby"));
    expect(new Set(ids).size).toBe(2);
    for (const id of ids)
      expect(document.getElementById(id ?? "")).not.toBeNull();
  });

  it("shows an existing Inbox folder instead of asking", async () => {
    fakeBackend([folder(1, String.raw`E:\New`, "inbox")]);
    renderStep();
    expect(await screen.findByText(String.raw`E:\New`)).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: tx("firstRun:inbox.choose") }),
    ).toBeNull();
  });

  it("assumes nothing when skipped", async () => {
    const backend = fakeBackend();
    renderStep();
    await userEvent.click(
      await screen.findByRole("button", { name: tx("firstRun:inbox.skip") }),
    );
    expect(
      screen.queryByRole("heading", { name: tx("firstRun:inbox.title") }),
    ).toBeNull();
    expect(dialog.open).not.toHaveBeenCalled();
    expect(acted(backend)).toEqual([]);
  });
});

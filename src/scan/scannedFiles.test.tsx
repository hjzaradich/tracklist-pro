import { act, cleanup, render, renderHook, screen } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ScannedFile } from "../bindings";
import { useScannedFiles, useScannedFilesStore } from "./scannedFilesStore";
import { useScannedFilesSync } from "./useScannedFilesSync";

// The name tauri-specta gives the ScannedFiles event (see src/bindings.ts).
const SCANNED_FILES_EVENT = "scanned-files";

function file(id: number, relPath = `Track ${id}.mp3`): ScannedFile {
  return {
    id,
    musicFolderId: 1,
    relPath,
    size: 1000 + id,
    modifiedMs: 1_700_000_000_000,
    onlineOnly: false,
  };
}

/** Mounts the listener and waits until it's listening. */
async function listen() {
  const hook = renderHook(() => useScannedFilesSync());
  await act(async () => {});
  return hook;
}

/** What the walk does with each batch of new rows. */
async function send(...files: ScannedFile[]) {
  await act(() => emit(SCANNED_FILES_EVENT, files));
}

const state = () => useScannedFilesStore.getState();

beforeEach(() => {
  state().reset();
  mockIPC(() => undefined, { shouldMockEvents: true });
});

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("scanned files store", () => {
  it("keeps each file once, by row id, across batches", () => {
    state().add([file(1), file(2)]);
    state().add([file(2), file(3)]);
    expect(state().count).toBe(3);
    expect([...state().files.keys()]).toEqual([1, 2, 3]);
    expect(state().version).toBe(2);
  });

  it("ignores an empty batch, so nothing re-renders", () => {
    state().add([file(1)]);
    const before = state();
    state().add([]);
    expect(state()).toBe(before);
  });

  it("takes a 100k-file first scan in batches, each costing only its own files", () => {
    // The Map is changed in place: the same one after every batch, so a
    // batch never copies the files before it. Checked by identity, not by
    // the clock, which says nothing on the shared CI laptop.
    const files = state().files;
    for (let batch = 0; batch < 100; batch++) {
      state().add(Array.from({ length: 1000 }, (_, i) => file(batch * 1000 + i + 1)));
      expect(state().files).toBe(files);
    }
    expect(state().count).toBe(100_000);
    expect(state().version).toBe(100);
  });
});

describe("scanned files listener", () => {
  it("adds every batch the walk sends while it's mounted", async () => {
    await listen();
    await send(file(1), file(2, "Q.V.X./Dot.mp3"));
    await send(file(3));
    expect(state().count).toBe(3);
    expect(state().files.get(2)).toEqual(file(2, "Q.V.X./Dot.mp3"));
  });

  it("stops listening when unmounted", async () => {
    const { unmount } = await listen();
    await send(file(1));
    unmount();
    await send(file(2));
    expect(state().count).toBe(1);
  });
});

describe("useScannedFiles", () => {
  function FileCount() {
    const files = useScannedFiles();
    return <output aria-label="files">{String(files.size)}</output>;
  }

  it("re-renders its component after each batch, though the Map changes in place", () => {
    render(<FileCount />);
    const count = () => screen.getByRole("status", { name: "files" }).textContent;
    expect(count()).toBe("0");
    act(() => state().add([file(1), file(2)]));
    expect(count()).toBe("2");
    act(() => state().add([file(3)]));
    expect(count()).toBe("3");
  });
});

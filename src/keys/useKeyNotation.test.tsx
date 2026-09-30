import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { useKeyNotation, useSetKeyNotation } from "./useKeyNotation";

/**
 * Stands in for the Rust side over Tauri's IPC, so the generated bindings
 * are exercised as the app uses them. `stored` is the setting's value.
 */
function fakeBackend(initial: string, { failReads = false } = {}) {
  const backend = { stored: initial, calls: [] as string[] };
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    if (cmd === "key_notation") {
      if (failReads) throw { kind: "database", params: {} };
      return backend.stored;
    }
    if (cmd === "set_key_notation") {
      backend.stored = (args as { notation: string }).notation;
      return null;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={createQueryClient()}>{children}</QueryClientProvider>;
}

describe("the key notation setting", () => {
  afterEach(() => clearMocks());

  it("shows keys in Camelot until the stored setting has loaded", async () => {
    const backend = fakeBackend("musical_flats");
    const { result } = renderHook(() => useKeyNotation(), { wrapper });
    expect(result.current).toBe("camelot");
    await waitFor(() => expect(result.current).toBe("musical_flats"));
    expect(backend.calls).toContain("key_notation");
  });

  it("falls back to Camelot if the setting can't be read", async () => {
    const backend = fakeBackend("musical_flats", { failReads: true });
    const { result } = renderHook(() => useKeyNotation(), { wrapper });
    await waitFor(() => expect(backend.calls).toContain("key_notation"));
    expect(result.current).toBe("camelot");
  });

  it("falls back to Camelot for a notation this build doesn't know", async () => {
    const backend = fakeBackend("open_key");
    const { result } = renderHook(() => useKeyNotation(), { wrapper });
    await waitFor(() => expect(backend.calls).toContain("key_notation"));
    expect(result.current).toBe("camelot");
  });

  it("saves a new notation in the backend, and every reader switches to it", async () => {
    const backend = fakeBackend("camelot");
    const { result } = renderHook(() => ({ notation: useKeyNotation(), set: useSetKeyNotation() }), {
      wrapper,
    });
    await waitFor(() => expect(backend.calls).toContain("key_notation"));
    await act(() => result.current.set.mutateAsync("musical_sharps"));
    expect(backend.stored).toBe("musical_sharps");
    await waitFor(() => expect(result.current.notation).toBe("musical_sharps"));
  });
});

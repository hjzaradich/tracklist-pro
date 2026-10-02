import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import "../i18n";
import { ReadOnlineOnlySwitch } from "./ReadOnlineOnlySwitch";
import { tx } from "../test/tx";

/** Stands in for the Rust side: the opt-in as stored, and every call made. */
function fakeBackend(stored: boolean) {
  const backend = { stored, calls: [] as string[] };
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    if (cmd === "read_online_only_files") return backend.stored;
    if (cmd === "set_read_online_only_files") {
      backend.stored = (args as { on: boolean }).on;
      return null;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

function renderSwitch() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <ReadOnlineOnlySwitch />
    </QueryClientProvider>,
  );
  return screen.getByRole("checkbox", { name: tx("musicFolderStatus:readOnlineOnly") });
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the online-only opt-in", () => {
  it("is off by default and explains what turning it on does", async () => {
    const backend = fakeBackend(false);
    const box = renderSwitch();
    await waitFor(() => expect(backend.calls).toContain("read_online_only_files"));
    expect(box).not.toBeChecked();
    expect(box).toHaveAccessibleDescription(
      tx("musicFolderStatus:readOnlineOnlyHelp"),
    );
  });

  it("stores the choice when it's turned on, and again when it's turned off", async () => {
    const backend = fakeBackend(false);
    const box = renderSwitch();
    await waitFor(() => expect(backend.calls).toContain("read_online_only_files"));
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    expect(backend.stored).toBe(true);
    await userEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(backend.stored).toBe(false);
  });

  it("shows a stored opt-in as on", async () => {
    fakeBackend(true);
    const box = renderSwitch();
    await waitFor(() => expect(box).toBeChecked());
  });
});

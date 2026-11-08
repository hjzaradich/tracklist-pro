import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { AudioModelStatus } from "../bindings";
import "../i18n";
import { tx } from "../test/tx";
import { AudioModelRow } from "./AudioModelRow";

const OFF: AudioModelStatus = { enabled: false, fingerprinted: 0, total: 40, failed: 0, running: false };

/** Stands in for the Rust side: the switch, the counts and the setter's calls. */
function fakeBackend(status: AudioModelStatus, { failSave = false }: { failSave?: boolean } = {}) {
  const backend = { status, saved: [] as boolean[] };
  mockIPC((cmd, args) => {
    switch (cmd) {
      case "audio_model_status":
        return backend.status;
      case "set_audio_model_enabled": {
        if (failSave) throw { kind: "database", params: {} };
        const on = (args as { on: boolean }).on;
        backend.saved.push(on);
        backend.status = { ...backend.status, enabled: on, running: on };
        return backend.status;
      }
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return backend;
}

function renderRow() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <AudioModelRow />
    </QueryClientProvider>,
  );
  return screen.getByRole("checkbox", { name: tx("audioModel:switch.label") });
}

describe("AudioModelRow", () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it("is off by default, says what it does, and shows no status while off", async () => {
    fakeBackend(OFF);
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
    expect(screen.getByText(tx("audioModel:switch.help"))).toBeVisible();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("calls the setter when switched on and then shows the status", async () => {
    const backend = fakeBackend(OFF);
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    await userEvent.click(box);
    expect(await screen.findByText(tx("audioModel:switch.progress", { done: 0, count: 40 }))).toBeVisible();
    expect(backend.saved).toEqual([true]);
    expect(box).toBeChecked();
    expect(screen.getByText(tx("audioModel:switch.running"))).toBeVisible();
  });

  it("calls the setter when switched off", async () => {
    const backend = fakeBackend({ ...OFF, enabled: true, fingerprinted: 12, running: true });
    const box = renderRow();
    await waitFor(() => expect(box).toBeChecked());
    await userEvent.click(box);
    await waitFor(() => expect(box).not.toBeChecked());
    expect(backend.saved).toEqual([false]);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("shows how many were listened to, how many could not be read, and that it is not running", async () => {
    fakeBackend({ enabled: true, fingerprinted: 12, total: 40, failed: 2, running: false });
    renderRow();
    expect(await screen.findByText(tx("audioModel:switch.progress", { done: 12, count: 40 }))).toBeVisible();
    expect(screen.getByText(tx("audioModel:switch.failed", { count: 2 }))).toBeVisible();
    expect(screen.getByText(tx("audioModel:switch.paused"))).toBeVisible();
  });

  it("says it has finished when every track is listened to or could not be read", async () => {
    fakeBackend({ enabled: true, fingerprinted: 38, total: 40, failed: 2, running: false });
    renderRow();
    expect(await screen.findByText(tx("audioModel:switch.finished"))).toBeVisible();
  });

  it("says there are no Library tracks to listen to when the Library is empty", async () => {
    fakeBackend({ enabled: true, fingerprinted: 0, total: 0, failed: 0, running: false });
    renderRow();
    expect(await screen.findByText(tx("audioModel:switch.noTracks"))).toBeVisible();
  });

  it("says when the switch could not be saved", async () => {
    fakeBackend(OFF, { failSave: true });
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    await userEvent.click(box);
    expect(await screen.findByRole("alert")).toHaveTextContent(tx("audioModel:switch.error"));
  });
});

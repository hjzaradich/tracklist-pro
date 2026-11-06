import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { RekordboxDatabase } from "../bindings";
import { tx } from "../test/tx";
import { RekordboxDatabaseRow } from "./RekordboxDatabaseRow";

const PICKED = String.raw`C:\Synthetic\master.db`;

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => PICKED) }));

const READ_AT = "2026-11-06T10:00:00.000Z";

/** Stands in for the Rust side: the switch, the session's key, and each read's outcome. */
function fakeBackend({ on = false, outcome = "read" }: { on?: boolean; outcome?: "read" | "wrongKey" } = {}) {
  const backend = { on, keySet: false, keys: [] as string[], calls: [] as string[], failures: 0, read: false };
  const state = (): RekordboxDatabase => ({
    keySet: backend.keySet,
    failures: backend.failures,
    failure: backend.failures > 0 ? "wrongKey" : null,
    lastRead: backend.read
      ? {
          path: PICKED,
          modifiedMs: 0,
          readAt: READ_AT,
          summary: { tracks: 2, streaming: 0, kept: 0, notStored: 0, complete: true },
          source: "database",
        }
      : null,
    usualFolder: null,
  });
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case "feature_enabled":
        return backend.on;
      case "set_feature_enabled":
        backend.on = a.on as boolean;
        return null;
      case "rekordbox_database":
        return state();
      case "set_rekordbox_database_key":
        backend.keys.push(a.key as string);
        backend.keySet = a.key !== "";
        return backend.keySet;
      case "read_rekordbox_database":
        if (outcome === "read") backend.read = true;
        else backend.failures += 1;
        return { kind: "started", job: 1 };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return backend;
}

function renderRow(client = createQueryClient()) {
  render(
    <QueryClientProvider client={client}>
      <RekordboxDatabaseRow />
    </QueryClientProvider>,
  );
  return screen.getByRole("checkbox", { name: tx("rekordboxDb:label") });
}

afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the rekordbox database row", () => {
  it("is off by default: its switch alone, and nothing asked of the reader", async () => {
    const backend = fakeBackend();
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
    expect(screen.getByText(tx("rekordboxDb:helpOff"))).toBeInTheDocument();
    expect(screen.queryByLabelText(tx("rekordboxDb:keyLabel"))).toBeNull();
    expect(screen.queryAllByRole("button")).toEqual([]);
    expect(backend.calls).toEqual(["feature_enabled"]);
  });

  it("turned on, hands the key over once, never shows it back, and reads the picked file", async () => {
    const backend = fakeBackend();
    const user = userEvent.setup();
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    await user.click(box);
    const field = await screen.findByLabelText(tx("rekordboxDb:keyLabel"));
    expect(field).toHaveAttribute("type", "password");
    const readNow = screen.getByRole("button", { name: tx("rekordboxDb:read") });
    expect(readNow).toBeDisabled();

    await user.type(field, "made-up words");
    await user.click(screen.getByRole("button", { name: tx("rekordboxDb:pick") }));
    expect(await screen.findByText(tx("rekordboxDb:picked", { path: PICKED }))).toBeInTheDocument();
    await user.click(readNow);
    expect(await screen.findByText(tx("rekordboxDb:done", { count: 2 }))).toBeInTheDocument();
    expect(backend.keys).toEqual(["made-up words"]);
    expect(field).toHaveValue("");
    expect(screen.getByText(tx("rekordboxDb:keySet"))).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("made-up words");

    // Off again: the key is forgotten and the row is its switch alone.
    await user.click(box);
    await waitFor(() => expect(screen.queryByLabelText(tx("rekordboxDb:keyLabel"))).toBeNull());
    expect(backend.keys).toEqual(["made-up words", ""]);
  });

  it("keeps the typed key nowhere in the frontend once it is handed over", async () => {
    fakeBackend({ on: true });
    const user = userEvent.setup();
    const client = createQueryClient();
    renderRow(client);
    const field = await screen.findByLabelText(tx("rekordboxDb:keyLabel"));
    expect(screen.getByText(tx("rekordboxDb:helpOn"))).toBeInTheDocument();
    await user.type(field, "made-up words");
    await user.click(screen.getByRole("button", { name: tx("rekordboxDb:pick") }));
    await user.click(screen.getByRole("button", { name: tx("rekordboxDb:read") }));
    expect(await screen.findByText(tx("rekordboxDb:done", { count: 2 }))).toBeInTheDocument();
    const held = JSON.stringify([
      client.getMutationCache().getAll().map((m) => m.state.variables ?? null),
      client.getQueryCache().getAll().map((q) => q.state.data ?? null),
    ]);
    expect(held).not.toContain("made-up words");
    expect(field).toHaveValue("");
  });

  it("says why a read failed", async () => {
    fakeBackend({ on: true, outcome: "wrongKey" });
    const user = userEvent.setup();
    renderRow();
    const field = await screen.findByLabelText(tx("rekordboxDb:keyLabel"));
    await user.type(field, "not it");
    await user.click(screen.getByRole("button", { name: tx("rekordboxDb:pick") }));
    await user.click(screen.getByRole("button", { name: tx("rekordboxDb:read") }));
    expect(await screen.findByText(tx("rekordboxDb:failure.wrongKey"))).toBeInTheDocument();
  });
});

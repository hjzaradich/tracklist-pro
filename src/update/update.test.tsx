import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { UpdateStatus } from "../bindings";
import { AboutPanel } from "../settings/AboutPanel";
import { UpdateChecksRow } from "../settings/UpdateChecksRow";
import { tx } from "../test/tx";
import { UpdateQuestion } from "./UpdateQuestion";
import { resetStartupUpdateCheck } from "./useUpdates";

const URL = "https://example.test/releases/tag/v9.0.0";

function status(over: Partial<UpdateStatus> = {}): UpdateStatus {
  return { asked: false, enabled: false, current: "0.1.0", available: null, checkedAt: null, ...over };
}

/** Stands in for the Rust side: the stored answer, and what each command does to it. */
function fakeBackend(start: UpdateStatus, found: UpdateStatus["available"] = null, holdCheck?: Promise<void>) {
  const backend = { status: start, calls: [] as string[], sets: [] as boolean[] };
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case "update_status":
        return backend.status;
      case "set_update_checks":
        backend.sets.push(a.on as boolean);
        backend.status = { ...backend.status, asked: true, enabled: a.on as boolean };
        return null;
      case "check_for_updates":
        if (holdCheck) return holdCheck.then(() => backend.status);
        if (backend.status.enabled) backend.status = { ...backend.status, available: found, checkedAt: "2026-11-07T10:00:00.000Z" };
        return backend.status;
      default:
        // getVersion and the like.
        return "0.1.0";
    }
  });
  return backend;
}

function renderWith(ui: React.ReactElement) {
  return render(<QueryClientProvider client={createQueryClient()}>{ui}</QueryClientProvider>);
}

beforeEach(() => resetStartupUpdateCheck());
afterEach(() => {
  cleanup();
  clearMocks();
  vi.unstubAllGlobals();
});

describe("the first-run question about update checks", () => {
  it("asks until it is answered, and the answer is stored", async () => {
    const backend = fakeBackend(status());
    renderWith(<UpdateQuestion />);
    await userEvent.click(await screen.findByRole("button", { name: tx("updates:ask.no") }));
    await waitFor(() => expect(screen.queryByRole("heading", { name: tx("updates:ask.title") })).toBeNull());
    expect(backend.sets).toEqual([false]);
    // No check was asked for after a no.
    expect(backend.calls).not.toContain("check_for_updates");
  });

  it("turns checks on and checks at once when the answer is yes", async () => {
    const backend = fakeBackend(status());
    renderWith(<UpdateQuestion />);
    await userEvent.click(await screen.findByRole("button", { name: tx("updates:ask.yes") }));
    await waitFor(() => expect(backend.sets).toEqual([true]));
    await waitFor(() => expect(backend.calls).toContain("check_for_updates"));
  });

  it("closes as soon as the answer is stored, before the check itself finishes", async () => {
    // The check is held open until the test lets it go: no timing involved.
    let release = () => {};
    const latch = new Promise<void>((resolve) => {
      release = resolve;
    });
    const backend = fakeBackend(status(), null, latch);
    renderWith(<UpdateQuestion />);
    await userEvent.click(await screen.findByRole("button", { name: tx("updates:ask.yes") }));
    await waitFor(() => expect(backend.calls).toContain("check_for_updates"));
    await waitFor(() => expect(screen.queryByRole("heading", { name: tx("updates:ask.title") })).toBeNull());
    release();
  });

  it("does not show again once answered, whatever the Library holds", async () => {
    fakeBackend(status({ asked: true }));
    renderWith(<UpdateQuestion />);
    await waitFor(() => expect(screen.queryByRole("heading")).toBeNull());
  });

  it("says what is sent and where", async () => {
    fakeBackend(status());
    renderWith(<UpdateQuestion />);
    expect(await screen.findByText(tx("updates:ask.help"))).toBeInTheDocument();
  });
});

describe("the Settings switch", () => {
  it("is off by default, offers no Check now while off, and turns on", async () => {
    const backend = fakeBackend(status());
    renderWith(<UpdateChecksRow />);
    const box = screen.getByRole("checkbox", { name: tx("updates:settings.label") });
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
    expect(screen.queryByRole("button", { name: tx("updates:settings.checkNow") })).toBeNull();
    await userEvent.click(box);
    await waitFor(() => expect(box).toBeChecked());
    expect(backend.sets).toEqual([true]);
  });

  it("Check now asks the backend to check regardless of the day", async () => {
    const backend = fakeBackend(status({ asked: true, enabled: true }));
    renderWith(<UpdateChecksRow />);
    await userEvent.click(await screen.findByRole("button", { name: tx("updates:settings.checkNow") }));
    const when = new Date("2026-11-07T10:00:00.000Z").toLocaleString();
    expect(await screen.findByText(tx("updates:settings.upToDate", { when }))).toBeInTheDocument();
    expect(backend.calls).toContain("check_for_updates");
  });
});

describe("telling about a newer version", () => {
  it("shows the version and the release address as text with a Copy button", async () => {
    fakeBackend(status({ asked: true, enabled: true, available: { version: "9.0.0", url: URL } }));
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("fetch", vi.fn(async () => new Response("", { status: 404 })));
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    renderWith(<AboutPanel />);
    expect(await screen.findByText(tx("updates:about.available", { version: "9.0.0" }))).toBeInTheDocument();
    expect(screen.getByText(URL)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: tx("updates:about.copy") }));
    expect(writeText).toHaveBeenCalledWith(URL);
    expect(await screen.findByText(tx("updates:about.copied"))).toBeInTheDocument();
    // Only text: no link to follow.
    expect(screen.queryByRole("link", { name: URL })).toBeNull();
  });

  it("shows nothing when there is nothing newer", async () => {
    fakeBackend(status({ asked: true, enabled: true }));
    vi.stubGlobal("fetch", vi.fn(async () => new Response("", { status: 404 })));
    renderWith(<AboutPanel />);
    await screen.findByText(tx("about:licences.missing"));
    expect(screen.queryByText(tx("updates:about.copy"))).toBeNull();
  });
});

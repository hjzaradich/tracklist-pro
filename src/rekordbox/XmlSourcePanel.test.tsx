import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LastRead, ReadFailure, XmlSource } from "../bindings";
import i18n from "../i18n";
import { READING_INTERVAL_MS } from "./useXmlSource";
import { XmlSourcePanel } from "./XmlSourcePanel";
import { tx } from "../test/tx";

const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@tauri-apps/plugin-dialog", () => dialog);

// The date a read is shown with: the same formatting the panel uses.
const when = (iso: string) =>
  new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" }).format(
    new Date(iso),
  );
const READ_AT = "2026-09-29T13:00:00.000Z";

const EXPORT = "C:\\Users\\dj\\Documents\\rekordbox.xml";

function lastRead(fields: Partial<LastRead["summary"]> = {}, readAt = "2026-09-29T12:00:00.000Z"): LastRead {
  return {
    path: EXPORT,
    modifiedMs: 1,
    readAt,
    summary: { tracks: 1234, streaming: 0, kept: 0, notStored: 0, complete: true, ...fields },
  };
}

/**
 * Stands in for the Rust side. A read finishes at once: `onRead` decides
 * what the source records, or the read is refused with `refuse`.
 */
function fakeBackend(initial: Partial<XmlSource> = {}) {
  const backend = {
    source: {
      path: null,
      watch: false,
      lastRead: null,
      lastFailure: null,
      newerExport: null,
      exportFolder: "C:\\Users\\dj\\Documents",
      ...initial,
    } as XmlSource,
    reads: [] as (string | null)[],
    refuse: null as unknown,
    onRead: (path: string) => {
      backend.source = { ...backend.source, path, lastRead: { ...lastRead(), path, readAt: new Date().toISOString() } };
    },
  };
  mockIPC((cmd, args) => {
    const a = args as Record<string, unknown>;
    if (cmd === "rekordbox_xml_source") return backend.source;
    if (cmd === "read_rekordbox_xml") {
      const path = (a.path as string | null) ?? null;
      backend.reads.push(path);
      if (backend.refuse) throw backend.refuse;
      backend.onRead(path ?? backend.source.path ?? "");
      return 42;
    }
    if (cmd === "set_rekordbox_xml_watch") {
      backend.source = { ...backend.source, watch: a.watch as boolean };
      return null;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  return backend;
}

function renderPanel() {
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <XmlSourcePanel />
    </QueryClientProvider>,
  );
}

beforeEach(() => dialog.open.mockReset());
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("the rekordbox XML source", () => {
  it("says no export is chosen and how to make one", async () => {
    fakeBackend();
    renderPanel();
    expect(await screen.findByText(tx("rekordbox:noneChosen"))).toBeInTheDocument();
    expect(screen.getByText(tx("rekordbox:howTo"))).toBeInTheDocument();
    expect(await screen.findByText(tx("rekordbox:neverRead"))).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("rekordbox:choose") })).toBeEnabled();
    expect(screen.queryByRole("button", { name: tx("rekordbox:readAgain") })).not.toBeInTheDocument();
  });

  it("opens a picker for xml files in Documents and reads the file picked", async () => {
    const backend = fakeBackend();
    // The read is recorded with a known time, so its line can be looked up in full.
    backend.onRead = (path) => {
      backend.source = { ...backend.source, path, lastRead: { ...lastRead(), path, readAt: READ_AT } };
    };
    dialog.open.mockResolvedValue(EXPORT);
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordbox:choose") }));
    expect(dialog.open).toHaveBeenCalledWith({
      multiple: false,
      directory: false,
      defaultPath: "C:\\Users\\dj\\Documents",
      filters: [{ name: tx("rekordbox:fileFilter"), extensions: ["xml"] }],
    });
    await waitFor(() => expect(backend.reads).toEqual([EXPORT]));
    expect(
      await screen.findByText(tx("rekordbox:lastRead", { when: when(READ_AT), count: 1234 })),
    ).toBeInTheDocument();
    expect(screen.getByText(EXPORT)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("rekordbox:chooseAnother") })).toBeEnabled();
  });

  it("reads nothing when the picker is closed without a file", async () => {
    const backend = fakeBackend();
    dialog.open.mockResolvedValue(null);
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordbox:choose") }));
    await waitFor(() => expect(dialog.open).toHaveBeenCalled());
    expect(backend.reads).toEqual([]);
  });

  it("reads the chosen export again", async () => {
    const backend = fakeBackend({ path: EXPORT, lastRead: lastRead() });
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordbox:readAgain") }));
    await waitFor(() => expect(backend.reads).toEqual([null]));
  });

  it("says a read is under way until the source records it, asked again every poll", async () => {
    // Only the poll's interval runs on a fake clock, so the test moves time
    // instead of sitting through it. Everything else stays real: TanStack
    // Query notifies through 0 ms setTimeouts that re-schedule while a fake
    // tick runs, so faking those never returns from the tick.
    vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
    try {
      const backend = fakeBackend({ path: EXPORT, lastRead: lastRead() });
      // This read doesn't finish until the test says so.
      backend.onRead = () => {};
      renderPanel();
      await userEvent.click(await screen.findByRole("button", { name: tx("rekordbox:readAgain") }));
      expect(await screen.findByText(tx("rekordbox:reading"))).toBeInTheDocument();
      expect(screen.getByRole("button", { name: tx("rekordbox:readAgain") })).toBeDisabled();
      backend.source = { ...backend.source, lastRead: lastRead({ tracks: 1 }, "2026-09-30T08:00:00.000Z") };
      // The source is asked again once per READING_INTERVAL_MS, not before.
      await act(() => vi.advanceTimersByTimeAsync(READING_INTERVAL_MS - 1));
      expect(screen.getByText(tx("rekordbox:reading"))).toBeInTheDocument();
      expect(backend.reads).toEqual([null]);
      await act(() => vi.advanceTimersByTimeAsync(1));
      expect(
        await screen.findByText(
          tx("rekordbox:lastRead", { when: when("2026-09-30T08:00:00.000Z"), count: 1 }),
        ),
      ).toBeInTheDocument();
      expect(screen.queryByText(tx("rekordbox:reading"))).not.toBeInTheDocument();
    } finally {
      vi.useRealTimers();
    }
  });

  it("points out an incomplete export and tracks that couldn't be read", async () => {
    fakeBackend({ path: EXPORT, lastRead: lastRead({ complete: false, kept: 3, notStored: 2 }) });
    renderPanel();
    expect(await screen.findByText(tx("rekordbox:incomplete"))).toBeInTheDocument();
    expect(screen.getByText(tx("rekordbox:notStored", { count: 2 }))).toBeInTheDocument();
  });

  it.each(["notFound", "cantRead", "damaged", "notAnExport"] as const)(
    "says why the last read failed (%s)",
    async (reason) => {
    const message = tx(`rekordbox:failed.${reason}`, { path: EXPORT });
    const lastFailure: ReadFailure = { path: EXPORT, reason, at: "2026-09-29T12:00:00.000Z" };
    fakeBackend({ path: EXPORT, lastFailure });
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    },
  );

  it("shows why a file was refused", async () => {
    const backend = fakeBackend();
    backend.refuse = { kind: "notRekordboxXml", params: { path: "C:\\notes.xml" } };
    dialog.open.mockResolvedValue("C:\\notes.xml");
    renderPanel();
    await userEvent.click(await screen.findByRole("button", { name: tx("rekordbox:choose") }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      tx("rekordbox:error.notAnExport", { path: "C:\\notes.xml" }),
    );
  });

  it("offers a newer export the watch found, and reads it only when asked", async () => {
    const newer = "C:\\Users\\dj\\Documents\\export 2.xml";
    const backend = fakeBackend({
      path: EXPORT,
      watch: true,
      lastRead: lastRead(),
      newerExport: { path: newer, name: "export 2.xml", modifiedMs: 2 },
    });
    renderPanel();
    expect(await screen.findByText(tx("rekordbox:newer", { name: "export 2.xml" }))).toBeInTheDocument();
    expect(backend.reads).toEqual([]);
    backend.onRead = (path) => {
      backend.source = { ...backend.source, path, newerExport: null, lastRead: { ...lastRead(), path, readAt: "2026-09-30T00:00:00.000Z" } };
    };
    await userEvent.click(screen.getByRole("button", { name: tx("rekordbox:readNewer") }));
    await waitFor(() => expect(backend.reads).toEqual([newer]));
    await waitFor(() => expect(screen.queryByText(tx("rekordbox:newer", { name: "export 2.xml" }))).not.toBeInTheDocument());
  });

  it("remembers the watch when it's switched on and off", async () => {
    const backend = fakeBackend();
    renderPanel();
    const watch = await screen.findByRole("checkbox", { name: tx("rekordbox:watch") });
    await waitFor(() => expect(watch).toBeEnabled());
    await userEvent.click(watch);
    await waitFor(() => expect(watch).toBeChecked());
    expect(backend.source.watch).toBe(true);
    await userEvent.click(watch);
    await waitFor(() => expect(watch).not.toBeChecked());
    expect(backend.source.watch).toBe(false);
  });

  it("names the read in Activity", () => {
    expect(i18n.exists("task.read_rekordbox", { ns: "activity" })).toBe(true);
  });
});

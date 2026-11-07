import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { LogEvent } from "../bindings";
import { tx } from "../test/tx";
import { DiagnosticsScreen } from "./DiagnosticsScreen";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => <a href={to}>{children}</a>,
}));

const REPORT = "Tracklist Pro diagnostics report\ncrates: 3\nplaylists: 2\n";
const SAVED_PATH = String.raw`C:\Synthetic\data\diagnostics\20261107-120000000.txt`;

const event = (over: Partial<LogEvent>): LogEvent => ({
  atMs: 1_700_000_000_000,
  kind: "jobStarted",
  job: null,
  reason: null,
  count: null,
  ms: null,
  ...over,
});

// Newest first, as the backend sends them.
const EVENTS: LogEvent[] = [
  event({ atMs: 1_700_000_003_000, kind: "rekordboxReadFailed", job: "read_rekordbox", reason: "damaged", ms: 12 }),
  event({ atMs: 1_700_000_002_000, kind: "jobFinished", job: "scan", ms: 40 }),
  event({ atMs: 1_700_000_001_000, kind: "startup", count: 0, ms: 300 }),
];

/** Stands in for the Rust side, and remembers every command the screen sent. */
function fakeBackend({
  events = EVENTS,
  lostWrites = 0,
  failSave = false,
}: { events?: LogEvent[]; lostWrites?: number; failSave?: boolean } = {}) {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    switch (cmd) {
      case "diagnostic_events":
        return { events, lostWrites };
      case "preview_diagnostics_report":
        return REPORT;
      case "save_diagnostics_report":
        if (failSave) throw { kind: "cannotWrite", params: {} };
        return { path: SAVED_PATH };
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return calls;
}

function renderScreen() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <DiagnosticsScreen />
    </QueryClientProvider>,
  );
}

afterEach(() => {
  cleanup();
  clearMocks();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe("the event log", () => {
  it("lists the events newest first, in words", async () => {
    fakeBackend();
    renderScreen();
    const rows = await screen.findAllByRole("row");
    // The header row, then one row per event.
    expect(rows).toHaveLength(EVENTS.length + 1);
    expect(within(rows[1]).getByText(tx("diagnostics:kind.rekordboxReadFailed"))).toBeInTheDocument();
    expect(within(rows[1]).getByText(tx("diagnostics:reason.damaged"))).toBeInTheDocument();
    expect(within(rows[1]).getByText(tx("activity:task.read_rekordbox"))).toBeInTheDocument();
    expect(within(rows[2]).getByText(tx("diagnostics:kind.jobFinished"))).toBeInTheDocument();
    expect(within(rows[3]).getByText(tx("diagnostics:kind.startup"))).toBeInTheDocument();
  });

  it("narrows the list to one kind of event", async () => {
    fakeBackend();
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: tx("diagnostics:log.filter") }),
      tx("diagnostics:kind.jobFinished"),
    );
    const rows = screen.getAllByRole("row");
    expect(rows).toHaveLength(2);
    expect(within(rows[1]).getByText(tx("diagnostics:kind.jobFinished"))).toBeInTheDocument();
    await userEvent.selectOptions(
      screen.getByRole("combobox", { name: tx("diagnostics:log.filter") }),
      tx("diagnostics:kind.sendDone"),
    );
    expect(screen.getByText(tx("diagnostics:log.noMatch"))).toBeInTheDocument();
  });

  it("says so when there are no events, and when the log file lost some", async () => {
    fakeBackend({ events: [], lostWrites: 2 });
    renderScreen();
    expect(await screen.findByText(tx("diagnostics:log.empty"))).toBeInTheDocument();
    expect(screen.getByText(tx("diagnostics:log.lost", { count: 2 }))).toBeInTheDocument();
  });
});

describe("the report", () => {
  it("shows the exact text before anything is saved", async () => {
    const calls = fakeBackend();
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.create") }));
    const preview = await screen.findByRole("textbox", { name: tx("diagnostics:report.previewLabel") });
    expect(preview).toHaveValue(REPORT);
    expect(preview).toHaveAttribute("readonly");
    expect(calls).toContain("preview_diagnostics_report");
    expect(calls).not.toContain("save_diagnostics_report");
  });

  it("saves it, shows where as text, and copies the text it showed", async () => {
    const calls = fakeBackend();
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.create") }));
    await screen.findByRole("textbox", { name: tx("diagnostics:report.previewLabel") });
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.save") }));
    const path = await screen.findByRole("textbox", { name: tx("diagnostics:report.pathLabel") });
    expect(path).toHaveValue(SAVED_PATH);
    expect(path).toHaveAttribute("readonly");
    expect(calls.filter((c) => c === "save_diagnostics_report")).toHaveLength(1);
    // Saved once: the button goes away.
    expect(screen.queryByRole("button", { name: tx("diagnostics:report.save") })).not.toBeInTheDocument();

    // After the render: userEvent installs its own clipboard on setup.
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { ...navigator, clipboard: { writeText } });
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.copy") }));
    expect(await screen.findByText(tx("diagnostics:report.copied"))).toBeInTheDocument();
    expect(writeText).toHaveBeenLastCalledWith(REPORT);
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.copyPath") }));
    expect(writeText).toHaveBeenLastCalledWith(SAVED_PATH);
  });

  it("says so when the text can't be copied", async () => {
    fakeBackend();
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.create") }));
    await screen.findByRole("textbox", { name: tx("diagnostics:report.previewLabel") });
    vi.stubGlobal("navigator", {
      ...navigator,
      clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.copy") }));
    expect(await screen.findByText(tx("diagnostics:report.copyFailed"))).toBeInTheDocument();
  });

  it("keeps the preview and says so when the save fails", async () => {
    fakeBackend({ failSave: true });
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.create") }));
    await screen.findByRole("textbox", { name: tx("diagnostics:report.previewLabel") });
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.save") }));
    expect(await screen.findByText(tx("diagnostics:report.saveError"))).toBeInTheDocument();
    expect(screen.getByRole("textbox", { name: tx("diagnostics:report.previewLabel") })).toHaveValue(REPORT);
    expect(screen.getByRole("button", { name: tx("diagnostics:report.save") })).toBeEnabled();
  });

  it("sends nothing anywhere: it asks the app for the three diagnostics commands and nothing else", async () => {
    const calls = fakeBackend();
    renderScreen();
    await screen.findAllByRole("row");
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.create") }));
    await screen.findByRole("textbox", { name: tx("diagnostics:report.previewLabel") });
    await userEvent.click(screen.getByRole("button", { name: tx("diagnostics:report.save") }));
    await screen.findByRole("textbox", { name: tx("diagnostics:report.pathLabel") });
    expect([...new Set(calls)].sort()).toEqual([
      "diagnostic_events",
      "preview_diagnostics_report",
      "save_diagnostics_report",
    ]);
  });
});

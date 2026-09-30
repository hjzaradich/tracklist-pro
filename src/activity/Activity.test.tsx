import { act, cleanup, render, screen } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ActivitySnapshot, JobUpdate } from "../bindings";
import { renderApp } from "../app/testApp";
import { DEFAULT_THEME, syncThemeToDocument, useThemeStore } from "../theme/themeStore";
import "../theme/tokens.css";
import { Activity } from "./Activity";

// The name tauri-specta gives the JobUpdates event (see src/bindings.ts).
const JOB_UPDATES_EVENT = "job-updates";

function job(fields: Partial<JobUpdate> & Pick<JobUpdate, "id" | "seq">): JobUpdate {
  return { kind: "scan", status: "running", progress: null, priority: 0, ...fields };
}

/** Stands in for the Rust side: answers `activity` with `snapshot`. */
function mockApp(snapshot: ActivitySnapshot = { seq: 0, jobs: [] }) {
  mockIPC(
    (cmd) => {
      if (cmd === "activity") return snapshot;
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
}

/** What the Rust side does with each batch of job changes. */
async function send(...updates: JobUpdate[]) {
  await act(() => emit(JOB_UPDATES_EVENT, updates));
}

const status = () => screen.getByRole("status", { name: "Activity" });

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("Activity status", () => {
  it("says there are no background tasks when nothing is queued or running", async () => {
    mockApp();
    render(<Activity />);
    expect(await screen.findByText("No background tasks")).toBe(status());
    expect(status()).toHaveAttribute("data-state", "idle");
  });

  it("shows the running job from the snapshot as soon as it loads", async () => {
    mockApp({ seq: 3, jobs: [job({ seq: 3, id: 1, kind: "fingerprint", progress: 0.42 })] });
    render(<Activity />);
    expect(await screen.findByText("Fingerprinting files (42%)")).toBe(status());
    expect(status()).toHaveAttribute("data-state", "running");
  });

  it("follows a job's updates from queued to done as they arrive", async () => {
    mockApp();
    render(<Activity />);
    await screen.findByText("No background tasks");

    await send(job({ seq: 1, id: 7, status: "queued" }));
    expect(status()).toHaveTextContent("1 task waiting");
    await send(job({ seq: 2, id: 7 }));
    expect(status()).toHaveTextContent(/^Scanning music folders$/);
    await send(job({ seq: 3, id: 7, progress: 0.5 }));
    expect(status()).toHaveTextContent("Scanning music folders (50%)");
    await send(job({ seq: 4, id: 7, status: "done", progress: 1 }));
    expect(status()).toHaveTextContent("No background tasks");
  });

  it("names the job that matters most and counts the others", async () => {
    mockApp({
      seq: 4,
      jobs: [
        job({ seq: 2, id: 1, kind: "fingerprint", priority: -10, progress: 0.1 }),
        job({ seq: 3, id: 2, kind: "analyze", priority: 10, progress: 0.25 }),
        job({ seq: 4, id: 3, kind: "hash", status: "queued" }),
      ],
    });
    render(<Activity />);
    expect(await screen.findByText("Estimating BPM and key (25%) (2 other tasks)")).toBe(status());
    await send(job({ seq: 5, id: 2, kind: "analyze", status: "cancelled" }));
    expect(status()).toHaveTextContent("Fingerprinting files (10%) (1 other task)");
  });

  it("applies a whole batch of updates at once", async () => {
    mockApp();
    render(<Activity />);
    await screen.findByText("No background tasks");
    await send(
      job({ seq: 1, id: 1, status: "done", progress: 1 }),
      job({ seq: 2, id: 2, kind: "hash", progress: 0.3 }),
      job({ seq: 3, id: 3, status: "queued" }),
    );
    expect(status()).toHaveTextContent("Checking files (30%) (1 other task)");
  });

  it("puts every kind of job into words", async () => {
    const kinds = [
      "scan",
      "read",
      "hash",
      "fingerprint",
      "analyze",
      "embed",
      "convert",
      "export",
      "read_rekordbox",
    ] as const;
    mockApp();
    render(<Activity />);
    await screen.findByText("No background tasks");
    const seen: (string | null)[] = [];
    for (const [i, kind] of kinds.entries()) {
      await send(job({ seq: 2 * i + 1, id: i + 1, kind }));
      seen.push(status().textContent);
      await send(job({ seq: 2 * i + 2, id: i + 1, kind, status: "done" }));
    }
    expect(seen).toEqual([
      "Scanning music folders",
      "Reading tags",
      "Checking files",
      "Fingerprinting files",
      "Estimating BPM and key",
      "Running the audio model",
      "Converting files",
      "Exporting",
      "Reading the rekordbox collection",
    ]);
  });

  it("stays idle outside the app, where there is no backend to ask", async () => {
    render(<Activity />);
    await act(async () => {});
    expect(status()).toHaveTextContent("No background tasks");
  });
});

describe("Activity status in both themes", () => {
  let unsync: () => void;

  beforeEach(() => {
    useThemeStore.setState({ theme: DEFAULT_THEME });
    unsync = syncThemeToDocument();
  });

  afterEach(() => {
    unsync();
    delete document.documentElement.dataset.theme;
  });

  // Idle text is muted; busy text is full strength, so work in progress
  // stands out. Both come from theme tokens.
  const cases: {
    state: string;
    text: string;
    token: string;
    snapshot: ActivitySnapshot;
  }[] = [
    {
      state: "idle",
      text: "No background tasks",
      token: "--color-text-muted",
      snapshot: { seq: 0, jobs: [] },
    },
    {
      state: "running",
      text: "Scanning music folders (60%)",
      token: "--color-text",
      snapshot: { seq: 1, jobs: [job({ seq: 1, id: 1, progress: 0.6 })] },
    },
  ];

  it.each(cases)("renders the $state state in the top bar in dark and light", async (c) => {
    const colors: Record<string, string> = {};
    for (const theme of ["dark", "light"] as const) {
      act(() => useThemeStore.setState({ theme }));
      mockApp(c.snapshot);
      const { unmount } = renderApp("/overview");
      const topBar = await screen.findByRole("banner");
      expect(await screen.findByText(c.text)).toBe(status());
      expect(topBar).toContainElement(status());
      expect(status()).toHaveAttribute("data-state", c.state);
      expect(document.documentElement.dataset.theme).toBe(theme);
      colors[theme] = getComputedStyle(status()).getPropertyValue(c.token).trim();
      unmount();
      clearMocks();
    }
    expect(colors.dark).toMatch(/^#[0-9a-f]{6}$/i);
    expect(colors.light).toMatch(/^#[0-9a-f]{6}$/i);
    expect(colors.dark).not.toBe(colors.light);
  });
});

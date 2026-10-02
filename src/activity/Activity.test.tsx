import { act, cleanup, render, screen } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ActivitySnapshot, JobUpdate } from "../bindings";
import { renderApp } from "../app/testApp";
import { DEFAULT_THEME, syncThemeToDocument, useThemeStore } from "../theme/themeStore";
import "../theme/tokens.css";
import { Activity } from "./Activity";
import { tx } from "../test/tx";

// The name tauri-specta gives the JobUpdates event (see src/bindings.ts).
const JOB_UPDATES_EVENT = "job-updates";

// The Activity texts, looked up by key (a test never pins the wording).
const task = (kind: string) => tx(`activity:task.${kind}`);
const percent = (kind: string, value: number) =>
  tx("activity:percent", { task: task(kind), percent: value });

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

const status = () => screen.getByRole("status", { name: tx("activity:label") });

// Unmount first: unmounting stops listening, which needs the mocks.
afterEach(() => {
  cleanup();
  clearMocks();
});

describe("Activity status", () => {
  it("says there are no background tasks when nothing is queued or running", async () => {
    mockApp();
    render(<Activity />);
    expect(await screen.findByText(tx("activity:idle"))).toBe(status());
    expect(status()).toHaveAttribute("data-state", "idle");
  });

  it("shows the running job from the snapshot as soon as it loads", async () => {
    mockApp({ seq: 3, jobs: [job({ seq: 3, id: 1, kind: "fingerprint", progress: 0.42 })] });
    render(<Activity />);
    expect(await screen.findByText(percent("fingerprint", 42))).toBe(status());
    expect(status()).toHaveAttribute("data-state", "running");
  });

  it("follows a job's updates from queued to done as they arrive", async () => {
    mockApp();
    render(<Activity />);
    await screen.findByText(tx("activity:idle"));

    await send(job({ seq: 1, id: 7, status: "queued" }));
    expect(status()).toHaveTextContent(tx("activity:waiting", { count: 1 }));
    await send(job({ seq: 2, id: 7 }));
    expect(status().textContent).toBe(task("scan"));
    await send(job({ seq: 3, id: 7, progress: 0.5 }));
    expect(status()).toHaveTextContent(percent("scan", 50));
    await send(job({ seq: 4, id: 7, status: "done", progress: 1 }));
    expect(status()).toHaveTextContent(tx("activity:idle"));
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
    expect(await screen.findByText(
        tx("activity:andMore", { summary: percent("analyze", 25), count: 2 }),
      )).toBe(status());
    await send(job({ seq: 5, id: 2, kind: "analyze", status: "cancelled" }));
    expect(status()).toHaveTextContent(
      tx("activity:andMore", { summary: percent("fingerprint", 10), count: 1 }),
    );
  });

  it("applies a whole batch of updates at once", async () => {
    mockApp();
    render(<Activity />);
    await screen.findByText(tx("activity:idle"));
    await send(
      job({ seq: 1, id: 1, status: "done", progress: 1 }),
      job({ seq: 2, id: 2, kind: "hash", progress: 0.3 }),
      job({ seq: 3, id: 3, status: "queued" }),
    );
    expect(status()).toHaveTextContent(
      tx("activity:andMore", { summary: percent("hash", 30), count: 1 }),
    );
  });

  it("puts every kind of job into words", async () => {
    const kinds = [
      "scan",
      "read",
      "hash",
      "fingerprint",
      "group",
      "analyze",
      "embed",
      "convert",
      "export",
      "read_rekordbox",
      "relink",
    ] as const;
    mockApp();
    render(<Activity />);
    await screen.findByText(tx("activity:idle"));
    const seen: (string | null)[] = [];
    for (const [i, kind] of kinds.entries()) {
      await send(job({ seq: 2 * i + 1, id: i + 1, kind }));
      seen.push(status().textContent);
      await send(job({ seq: 2 * i + 2, id: i + 1, kind, status: "done" }));
    }
    expect(seen).toEqual(kinds.map((kind) => task(kind)));
  });

  it("stays idle outside the app, where there is no backend to ask", async () => {
    render(<Activity />);
    await act(async () => {});
    expect(status()).toHaveTextContent(tx("activity:idle"));
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
      text: tx("activity:idle"),
      token: "--color-text-muted",
      snapshot: { seq: 0, jobs: [] },
    },
    {
      state: "running",
      text: percent("scan", 60),
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

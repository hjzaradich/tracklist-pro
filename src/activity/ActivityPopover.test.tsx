import { act, cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, describe, expect, it } from "vitest";
import type { ActivitySnapshot, CancelOutcome, JobUpdate } from "../bindings";
import { Activity } from "./Activity";
import { useActivityStore } from "./activityStore";
import { tx } from "../test/tx";

// The name tauri-specta gives the JobUpdates event (see src/bindings.ts).
const JOB_UPDATES_EVENT = "job-updates";

// The Activity texts, looked up by key (a test never pins the wording).
const task = (kind: string) => tx(`activity:task.${kind}`);
const percent = (kind: string, value: number) =>
  tx("activity:percent", { task: task(kind), percent: value });
const queued = (kind: string) => tx("activity:queued", { task: task(kind) });
const stopping = (kind: string) => tx("activity:stopping", { task: task(kind) });

function job(fields: Partial<JobUpdate> & Pick<JobUpdate, "id" | "seq">): JobUpdate {
  return { kind: "scan", status: "running", progress: null, priority: 0, ...fields };
}

/**
 * Stands in for the Rust side: answers `activity` with `snapshot`, and
 * `cancel_job` with `cancel` (a thrown string is an error from the app).
 * Returns the ids it was asked to cancel.
 */
function mockApp(snapshot: ActivitySnapshot, cancel: () => CancelOutcome = () => "cancelled") {
  const cancelled: number[] = [];
  mockIPC(
    (cmd, args) => {
      if (cmd === "activity") return snapshot;
      if (cmd === "cancel_job") {
        cancelled.push((args as { id: number }).id);
        return cancel();
      }
      throw new Error(`unexpected command ${cmd}`);
    },
    { shouldMockEvents: true },
  );
  return cancelled;
}

async function send(...updates: JobUpdate[]) {
  await act(() => emit(JOB_UPDATES_EVENT, updates));
}

const trigger = () => screen.getByRole("button", { name: tx("activity:label") });
/** The panel the status button shows (a disclosure: found by aria-controls). */
const popover = () => {
  const id = trigger().getAttribute("aria-controls");
  const panel = id ? document.getElementById(id) : null;
  if (!panel) throw new Error("the Activity panel isn't open");
  return panel;
};
const isOpen = () => trigger().getAttribute("aria-expanded") === "true";
const rows = () => within(popover()).queryAllByRole("listitem").map((row) => row.firstChild?.textContent);

/** Renders the status with `snapshot` loaded, then opens the popover. */
async function open(snapshot: ActivitySnapshot, cancel?: () => CancelOutcome) {
  const cancelled = mockApp(snapshot, cancel);
  const user = userEvent.setup();
  render(<Activity />);
  // Wait until the snapshot is in.
  await act(async () => {});
  await act(async () => {});
  await user.click(trigger());
  return { user, cancelled };
}

afterEach(() => {
  cleanup();
  clearMocks();
  useActivityStore.getState().reset();
});

describe("Activity popover", () => {
  it("lists running jobs, then queued ones, each with Cancel", async () => {
    await open({
      seq: 4,
      jobs: [
        job({ seq: 1, id: 1, kind: "hash", status: "queued" }),
        job({ seq: 2, id: 2, kind: "fingerprint", priority: -10, progress: 0.1 }),
        job({ seq: 3, id: 3, kind: "export", status: "queued", priority: 10 }),
        job({ seq: 4, id: 4, progress: 0.426 }),
      ],
    });
    expect(trigger()).toHaveAttribute("aria-expanded", "true");
    expect(rows()).toEqual([
      percent("scan", 42),
      percent("fingerprint", 10),
      queued("export"),
      queued("hash"),
    ]);
    expect(within(popover()).getAllByRole("button", { name: tx("activity:cancel") })).toHaveLength(4);
  });

  it("says there are no background tasks when nothing is queued or running", async () => {
    await open({ seq: 0, jobs: [] });
    expect(popover()).toHaveTextContent(tx("activity:idle"));
    expect(within(popover()).queryAllByRole("listitem")).toEqual([]);
  });

  it("follows each job while it's open, and drops it when it finishes", async () => {
    await open({ seq: 1, jobs: [job({ seq: 1, id: 7, progress: 0.2 })] });
    await send(job({ seq: 2, id: 7, progress: 0.55 }), job({ seq: 3, id: 8, kind: "hash", status: "queued" }));
    expect(rows()).toEqual([percent("scan", 55), queued("hash")]);
    await send(job({ seq: 4, id: 7, status: "done", progress: 1 }));
    expect(rows()).toEqual([queued("hash")]);
  });

  it("cancels a queued job, which then leaves the list", async () => {
    const { user, cancelled } = await open({
      seq: 1,
      jobs: [job({ seq: 1, id: 5, kind: "hash", status: "queued" })],
    });
    const row = within(popover()).getByRole("listitem");
    await user.click(within(row).getByRole("button", { name: tx("activity:cancel") }));
    expect(cancelled).toEqual([5]);
    await send(job({ seq: 2, id: 5, kind: "hash", status: "cancelled" }));
    expect(popover()).toHaveTextContent(tx("activity:idle"));
  });

  it("shows a running job as stopping until it stops", async () => {
    const { user, cancelled } = await open(
      { seq: 1, jobs: [job({ seq: 1, id: 9, progress: 0.3 })] },
      () => "stopping",
    );
    const cancel = within(popover()).getByRole("button", { name: tx("activity:cancel") });
    await user.click(cancel);
    expect(cancelled).toEqual([9]);
    expect(rows()).toEqual([stopping("scan")]);
    expect(cancel).toBeDisabled();
    // Its last progress may still arrive; it stays "stopping".
    await send(job({ seq: 2, id: 9, progress: 0.31 }));
    expect(rows()).toEqual([stopping("scan")]);
    await send(job({ seq: 3, id: 9, status: "cancelled", progress: 0.31 }));
    expect(popover()).toHaveTextContent(tx("activity:idle"));
  });

  it("lets Cancel be tried again if the app couldn't cancel", async () => {
    const { user, cancelled } = await open({ seq: 1, jobs: [job({ seq: 1, id: 3 })] }, () => {
      throw "database error";
    });
    const cancel = within(popover()).getByRole("button", { name: tx("activity:cancel") });
    await user.click(cancel);
    expect(cancel).toBeEnabled();
    expect(rows()).toEqual([task("scan")]);
    await user.click(cancel);
    expect(cancelled).toEqual([3, 3]);
  });

  it("describes each Cancel button with its job, for screen readers", async () => {
    await open({
      seq: 2,
      jobs: [job({ seq: 1, id: 1, progress: 0.5 }), job({ seq: 2, id: 2, kind: "hash", status: "queued" })],
    });
    const cancels = within(popover()).getAllByRole("button", { name: tx("activity:cancel") });
    expect(cancels.map((button) => button.getAttribute("aria-describedby"))).not.toContain(null);
    expect(cancels[0]).toHaveAccessibleDescription(percent("scan", 50));
    expect(cancels[1]).toHaveAccessibleDescription(queued("hash"));
  });

  it("is a disclosure panel, not a dialog", async () => {
    await open({ seq: 0, jobs: [] });
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(trigger()).toHaveAttribute("aria-controls", popover().id);
  });

  it("closes on Escape from inside it, giving focus back to the status", async () => {
    const { user } = await open({ seq: 1, jobs: [job({ seq: 1, id: 1 })] });
    const cancel = within(popover()).getByRole("button", { name: tx("activity:cancel") });
    cancel.focus();
    expect(cancel).toHaveFocus();
    await user.keyboard("{Escape}");
    expect(isOpen()).toBe(false);
    expect(screen.queryByRole("button", { name: tx("activity:cancel") })).toBeNull();
    expect(trigger()).toHaveFocus();
  });

  it("closes on a click elsewhere, and the status button toggles it", async () => {
    const { user } = await open({ seq: 0, jobs: [] });
    await user.click(popover());
    expect(isOpen()).toBe(true);
    await user.click(document.body);
    expect(isOpen()).toBe(false);
    expect(trigger()).not.toHaveAttribute("aria-controls");

    await user.click(trigger());
    expect(isOpen()).toBe(true);
    await user.click(trigger());
    expect(isOpen()).toBe(false);
  });

  it("still shows a job as stopping after the panel is closed and reopened", async () => {
    const { user } = await open(
      { seq: 1, jobs: [job({ seq: 1, id: 9, progress: 0.3 })] },
      () => "stopping",
    );
    await user.click(within(popover()).getByRole("button", { name: tx("activity:cancel") }));
    await user.click(trigger());
    expect(isOpen()).toBe(false);
    await user.click(trigger());
    expect(rows()).toEqual([stopping("scan")]);
    expect(within(popover()).getByRole("button", { name: tx("activity:cancel") })).toBeDisabled();
    // Once it stops, nothing is remembered about it.
    await send(job({ seq: 2, id: 9, status: "cancelled" }));
    expect(useActivityStore.getState().stopping.size).toBe(0);
  });
});

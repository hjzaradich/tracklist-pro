import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { KeyComparison } from "../bindings";
import { tx } from "../test/tx";
import { LocalKeyRow } from "./LocalKeyRow";

const COMPARISON: KeyComparison = {
  overall: { tracks: 4, exact: 2, relative: 1, oneStep: 1, parallel: 0, other: 0 },
  genres: [
    { genre: "House", counts: { tracks: 3, exact: 2, relative: 1, oneStep: 0, parallel: 0, other: 0 } },
    { genre: null, counts: { tracks: 1, exact: 0, relative: 0, oneStep: 1, parallel: 0, other: 0 } },
  ],
};

/** Stands in for the Rust side: the switch, and the comparison it answers while on. */
function fakeBackend({ on = false, comparison = COMPARISON }: { on?: boolean; comparison?: KeyComparison } = {}) {
  const backend = { on, calls: [] as string[], features: [] as unknown[] };
  mockIPC((cmd, args) => {
    backend.calls.push(cmd);
    const a = args as Record<string, unknown>;
    switch (cmd) {
      case "feature_enabled":
        backend.features.push(a.feature);
        return backend.on;
      case "set_feature_enabled":
        backend.features.push(a.feature);
        backend.on = a.on as boolean;
        return null;
      case "compare_local_keys":
        return backend.on ? comparison : null;
      default:
        throw new Error(`unexpected command ${cmd}`);
    }
  });
  return backend;
}

function renderRow() {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <LocalKeyRow />
    </QueryClientProvider>,
  );
  return screen.getByRole("checkbox", { name: tx("estimate:localKey.label") });
}

describe("LocalKeyRow", () => {
  afterEach(() => {
    cleanup();
    clearMocks();
  });

  it("is off by default and never asks for the comparison while off", async () => {
    const backend = fakeBackend();
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    expect(box).not.toBeChecked();
    expect(screen.getByText(tx("estimate:localKey.helpOff"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
    expect(backend.calls).not.toContain("compare_local_keys");
    expect(backend.features.every((f) => f === "localKey")).toBe(true);
  });

  it("turned on, shows the counts overall and per genre with their shares", async () => {
    const backend = fakeBackend();
    const box = renderRow();
    await waitFor(() => expect(box).toBeEnabled());
    await userEvent.click(box);
    const table = await screen.findByRole("table", { name: tx("estimate:localKey.compare.title") });
    expect(backend.on).toBe(true);
    expect(table).toHaveTextContent(tx("estimate:localKey.compare.all"));
    expect(table).toHaveTextContent("House");
    expect(table).toHaveTextContent(tx("estimate:localKey.compare.noGenre"));
    expect(table).toHaveTextContent(tx("estimate:localKey.compare.cell", { value: 2, percent: 50 }));
    expect(table).toHaveTextContent(tx("estimate:localKey.compare.cell", { value: 2, percent: 67 }));
  });

  it("says so when no track has both keys yet", async () => {
    const empty = { tracks: 0, exact: 0, relative: 0, oneStep: 0, parallel: 0, other: 0 };
    fakeBackend({ on: true, comparison: { overall: empty, genres: [] } });
    renderRow();
    expect(await screen.findByText(tx("estimate:localKey.compare.empty"))).toBeInTheDocument();
    expect(screen.queryByRole("table")).not.toBeInTheDocument();
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../../app/queryClient";
import type { Crate, SmartRule, TableQuery } from "../../bindings";
import "../../i18n";
import { tx } from "../../test/tx";
import { RuleBuilder } from "./RuleBuilder";
import { clampNumber, EMPTY_RULE, MAX_CONDITIONS, newCondition, RANGES } from "./rule";

const CRATES: Crate[] = [
  { id: 1, parentId: null, kind: "static", name: "Picked", trackCount: 3 },
  { id: 2, parentId: null, kind: "smart", name: "Peak", trackCount: 0 },
];

/** The rules the live count was asked about, newest last. */
let asked: SmartRule[];

beforeEach(() => {
  asked = [];
  mockIPC((cmd, rawArgs) => {
    if (cmd !== "track_table_page") throw new Error(`unexpected ${cmd}`);
    const { query } = rawArgs as { query: TableQuery };
    if (query.source.kind !== "smart" || query.source.rule === null) throw new Error("not a draft");
    asked.push(query.source.rule);
    // One track per condition, so the count shows which draft was asked.
    return { total: query.source.rule.conditions.length, offset: 0, rows: [], behind: false };
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

function show(saved: SmartRule, onSave = vi.fn()) {
  render(
    <QueryClientProvider client={createQueryClient()}>
      <RuleBuilder crateId={2} saved={saved} all={CRATES} pending={false} onSave={onSave} />
    </QueryClientProvider>,
  );
  return onSave;
}

describe("RuleBuilder", () => {
  it("builds a rule from dropdowns, counts it live and saves it", async () => {
    const user = userEvent.setup();
    const onSave = show(EMPTY_RULE);
    expect(await screen.findByText(tx("smartCrates:builder.count", { count: 0 }))).toBeTruthy();
    expect(screen.getByText(tx("smartCrates:builder.none"))).toBeTruthy();

    await user.click(screen.getByRole("button", { name: tx("smartCrates:builder.add") }));
    expect(await screen.findByText(tx("smartCrates:builder.count", { count: 1 }))).toBeTruthy();

    // Energy at least 7, made BPM between 120 and 126 at half/double time.
    await user.selectOptions(
      screen.getByRole("combobox", { name: tx("smartCrates:builder.fieldFor", { n: 1 }) }),
      "bpm",
    );
    await user.click(screen.getByRole("checkbox", { name: tx("smartCrates:builder.halfDouble") }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: tx("smartCrates:builder.combineLabel") }),
      "any",
    );
    await waitFor(() => expect(asked.at(-1)?.combine).toBe("any"));

    await user.click(screen.getByRole("button", { name: tx("smartCrates:builder.save") }));
    expect(onSave).toHaveBeenCalledWith({
      combine: "any",
      conditions: [{ field: "bpm", op: "between", value: 120, to: 126, halfDouble: true }],
    });
  });

  it("offers hand-made crates for 'in crate' and drops unsaved changes", async () => {
    const user = userEvent.setup();
    const saved: SmartRule = { combine: "all", conditions: [newCondition("energy", null)] };
    show(saved);
    await user.selectOptions(
      screen.getByRole("combobox", { name: tx("smartCrates:builder.fieldFor", { n: 1 }) }),
      "inCrate",
    );
    const choice = screen.getByRole("combobox", { name: tx("smartCrates:builder.valueFor", { n: 1 }) });
    expect([...choice.querySelectorAll("option")].map((o) => o.textContent)).toEqual(["Picked"]);
    await user.click(screen.getByRole("button", { name: tx("smartCrates:builder.discard") }));
    expect(screen.queryByRole("button", { name: tx("smartCrates:builder.save") })).toBeNull();
    expect(
      (screen.getByRole("combobox", { name: tx("smartCrates:builder.fieldFor", { n: 1 }) }) as HTMLSelectElement)
        .value,
    ).toBe("energy");
  });

  it("stops adding conditions at the most a rule holds", () => {
    const full: SmartRule = {
      combine: "all",
      conditions: Array.from({ length: MAX_CONDITIONS }, () => newCondition("genre", null)),
    };
    show(full);
    expect(screen.getByRole("button", { name: tx("smartCrates:builder.add") }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText(tx("smartCrates:builder.full", { count: MAX_CONDITIONS }))).toBeTruthy();
  });

  it("asks for the live count once the rule has rested, not on every keystroke", async () => {
    const user = userEvent.setup();
    show({ combine: "all", conditions: [{ field: "bpm", op: "is", value: 1, to: null, halfDouble: false }] });
    await screen.findByText(tx("smartCrates:builder.count", { count: 1 }));
    const before = asked.length;
    const box = screen.getByRole("spinbutton", { name: tx("smartCrates:builder.valueFor", { n: 1 }) });
    await user.type(box, "26");
    // Each keystroke changed the draft; none of them asked.
    expect(asked.length).toBe(before);
    await waitFor(() => expect(asked.length).toBe(before + 1));
    const last = asked.at(-1)?.conditions[0];
    expect(last && "value" in last ? last.value : null).toBe(126);
  });

  it("refuses to save a genre “contains” with nothing typed", async () => {
    const user = userEvent.setup();
    const onSave = show(EMPTY_RULE);
    await user.click(screen.getByRole("button", { name: tx("smartCrates:builder.add") }));
    await user.selectOptions(
      screen.getByRole("combobox", { name: tx("smartCrates:builder.fieldFor", { n: 1 }) }),
      "genre",
    );
    expect(screen.getByText(tx("smartCrates:builder.textEmpty"))).toBeTruthy();
    const save = screen.getByRole("button", { name: tx("smartCrates:builder.save") });
    expect(save).toBeDisabled();
    await user.click(save);
    expect(onSave).not.toHaveBeenCalled();
    await user.type(screen.getByRole("textbox", { name: tx("smartCrates:builder.valueFor", { n: 1 }) }), "house");
    expect(screen.queryByText(tx("smartCrates:builder.textEmpty"))).toBeNull();
    expect(save).toBeEnabled();
  });

  it("offers ratings from one star: no star is no rating", () => {
    expect(RANGES.rating).toEqual([1, 5]);
    expect(clampNumber("rating", "0")).toBe(1);
  });
});

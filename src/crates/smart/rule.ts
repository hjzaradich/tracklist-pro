import type { Condition, SmartRule } from "../../bindings";

/** The most conditions a rule holds (the Rust side's `MAX_CONDITIONS`). */
export const MAX_CONDITIONS = 32;

/** The glyph that marks a smart crate in the lists (proposed). */
export const SMART_ICON = "✦";

export type Field = Condition["field"];
export type Op = Condition["op"];

/** The fields the builder offers, in its dropdown's order. */
export const FIELDS: readonly Field[] = ["energy", "bpm", "key", "genre", "rating", "inCrate", "tag", "tagGroup"];

const NUMBER_OPS = ["atLeast", "atMost", "between", "is", "isNot"] as const;

/** The operators each field offers. */
export const OPS: Record<Field, readonly Op[]> = {
  energy: NUMBER_OPS,
  bpm: NUMBER_OPS,
  rating: NUMBER_OPS,
  key: ["is", "isNot"],
  genre: ["is", "isNot", "contains"],
  inCrate: ["is", "isNot"],
  tag: ["has", "hasNot"],
  tagGroup: ["has", "hasNot"],
};

/** Every Camelot key, for the key dropdown. */
export const KEYS: readonly string[] = Array.from({ length: 12 }, (_, i) => i + 1).flatMap((n) => [
  `${n}A`,
  `${n}B`,
]);

/** The smallest and largest value a number field takes. */
export const RANGES: Record<"energy" | "bpm" | "rating", [number, number]> = {
  energy: [1, 10],
  bpm: [1, 999],
  // 0 stars is no rating, which no rule compares with.
  rating: [1, 5],
};

/** A new condition on `field`, with a value that makes sense for it. */
export function newCondition(
  field: Field,
  firstCrate: number | null,
  /** The first tag and the first tag group, for the tag fields. */
  firstTag: { tag: number; group: number } | null = null,
): Condition {
  switch (field) {
    case "energy":
      return { field, op: "atLeast", value: 7, to: null };
    case "bpm":
      return { field, op: "between", value: 120, to: 126, halfDouble: false };
    case "rating":
      return { field, op: "atLeast", value: 4, to: null };
    case "key":
      return { field, op: "is", value: "8A" };
    case "genre":
      return { field, op: "contains", value: "" };
    case "inCrate":
      return { field, op: "is", crateId: firstCrate ?? 0 };
    case "tag":
      return { field, op: "has", tagId: firstTag?.tag ?? 0 };
    case "tagGroup":
      return { field, op: "has", groupId: firstTag?.group ?? 0 };
  }
}

/** `condition` with operator `op`, if the field offers it. */
export function withOp(condition: Condition, op: string): Condition {
  const offered = OPS[condition.field] as readonly string[];
  if (!offered.includes(op)) return condition;
  if (condition.field === "energy" || condition.field === "bpm" || condition.field === "rating") {
    const op_ = op as (typeof NUMBER_OPS)[number];
    return { ...condition, op: op_, to: op_ === "between" ? (condition.to ?? condition.value) : null };
  }
  if (condition.field === "genre") return { ...condition, op: op as typeof condition.op };
  if (condition.field === "tag" || condition.field === "tagGroup") {
    return { ...condition, op: op as "has" | "hasNot" };
  }
  return { ...condition, op: op as "is" | "isNot" };
}

/** Which tags and groups the vocabulary has now: a tag condition names one that isn't there as gone. */
export function tagIsGone(condition: Condition, groups: readonly { id: number; tags: readonly { id: number }[] }[]) {
  if (condition.field === "tag") return !groups.some((g) => g.tags.some((t) => t.id === condition.tagId));
  if (condition.field === "tagGroup") return !groups.some((g) => g.id === condition.groupId);
  return false;
}

/** A whole number from what was typed, kept in `field`'s range; `null` for nothing usable. */
export function clampNumber(field: "energy" | "bpm" | "rating", typed: string): number | null {
  const n = Number.parseInt(typed, 10);
  if (Number.isNaN(n)) return null;
  const [low, high] = RANGES[field];
  return Math.min(high, Math.max(low, n));
}

/** An empty rule: matches nothing until a condition is added. */
export const EMPTY_RULE: SmartRule = { combine: "all", conditions: [] };

/** Whether a genre condition still has no text to compare with (it would match nothing). */
export function hasEmptyText(rule: SmartRule): boolean {
  return rule.conditions.some((c) => c.field === "genre" && c.op === "contains" && c.value.trim() === "");
}

/** Whether two rules say the same thing. */
export function sameRule(a: SmartRule, b: SmartRule): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

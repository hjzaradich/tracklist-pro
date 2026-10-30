import { describe, expect, it } from "vitest";
import type { FitReason, RowFit } from "../bindings";
import i18n from "../i18n";
import { tx } from "../test/tx";
import { fitText, reasonText, signedBpm } from "./fitText";

const t = i18n.getFixedT("en", "trackTable");
const sep = () => tx("trackTable:fit.separator");

const reason = (key: FitReason["key"], bpm: FitReason["bpm"]): FitReason => ({ key, bpm });

describe("the words of a fit reason", () => {
  it.each([
    [1.5, "+1.5"],
    [-2, "-2.0"],
    [0.04, "+0.0"],
    [12.34, "+12.3"],
  ])("writes a BPM difference of %s with its sign as %s", (delta, written) => {
    expect(signedBpm(delta)).toBe(written);
  });

  it("gives the key change and then the signed BPM difference", () => {
    const text = reasonText(
      reason({ kind: "neighbor", from: "8A", to: "9A" }, { kind: "delta", delta: 1.5, time: "straight" }),
      t,
      "camelot",
    );
    expect(text).toBe(
      [tx("trackTable:fit.keyChange", { from: "8A", to: "9A" }), tx("trackTable:fit.bpm", { delta: "+1.5" })].join(
        sep(),
      ),
    );
  });

  it("writes keys in the user's notation, not only Camelot", () => {
    const text = reasonText(
      reason({ kind: "neighbor", from: "8A", to: "9A" }, { kind: "noBpm" }),
      t,
      "musical_standard",
    );
    // 8A is A minor and 9A is E minor.
    expect(text).toContain(tx("trackTable:fit.keyChange", { from: "Am", to: "Em" }));
  });

  it.each([
    [
      "the same key",
      reason({ kind: "same", key: "8A" }, { kind: "delta", delta: 0, time: "straight" }),
      () => [tx("trackTable:fit.sameKey", { key: "8A" }), tx("trackTable:fit.sameBpm")],
    ],
    [
      "a key that is not a neighbor",
      reason({ kind: "far", from: "8A", to: "3B" }, { kind: "delta", delta: -6, time: "straight" }),
      () => [tx("trackTable:fit.keyFar", { from: "8A", to: "3B" }), tx("trackTable:fit.bpm", { delta: "-6.0" })],
    ],
    [
      "half time",
      reason({ kind: "same", key: "8A" }, { kind: "delta", delta: 1, time: "half" }),
      () => [
        tx("trackTable:fit.sameKey", { key: "8A" }),
        tx("trackTable:fit.halfTime", { bpm: tx("trackTable:fit.bpm", { delta: "+1.0" }) }),
      ],
    ],
    [
      "double time with no difference",
      reason({ kind: "same", key: "8A" }, { kind: "delta", delta: 0, time: "double" }),
      () => [
        tx("trackTable:fit.sameKey", { key: "8A" }),
        tx("trackTable:fit.doubleTime", { bpm: tx("trackTable:fit.sameBpm") }),
      ],
    ],
    [
      "a track with neither key nor tempo",
      reason({ kind: "noKey" }, { kind: "noBpm" }),
      () => [tx("trackTable:fit.noKey"), tx("trackTable:fit.noBpm")],
    ],
    [
      "a target without a key",
      reason({ kind: "noTargetKey" }, { kind: "delta", delta: 1.5, time: "straight" }),
      () => [tx("trackTable:fit.noTargetKey"), tx("trackTable:fit.bpm", { delta: "+1.5" })],
    ],
    [
      "a target without a tempo",
      reason({ kind: "same", key: "8A" }, { kind: "noTargetBpm" }),
      () => [tx("trackTable:fit.sameKey", { key: "8A" }), tx("trackTable:fit.noTargetBpm")],
    ],
  ])("says %s", (_, given, parts) => {
    expect(reasonText(given, t, "camelot")).toBe(parts().join(sep()));
  });

  it("says once that the selected track has neither a key nor a BPM", () => {
    expect(reasonText(reason({ kind: "noTargetKey" }, { kind: "noTargetBpm" }), t, "camelot")).toBe(
      tx("trackTable:fit.noTarget"),
    );
  });

  it("says that the selected track is the selected track, and nothing for a row with no fit", () => {
    const selected: RowFit = { dots: 0, reason: null };
    expect(fitText(selected, t, "camelot")).toBe(tx("trackTable:fit.selected"));
    expect(fitText(null, t, "camelot")).toBe("");
  });
});

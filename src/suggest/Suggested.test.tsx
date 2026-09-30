import { render, screen } from "@testing-library/react";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import type { Reason, Suggestion } from "../bindings";
import i18n from "../i18n";
import { reasonProblem } from "./reasons";
import { Suggested } from "./Suggested";

// Stand-in reason texts, loaded only for these tests. Real reasons live in
// each feature's own namespace file once the owner has worded them.
beforeAll(() => {
  i18n.addResourceBundle("en", "suggestTest", {
    reason: {
      sameKey: "same key",
      nearBpm: "+{{bpm}} BPM",
      tagged: "tagged {{tag}}",
      soundsDark: "sounds dark",
      played_one: "follows this track in your history once",
      played_other: "follows this track in your history {{count}} times",
      withFormat: "{{bpm, number}} BPM",
    },
  });
});

let consoleError: ReturnType<typeof vi.spyOn>;
beforeEach(() => {
  consoleError = vi.spyOn(console, "error").mockImplementation(() => {});
});

function fact(key: string, params: Reason["params"] = {}): Reason {
  return { key: `suggestTest:reason.${key}`, params, source: "fact" };
}

function audio(key: string): Reason {
  return { key: `suggestTest:reason.${key}`, params: {}, source: "audioModel" };
}

function suggestion(reasons: Reason[]): Suggestion<string> {
  return { what: "Track A", reasons };
}

/** The paragraph whose whole text is `text`, even when split across spans. */
function line(text: string) {
  return screen.getByText((_, el) => el?.tagName === "P" && el.textContent === text);
}

function renderSuggested(s: Suggestion<string>) {
  return render(
    <Suggested suggestion={s}>
      <span>{s.what}</span>
    </Suggested>,
  );
}

describe("a suggestion is shown with its reasons", () => {
  it("shows the suggestion and its reasons as one line of fragments", () => {
    renderSuggested(
      suggestion([fact("sameKey"), fact("nearBpm", { bpm: 2 }), fact("tagged", { tag: "Peak Time" })]),
    );
    expect(screen.getByText("Track A")).toBeInTheDocument();
    expect(line("same key, +2 BPM, tagged Peak Time")).toBeInTheDocument();
    expect(consoleError).not.toHaveBeenCalled();
  });

  it("labels the audio model's opinion as such, and only that reason", () => {
    const { container } = renderSuggested(suggestion([fact("sameKey"), audio("soundsDark")]));
    expect(line("same key, sounds dark (audio model)")).toBeInTheDocument();
    const fragments = container.querySelectorAll("[data-source]");
    expect([...fragments].map((f) => [f.getAttribute("data-source"), f.textContent])).toEqual([
      ["fact", "same key"],
      ["audioModel", ", sounds dark (audio model)"],
    ]);
  });

  it("picks the plural form from a count parameter", () => {
    renderSuggested(suggestion([fact("played", { count: 1 })]));
    expect(line("follows this track in your history once")).toBeInTheDocument();
    renderSuggested(suggestion([fact("played", { count: 4 })]));
    expect(line("follows this track in your history 4 times")).toBeInTheDocument();
  });

  it("fills parameters only into the text, even when named like an i18next option", () => {
    // Passed as options, `keySeparator` would break the key lookup (raw key
    // shown) and `returnDetails` would return an object (render crash).
    renderSuggested(
      suggestion([fact("sameKey", { keySeparator: "x", returnDetails: "yes", ns: "common" })]),
    );
    expect(line("same key")).toBeInTheDocument();
  });

  it("never shows a raw reason key", () => {
    const { container } = renderSuggested(suggestion([fact("sameKey")]));
    expect(container.textContent).not.toContain("suggestTest:");
  });
});

describe("the UI refuses to render a suggestion that breaks the rules", () => {
  it("renders nothing at all, not even the suggestion, when it has no reason", () => {
    const { container } = renderSuggested(suggestion([]));
    expect(container).toBeEmptyDOMElement();
    expect(screen.queryByText("Track A")).not.toBeInTheDocument();
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining("noReason"),
      expect.anything(),
    );
  });

  it("renders nothing when the reasons are missing from a malformed payload", () => {
    const malformed = { what: "Track A" } as unknown as Suggestion<string>;
    const { container } = renderSuggested(malformed);
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing when the audio model is the only reason", () => {
    const { container } = renderSuggested(suggestion([audio("soundsDark")]));
    expect(container).toBeEmptyDOMElement();
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining("onlyAudioModel"),
      expect.anything(),
    );
  });

  it("renders nothing when a reason's key has no text in the locale files", () => {
    const { container } = renderSuggested(suggestion([fact("sameKey"), fact("noSuchReason")]));
    expect(container).toBeEmptyDOMElement();
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining("unknownKey"),
      expect.anything(),
    );
  });

  it("renders nothing when the text needs a parameter the reason doesn't give", () => {
    const { container } = renderSuggested(suggestion([fact("nearBpm")]));
    expect(container).toBeEmptyDOMElement();
    expect(container.textContent).not.toContain("{{");
    expect(consoleError).toHaveBeenCalledWith(
      expect.stringContaining("missingParam"),
      expect.anything(),
    );
  });

  it("renders nothing when a key names a group of texts, not one text", () => {
    const group: Reason = { key: "suggestTest:reason", params: {}, source: "fact" };
    const { container } = renderSuggested(suggestion([group]));
    expect(container).toBeEmptyDOMElement();
  });

  it("renders nothing when a reason's source isn't exactly fact or audioModel", () => {
    for (const source of ["vibes", "Fact", undefined]) {
      const odd = { ...fact("sameKey"), source } as unknown as Reason;
      const { container } = renderSuggested(suggestion([odd, fact("sameKey")]));
      expect(container).toBeEmptyDOMElement();
    }
  });

  it("renders nothing when a parameter has no value", () => {
    const { container } = renderSuggested(suggestion([fact("nearBpm", { bpm: null })]));
    expect(container).toBeEmptyDOMElement();
  });
});

describe("reasonProblem", () => {
  it("accepts facts, and the audio model alongside a fact", () => {
    expect(reasonProblem([fact("sameKey")])).toBeNull();
    expect(reasonProblem([audio("soundsDark"), fact("sameKey")])).toBeNull();
  });

  it("names the rule a set of reasons breaks", () => {
    expect(reasonProblem([])).toBe("noReason");
    expect(reasonProblem(undefined)).toBe("noReason");
    expect(reasonProblem(null)).toBe("noReason");
    expect(reasonProblem("same key")).toBe("noReason");
    expect(reasonProblem([audio("soundsDark"), audio("soundsDark")])).toBe("onlyAudioModel");
    expect(reasonProblem([fact("nope")])).toBe("unknownKey");
    expect(reasonProblem([{ ...fact("sameKey"), key: "same key, +2 BPM" }])).toBe("unknownKey");
    expect(reasonProblem([fact("nearBpm", { bpm: Number.NaN })])).toBe("badParam");
    expect(reasonProblem([{ ...fact("sameKey"), params: null } as unknown as Reason])).toBe(
      "badParam",
    );
    expect(reasonProblem([fact("nearBpm")])).toBe("missingParam");
    expect(reasonProblem([fact("withFormat")])).toBe("missingParam");
    expect(reasonProblem([fact("withFormat", { bpm: 128 })])).toBeNull();
    // Plural texts exist only as played_one / played_other: without a count
    // there's no text to pick.
    expect(reasonProblem([fact("played")])).toBe("unknownKey");
    expect(reasonProblem([{ ...fact("sameKey"), source: "vibes" } as unknown as Reason])).toBe(
      "badSource",
    );
  });
});

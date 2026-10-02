import { afterAll, beforeAll, describe, expect, it } from "vitest";
import i18n, { englishResources } from "../i18n";
import { marked } from "./copyMarkers";
import { tx } from "./tx";

// Texts of a namespace made for these tests, so the literals here are the
// point (they're not the app's wording).
beforeAll(() => {
  i18n.addResourceBundle("en", "txFixture", {
    plain: "Plain text",
    greeting: "Hello {{name}}",
    total: "{{n, number}} in total",
    group: { inner: "Inner text" },
    things_one: "{{count}} thing",
    things_other: "{{count}} things",
  });
});
afterAll(() => {
  i18n.removeResourceBundle("en", "txFixture");
});

describe("tx, the English text of a key for tests", () => {
  it("returns the text of a key", () => {
    expect(tx("txFixture:plain")).toBe("Plain text");
    expect(tx("txFixture:group.inner")).toBe("Inner text");
  });

  it("fills the placeholders from the params, formats included", () => {
    expect(tx("txFixture:greeting", { name: "Sam" })).toBe("Hello Sam");
    expect(tx("txFixture:total", { n: 1234 })).toBe("1,234 in total");
  });

  it("picks the plural form from count", () => {
    expect(tx("txFixture:things", { count: 1 })).toBe("1 thing");
    expect(tx("txFixture:things", { count: 3 })).toBe("3 things");
    expect(tx("txFixture:things", { count: 0 })).toBe("0 things");
  });

  it("looks a key up in the app's real resources", () => {
    expect(tx("common:appName")).toBe(i18n.t("common:appName"));
    expect(tx("theme:label")).toBe(englishResources.theme.label);
  });

  it("throws, naming the key, when the key has no text, instead of returning the key", () => {
    expect(() => tx("txFixture:nope")).toThrow('no English text for "txFixture:nope"');
    expect(() => tx("noSuchNamespace:plain")).toThrow('no English text for "noSuchNamespace:plain"');
    expect(() => tx("txFixture:things")).toThrow('no English text for "txFixture:things"');
  });

  it("throws when the key names a group of texts, not one text", () => {
    expect(() => tx("txFixture:group")).toThrow("a group of texts");
  });

  it("throws when a placeholder is left unfilled, naming it", () => {
    expect(() => tx("txFixture:greeting")).toThrow("{{name}}");
    expect(() => tx("txFixture:greeting", { other: "x" })).toThrow("{{name}}");
  });
});

describe("copy markers", () => {
  it("name each text's key and keep its placeholders, plural forms included", () => {
    expect(
      marked({ a: "Text", b: { c: "Hi {{name}}", d_one: "{{count, number}} one", d_other: "{{count, number}} many" } }, "ns"),
    ).toEqual({
      a: "[[ns:a]]",
      b: {
        c: "[[ns:b.c]] {{name}}",
        d_one: "[[ns:b.d_one]] {{count, number}}",
        d_other: "[[ns:b.d_other]] {{count, number}}",
      },
    });
  });

  it.runIf(import.meta.env.VITE_COPY_MARKERS === "1")(
    "replace the real texts when switched on (npm run test:markers)",
    () => {
      expect(tx("common:appName")).toBe("[[common:appName]]");
    },
  );

  it.runIf(import.meta.env.VITE_COPY_MARKERS !== "1")("leave the real texts alone otherwise", () => {
    expect(tx("common:appName")).not.toContain("[[");
  });
});

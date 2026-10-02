import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CratesScreen } from "../crates/CratesScreen";
import i18n, { englishResources, namespaceFromPath, namespaces } from "./index";
import { tx } from "../test/tx";

type Messages = { [key: string]: string | Messages };

function allValuesAreStrings(messages: Messages): boolean {
  return Object.values(messages).every((value) =>
    typeof value === "string" ? true : allValuesAreStrings(value),
  );
}

describe("i18n", () => {
  it("runs in English", () => {
    expect(i18n.language).toBe("en");
  });

  it("loads each file in src/locales/en as its own namespace", () => {
    expect(namespaces).toEqual(expect.arrayContaining(["common", "crates", "overview", "theme"]));
    for (const ns of namespaces) {
      expect(i18n.hasResourceBundle("en", ns)).toBe(true);
    }
  });

  it("names a namespace after its file", () => {
    expect(namespaceFromPath("../locales/en/crates.json")).toBe("crates");
  });

  it("namespace files hold only text (nested groups allowed)", () => {
    for (const [ns, messages] of Object.entries(englishResources)) {
      expect(allValuesAreStrings(messages), `namespace "${ns}"`).toBe(true);
    }
  });

  it("a component renders a string from its namespace", () => {
    // A screen that asks the backend for nothing, so its text is there at once.
    render(<CratesScreen />);
    expect(screen.getByRole("heading", { name: tx("shell:stages.crates") })).toBeInTheDocument();
    expect(
      screen.getByText(tx("crates:empty")),
    ).toBeInTheDocument();
  });

  it("translates a key from a named namespace", () => {
    expect(i18n.t("label", { ns: "theme" })).toBe(englishResources.theme.label);
    // Without a namespace, the default one (common) is used.
    expect(i18n.t("appName")).toBe(englishResources.common.appName);
  });
});

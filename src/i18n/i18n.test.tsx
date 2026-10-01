import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CratesScreen } from "../crates/CratesScreen";
import i18n, { englishResources, namespaceFromPath, namespaces } from "./index";

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

  it("a component renders an English string from its namespace", () => {
    // A screen that asks the backend for nothing, so its text is there at once.
    render(<CratesScreen />);
    expect(screen.getByRole("heading", { name: "Crates" })).toBeInTheDocument();
    expect(
      screen.getByText("Groups of Library tracks, collected for a purpose."),
    ).toBeInTheDocument();
  });

  it("translates a key from a named namespace", () => {
    expect(i18n.t("label", { ns: "theme" })).toBe("Theme");
    expect(i18n.t("appName")).toBe("tracklist-pro");
  });
});

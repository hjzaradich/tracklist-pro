import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { ThemeSwitch } from "./ThemeSwitch";
import {
  DEFAULT_THEME,
  THEME_STORAGE_KEY,
  syncThemeToDocument,
  useThemeStore,
} from "./themeStore";

function savedTheme(): string | undefined {
  const raw = localStorage.getItem(THEME_STORAGE_KEY);
  return raw ? JSON.parse(raw).state?.theme : undefined;
}

describe("theme switch", () => {
  let unsync: () => void;

  beforeEach(() => {
    useThemeStore.setState({ theme: DEFAULT_THEME });
    localStorage.clear();
    delete document.documentElement.dataset.theme;
    unsync = syncThemeToDocument();
  });

  afterEach(() => unsync());

  it("starts in the dark theme when nothing is saved", () => {
    expect(DEFAULT_THEME).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("labels itself and its options in English from the theme namespace", () => {
    render(<ThemeSwitch />);
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Dark" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Light" })).not.toBeChecked();
  });

  it("flips the page to the light theme and back", async () => {
    const user = userEvent.setup();
    render(<ThemeSwitch />);

    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(screen.getByRole("radio", { name: "Light" })).toBeChecked();

    await user.click(screen.getByRole("radio", { name: "Dark" }));
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("saves the choice so it survives a restart", async () => {
    const user = userEvent.setup();
    render(<ThemeSwitch />);
    await user.click(screen.getByRole("radio", { name: "Light" }));
    expect(savedTheme()).toBe("light");
  });

  it("restores the saved choice on the next start", async () => {
    localStorage.setItem(
      THEME_STORAGE_KEY,
      JSON.stringify({ state: { theme: "light" }, version: 1 }),
    );
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState().theme).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("falls back to dark when the saved value is unreadable", async () => {
    localStorage.setItem(
      THEME_STORAGE_KEY,
      JSON.stringify({ state: { theme: "neon" }, version: 1 }),
    );
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState().theme).toBe("dark");
  });
});

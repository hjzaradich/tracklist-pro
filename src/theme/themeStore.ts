import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

export const THEMES = ["dark", "light"] as const;
export type Theme = (typeof THEMES)[number];

export const DEFAULT_THEME: Theme = "dark";

// Persisted in the webview's localStorage for now. When the `setting` table
// arrives (Stage 0D), this moves to the backend so it survives a data-folder
// backup and restore.
export const THEME_STORAGE_KEY = "tracklist-pro.theme";

interface ThemeState {
  theme: Theme;
  setTheme: (theme: Theme) => void;
}

function isTheme(value: unknown): value is Theme {
  return THEMES.includes(value as Theme);
}

export const useThemeStore = create<ThemeState>()(
  persist(
    (set) => ({
      theme: DEFAULT_THEME,
      setTheme: (theme) => set({ theme }),
    }),
    {
      name: THEME_STORAGE_KEY,
      version: 1,
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({ theme: state.theme }),
      // A corrupted or unknown stored value falls back to the default.
      merge: (persisted, current) => {
        const stored = (persisted as Partial<ThemeState> | undefined)?.theme;
        return { ...current, theme: isTheme(stored) ? stored : DEFAULT_THEME };
      },
    },
  ),
);

/** Puts the theme on <html data-theme="…">, where tokens.css picks it up. */
export function applyTheme(theme: Theme, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = theme;
}

/** Applies the current theme now and whenever it changes. Returns an unsubscribe. */
export function syncThemeToDocument(): () => void {
  applyTheme(useThemeStore.getState().theme);
  return useThemeStore.subscribe((state) => applyTheme(state.theme));
}

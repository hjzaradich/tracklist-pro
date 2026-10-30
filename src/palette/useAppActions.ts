import { useNavigate } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { useLibrarySummary } from "../library/useLibrarySummary";
import { sendAvailable } from "../send/sendAvailable";
import { STAGES } from "../shell/stages";
import { THEMES, useThemeStore } from "../theme/themeStore";
import { type PaletteAction, useRegisterActions } from "./registry";
import { useRequests } from "./requests";

/**
 * The palette's own actions: the app's screens, the search box, the send and
 * the theme. Each one only does what a click on the sidebar, a button or the
 * theme switch does, through the same function. Undo registers itself, from
 * the top bar's button (`UndoButton`), so it is that button's handler.
 */
export function useAppActions() {
  const { t } = useTranslation(["palette", "shell", "afterSend", "send", "theme"]);
  const navigate = useNavigate();
  const ask = useRequests((state) => state.ask);
  const library = useLibrarySummary();
  const theme = useThemeStore((state) => state.theme);
  const setTheme = useThemeStore((state) => state.setTheme);

  const goTo = (id: string, name: string, to: (typeof STAGES)[number]["path"] | "/after-send") => ({
    id: `go:${id}`,
    label: t("palette:goTo", { name }),
    run: () => void navigate({ to }),
  });

  const actions: PaletteAction[] = [
    ...STAGES.map((stage) => goTo(stage.id, t(`shell:stages.${stage.id}`), stage.path)),
    goTo("afterSend", t("afterSend:page"), "/after-send"),
    {
      id: "searchAllMusic",
      label: t("palette:searchAllMusic"),
      run: () => {
        ask("searchAllMusic");
        void navigate({ to: "/all-music" });
      },
    },
    {
      // Opens the guided send, which sends nothing until its own steps say so.
      id: "send",
      label: t("send:open"),
      // The Overview's Send button is greyed out under the same rule.
      available: sendAvailable(library.data?.tracks),
      run: () => {
        ask("send");
        void navigate({ to: "/overview" });
      },
    },
    ...THEMES.map((option) => ({
      id: `theme:${option}`,
      label: t("palette:theme", { name: t(`theme:${option}`) }),
      available: theme !== option,
      run: () => setTheme(option),
    })),
  ];
  useRegisterActions("app", actions);
}

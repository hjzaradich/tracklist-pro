import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { STAGES } from "../shell/stages";
import { renderApp } from "./testApp";

// Each stage's name and its screen's placeholder line, in English.
const SCREENS = [
  { path: "/overview", name: "Overview", empty: "Your Library at a glance, and what to do next." },
  { path: "/review", name: "Review", empty: "Every decision the app can't make on its own." },
  { path: "/all-music", name: "All music", empty: "Every track the scan found in your music folders." },
  { path: "/library", name: "Library", empty: "The tracks you've added to play with." },
  { path: "/crates", name: "Crates", empty: "Groups of Library tracks, collected for a purpose." },
];

describe("stage routes", () => {
  it("cover exactly the stages the sidebar lists", () => {
    expect(SCREENS.map((s) => s.path)).toEqual(STAGES.map((s) => s.path));
  });

  it.each(SCREENS)("$path renders the $name screen", async ({ path, name, empty }) => {
    renderApp(path);
    expect(await screen.findByRole("heading", { level: 1, name })).toBeInTheDocument();
    expect(screen.getByRole("main")).toHaveTextContent(empty);
  });

  it("opens on Overview at /", async () => {
    const { router } = renderApp("/");
    expect(await screen.findByRole("heading", { level: 1, name: "Overview" })).toBeInTheDocument();
    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
  });
});

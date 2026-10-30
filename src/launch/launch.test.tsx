import { clearMocks } from "@tauri-apps/api/mocks";
import { act, cleanup, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { crate } from "../test/crateBackend";
import { fakeBackend, type World } from "../test/overviewBackend";
import { tx } from "../test/tx";
import { SCREEN_STORAGE_KEY } from "./savedScreen";
import { readSeen, SEEN_STORAGE_KEY, type Seen } from "./seenNews";

// Launch (ROADMAP 1.13): the app reopens where the user left off, and on
// Overview on a first run or when a scan or read has found something new.

afterEach(() => {
  cleanup();
  clearMocks();
});

const NOTHING_SEEN: Seen = {
  outsideLibrary: 0,
  needsCloserLook: 0,
  clearBestFile: 0,
  qualityProblems: 0,
  conflicts: 0,
};

function remember(screen: unknown) {
  localStorage.setItem(SCREEN_STORAGE_KEY, typeof screen === "string" ? screen : JSON.stringify(screen));
}

function seen(numbers: Seen) {
  localStorage.setItem(SEEN_STORAGE_KEY, JSON.stringify(numbers));
}

/** The app launched: at "/", as the desktop window opens. */
function launch(world: World = {}) {
  const backend = fakeBackend(world);
  return { backend, ...renderApp("/") };
}

/** Waits until the launch has asked for the numbers that decide about news. */
async function untilNewsIsKnown(calls: string[]) {
  await waitFor(() => {
    expect(calls).toContain("overview");
    expect(calls).toContain("review_counts");
  });
  // Let the answer be acted on.
  await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
}

describe("reopening where the user left off", () => {
  it("reopens the last screen", async () => {
    remember({ path: "/library" });
    const { router } = launch();
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.library") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/library");
  });

  it("reopens the crate", async () => {
    remember({ path: "/crates", crate: 7 });
    const { router } = launch({ crates: [crate(3, "Warmup"), crate(7, "Peak time")] });
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.crates") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/crates");
    expect(router.state.location.search).toEqual({ crate: 7 });
  });

  it("a deleted crate falls back to the default screen", async () => {
    remember({ path: "/crates", crate: 7 });
    const { router } = launch({ crates: [crate(3, "Warmup")] });
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/overview");
    expect(router.state.location.search).toEqual({});
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("first run opens on Overview", async () => {
    const { router } = launch({ library: 0 });
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/overview");
  });

  it.each([
    ["not JSON", "{not json"],
    ["not an object", "42"],
    ["a screen that doesn't exist", JSON.stringify({ path: "/nowhere" })],
    ["a screen that isn't a stage", JSON.stringify({ path: "/after-send" })],
    ["a path that isn't text", JSON.stringify({ path: 5 })],
  ])("a corrupt saved value is ignored (%s)", async (_what, value) => {
    remember(value);
    const { router } = launch();
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/overview");
  });

  it("a crate id that isn't a whole number is ignored, the screen is kept", async () => {
    remember({ path: "/crates", crate: -4 });
    const { router } = launch({ crates: [crate(3, "Warmup")] });
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.crates") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/crates");
    expect(router.state.location.search).toEqual({});
  });

  it("opens on Overview when storage refuses to answer", async () => {
    const original = Storage.prototype.getItem;
    Storage.prototype.getItem = () => {
      throw new Error("blocked");
    };
    try {
      const { router } = launch();
      expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
      expect(router.state.location.pathname).toBe("/overview");
    } finally {
      Storage.prototype.getItem = original;
    }
  });
});

describe("remembering the screen as the user moves", () => {
  it("keeps the screen, and the crate, once the user stops moving", async () => {
    fakeBackend({ crates: [crate(7, "Peak time")] });
    const { router } = renderApp("/overview");
    await userEvent.click(await screen.findByRole("link", { name: tx("shell:stages.library") }));
    await waitFor(() => expect(localStorage.getItem(SCREEN_STORAGE_KEY)).toBe(JSON.stringify({ path: "/library" })));

    await act(() => router.navigate({ to: "/crates", search: { crate: 7 } }));
    await waitFor(() =>
      expect(localStorage.getItem(SCREEN_STORAGE_KEY)).toBe(JSON.stringify({ path: "/crates", crate: 7 })),
    );
  });

  it("writes only after the user stops moving, and only the last screen", async () => {
    fakeBackend();
    const { router } = renderApp("/overview");
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") });
    await act(() => router.navigate({ to: "/review" }));
    await act(() => router.navigate({ to: "/library" }));
    expect(localStorage.getItem(SCREEN_STORAGE_KEY)).toBeNull();
    await waitFor(() => expect(localStorage.getItem(SCREEN_STORAGE_KEY)).toBe(JSON.stringify({ path: "/library" })));
  });

  it("does not remember a screen that isn't a stage", async () => {
    remember({ path: "/library" });
    fakeBackend();
    const { router } = renderApp("/after-send");
    await screen.findByRole("heading", { level: 1 });
    await act(() => new Promise((resolve) => setTimeout(resolve, 600)));
    expect(router.state.location.pathname).toBe("/after-send");
    expect(localStorage.getItem(SCREEN_STORAGE_KEY)).toBe(JSON.stringify({ path: "/library" }));
  });
});

describe("opening on Overview when a scan or read finds something new", () => {
  it("news opens Overview: more tracks outside the Library than when last seen", async () => {
    remember({ path: "/library" });
    seen(NOTHING_SEEN);
    const { router } = launch({ overview: { outsideLibrary: 4 } });
    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.overview") })).toBeInTheDocument();
  });

  it("news opens Overview: more waiting in Review than when last seen", async () => {
    remember({ path: "/all-music" });
    seen({ ...NOTHING_SEEN, conflicts: 1 });
    const { router } = launch({ review: { conflicts: 2 } });
    await waitFor(() => expect(router.state.location.pathname).toBe("/overview"));
  });

  it("the same numbers as when last seen are no news", async () => {
    remember({ path: "/library" });
    seen({ ...NOTHING_SEEN, outsideLibrary: 4, needsCloserLook: 2 });
    const { backend, router } = launch({ overview: { outsideLibrary: 4 }, review: { needsCloserLook: 2 } });
    await untilNewsIsKnown(backend.calls);
    expect(router.state.location.pathname).toBe("/library");
  });

  it("fewer than when last seen is no news", async () => {
    remember({ path: "/library" });
    seen({ ...NOTHING_SEEN, outsideLibrary: 9 });
    const { backend, router } = launch({ overview: { outsideLibrary: 4 } });
    await untilNewsIsKnown(backend.calls);
    expect(router.state.location.pathname).toBe("/library");
  });

  it("the Library changing under the user's hands is no news", async () => {
    remember({ path: "/library" });
    seen(NOTHING_SEEN);
    const { backend, router } = launch({
      overview: { missingTags: 12, missingArt: 3, lowQuality: 2, notAnalyzed: 5, inNoCrate: 9 },
    });
    await untilNewsIsKnown(backend.calls);
    expect(router.state.location.pathname).toBe("/library");
  });

  it("without a record of what was seen, nothing is known to be new", async () => {
    remember({ path: "/library" });
    const { backend, router } = launch({ overview: { outsideLibrary: 4 } });
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.library") });
    await act(() => new Promise((resolve) => setTimeout(resolve, 100)));
    expect(router.state.location.pathname).toBe("/library");
    expect(backend.calls).not.toContain("overview");
  });

  it("a damaged record of what was seen is ignored", async () => {
    remember({ path: "/library" });
    localStorage.setItem(SEEN_STORAGE_KEY, JSON.stringify({ outsideLibrary: "many" }));
    const { router } = launch({ overview: { outsideLibrary: 4 } });
    expect(await screen.findByRole("heading", { level: 1, name: tx("shell:stages.library") })).toBeInTheDocument();
    expect(router.state.location.pathname).toBe("/library");
  });

  it("never moves a user who has already gone somewhere", async () => {
    remember({ path: "/library" });
    seen(NOTHING_SEEN);
    const { backend, router } = launch({ overview: { outsideLibrary: 4 }, overviewHeld: true });
    await screen.findByRole("heading", { level: 1, name: tx("shell:stages.library") });
    await act(() => router.navigate({ to: "/review" }));
    backend.release();
    await untilNewsIsKnown(backend.calls);
    expect(router.state.location.pathname).toBe("/review");
  });

  it("an Overview that can't be read is no news", async () => {
    remember({ path: "/library" });
    seen(NOTHING_SEEN);
    const { backend, router } = launch({ overviewBroken: true, overview: { outsideLibrary: 4 } });
    await untilNewsIsKnown(backend.calls);
    expect(router.state.location.pathname).toBe("/library");
  });
});

describe("keeping what the user has seen", () => {
  it("the Overview, with every number in, is what the user has seen", async () => {
    fakeBackend({ overview: { outsideLibrary: 4 }, review: { conflicts: 2 } });
    renderApp("/overview");
    await waitFor(() => expect(readSeen()).toEqual({ ...NOTHING_SEEN, outsideLibrary: 4, conflicts: 2 }));
  });

  it("numbers still being counted are not recorded", async () => {
    const backend = fakeBackend({
      overview: { outsideLibrary: 4, counting: { files: true, rekordbox: false, quality: false } },
    });
    renderApp("/overview");
    await untilNewsIsKnown(backend.calls);
    expect(readSeen()).toBeNull();
  });

  it("away from the Overview a number can only lower what was seen", async () => {
    remember({ path: "/library" });
    seen({ ...NOTHING_SEEN, outsideLibrary: 9, conflicts: 1 });
    // Outside the Library fell and conflicts stayed: no news, and the record follows the fall.
    const { backend } = launch({ overview: { outsideLibrary: 4 }, review: { conflicts: 1 } });
    await untilNewsIsKnown(backend.calls);
    expect(readSeen()).toEqual({ ...NOTHING_SEEN, outsideLibrary: 4, conflicts: 1 });
  });
});

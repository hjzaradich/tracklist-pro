import "@testing-library/jest-dom/vitest";
import { cleanup, configure } from "@testing-library/react";
import { afterEach } from "vitest";
import "../i18n";

// How long findBy* and waitFor keep looking. The 1 s default fails on the
// shared CI laptop, where a render can take seconds under load (1aC-11).
// Under the suite's 20 s test timeout, so a hang still fails for the
// element it waited for. No test proves speed by wall-clock time; it counts
// work instead.
configure({ asyncUtilTimeout: 10_000 });

// jsdom has no scrolling; the router calls scrollTo on navigation.
window.scrollTo = () => {};

afterEach(() => {
  cleanup();
  localStorage.clear();
});

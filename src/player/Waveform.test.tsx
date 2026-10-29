import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { tx } from "../test/tx";
import { Waveform, WaveformPeaks, WaveformPlaceholder } from "./Waveform";
import { outline, playedFraction } from "./waveformShape";

afterEach(() => {
  cleanup();
  clearMocks();
});

const PEAKS = [0, 128, 255, 64];
const LEVELS = [0, 64, 128, 32];

function drawn(position: number, duration?: number) {
  render(
    <WaveformPeaks
      peaks={PEAKS}
      levels={LEVELS}
      durationMs={200_000}
      position={position}
      duration={duration}
    />,
  );
  return screen.getByRole("img", { name: tx("player:waveform.label") });
}

describe("the waveform drawing", () => {
  it("draws the peaks and the level of each bucket as two shapes across the width", () => {
    const wave = drawn(0);
    const [peaks, levels] = Array.from(wave.querySelectorAll("path"));
    expect(peaks?.getAttribute("d")).toBe(outline(PEAKS));
    expect(levels?.getAttribute("d")).toBe(outline(LEVELS));
    // One unit of width per bucket, stretched to whatever width it is given.
    expect(wave.querySelector("svg")?.getAttribute("viewBox")).toBe("0 0 4 100");
    expect(wave.querySelector("svg")?.getAttribute("preserveAspectRatio")).toBe("none");
  });

  it("mirrors each bucket around the middle, and keeps silence as a thin line", () => {
    // Full scale reaches the edges; silence is half a unit either side.
    expect(outline([255])).toBe("M 0.5 0 L 0.5 100 Z");
    expect(outline([0])).toBe("M 0.5 49.5 L 0.5 50.5 Z");
    expect(outline([])).toBe("");
  });

  it("puts the playhead where playback is in the track, by the player's length when it has one", () => {
    // The waveform was drawn from 200 s; the player says the file is 100 s.
    expect(drawn(25, 100).querySelector<HTMLElement>("[data-playhead]")?.style.left).toBe("25%");
    cleanup();
    expect(drawn(50).querySelector<HTMLElement>("[data-playhead]")?.style.left).toBe("25%");
  });

  it("keeps the playhead inside the drawing for a position that is out of range or not a number", () => {
    expect(playedFraction(-5, 100)).toBe(0);
    expect(playedFraction(500, 100)).toBe(1);
    expect(playedFraction(Number.NaN, 100)).toBe(0);
    expect(playedFraction(10, 0)).toBe(0);
  });

  it("splits the colors at the playhead: played before it, unplayed after", () => {
    const wave = drawn(50, 100);
    const stops = Array.from(wave.querySelectorAll("stop")).map((s) => s.getAttribute("offset"));
    expect(stops).toEqual(["0", "0.5", "0.5", "1"]);
  });

  it("uses only theme colors: no color is written in the drawing", () => {
    const wave = drawn(10, 100);
    expect(wave.innerHTML).not.toMatch(/#[0-9a-f]{3,8}\b|rgb\(|hsl\(/i);
  });
});

describe("the waveform placeholder", () => {
  it("is named for why there is no waveform", () => {
    render(<WaveformPlaceholder />);
    expect(screen.getByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    cleanup();
    render(<WaveformPlaceholder unavailable />);
    expect(screen.getByRole("img", { name: tx("player:waveform.unavailable") })).toBeInTheDocument();
  });
});

describe("the waveform of a file", () => {
  function renderFor(fileId: number | null) {
    return render(
      <QueryClientProvider client={createQueryClient()}>
        <Waveform fileId={fileId} position={0} />
      </QueryClientProvider>,
    );
  }

  it("asks for nothing and shows the placeholder when there is no file", () => {
    const asked: string[] = [];
    mockIPC((cmd) => {
      asked.push(cmd);
      return null;
    });
    renderFor(null);
    expect(screen.getByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    expect(asked.filter((c) => c === "waveform_peaks")).toEqual([]);
  });

  it("asks for the file's peaks by its id and draws them", async () => {
    let asked: unknown;
    mockIPC((cmd, args) => {
      if (cmd !== "waveform_peaks") return null;
      asked = args;
      return { state: "ready", peaks: PEAKS, levels: LEVELS, durationMs: 200_000 };
    });
    renderFor(42);
    expect(await screen.findByRole("img", { name: tx("player:waveform.label") })).toBeInTheDocument();
    expect(asked).toEqual({ fileId: 42 });
  });

  it("asks for a file's waveform first when it isn't drawn, and draws it when it lands", async () => {
    const calls: string[] = [];
    let landed = false;
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "waveform_first") {
        // The job draws the file; the next look finds it.
        landed = true;
        return null;
      }
      if (cmd !== "waveform_peaks") return null;
      return landed
        ? { state: "ready", peaks: PEAKS, levels: LEVELS, durationMs: 200_000 }
        : { state: "notReady" };
    });
    renderFor(5);
    expect(await screen.findByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    // No job ended and nothing was reloaded by hand: it is looked at again.
    expect(await screen.findByRole("img", { name: tx("player:waveform.label") })).toBeInTheDocument();
    expect(calls.filter((c) => c === "waveform_first")).toHaveLength(1);
  });

  it("does not ask for a waveform that is already drawn, or that never will be", async () => {
    const calls: string[] = [];
    mockIPC((cmd) => {
      calls.push(cmd);
      if (cmd === "waveform_peaks") return { state: "unavailable" };
      return null;
    });
    renderFor(5);
    expect(await screen.findByRole("img", { name: tx("player:waveform.unavailable") })).toBeInTheDocument();
    expect(calls.filter((c) => c === "waveform_first")).toEqual([]);
  });

  it("keeps the placeholder when the answer is not ready, and when asking fails", async () => {
    mockIPC((cmd) => {
      if (cmd === "waveform_peaks") return { state: "notReady" };
      return null;
    });
    renderFor(7);
    expect(await screen.findByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
    cleanup();
    clearMocks();
    mockIPC(() => {
      throw new Error("no backend");
    });
    renderFor(7);
    expect(await screen.findByRole("img", { name: tx("player:waveform.notReady") })).toBeInTheDocument();
  });
});

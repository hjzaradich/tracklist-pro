import { clearMocks, mockConvertFileSrc, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { PreviewRefusal } from "../bindings";
import { PlayerBar } from "../shell/PlayerBar";
import { tx } from "../test/tx";
import { formatTime } from "./formatTime";
import { PlayButton } from "./PlayButton";
import { usePlayer } from "./playerStore";

const A = { id: 11, name: "Synthetic Tune A" };
const B = { id: 12, name: "Synthetic Tune B" };

/** The audio element the player made, once it loads something. */
let element: HTMLAudioElement | null;
let asked: number[];
let answers: Record<number, PreviewRefusal | null>;

beforeEach(() => {
  element = null;
  asked = [];
  answers = {};
  // jsdom plays nothing; these record what the player asks of the element.
  vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(function (this: HTMLMediaElement) {
    element = this as HTMLAudioElement;
  });
  vi.spyOn(HTMLMediaElement.prototype, "play").mockResolvedValue(undefined);
  vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {});
  mockIPC((cmd, args) => {
    if (cmd === "preview_check") {
      const id = (args as { fileId: number }).fileId;
      asked.push(id);
      return answers[id] ?? null;
    }
    throw new Error(`unexpected command ${cmd}`);
  });
  mockConvertFileSrc("windows");
  usePlayer.setState({ file: null, status: "idle", refusal: null, position: 0, duration: 0, playing: false });
});

afterEach(() => {
  cleanup();
  clearMocks();
});

/** What the browser does once the file's header is read. */
function loaded(duration: number) {
  Object.defineProperty(element, "duration", { value: duration, configurable: true });
  act(() => {
    element?.dispatchEvent(new Event("loadedmetadata"));
  });
}

function renderPlayer() {
  return render(
    <>
      <PlayButton file={A} />
      <PlayButton file={B} />
      <PlayerBar />
    </>,
  );
}

describe("the player bar", () => {
  it("says nothing is playing until a file is chosen", () => {
    render(<PlayerBar />);
    expect(screen.getByText(tx("shell:player.empty"))).toBeInTheDocument();
    expect(screen.queryByRole("slider")).toBeNull();
  });

  it("plays a file from its play button and shows its name, play/pause and a scrub bar", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    expect(asked).toEqual([A.id]);
    // By id, on the app's own protocol.
    expect(element?.getAttribute("src")).toContain(`audio-preview.localhost/${A.id}`);
    loaded(300);

    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    expect(bar).toHaveTextContent(A.name);
    expect(bar).toHaveTextContent("0:00 / 5:00");
    expect(screen.getByRole("slider", { name: tx("player:position") })).toBeEnabled();
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(1);
  });

  it("pauses and plays again from the bar", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);

    const bar = screen.getByRole("region", { name: tx("shell:player.label") });
    const toggle = () => bar.querySelector("button") as HTMLButtonElement;
    // An icon: the name is the label, not text.
    expect(toggle()).toHaveAccessibleName(tx("player:pause"));
    await user.click(toggle());
    expect(toggle()).toHaveAccessibleName(tx("player:play"));
    expect(HTMLMediaElement.prototype.pause).toHaveBeenCalled();
    await user.click(toggle());
    expect(toggle()).toHaveAccessibleName(tx("player:pause"));
    expect(HTMLMediaElement.prototype.play).toHaveBeenCalledTimes(2);
  });

  it("scrubs to the time the bar is moved to", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);

    fireEvent.change(screen.getByRole("slider", { name: tx("player:position") }), { target: { value: "90" } });
    expect(element?.currentTime).toBe(90);
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent("1:30 / 5:00");
  });

  it("is one player: choosing a second file shows the first as stopped", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: A.name }) })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: B.name }) }));
    await waitFor(() => expect(asked).toEqual([A.id, B.id]));
    loaded(300);
    // Only the second file's button says it is playing; the bar names it.
    expect(screen.getByRole("button", { name: tx("player:pauseNamed", { name: B.name }) })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent(B.name);
  });

  it("pressing the button of the file that is playing pauses it", async () => {
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    loaded(300);
    await user.click(screen.getByRole("button", { name: tx("player:pauseNamed", { name: A.name }) }));
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();
    expect(asked).toEqual([A.id]);
  });
});

describe("a file that can't be played", () => {
  const lines: [PreviewRefusal, string][] = [
    ["missing", "player:refused.missing"],
    ["drive_not_connected", "player:refused.driveNotConnected"],
    ["online_only", "player:refused.onlineOnly"],
    ["unknown_file", "player:refused.cantPlay"],
    ["unreadable", "player:refused.cantPlay"],
    ["not_decodable", "player:refused.cantPlay"],
  ];

  it.each(lines)("%s gets its line, with no scrub bar and nothing playing", async (code, key) => {
    answers[A.id] = code;
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    expect(await screen.findByRole("status")).toHaveTextContent(tx(key));
    expect(screen.queryByRole("slider")).toBeNull();
    expect(element).toBeNull();
    expect(HTMLMediaElement.prototype.play).not.toHaveBeenCalled();
    // The file stays named, so the line says which one.
    expect(screen.getByRole("region", { name: tx("shell:player.label") })).toHaveTextContent(A.name);
  });

  it("shows a refused file's button as play, and pressing it asks again", async () => {
    answers[A.id] = "drive_not_connected";
    const user = userEvent.setup();
    renderPlayer();
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    expect(await screen.findByRole("status")).toHaveTextContent(tx("player:refused.driveNotConnected"));
    // Not "Pause" beside a line saying it can't play.
    expect(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) })).toBeInTheDocument();

    answers[A.id] = null;
    await user.click(screen.getByRole("button", { name: tx("player:playNamed", { name: A.name }) }));
    await waitFor(() => expect(element).not.toBeNull());
    expect(asked).toEqual([A.id, A.id]);
    loaded(300);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByRole("slider", { name: tx("player:position") })).toBeEnabled();
  });

  it("uses four different lines for the six codes", () => {
    expect(new Set(lines.map(([, key]) => tx(key))).size).toBe(4);
  });
});

describe("time", () => {
  it("is written m:ss, and h:mm:ss from an hour", () => {
    expect(formatTime(0)).toBe("0:00");
    expect(formatTime(90)).toBe("1:30");
    expect(formatTime(599.9)).toBe("9:59");
    expect(formatTime(3600)).toBe("1:00:00");
    expect(formatTime(3725)).toBe("1:02:05");
    expect(formatTime(Number.NaN)).toBe("0:00");
    expect(formatTime(-4)).toBe("0:00");
  });
});

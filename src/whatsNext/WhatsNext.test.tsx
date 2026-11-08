import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import type { Direction, NextReason, NextReply, NextSettings } from "../bindings";
import "../i18n";
import { DetailsPanel } from "../shell/DetailsPanel";
import { useSelectedTrack } from "../shell/selectedTrack";
import { quietAnswer } from "../test/quietCommands";
import { tableRow } from "../test/trackTableIpc";
import { tx } from "../test/tx";

const SETTINGS: NextSettings = { weights: { key: 4, bpm: 4, energy: 2, played: 2, genre: 0, tags: 1, similar: 1 }, tolerance: "tight" };

function reason(code: NextReason["code"], fields: Partial<NextReason> = {}): NextReason {
  return { code, good: true, from: null, to: null, delta: null, step: null, count: null, tags: null, percent: null, ...fields };
}

let asked: { recordingId: number; direction: Direction }[];
let saved: NextSettings[];
let reply: NextReply;

beforeEach(() => {
  asked = [];
  saved = [];
  reply = {
    results: [
      {
        row: tableRow(2, { artist: "Made Up Artist 2", title: "Synthetic Tune 2" }),
        score: 90,
        reasons: [
          reason("keyAdjacent", { from: "8A", to: "9A" }),
          reason("bpm", { delta: 1.5 }),
          reason("playedAfter", { count: 2 }),
        ],
      },
      {
        row: tableRow(3, { artist: null, title: "Synthetic Tune 3" }),
        score: 40,
        reasons: [reason("energyNone", { good: false })],
      },
    ],
    considered: 7,
    ignored: ["bpm"],
    similarGap: null,
    settings: SETTINGS,
  };
  mockIPC((cmd, args) => {
    if (cmd === "next_for") {
      asked.push(args as { recordingId: number; direction: Direction });
      return reply;
    }
    if (cmd === "next_set_settings") {
      const { settings } = args as { settings: NextSettings };
      saved.push(settings);
      return settings;
    }
    if (cmd === "track_tag_sources") return { inRekordbox: false, fields: [], confirmedEntry: null };
    if (cmd === "crates_of_track") return [];
    return quietAnswer(cmd);
  });
});

afterEach(() => {
  cleanup();
  clearMocks();
  useSelectedTrack.setState({ row: null, ids: [], tables: 0 });
});

function renderPanel() {
  const row = tableRow(1);
  useSelectedTrack.setState({ row, ids: [row.recordingId] });
  render(
    <QueryClientProvider client={createQueryClient()}>
      <DetailsPanel />
    </QueryClientProvider>,
  );
  return row;
}

const section = () => screen.findByRole("region", { name: tx("next:title") });

describe("What's next in the Details panel", () => {
  it("lists the suggestions with their reasons, worded from the reason codes", async () => {
    const row = renderPanel();
    const next = within(await section());
    const items = await next.findAllByRole("listitem");
    expect(items).toHaveLength(2);
    expect(items[0]).toHaveTextContent("Made Up Artist 2 - Synthetic Tune 2");
    const line = [
      tx("next:reason.keyAdjacent", { from: "8A", to: "9A" }),
      tx("next:reason.bpm", { delta: "+1.5" }),
      tx("next:reason.playedAfter", { count: 2 }),
    ].join(tx("next:reason.separator"));
    expect(items[0]).toHaveTextContent(line);
    expect(items[1]).toHaveTextContent("Synthetic Tune 3");
    expect(items[1]).toHaveTextContent(tx("next:reason.energyNone"));
    expect(next.getByText(tx("next:considered", { count: 7 }))).toBeInTheDocument();
    expect(next.getByText(tx("next:ignored.label", { signals: tx("next:signal.bpm") }))).toBeInTheDocument();
    expect(asked[0]).toEqual({ recordingId: row.recordingId, direction: "same" });
  });

  it("words the genre and tag reasons, and names the shared tags and the genres", async () => {
    reply = {
      ...reply,
      results: [
        {
          row: tableRow(2, { artist: "Made Up Artist 2", title: "Synthetic Tune 2" }),
          score: 80,
          reasons: [
            reason("genreSame", { from: "Deep House" }),
            reason("tagsShared", { count: 2, tags: ["peak", "vocal"] }),
          ],
        },
        {
          row: tableRow(3, { artist: null, title: "Synthetic Tune 3" }),
          score: 50,
          reasons: [
            reason("genreRelated", { good: false, from: "Deep House", to: "Tech House" }),
            reason("tagsShared", { count: 1, tags: ["warm"] }),
          ],
        },
      ],
      ignored: ["genre", "tags"],
    };
    renderPanel();
    const next = within(await section());
    const items = await next.findAllByRole("listitem");
    expect(items[0]).toHaveTextContent(
      [
        tx("next:reason.genreSame", { genre: "Deep House" }),
        tx("next:reason.tagsShared", { count: 2, tags: ["peak", "vocal"].join(tx("next:reason.separator")) }),
      ].join(tx("next:reason.separator")),
    );
    expect(items[1]).toHaveTextContent(
      [
        tx("next:reason.genreRelated", { from: "Deep House", to: "Tech House" }),
        tx("next:reason.tagsShared", { count: 1, tags: "warm" }),
      ].join(tx("next:reason.separator")),
    );
    const signals = [tx("next:signal.genre"), tx("next:signal.tags")].join(tx("next:ignored.separator"));
    expect(next.getByText(tx("next:ignored.label", { signals }))).toBeInTheDocument();
  });

  it("asks again when the energy direction changes", async () => {
    const row = renderPanel();
    const next = within(await section());
    await next.findAllByRole("listitem");
    await userEvent.click(next.getByRole("radio", { name: tx("next:direction.up") }));
    await waitFor(() => expect(asked.at(-1)).toEqual({ recordingId: row.recordingId, direction: "up" }));
  });

  it("says plainly when nothing can follow the track", async () => {
    reply = { ...reply, results: [], ignored: [] };
    renderPanel();
    expect(await within(await section()).findByText(tx("next:none"))).toBeInTheDocument();
  });

  it("shows the weights and the tolerance and saves a change at once", async () => {
    renderPanel();
    const next = within(await section());
    await next.findAllByRole("listitem");
    await userEvent.click(await next.findByText(tx("next:weights.title")));
    const energy = next.getByLabelText(tx("next:weights.energy"));
    expect(energy).toHaveValue(2);
    fireEvent.change(energy, { target: { value: "7" } });
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ ...SETTINGS, weights: { ...SETTINGS.weights, energy: 7 } });

    await userEvent.selectOptions(next.getByLabelText(tx("next:weights.tolerance.label")), "wide");
    await waitFor(() => expect(saved).toHaveLength(2));
    expect(saved[1]?.tolerance).toBe("wide");
  });

  it("shows the genre and tag weights and saves a change to either", async () => {
    renderPanel();
    const next = within(await section());
    await userEvent.click(await next.findByText(tx("next:weights.title")));
    const genre = next.getByLabelText(tx("next:weights.genre"));
    const tags = next.getByLabelText(tx("next:weights.tags"));
    expect(genre).toHaveValue(0);
    expect(tags).toHaveValue(1);
    fireEvent.change(genre, { target: { value: "5" } });
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ ...SETTINGS, weights: { ...SETTINGS.weights, genre: 5 } });
    fireEvent.change(tags, { target: { value: "0" } });
    await waitFor(() => expect(saved).toHaveLength(2));
    expect(saved[1]?.weights.tags).toBe(0);
  });

  it("words the audio model's reason with its percent", async () => {
    reply = {
      ...reply,
      results: [
        {
          row: tableRow(2, { artist: "Made Up Artist 2", title: "Synthetic Tune 2" }),
          score: 90,
          reasons: [reason("keySame", { from: "8A" }), reason("similar", { percent: 82 })],
        },
      ],
    };
    renderPanel();
    const next = within(await section());
    expect(await next.findByText(tx("next:reason.similar", { percent: 82 }), { exact: false })).toBeInTheDocument();
  });

  it("says why sounds similar is not used, on its own line and not among what the track lacks", async () => {
    reply = { ...reply, ignored: ["bpm", "similar"], similarGap: "modelOff" };
    renderPanel();
    const next = within(await section());
    expect(await next.findByText(tx("next:ignored.similar.modelOff"))).toBeInTheDocument();
    expect(next.getByText(tx("next:ignored.label", { signals: tx("next:signal.bpm") }))).toBeInTheDocument();
    cleanup();

    reply = { ...reply, ignored: ["similar"], similarGap: "noFingerprint" };
    renderPanel();
    const again = within(await section());
    expect(await again.findByText(tx("next:ignored.similar.noFingerprint"))).toBeInTheDocument();
    expect(again.queryByText((text) => text.startsWith(tx("next:ignored.label", { signals: "" })))).toBeNull();
  });

  it("says nothing about sounds similar when it is used", async () => {
    renderPanel();
    const next = within(await section());
    await next.findAllByRole("listitem");
    expect(next.queryByText(tx("next:ignored.similar.modelOff"))).toBeNull();
    expect(next.queryByText(tx("next:ignored.similar.noFingerprint"))).toBeNull();
  });

  it("shows the sounds-similar weight and saves a change to it", async () => {
    renderPanel();
    const next = within(await section());
    await userEvent.click(await next.findByText(tx("next:weights.title")));
    const similar = next.getByLabelText(tx("next:weights.similar"));
    expect(similar).toHaveValue(1);
    fireEvent.change(similar, { target: { value: "6" } });
    await waitFor(() => expect(saved).toHaveLength(1));
    expect(saved[0]).toEqual({ ...SETTINGS, weights: { ...SETTINGS.weights, similar: 6 } });
  });

  it("does not save a weight outside 0 to 10", async () => {
    renderPanel();
    const next = within(await section());
    await userEvent.click(await next.findByText(tx("next:weights.title")));
    fireEvent.change(next.getByLabelText(tx("next:weights.key")), { target: { value: "11" } });
    fireEvent.change(next.getByLabelText(tx("next:weights.key")), { target: { value: "2.5" } });
    expect(saved).toHaveLength(0);
  });
});

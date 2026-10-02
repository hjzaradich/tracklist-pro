import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { renderApp } from "../app/testApp";
import { tx } from "../test/tx";

afterEach(() => clearMocks());

describe("the after-send page", () => {
  it("is reachable at /after-send and shows both lists with their empty states", async () => {
    mockIPC((cmd) => {
      if (cmd === "after_send_lists") {
        return { playlistsChecked: true, stalePlaylists: [], manualRemovals: [] };
      }
      throw new Error(`unexpected command ${cmd}`);
    });
    renderApp("/after-send");
    expect(await screen.findByRole("heading", { level: 1, name: tx("afterSend:page") })).toBeInTheDocument();
    expect(await screen.findByText(tx("afterSend:stale.empty"))).toBeInTheDocument();
    expect(screen.getByText(tx("afterSend:removals.empty"))).toBeInTheDocument();
  });
});

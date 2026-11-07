import { QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { tx } from "../test/tx";
import { AboutPanel, type LicenceList } from "./AboutPanel";

afterEach(() => {
  clearMocks();
  vi.unstubAllGlobals();
});

function renderAbout(reply: () => Promise<Response>) {
  mockIPC(() => "1.2.3");
  vi.stubGlobal("fetch", vi.fn(reply));
  return render(
    <QueryClientProvider client={createQueryClient()}>
      <AboutPanel />
    </QueryClientProvider>,
  );
}

const list: LicenceList = {
  texts: ["Shared licence text"],
  packages: [
    { name: "alpha", version: "1.0.0", license: "MIT", text: 0, repository: null },
    { name: "beta", version: "2.0.0", license: "MIT", text: 0, repository: null },
    { name: "gamma", version: "3.0.0", license: "ISC", text: null, repository: "https://example.test/gamma" },
  ],
};

describe("About panel", () => {
  it("shows the shipped packages and their licence text when the list is there", async () => {
    renderAbout(async () => new Response(JSON.stringify(list)));
    expect(await screen.findByText(tx("about:licences.count", { count: 3 }))).toBeInTheDocument();
    expect(screen.getByText("Shared licence text")).toBeInTheDocument();
    expect(screen.getByText(/gamma 3\.0\.0/)).toBeInTheDocument();
  });

  it("says the list comes with the installer when the file is missing", async () => {
    renderAbout(async () => new Response("", { status: 404 }));
    expect(await screen.findByText(tx("about:licences.missing"))).toBeInTheDocument();
  });

  it("shows the app's version", async () => {
    renderAbout(async () => new Response("", { status: 404 }));
    expect(await screen.findByText(tx("about:version", { version: "1.2.3" }))).toBeInTheDocument();
  });

  it("gives the source address as text, not a link", async () => {
    renderAbout(async () => new Response("", { status: 404 }));
    await screen.findByText(tx("about:licences.missing"));
    expect(screen.getByText(tx("about:sourceUrl"))).toBeInTheDocument();
    expect(screen.queryByRole("link")).toBeNull();
  });
});

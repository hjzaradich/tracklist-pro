import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import type { JobUpdate } from "../bindings";
import { useReloadListsWhenJobsEnd } from "../shell/useReloadListsWhenJobsEnd";
import { createQueryClient } from "./queryClient";
import { listKeys, reloadLists, type ListGroup } from "./lists";

// The one list of which lists are asked for again after a change.

afterEach(() => {
  cleanup();
  clearMocks();
});

const EVERY_GROUP: ListGroup[] = [
  "library",
  "offer",
  "review",
  "missing",
  "crates",
  "tagSources",
  "folders",
  "afterSend",
  "overview",
];

/** A client that holds an answer for every list, as if every screen had been open. */
function everyListRead() {
  const queryClient = createQueryClient();
  for (const queryKey of listKeys(...EVERY_GROUP)) queryClient.setQueryData([...queryKey], []);
  return queryClient;
}

/** The groups whose lists are marked out of date. */
function staleGroups(queryClient: QueryClient): ListGroup[] {
  return EVERY_GROUP.filter((group) =>
    listKeys(group).every((queryKey) => queryClient.getQueryState([...queryKey])?.isInvalidated),
  );
}

describe("the lists asked for again after a change", () => {
  it("names each list once, whichever groups ask for it", () => {
    const keys = listKeys(...EVERY_GROUP, ...EVERY_GROUP).map((key) => JSON.stringify(key));
    expect(new Set(keys).size).toBe(keys.length);
  });

  it("asks again for the lists of the groups named and no others", async () => {
    const queryClient = everyListRead();
    await reloadLists(queryClient, "library", "offer");
    expect(staleGroups(queryClient)).toEqual(["library", "offer"]);
  });

  it("includes the Library's track table and All music's under the Library group", async () => {
    const queryClient = createQueryClient();
    queryClient.setQueryData(["libraryTracks", "table", "anything"], []);
    queryClient.setQueryData(["libraryTracks", "summary"], { tracks: 1, changeStamp: 1 });
    queryClient.setQueryData(["allMusic", "table", "anything"], []);
    await reloadLists(queryClient, "library");
    for (const queryKey of [
      ["libraryTracks", "table", "anything"],
      ["libraryTracks", "summary"],
      ["allMusic", "table", "anything"],
    ]) {
      expect(queryClient.getQueryState(queryKey)?.isInvalidated).toBe(true);
    }
  });
});

describe("a job ending", () => {
  it("asks again for the lists background work feeds, and not for the offer or the after-send lists", async () => {
    mockIPC(() => undefined, { shouldMockEvents: true });
    const queryClient = everyListRead();
    function Wrapper({ children }: { children: ReactNode }) {
      return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
    }
    renderHook(() => useReloadListsWhenJobsEnd(), { wrapper: Wrapper });
    // Listening starts when the hook has mounted.
    await act(async () => {});
    expect(staleGroups(queryClient)).toEqual([]);

    const ended: JobUpdate = { seq: 9, id: 9, kind: "scan", status: "done", progress: null, priority: 0 };
    await act(() => emit("job-updates", [ended]));
    await waitFor(() =>
      expect(staleGroups(queryClient)).toEqual([
        "library",
        "review",
        "missing",
        "crates",
        "tagSources",
        "folders",
        "overview",
      ]),
    );
  });
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { REVIEW_QUERY_KEYS } from "../review/useReview";
import { useAddRekordboxTracks, useUndoAddRekordboxTracks } from "./useRekordboxOffer";

// The add of rekordbox tracks and its undo change the Library, so they change
// what Review holds: a track added can have an item, and undoing takes it away.

afterEach(() => {
  cleanup();
  clearMocks();
});

function inClient(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

/** A client that holds an answer for each Review query, as if Review had been open. */
function reviewHasBeenRead() {
  const queryClient = createQueryClient();
  for (const queryKey of REVIEW_QUERY_KEYS) queryClient.setQueryData([...queryKey], []);
  return queryClient;
}

const stale = (queryClient: QueryClient) =>
  REVIEW_QUERY_KEYS.map((queryKey) => queryClient.getQueryState([...queryKey])?.isInvalidated);

describe("the add of rekordbox tracks", () => {
  it("asks Review for its queue again, after the add", async () => {
    mockIPC(() => ({ added: 0, alreadyThere: 0, skipped: 0, notFound: 0 }));
    const queryClient = reviewHasBeenRead();
    const { result } = renderHook(() => useAddRekordboxTracks(), { wrapper: inClient(queryClient) });
    expect(stale(queryClient)).toEqual(REVIEW_QUERY_KEYS.map(() => false));
    act(() => result.current.mutate(null));
    await waitFor(() => expect(stale(queryClient)).toEqual(REVIEW_QUERY_KEYS.map(() => true)));
  });

  it("asks Review for its queue again, after the undo of the add", async () => {
    mockIPC(() => ({ status: "nothingToUndo" }));
    const queryClient = reviewHasBeenRead();
    const { result } = renderHook(() => useUndoAddRekordboxTracks(), {
      wrapper: inClient(queryClient),
    });
    act(() => result.current.mutate(9));
    await waitFor(() => expect(stale(queryClient)).toEqual(REVIEW_QUERY_KEYS.map(() => true)));
  });
});

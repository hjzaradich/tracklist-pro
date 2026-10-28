import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { afterEach, describe, expect, it } from "vitest";
import { createQueryClient } from "../app/queryClient";
import { REVIEW_QUERY_KEYS } from "../review/useReview";
import { useUndoOperation } from "./useUndo";

afterEach(() => {
  cleanup();
  clearMocks();
});

function inClient(queryClient: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>;
  };
}

const stale = (queryClient: QueryClient) =>
  REVIEW_QUERY_KEYS.map((queryKey) => queryClient.getQueryState([...queryKey])?.isInvalidated);

describe("Undo", () => {
  // Undoing a Review action (or an add of rekordbox tracks) changes what
  // Review holds: an item comes back, or goes.
  it("asks Review for its queue again, after an undo", async () => {
    mockIPC(() => ({ status: "nothingToUndo" }));
    const queryClient = createQueryClient();
    for (const queryKey of REVIEW_QUERY_KEYS) queryClient.setQueryData([...queryKey], []);
    const { result } = renderHook(() => useUndoOperation(), { wrapper: inClient(queryClient) });
    expect(stale(queryClient)).toEqual(REVIEW_QUERY_KEYS.map(() => false));
    act(() => result.current.mutate(9));
    await waitFor(() => expect(stale(queryClient)).toEqual(REVIEW_QUERY_KEYS.map(() => true)));
  });
});

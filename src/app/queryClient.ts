import { QueryClient } from "@tanstack/react-query";

// TanStack Query holds server state: everything that comes from the Rust
// side. UI-only state lives in Zustand stores next to the feature that owns it.
export function createQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        // Data comes from the local backend, not a remote server, so there is
        // no point refetching just because the window regained focus.
        refetchOnWindowFocus: false,
        retry: false,
      },
    },
  });
}

import { QueryClientProvider, type QueryClient } from "@tanstack/react-query";
import { RouterProvider, type AnyRouter } from "@tanstack/react-router";
import { I18nextProvider } from "react-i18next";
import i18n from "../i18n";

interface AppProvidersProps {
  // Any router: the app passes createAppRouter(); tests may pass their own.
  router: AnyRouter;
  queryClient: QueryClient;
}

/**
 * Every provider the app needs, in one place: i18n, server state (TanStack
 * Query) and routing. UI state uses Zustand stores, which need no provider.
 */
export function AppProviders({ router, queryClient }: AppProvidersProps) {
  return (
    <I18nextProvider i18n={i18n}>
      <QueryClientProvider client={queryClient}>
        <RouterProvider router={router} />
      </QueryClientProvider>
    </I18nextProvider>
  );
}

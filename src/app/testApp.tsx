import { createMemoryHistory } from "@tanstack/react-router";
import { render } from "@testing-library/react";
import { AppProviders } from "./AppProviders";
import { createQueryClient } from "./queryClient";
import { createAppRouter } from "./router";

/** Test helper: the whole app, with every provider, starting at `path`. */
export function renderApp(path = "/") {
  const router = createAppRouter({
    history: createMemoryHistory({ initialEntries: [path] }),
  });
  const result = render(<AppProviders router={router} queryClient={createQueryClient()} />);
  return { ...result, router };
}

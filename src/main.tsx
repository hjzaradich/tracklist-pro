import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./theme/tokens.css";
import "./i18n";
import { AppProviders } from "./app/AppProviders";
import { createQueryClient } from "./app/queryClient";
import { createAppRouter } from "./app/router";
import { syncThemeToDocument } from "./theme/themeStore";

// Apply the saved theme before the first render so there's no flash.
syncThemeToDocument();

const router = createAppRouter();
const queryClient = createQueryClient();

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <AppProviders router={router} queryClient={queryClient} />
  </StrictMode>,
);

import i18n from "i18next";
import { initReactI18next } from "react-i18next";

type Messages = { [key: string]: string | Messages };

// One namespace file per feature: src/locales/en/<feature>.json becomes the
// namespace "<feature>". Files are picked up automatically, so a new feature
// adds its own file and never edits a shared list (fewer merge conflicts).
const englishFiles = import.meta.glob<Messages>("../locales/en/*.json", {
  eager: true,
  import: "default",
});

export function namespaceFromPath(path: string): string {
  const file = path.split("/").pop() ?? path;
  return file.replace(/\.json$/, "");
}

export const englishResources: Record<string, Messages> = Object.fromEntries(
  Object.entries(englishFiles).map(([path, messages]) => [
    namespaceFromPath(path),
    messages,
  ]),
);

export const namespaces = Object.keys(englishResources);

// English only for now; the app is translation-ready (ROADMAP 1.1, Language).
void i18n.use(initReactI18next).init({
  lng: "en",
  fallbackLng: "en",
  supportedLngs: ["en"],
  resources: { en: englishResources },
  ns: namespaces,
  defaultNS: "common",
  interpolation: {
    // React already escapes rendered text.
    escapeValue: false,
  },
  // Resources are bundled, so init finishes synchronously and the first
  // render already has its strings.
  initAsync: false,
  returnNull: false,
});

export default i18n;

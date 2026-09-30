// Types every t() call against the English namespace files, so a mistyped
// key or namespace is a compile error. The namespace list is generated
// (resources.gen.ts); features never edit this file.
import "i18next";
import type { EnglishResources } from "./resources.gen";

declare module "i18next" {
  interface CustomTypeOptions {
    defaultNS: "common";
    resources: EnglishResources;
    returnNull: false;
  }
}

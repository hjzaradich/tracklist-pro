import ts from "typescript";
import { beforeAll, describe, expect, it } from "vitest";

// Type-checks the real project (tsconfig.json, the same check `npm run build`
// runs) plus a few in-memory files that use t(), and reports the errors in
// each. A mistyped key has to be a compile error, or the build wouldn't catch it.

const CASES = {
  validKeys: `
    import { useTranslation } from "react-i18next";
    import i18n from "./index";
    export function useValid() {
      const overview = useTranslation("overview").t;
      const both = useTranslation(["common", "overview"]).t;
      const common = useTranslation().t;
      return [overview("empty"), both("overview:empty"), common("appName"), i18n.t("theme:label")];
    }`,
  typoWithNamespacePrefix: `
    import i18n from "./index";
    export const text = i18n.t("overview:typo");`,
  typoInNamespaceHook: `
    import { useTranslation } from "react-i18next";
    export function useTypo() {
      const { t } = useTranslation("overview");
      return t("typo");
    }`,
  keyFromAnotherNamespace: `
    import { useTranslation } from "react-i18next";
    export function useWrongNamespace() {
      const { t } = useTranslation("theme");
      return t("empty"); // "empty" is in overview.json, not theme.json
    }`,
  unknownNamespace: `
    import { useTranslation } from "react-i18next";
    export function useUnknown() {
      return useTranslation("nope");
    }`,
};
type Case = keyof typeof CASES;

let errorsByCase: Record<Case, string[]>;

beforeAll(() => {
  const config = ts.getParsedCommandLineOfConfigFile("tsconfig.json", undefined, {
    ...ts.sys,
    onUnRecoverableConfigFileDiagnostic: (d) => {
      throw new Error(ts.flattenDiagnosticMessageText(d.messageText, "\n"));
    },
  });
  if (!config) throw new Error("could not read tsconfig.json");

  const virtual = new Map(
    Object.entries(CASES).map(([name, code]) => [
      ts.sys.resolvePath(`src/i18n/__keyTypes_${name}.ts`).replace(/\\/g, "/"),
      code,
    ]),
  );
  const host = ts.createCompilerHost(config.options);
  const { fileExists, readFile, getSourceFile } = host;
  host.fileExists = (f) => virtual.has(f) || fileExists.call(host, f);
  host.readFile = (f) => virtual.get(f) ?? readFile.call(host, f);
  host.getSourceFile = (f, lang, ...rest) => {
    const code = virtual.get(f);
    return code !== undefined
      ? ts.createSourceFile(f, code, lang)
      : getSourceFile.call(host, f, lang, ...rest);
  };

  const program = ts.createProgram({
    rootNames: [...config.fileNames, ...virtual.keys()],
    options: config.options,
    host,
  });
  const diagnostics = ts.getPreEmitDiagnostics(program);

  errorsByCase = Object.fromEntries(
    Object.keys(CASES).map((name) => [name, [] as string[]]),
  ) as Record<Case, string[]>;
  const outside: string[] = [];
  for (const d of diagnostics) {
    const text = ts.flattenDiagnosticMessageText(d.messageText, "\n");
    const name = /__keyTypes_(\w+)\.ts$/.exec(d.file?.fileName ?? "")?.[1] as Case | undefined;
    if (name) errorsByCase[name].push(text);
    else outside.push(`${d.file?.fileName ?? "?"}: ${text}`);
  }
  // The rest of the project must compile, or the cases prove nothing.
  expect(outside).toEqual([]);
}, 120_000);

describe("i18n keys are checked at compile time", () => {
  it("existing keys compile, with or without a namespace prefix", () => {
    expect(errorsByCase.validKeys).toEqual([]);
  });

  it('t("overview:typo") is a compile error', () => {
    expect(errorsByCase.typoWithNamespacePrefix.join("\n")).toContain('"overview:typo"');
  });

  it('a typo in a namespace\'s own t() is a compile error: useTranslation("overview").t("typo")', () => {
    expect(errorsByCase.typoInNamespaceHook.join("\n")).toContain('"typo"');
  });

  it("a key missing from its namespace file is a compile error, even if another namespace has it", () => {
    expect(errorsByCase.keyFromAnotherNamespace.join("\n")).toContain('"empty"');
  });

  it("an unknown namespace is a compile error", () => {
    expect(errorsByCase.unknownNamespace.join("\n")).toContain('"nope"');
  });
});

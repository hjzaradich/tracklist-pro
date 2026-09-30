import ts from "typescript";
import { describe, expect, it } from "vitest";

// The network gate (ROADMAP 0.1, 0E-6), frontend side. Every outbound
// request goes through the Rust `net` module, reached over Tauri IPC; the
// webview itself never talks to the network. The CSP already blocks remote
// connections (src-tauri/tests/csp.rs); this check keeps the code from
// trying. It parses the source, so comments and strings that merely mention
// these names don't count.

/** Browser APIs that open a connection, whatever URL they're given. */
const CONNECTION_APIS = new Set([
  "XMLHttpRequest",
  "WebSocket",
  "EventSource",
  "WebTransport",
  "RTCPeerConnection",
  "importScripts",
]);

/** Objects `fetch` hangs off when it's the browser's. */
const GLOBALS = new Set(["window", "globalThis", "self"]);

/** Tauri plugins that make requests from the webview side, or hand a URL
 * to the browser or the shell, which then fetches it. */
const NETWORK_PLUGINS = [
  "@tauri-apps/plugin-http",
  "@tauri-apps/plugin-websocket",
  "@tauri-apps/plugin-upload",
  "@tauri-apps/plugin-opener",
  "@tauri-apps/plugin-shell",
];

/** A URL the app's own origin serves, e.g. `/locales/en.json`. */
function isSameOrigin(url: string): boolean {
  return (url.startsWith("/") && !url.startsWith("//")) || /^\.\.?\//.test(url);
}

function literalText(node: ts.Node | undefined): string | undefined {
  return node && (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node))
    ? node.text
    : undefined;
}

function isGlobal(node: ts.Node): boolean {
  return ts.isIdentifier(node) && GLOBALS.has(node.text);
}

/**
 * Whether identifier `node` names something other than a global: a
 * property of another object (`query.fetch`), a key (`{ WebSocket: false }`),
 * a member being declared, or the imported name in `import { fetch as x }`.
 */
function isOtherName(node: ts.Identifier): boolean {
  const parent = node.parent;
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) {
    return !isGlobal(parent.expression);
  }
  return (
    ((ts.isPropertyAssignment(parent) ||
      ts.isMethodDeclaration(parent) ||
      ts.isPropertyDeclaration(parent) ||
      ts.isPropertySignature(parent) ||
      ts.isMethodSignature(parent)) &&
      parent.name === node) ||
    ts.isImportSpecifier(parent) ||
    ts.isExportSpecifier(parent)
  );
}

/** Every network call in one file, as `line: what`. */
function networkCalls(source: string, fileName = "file.tsx"): string[] {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const found: string[] = [];
  const report = (node: ts.Node, what: string) => {
    const { line } = file.getLineAndCharacterOfPosition(node.getStart(file));
    found.push(`${line + 1}: ${what}`);
  };

  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node)) {
      const url = literalText(node.arguments[0]);
      if (
        ts.isPropertyAccessExpression(node.expression) &&
        node.expression.name.text === "sendBeacon"
      ) {
        report(node, "sendBeacon()");
      }
      // import("https://…")
      if (node.expression.kind === ts.SyntaxKind.ImportKeyword && url !== undefined && /^[a-z]+:/i.test(url)) {
        report(node, `import("${url}")`);
      }
    }
    if (ts.isIdentifier(node) && CONNECTION_APIS.has(node.text) && !isOtherName(node)) {
      report(node, node.text);
    }
    // The browser's fetch may only be called directly, with a same-origin
    // literal. Anything else (`const f = fetch`, `fetch.call(…)`) is flagged.
    if (ts.isIdentifier(node) && node.text === "fetch" && !isOtherName(node)) {
      // `window.fetch` as a whole, or plain `fetch`.
      const ref: ts.Node =
        ts.isPropertyAccessExpression(node.parent) && node.parent.name === node
          ? node.parent
          : node;
      const call = ref.parent;
      if (ts.isCallExpression(call) && call.expression === ref) {
        const url = literalText(call.arguments[0]);
        if (url === undefined) {
          report(call, "fetch() to a URL that isn't a same-origin literal");
        } else if (!isSameOrigin(url)) {
          report(call, `fetch("${url}")`);
        }
      } else {
        report(ref, "fetch used other than as a direct call");
      }
    }
    // `window["fe" + "tch"]`: a computed global can't be checked.
    if (ts.isElementAccessExpression(node) && isGlobal(node.expression)) {
      const key = literalText(node.argumentExpression);
      if (key === undefined || key === "fetch" || CONNECTION_APIS.has(key)) {
        report(node, `${node.expression.getText(file)}[…]`);
      }
    }
    if (
      (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) &&
      node.moduleSpecifier
    ) {
      const from = literalText(node.moduleSpecifier) ?? "";
      if (NETWORK_PLUGINS.some((p) => from === p || from.startsWith(`${p}/`))) {
        report(node, `import from "${from}"`);
      }
      if (/^[a-z]+:\/\//i.test(from)) report(node, `import from "${from}"`);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return found;
}

describe("networkCalls", () => {
  it("flags fetch to a remote URL", () => {
    expect(networkCalls('fetch("https://musicbrainz.org/ws/2/artist");')).toEqual([
      '1: fetch("https://musicbrainz.org/ws/2/artist")',
    ]);
    expect(networkCalls("await window.fetch(`http://example.com`);")).toHaveLength(1);
    expect(networkCalls('globalThis.fetch("//cdn.example.com/x.js");')).toHaveLength(1);
  });

  it("flags fetch to a URL it can't see", () => {
    expect(networkCalls("const url = getUrl();\nfetch(url);")).toEqual([
      "2: fetch() to a URL that isn't a same-origin literal",
    ]);
    expect(networkCalls("fetch(`${base}/x`);")).toHaveLength(1);
  });

  it("allows fetch of the app's own files", () => {
    expect(networkCalls('fetch("/locales/en/common.json"); fetch("./a.json");')).toEqual([]);
  });

  it("flags connection APIs whatever their URL", () => {
    const source = `
      const xhr = new XMLHttpRequest();
      const ws = new WebSocket("ws://localhost:1");
      const es = new window.EventSource("/events");
      navigator.sendBeacon("/x", data);
      const pc = new RTCPeerConnection();
    `;
    expect(networkCalls(source)).toEqual([
      "2: XMLHttpRequest",
      "3: WebSocket",
      "4: EventSource",
      "5: sendBeacon()",
      "6: RTCPeerConnection",
    ]);
  });

  it("flags fetch taken aside, called indirectly or looked up by a computed name", () => {
    const source = `
      const f = fetch; f(url);
      fetch.call(null, "/ok.json");
      window.fetch.apply(window, [url]);
      const g = window["fe" + "tch"];
      const h = globalThis["XMLHttpRequest"];
      const { fetch: grab } = window;
    `;
    expect(networkCalls(source)).toEqual([
      "2: fetch used other than as a direct call",
      "3: fetch used other than as a direct call",
      "4: fetch used other than as a direct call",
      "5: window[…]",
      "6: globalThis[…]",
      "7: fetch used other than as a direct call",
    ]);
  });

  it("allows plain lookups on the globals", () => {
    expect(networkCalls('const w = window["innerWidth"]; const t = self["document"];')).toEqual([]);
  });

  it("flags Tauri's opener and shell plugins", () => {
    const source = `
      import { openUrl } from "@tauri-apps/plugin-opener";
      import { open } from "@tauri-apps/plugin-shell";
    `;
    expect(networkCalls(source)).toEqual([
      '2: import from "@tauri-apps/plugin-opener"',
      '3: import from "@tauri-apps/plugin-shell"',
    ]);
  });

  it("flags Tauri's network plugins and remote imports", () => {
    const source = `
      import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
      import WebSocketPlugin from "@tauri-apps/plugin-websocket";
      const mod = await import("https://example.com/mod.js");
    `;
    expect(networkCalls(source)).toEqual([
      '2: import from "@tauri-apps/plugin-http"',
      '3: import from "@tauri-apps/plugin-websocket"',
      '4: import("https://example.com/mod.js")',
    ]);
  });

  it("ignores comments, strings, other objects' methods and look-alike names", () => {
    const source = `
      // fetch("https://example.com") and new WebSocket(url)
      const label = "XMLHttpRequest is not used";
      await queryClient.fetchQuery({ queryKey: ["x"] });
      await query.fetch();
      const fetchAppVersion = () => commands.appInfo();
      const options = { WebSocket: false };
      const link = "https://www.beatport.com/search?q=x";
    `;
    expect(networkCalls(source)).toEqual([]);
  });
});

describe("the frontend makes no network calls", () => {
  // Every script under src, tests included (a test must never reach the
  // internet either), except this file, which plants violations on purpose.
  const sources = import.meta.glob<string>(
    ["../**/*.{ts,tsx}", "!./noNetworkCalls.test.ts"],
    { query: "?raw", import: "default", eager: true },
  );

  it("finds the source files to check", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(10);
    expect(Object.keys(sources)).toContain("../main.tsx");
  });

  it("only the Rust net module talks to the network", () => {
    const offenders = Object.entries(sources).flatMap(([path, text]) =>
      networkCalls(text, path).map((call) => `${path}:${call}`),
    );
    expect(offenders).toEqual([]);
  });
});

import { describe, expect, it } from "vitest";
import tokensCss from "./tokens.css?raw";

// Every CSS rule block in tokens.css: selector → declarations.
function blocks(css: string): { selector: string; body: string }[] {
  const withoutComments = css.replace(/\/\*[\s\S]*?\*\//g, "");
  return [...withoutComments.matchAll(/([^{}]+)\{([^{}]*)\}/g)].map((m) => ({
    selector: m[1].trim(),
    body: m[2],
  }));
}

function themeBlock(theme: "dark" | "light"): string {
  const found = blocks(tokensCss).find((b) =>
    b.selector.includes(`[data-theme="${theme}"]`),
  );
  if (!found) throw new Error(`no block for the ${theme} theme`);
  return found.body;
}

function colorTokens(body: string): Map<string, string> {
  return new Map(
    [...body.matchAll(/(--color-[\w-]+)\s*:\s*([^;]+);/g)].map((m) => [m[1], m[2].trim()]),
  );
}

const PURE_BLACK = /^(#000|#000000|black|rgb\(\s*0\s*,\s*0\s*,\s*0\s*\))$/i;
const PURE_WHITE = /^(#fff|#ffffff|white|rgb\(\s*255\s*,\s*255\s*,\s*255\s*\))$/i;

describe("theme tokens", () => {
  const dark = colorTokens(themeBlock("dark"));
  const light = colorTokens(themeBlock("light"));

  it("dark and light define exactly the same color token names", () => {
    expect(dark.size).toBeGreaterThan(0);
    expect([...light.keys()].sort()).toEqual([...dark.keys()].sort());
  });

  it("dark is the default: its tokens also apply when no theme is set", () => {
    const darkSelector = blocks(tokensCss).find((b) =>
      b.selector.includes('[data-theme="dark"]'),
    )!.selector;
    expect(darkSelector.split(",").map((s) => s.trim())).toContain(":root");
  });

  it("neither theme uses pure black", () => {
    for (const [name, value] of [...dark, ...light]) {
      expect(PURE_BLACK.test(value), `${name}: ${value}`).toBe(false);
    }
  });

  it("text colors are never pure white", () => {
    for (const [name, value] of [...dark, ...light]) {
      if (name.startsWith("--color-text") || name.startsWith("--color-on-")) {
        expect(PURE_WHITE.test(value), `${name}: ${value}`).toBe(false);
      }
    }
  });

  it("has one accent per theme: declared once, and no second accent", () => {
    for (const theme of ["dark", "light"] as const) {
      const body = themeBlock(theme);
      // Counted in the raw CSS, not the token map: a second declaration
      // (e.g. left behind by a merge) silently overrides the first, and a
      // map keeps only one of them.
      expect(body.match(/--color-accent\s*:/g), theme).toHaveLength(1);
      // "One restrained accent" (ROADMAP 1.1): no second accent family.
      const extraAccents = [...colorTokens(body).keys()].filter((name) =>
        /^--color-(secondary|tertiary|accent-?(\d|alt))/.test(name),
      );
      expect(extraAccents, theme).toEqual([]);
    }
  });
});

// Color math for the warning checks: OKLab/OKLCH (a perceptual color space,
// where equal distances look about equally different) and WCAG contrast.
function linearChannels(hex: string): [number, number, number] {
  const m = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(hex);
  if (!m) throw new Error(`expected a #rrggbb color, got ${hex}`);
  return [m[1], m[2], m[3]].map((h) => {
    const c = parseInt(h, 16) / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
}

function oklab(hex: string): [number, number, number] {
  const [r, g, b] = linearChannels(hex);
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  return [
    0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s,
    1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s,
    0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s,
  ];
}

/** How different two colors look (OKLab distance; about 0.02 is barely visible). */
function perceivedDistance(a: string, b: string): number {
  const [p, q] = [oklab(a), oklab(b)];
  return Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2]);
}

/** Gap between two colors' hues around the OKLCH color wheel, in degrees. */
function hueGap(a: string, b: string): number {
  const hue = (hex: string) => {
    const [, x, y] = oklab(hex);
    return (Math.atan2(y, x) * 180) / Math.PI;
  };
  const d = Math.abs(hue(a) - hue(b)) % 360;
  return d > 180 ? 360 - d : d;
}

/** WCAG 2 contrast ratio. */
function contrast(a: string, b: string): number {
  const luminance = (hex: string) => {
    const [r, g, b] = linearChannels(hex);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

describe("warning color", () => {
  for (const theme of ["dark", "light"] as const) {
    const tokens = colorTokens(themeBlock(theme));
    const get = (name: string) => {
      const value = tokens.get(name);
      if (!value) throw new Error(`the ${theme} theme has no ${name}`);
      return value;
    };
    const warning = get("--color-warning");

    it(`can't be mistaken for the amber accent (${theme})`, () => {
      // The old warning (#d9b64a dark, #8a6d10 light) was only 0.05 away and
      // 16–21° round the wheel: next to the accent it read as a second accent.
      // The hover shade counts too: it's what a selected control looks like
      // under the pointer.
      for (const accent of ["--color-accent", "--color-accent-hover"]) {
        expect(perceivedDistance(warning, get(accent)), accent).toBeGreaterThanOrEqual(0.08);
        expect(hueGap(warning, get(accent)), accent).toBeGreaterThanOrEqual(25);
      }
    });

    it(`stays clearly apart from danger and success (${theme})`, () => {
      expect(perceivedDistance(warning, get("--color-danger"))).toBeGreaterThanOrEqual(0.15);
      expect(hueGap(warning, get("--color-danger"))).toBeGreaterThanOrEqual(60);
      expect(perceivedDistance(warning, get("--color-success"))).toBeGreaterThanOrEqual(0.08);
    });

    it(`is readable as text on every background (${theme})`, () => {
      for (const bg of ["--color-bg", "--color-surface", "--color-surface-raised"]) {
        expect(contrast(warning, get(bg)), bg).toBeGreaterThanOrEqual(4.5);
      }
    });
  }

  it("the distance check would have caught the old, amber-like warning", () => {
    expect(perceivedDistance("#d9b64a", "#e0a13c")).toBeLessThan(0.08);
    expect(perceivedDistance("#8a6d10", "#a86a12")).toBeLessThan(0.08);
  });
});

// CSS named colors (CSS Color 4), except `transparent`, which is allowed.
// `currentColor` and `inherit` aren't colors of their own and are allowed too.
const NAMED_COLORS = [
  "aliceblue", "antiquewhite", "aqua", "aquamarine", "azure", "beige", "bisque", "black",
  "blanchedalmond", "blue", "blueviolet", "brown", "burlywood", "cadetblue", "chartreuse",
  "chocolate", "coral", "cornflowerblue", "cornsilk", "crimson", "cyan", "darkblue", "darkcyan",
  "darkgoldenrod", "darkgray", "darkgreen", "darkgrey", "darkkhaki", "darkmagenta",
  "darkolivegreen", "darkorange", "darkorchid", "darkred", "darksalmon", "darkseagreen",
  "darkslateblue", "darkslategray", "darkslategrey", "darkturquoise", "darkviolet", "deeppink",
  "deepskyblue", "dimgray", "dimgrey", "dodgerblue", "firebrick", "floralwhite", "forestgreen",
  "fuchsia", "gainsboro", "ghostwhite", "gold", "goldenrod", "gray", "green", "greenyellow",
  "grey", "honeydew", "hotpink", "indianred", "indigo", "ivory", "khaki", "lavender",
  "lavenderblush", "lawngreen", "lemonchiffon", "lightblue", "lightcoral", "lightcyan",
  "lightgoldenrodyellow", "lightgray", "lightgreen", "lightgrey", "lightpink", "lightsalmon",
  "lightseagreen", "lightskyblue", "lightslategray", "lightslategrey", "lightsteelblue",
  "lightyellow", "lime", "limegreen", "linen", "magenta", "maroon", "mediumaquamarine",
  "mediumblue", "mediumorchid", "mediumpurple", "mediumseagreen", "mediumslateblue",
  "mediumspringgreen", "mediumturquoise", "mediumvioletred", "midnightblue", "mintcream",
  "mistyrose", "moccasin", "navajowhite", "navy", "oldlace", "olive", "olivedrab", "orange",
  "orangered", "orchid", "palegoldenrod", "palegreen", "paleturquoise", "palevioletred",
  "papayawhip", "peachpuff", "peru", "pink", "plum", "powderblue", "purple", "rebeccapurple",
  "red", "rosybrown", "royalblue", "saddlebrown", "salmon", "sandybrown", "seagreen",
  "seashell", "sienna", "silver", "skyblue", "slateblue", "slategray", "slategrey", "snow",
  "springgreen", "steelblue", "tan", "teal", "thistle", "tomato", "turquoise", "violet",
  "wheat", "white", "whitesmoke", "yellow", "yellowgreen",
];
const NAMES = NAMED_COLORS.join("|");

const HEX_COLOR = /#[0-9a-f]{3,8}(?![\w-])/i;
// Not preceded by a name character or `.`, so `setColor(` or `theme.color(`
// aren't mistaken for the CSS `color(` function.
const COLOR_FUNCTION =
  /(?<![\w$.-])(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch|color-mix|color)\(/i;
// A whole word: not part of an identifier, class or token name
// (`--color-red`, `.red-button`, `isRed`), and not a function (CSS `tan()`).
const NAMED_COLOR = new RegExp(String.raw`(?<![\w$-])(?:${NAMES})(?![\w(-])`, "i");
// A string or template literal that is exactly a named color, e.g.
// style={{ color: "red" }} or style={{ color: `red` }}.
const NAMED_COLOR_STRING = new RegExp(String.raw`(["'\x60])\s*(?:${NAMES})\s*\1`, "gi");

/** Color literals in a stylesheet's declaration values (never selectors). */
function cssColorLiterals(css: string): string[] {
  const code = css.replace(/\/\*[\s\S]*?\*\//g, "");
  const found: string[] = [];
  for (const [declaration, rawValue] of code.matchAll(
    /(?<=^|[;{])\s*[-\w]+\s*:\s*([^;{}]+)/g,
  )) {
    const value = rawValue
      .replace(/"[^"]*"|'[^']*'/g, "") // quoted strings: font names, content
      .replace(/url\([^)]*\)/g, "")
      .replace(/--[\w-]+/g, ""); // token names, e.g. var(--color-red-500)
    if ([HEX_COLOR, COLOR_FUNCTION, NAMED_COLOR].some((re) => re.test(value))) {
      found.push(declaration.trim());
    }
  }
  return found;
}

/** Color literals in TypeScript/TSX source, outside comments and class names. */
function scriptColorLiterals(source: string): string[] {
  const code = source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/(^|[^:"'`\\])\/\/.*$/gm, "$1");
  // Every occurrence, not just the first, so one run lists all of a file's
  // violations. Reported in the order they appear.
  const found: { index: number; text: string }[] = [];
  for (const re of [HEX_COLOR, COLOR_FUNCTION]) {
    for (const match of code.matchAll(new RegExp(re.source, "gi"))) {
      found.push({ index: match.index, text: match[0] });
    }
  }
  for (const match of code.matchAll(NAMED_COLOR_STRING)) {
    const before = code.slice(Math.max(0, match.index - 40), match.index);
    if (!/className\s*[=:]\s*\{?\s*$/.test(before)) {
      found.push({ index: match.index, text: match[0] });
    }
  }
  return found.sort((a, b) => a.index - b.index).map((f) => f.text);
}

describe("color literal detection", () => {
  it("flags named colors and color-mix() in a stylesheet", () => {
    const css =
      ".x { color: red; background: white; border-color: color-mix(in srgb, black 20%, transparent); }";
    expect(cssColorLiterals(css)).toEqual([
      "color: red",
      "background: white",
      "border-color: color-mix(in srgb, black 20%, transparent)",
    ]);
  });

  it("flags hex and every color function in a stylesheet", () => {
    for (const value of [
      "#1a2b3c",
      "rgb(0 0 0)",
      "rgba(0,0,0,.5)",
      "hsl(0 0% 0%)",
      "hwb(0 0% 0%)",
      "lab(50% 0 0)",
      "oklch(0.5 0 0)",
      "color(display-p3 1 0 0)",
      "color-mix(in srgb, var(--a), var(--b))",
    ]) {
      expect(cssColorLiterals(`.x { color: ${value}; }`), value).toHaveLength(1);
    }
  });

  it("allows tokens, keywords, and color words in comments, selectors, names and strings", () => {
    const css = `
      /* white text on a red button */
      .red-button, .whiteBox:hover {
        color: currentColor;
        background: transparent;
        border-color: inherit;
        outline-color: var(--color-red-500);
        font-family: "Snow Sans", sans-serif;
        width: calc(tan(45deg) * 10px);
      }`;
    expect(cssColorLiterals(css)).toEqual([]);
  });

  it("flags color literals in component code", () => {
    expect(scriptColorLiterals(`<p style={{ color: "red" }} />`)).toEqual(['"red"']);
    expect(scriptColorLiterals(`const c = "hwb(0 0% 0%)";`)).toEqual(["hwb("]);
    expect(scriptColorLiterals(`const c = "#fff";`)).toEqual(["#fff"]);
  });

  it("reports every color literal in a component file, not just the first of each kind", () => {
    const source = `
      const a = "#fff";
      const b = "rgb(0 0 0)";
      const c = "#123456";
      const d = { color: "red", background: "hsl(0 0% 0%)", border: 'white' };`;
    expect(scriptColorLiterals(source)).toEqual([
      "#fff",
      "rgb(",
      "#123456",
      '"red"',
      "hsl(",
      "'white'",
    ]);
  });

  it("flags a template literal that is exactly a named color", () => {
    expect(scriptColorLiterals("<p style={{ color: `red` }} />")).toEqual(["`red`"]);
    expect(scriptColorLiterals("const c = ` Black `;")).toEqual(["` Black `"]);
  });

  it("allows template literals that only mention a color word, and class names in backticks", () => {
    const source = "const label = `${name} red`; const el = <p className={`red`} />;";
    expect(scriptColorLiterals(source)).toEqual([]);
  });

  it("allows color words in component code comments, names and class names", () => {
    const source = `
      // red and white
      /* black */
      const isRed = styles.whiteBox;
      setColor(x);
      const el = <p className="red" data-x={t("color.red")} />;`;
    expect(scriptColorLiterals(source)).toEqual([]);
  });
});

describe("no hardcoded colors", () => {
  // Every source and stylesheet under src, except the token file itself and tests.
  const sources = import.meta.glob<string>(
    ["../**/*.{ts,tsx,css}", "!./tokens.css", "!../**/*.test.{ts,tsx}"],
    { query: "?raw", import: "default", eager: true },
  );

  it("finds the source files to check", () => {
    expect(Object.keys(sources).length).toBeGreaterThan(3);
  });

  it("components and stylesheets use theme tokens, not color literals", () => {
    const offenders = Object.entries(sources).flatMap(([path, text]) =>
      (path.endsWith(".css") ? cssColorLiterals(text) : scriptColorLiterals(text)).map(
        (literal) => `${path}: ${literal}`,
      ),
    );
    expect(offenders).toEqual([]);
  });
});

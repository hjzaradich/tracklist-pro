// Local ESLint rule: user-visible text in JSX must come from t(), never a raw
// string (ROADMAP 1.1, Language: all UI strings go through i18next).
//
// Flags, when the string contains a letter:
//   - text children:            <p>Hello</p>, <p>{"Hello"}</p>, <p>{`Hi ${name}`}</p>
//   - user-visible attributes:  title, alt, placeholder, aria-label and the other
//                               aria text attributes, plus component props whose
//                               name ends in Label/Text/Title/Message/... (e.g.
//                               <EmptyState message="No tracks" />)
//   - text props in an object spread: <Comp {...{ label: "Hi" }} />
//   - in any of those places: either branch of `a ? "Yes" : "No"`, the right
//     side of `a && "Hi"`, either side of `"Hi " + name`, and a name bound
//     to a `const` in the same file whose value is one of these
//     (`const msg = "Hello"; <p>{msg}</p>`). No data flow across files, and
//     `let`/`var`, parameters and imports aren't followed.
// Allows: t("...") calls, text with no letters (" · ", "—", "42", "%"), and every
// other attribute (className, data-*, type, role, id, key, href, ...).

const VISIBLE_ATTRIBUTES = new Set([
  "alt",
  "title",
  "placeholder",
  "label",
  "aria-label",
  "aria-description",
  "aria-placeholder",
  "aria-roledescription",
  "aria-valuetext",
]);

// Component props that carry text: one of these words, or a camelCase name
// ending in one (ariaLabel, emptyText, errorMessage). Not `context` or `i18nKey`.
const TEXT_PROP =
  /^(label|text|title|subtitle|message|description|placeholder|caption|heading|tooltip|hint)$|(?<=[a-z0-9])(Label|Text|Title|Message|Description|Placeholder|Caption|Heading|Tooltip|Hint)$/;

const HAS_LETTER = /\p{L}/u;

function attributeName(node) {
  return node.name.type === "JSXNamespacedName"
    ? `${node.name.namespace.name}:${node.name.name.name}`
    : node.name.name;
}

function isUserVisibleAttribute(name) {
  if (name.startsWith("data-")) return false;
  return VISIBLE_ATTRIBUTES.has(name) || TEXT_PROP.test(name);
}

/** The variable a name refers to, found by walking up the enclosing scopes. */
function findVariable(scope, name) {
  for (let s = scope; s; s = s.upper) {
    const variable = s.set.get(name);
    if (variable) return variable;
  }
  return null;
}

/** The initial value of `const name = <value>`, or null for anything else. */
function constInit(variable) {
  if (!variable || variable.defs.length !== 1) return null;
  const [def] = variable.defs;
  if (def.type !== "Variable" || def.parent.kind !== "const") return null;
  if (def.node.id.type !== "Identifier") return null; // not a destructured binding
  return def.node.init ?? null;
}

/**
 * The raw string literals an expression can evaluate to, looking through
 * ?:, &&/||, +, TypeScript casts and same-file `const` names. Each comes with
 * `at`, the node to report: the literal, or the name that led to it.
 *
 * `path` holds the const values being followed on the way to this node, so a
 * cycle (const a = b; const b = a) stops. Each step copies it, so a const used
 * twice in one expression (`label + " " + label`) is followed both times.
 */
function rawStrings(node, context, path = new Set()) {
  switch (node.type) {
    case "Literal":
      return typeof node.value === "string" ? [{ literal: node, at: node }] : [];
    case "TemplateLiteral":
      return node.quasis.some((q) => HAS_LETTER.test(q.value.cooked ?? q.value.raw))
        ? [{ literal: node, at: node }]
        : [];
    case "ConditionalExpression":
      return [
        ...rawStrings(node.consequent, context, path),
        ...rawStrings(node.alternate, context, path),
      ];
    case "LogicalExpression":
      return rawStrings(node.right, context, path);
    case "BinaryExpression":
      return node.operator === "+"
        ? [...rawStrings(node.left, context, path), ...rawStrings(node.right, context, path)]
        : [];
    case "TSAsExpression":
    case "TSSatisfiesExpression":
    case "TSNonNullExpression":
      return rawStrings(node.expression, context, path);
    case "Identifier": {
      const init = constInit(resolve(node, context));
      if (!init || path.has(init)) return [];
      return rawStrings(init, context, new Set(path).add(init)).map(({ literal }) => ({
        literal,
        at: node,
      }));
    }
    default:
      return [];
  }
}

function resolve(identifier, context) {
  return findVariable(context.sourceCode.getScope(identifier), identifier.name);
}

/** The object literal a spread passes: {...{ a: 1 }} or {...props} with a same-file const. */
function spreadObject(argument, context) {
  if (argument.type === "ObjectExpression") return argument;
  if (argument.type === "Identifier") {
    const init = constInit(resolve(argument, context));
    if (init?.type === "ObjectExpression") return init;
  }
  return null;
}

function propertyName(property) {
  if (property.type !== "Property" || property.computed) return null;
  if (property.key.type === "Identifier") return property.key.name;
  if (property.key.type === "Literal" && typeof property.key.value === "string") {
    return property.key.value;
  }
  return null;
}

function textOf(node) {
  if (node.type === "TemplateLiteral") return node.quasis.map((q) => q.value.raw).join("…");
  return String(node.value);
}

function shorten(text) {
  const oneLine = text.replace(/\s+/g, " ").trim();
  return oneLine.length > 40 ? `${oneLine.slice(0, 39)}…` : oneLine;
}

/** @type {import("eslint").Rule.RuleModule} */
const rule = {
  meta: {
    type: "problem",
    docs: {
      description: "Forbid raw user-visible strings in JSX; use t() from react-i18next.",
    },
    schema: [],
    messages: {
      rawText:
        'Raw text in JSX: "{{text}}". Add it to src/locales/en/<feature>.json and use t().',
      rawAttribute:
        'Raw text in the "{{name}}" attribute: "{{text}}". Add it to src/locales/en/<feature>.json and use t().',
    },
  },
  create(context) {
    // Reported where the text is used in JSX (for a const, the name used
    // there), quoting the raw string itself.
    function reportStrings(expression, messageId, name, at) {
      for (const found of rawStrings(expression, context)) {
        const text = textOf(found.literal);
        if (HAS_LETTER.test(text)) {
          context.report({
            node: at ?? found.at,
            messageId,
            data: { name, text: shorten(text) },
          });
        }
      }
    }

    return {
      JSXText(node) {
        if (HAS_LETTER.test(node.value)) {
          context.report({ node, messageId: "rawText", data: { text: shorten(node.value) } });
        }
      },
      JSXExpressionContainer(node) {
        // Children only; attribute values are handled below.
        if (node.parent.type === "JSXElement" || node.parent.type === "JSXFragment") {
          reportStrings(node.expression, "rawText", "");
        }
      },
      JSXAttribute(node) {
        const name = attributeName(node);
        if (!node.value || !isUserVisibleAttribute(name)) return;
        const value =
          node.value.type === "JSXExpressionContainer" ? node.value.expression : node.value;
        reportStrings(value, "rawAttribute", name);
      },
      JSXSpreadAttribute(node) {
        const object = spreadObject(node.argument, context);
        if (!object) return;
        for (const property of object.properties) {
          const name = propertyName(property);
          if (name && isUserVisibleAttribute(name)) {
            // A const object is reported at the spread, where it reaches JSX.
            const at = object === node.argument ? undefined : node;
            reportStrings(property.value, "rawAttribute", name, at);
          }
        }
      },
    };
  },
};

export default {
  meta: { name: "tracklist-pro" },
  rules: { "no-raw-jsx-strings": rule },
};

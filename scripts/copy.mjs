// The copy pipeline's command line (1aF-3). See scripts/copy-lib.mjs.
//
//   node scripts/copy.mjs check                        fail if the notes don't match the locale files (CI)
//   node scripts/copy.mjs status                       list the proposed strings; always exits 0
//   node scripts/copy.mjs gate                         like status, but exits 1 if any string is proposed
//   node scripts/copy.mjs approve <ns>:<key> ...       approve these strings as they read now
//   node scripts/copy.mjs approve --all-in <ns> ...    approve a whole namespace as it reads now
//
// `approve` is for the copy editor, after the owner has approved the text.
import { pathToFileURL } from "node:url";
import { counts, approve, checkCopy, formatProposed, proposedStrings, readCopy, ROOT, writeNotes } from "./copy-lib.mjs";

/** Runs a command; returns `{ code, out, err }` (lines for stdout and stderr). Pure but for the file reads and writes. */
export function run(argv, root = ROOT) {
  const [command, ...rest] = argv;
  const out = [];
  const err = [];
  const copy = readCopy(root);

  if (command === "check") {
    const problems = checkCopy(copy);
    if (problems.length > 0) {
      err.push(...problems, "", `${problems.length} problem${problems.length === 1 ? "" : "s"} in the copy notes (src/locales/notes). See docs/copy-style.md, "Adding or changing a string".`);
      return { code: 1, out, err };
    }
    out.push(`Copy notes match the locale files (${counts(copy).total} strings).`);
    return { code: 0, out, err };
  }

  if (command === "status" || command === "gate") {
    const list = proposedStrings(copy);
    out.push(formatProposed(list, counts(copy).total));
    const problems = checkCopy(copy);
    if (problems.length > 0) {
      err.push(...problems, "", "The copy notes have problems (npm run copy:check); the list above may be incomplete.");
    }
    const failed = command === "gate" && (list.length > 0 || problems.length > 0);
    if (failed && list.length > 0) err.push("Copy gate: strings are still proposed. The owner must approve them first.");
    return { code: failed ? 1 : 0, out, err };
  }

  if (command === "approve") {
    const targets = [];
    for (let i = 0; i < rest.length; i += 1) {
      if (rest[i] === "--all-in") {
        const ns = rest[i + 1];
        if (ns === undefined || ns.startsWith("--")) {
          err.push("--all-in needs a namespace");
          return { code: 2, out, err };
        }
        targets.push({ allIn: ns });
        i += 1;
      } else {
        targets.push(rest[i]);
      }
    }
    if (targets.length === 0) {
      err.push("Usage: npm run copy:approve -- <namespace>:<key> ... | --all-in <namespace>");
      return { code: 2, out, err };
    }
    const result = approve(copy, targets);
    if (result.problems.length > 0) {
      err.push(...result.problems, "Nothing was approved.");
      return { code: 1, out, err };
    }
    for (const ns of result.changed) writeNotes(root, ns, copy[ns].notes);
    out.push(`Approved ${result.approved} string${result.approved === 1 ? "" : "s"}.`);
    return { code: 0, out, err };
  }

  err.push("Usage: node scripts/copy.mjs check | status | gate | approve <namespace>:<key> ... | approve --all-in <namespace>");
  return { code: 2, out, err };
}

const isMain = process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href;
if (isMain) {
  const { code, out, err } = run(process.argv.slice(2));
  for (const line of out) console.log(line);
  for (const line of err) console.error(line);
  process.exit(code);
}

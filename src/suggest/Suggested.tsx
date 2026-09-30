import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { Reason, Suggestion } from "../bindings";
import { reasonOptions, reasonProblem } from "./reasons";
import styles from "./Suggested.module.css";

/**
 * Shows a suggestion with its reasons beside it, e.g. "same key, +2 BPM,
 * tagged Peak Time" (ROADMAP §1, "No black boxes").
 *
 * Refuses to render anything, the suggestion included, when its reasons
 * break a rule (see reasonProblem): a suggestion without a reason never
 * reaches the screen.
 */
export function Suggested<T>({
  suggestion,
  children,
}: {
  suggestion: Suggestion<T>;
  children: ReactNode;
}) {
  const problem = reasonProblem(suggestion?.reasons);
  if (problem !== null) {
    console.error(`Suggestion not shown (${problem}):`, suggestion);
    return null;
  }
  return (
    <div className={styles.suggestion}>
      <div>{children}</div>
      <ReasonLine reasons={suggestion.reasons} />
    </div>
  );
}

/** The reasons as one short line of fragments. */
function ReasonLine({ reasons }: { reasons: Reason[] }) {
  const { t } = useTranslation("suggest");
  const separator = t("separator");
  return (
    <p className={styles.reasons}>
      {reasons.map((reason, i) => (
        <span key={i} data-source={reason.source}>
          {i > 0 && separator}
          <ReasonText reason={reason} />
        </span>
      ))}
    </p>
  );
}

function ReasonText({ reason }: { reason: Reason }) {
  const { t } = useTranslation("suggest");
  // The key comes from Rust, so it can't be typed as a literal key here; it
  // was checked against the locale files at runtime instead (reasonProblem).
  const translate = t as unknown as (key: string, options: object) => string;
  const text = translate(reason.key, reasonOptions(reason));
  if (reason.source !== "audioModel") return text;
  return <span className={styles.audioModel}>{t("audioModel", { reason: text })}</span>;
}

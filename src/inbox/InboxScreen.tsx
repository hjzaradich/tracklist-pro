import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { InboxItem, Label, LabelCounts } from "../bindings";
import { StageScreen } from "../shell/StageScreen";
import { useMusicFolders } from "../scan/useMusicFolders";
import { fileNameOf, type Result } from "./inboxText";
import { DoneForYou } from "./DoneForYou";
import { InboxFolders } from "./InboxFolders";
import { InboxRow, type RowAction } from "./InboxRow";
import { ResultLine } from "./ResultLine";
import { useDoneForYou, useInboxAction, useInboxCount, useInboxItems, type InboxAction } from "./useInbox";
import styles from "./inbox.module.css";

/** The labels that have a filter chip, in the order the chips show them. */
const CHIPS = [
  ["new_to_you", "newToYou"],
  ["better_file", "betterFile"],
  ["other_cut", "otherCut"],
  ["rework", "rework"],
  ["already_have", "alreadyHave"],
  ["checking", "checking"],
] as const satisfies readonly (readonly [Label, keyof LabelCounts])[];

/** The items of one batch. */
interface Batch {
  id: number;
  label: string | null;
  arrivedAt: string;
  items: InboxItem[];
}

/** Items grouped by the batch they arrived in, the newest batch first. */
export function batchesOf(items: readonly InboxItem[]): Batch[] {
  const byId = new Map<number, Batch>();
  for (const item of items) {
    const batch = byId.get(item.batchId) ?? {
      id: item.batchId,
      label: item.batchLabel,
      arrivedAt: item.arrivedAt,
      items: [],
    };
    batch.items.push(item);
    if (item.arrivedAt > batch.arrivedAt) batch.arrivedAt = item.arrivedAt;
    byId.set(item.batchId, batch);
  }
  return [...byId.values()].sort((a, b) =>
    a.arrivedAt === b.arrivedAt ? b.id - a.id : a.arrivedAt < b.arrivedAt ? 1 : -1,
  );
}

/**
 * The Inbox (ROADMAP 2.1): what landed in the watched folders, grouped by
 * how it arrived, each file with its label and the reason for it. Files
 * already in the Library wait in a collapsed group at the bottom with one
 * Clear. Every action is one operation the top bar's Undo takes back. The
 * Inbox folders are managed below the list. Files a wishlist asked for come
 * first, in Done for you (2.5).
 */
export function InboxScreen() {
  const { t, i18n } = useTranslation("inbox");
  const [label, setLabel] = useState<Label | null>(null);
  const [alreadyOpen, setAlreadyOpen] = useState(false);
  // What the last action on a row ("item:ID"), a group ("batch:ID") or the Already have group came to.
  const [results, setResults] = useState<Record<string, Result>>({});
  const count = useInboxCount();
  const items = useInboxItems(label);
  const doneForYou = useDoneForYou();
  const folders = useMusicFolders();
  const action = useInboxAction();

  const inboxFolders = (folders.data ?? []).filter((f) => f.role === "inbox");
  const all = items.data ?? [];
  const names = new Map(all.map((item) => [item.id, fileNameOf(item)]));
  const already = all.filter((item) => item.label === "already_have");
  const batches = batchesOf(all.filter((item) => item.label !== "already_have"));
  const failure = items.error ?? count.error ?? folders.error ?? action.error;
  const when = new Intl.DateTimeFormat(i18n.language, { dateStyle: "medium", timeStyle: "short" });

  const perform = (scope: string, request: InboxAction) =>
    action.mutate(request, {
      onSuccess: (outcome) =>
        setResults((before) => {
          const rest = { ...before };
          delete rest[scope];
          const worth = outcome.outcome !== "done" || outcome.skipped.length > 0;
          return worth ? { ...rest, [scope]: outcome } : rest;
        }),
    });
  const act = (item: InboxItem, what: RowAction) => {
    const scope = `item:${item.id}`;
    if (what === "add" || what === "addAnyway") {
      perform(scope, { kind: "add", itemIds: [item.id], confirmLowBitrate: what === "addAnyway" });
    } else {
      perform(scope, { kind: what, itemIds: [item.id] });
    }
  };
  const row = (item: InboxItem) => (
    <InboxRow
      key={item.id}
      item={item}
      busy={action.isPending}
      result={results[`item:${item.id}`]}
      names={names}
      onAct={(what) => act(item, what)}
    />
  );

  const counts = count.data?.byLabel;
  const total = count.data?.new ?? 0;
  const noInboxFolders = folders.data !== undefined && inboxFolders.length === 0;
  const empty = items.data !== undefined && all.length === 0;

  return (
    <StageScreen stage="inbox">
      <div className={styles.screen}>
        {failure != null && (
          <p role="alert" className={styles.problem}>
            {errorMessage(failure)}
          </p>
        )}
        <DoneForYou entries={doneForYou.data ?? []} />
        {counts && total > 0 && (
          <ul className={styles.chips} aria-label={t("filter.label")}>
            <Chip text={t("filter.all")} count={total} active={label === null} onClick={() => setLabel(null)} />
            {CHIPS.filter(([name, key]) => counts[key] > 0 || name === label).map(([name, key]) => (
              <Chip
                key={name}
                text={t(`label.${name}`)}
                count={counts[key]}
                active={label === name}
                onClick={() => setLabel(name)}
              />
            ))}
          </ul>
        )}
        {empty && <p className={styles.muted}>{noInboxFolders ? t("empty.noFolders") : t("empty.zero")}</p>}
        {batches.map((batch) => {
          const result = results[`batch:${batch.id}`];
          const addAll = (confirmLowBitrate: boolean) =>
            perform(`batch:${batch.id}`, { kind: "addBatch", batchId: batch.id, confirmLowBitrate });
          return (
            <section key={batch.id} className={styles.batch} aria-labelledby={`batch-${batch.id}`}>
              <div className={styles.batchHead}>
                <h2 id={`batch-${batch.id}`} className={styles.heading}>
                  {batch.label ?? t("batch.loose")}
                </h2>
                <span className={styles.muted}>
                  {t("batch.arrived", { when: when.format(new Date(batch.arrivedAt)) })}
                  {" · "}
                  {t("batch.count", { count: batch.items.length })}
                </span>
                <button type="button" className={styles.action} disabled={action.isPending} onClick={() => addAll(false)}>
                  {t("action.addAll")}
                </button>
              </div>
              {result && (
                <ResultLine
                  result={result}
                  names={names}
                  onAddAnyway={() => addAll(true)}
                  addAnywayDisabled={action.isPending}
                />
              )}
              <ul className={styles.list}>{batch.items.map(row)}</ul>
            </section>
          );
        })}
        {already.length > 0 && (
          <section className={styles.batch} aria-labelledby="inbox-already">
            <div className={styles.batchHead}>
              <h2 id="inbox-already" className={styles.heading}>
                {t("already.title", { count: already.length })}
              </h2>
              <button
                type="button"
                className={styles.action}
                aria-expanded={alreadyOpen}
                onClick={() => setAlreadyOpen(!alreadyOpen)}
              >
                {alreadyOpen ? t("already.hide") : t("already.show")}
              </button>
              <button
                type="button"
                className={styles.action}
                disabled={action.isPending}
                onClick={() => perform("already", { kind: "clear", itemIds: already.map((item) => item.id) })}
              >
                {t("action.clearAll")}
              </button>
            </div>
            {results.already && (
              <ResultLine result={results.already} names={names} onAddAnyway={() => {}} addAnywayDisabled />
            )}
            {alreadyOpen && <ul className={styles.list}>{already.map(row)}</ul>}
          </section>
        )}
        {folders.data !== undefined && count.data !== undefined && (
          <InboxFolders folders={inboxFolders} inbox={count.data.folders} />
        )}
      </div>
    </StageScreen>
  );
}

function Chip({
  text,
  count,
  active,
  onClick,
}: {
  text: string;
  count: number;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <li>
      <button type="button" className={styles.chip} aria-pressed={active} onClick={onClick}>
        <span>{text}</span> <span className={styles.count}>{count}</span>
      </button>
    </li>
  );
}

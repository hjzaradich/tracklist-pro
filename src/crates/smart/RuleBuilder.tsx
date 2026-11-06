import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Condition, Crate, SmartRule, TagGroup } from "../../bindings";
import { useVocab } from "../../vocab/useVocab";
import styles from "../CratesScreen.module.css";
import { crateChoices } from "../tree";
import {
  clampNumber,
  FIELDS,
  hasEmptyText,
  KEYS,
  MAX_CONDITIONS,
  newCondition,
  OPS,
  RANGES,
  sameRule,
  tagIsGone,
  withOp,
  type Field,
} from "./rule";
import { useRuleCount } from "./useSmartCrates";

/**
 * A smart crate's rule, built from dropdowns so it reads like a sentence:
 * "Library tracks that match all of these: energy at least 7, BPM between
 * 120 and 126". The count under it is live: what the crate would hold with
 * the rule as it stands, before it is saved. Saving is one operation (one
 * Undo); going back drops the changes.
 */
export function RuleBuilder({
  crateId,
  saved,
  all,
  pending,
  onSave,
}: {
  crateId: number;
  /** The stored rule. */
  saved: SmartRule;
  /** Every crate, for "in crate". */
  all: Crate[];
  pending: boolean;
  onSave: (rule: SmartRule) => void;
}) {
  const { t } = useTranslation("smartCrates");
  const [draft, setDraft] = useState<SmartRule>(saved);
  const count = useRuleCount(crateId, draft);
  const groups = useVocab().data;
  const handMade = crateChoices(all).filter(({ crate }) => crate.kind === "static");
  const changed = !sameRule(draft, saved);
  const unfinished = hasEmptyText(draft);
  const full = draft.conditions.length >= MAX_CONDITIONS;

  const setAt = (at: number, condition: Condition) =>
    setDraft({ ...draft, conditions: draft.conditions.map((c, i) => (i === at ? condition : c)) });

  return (
    <section className={styles.form} aria-label={t("builder.label")}>
      <p className={styles.label}>
        {t("builder.matchBefore")}{" "}
        <select
          className={styles.input}
          aria-label={t("builder.combineLabel")}
          value={draft.combine}
          onChange={(event) => setDraft({ ...draft, combine: event.target.value === "any" ? "any" : "all" })}
        >
          <option value="all">{t("builder.combine.all")}</option>
          <option value="any">{t("builder.combine.any")}</option>
        </select>{" "}
        {t("builder.matchAfter")}
      </p>
      {draft.conditions.length === 0 ? (
        <p>{t("builder.none")}</p>
      ) : (
        <ol className={styles.list}>
          {draft.conditions.map((condition, at) => (
            <li key={at}>
              <ConditionRow
                n={at + 1}
                condition={condition}
                crates={handMade.map(({ crate, label }) => ({ id: crate.id, label }))}
                groups={groups}
                onChange={(next) => setAt(at, next)}
                onRemove={() =>
                  setDraft({ ...draft, conditions: draft.conditions.filter((_, i) => i !== at) })
                }
              />
            </li>
          ))}
        </ol>
      )}
      <div className={styles.actions}>
        <button
          type="button"
          className={styles.button}
          disabled={full}
          onClick={() =>
            setDraft({
              ...draft,
              conditions: [...draft.conditions, newCondition("energy", handMade[0]?.crate.id ?? null)],
            })
          }
        >
          {t("builder.add")}
        </button>
      </div>
      {full && <p>{t("builder.full", { count: MAX_CONDITIONS })}</p>}
      <p role="status" aria-live="polite">
        {count.data === undefined ? t("builder.counting") : t("builder.count", { count: count.data })}
      </p>
      <p>{t("builder.lists")}</p>
      {unfinished && <p role="alert">{t("builder.textEmpty")}</p>}
      {changed && (
        <div className={styles.actions}>
          <button
            type="button"
            className={styles.primary}
            disabled={pending || unfinished}
            onClick={() => onSave(draft)}
          >
            {t("builder.save")}
          </button>
          <button type="button" className={styles.button} onClick={() => setDraft(saved)}>
            {t("builder.discard")}
          </button>
        </div>
      )}
    </section>
  );
}

/** One condition: field, comparison and value(s), each a dropdown or a box. */
function ConditionRow({
  n,
  condition,
  crates,
  groups,
  onChange,
  onRemove,
}: {
  n: number;
  condition: Condition;
  crates: { id: number; label: string }[];
  /** The tag groups with their tags; `undefined` until read. */
  groups: TagGroup[] | undefined;
  onChange: (condition: Condition) => void;
  onRemove: () => void;
}) {
  const { t } = useTranslation("smartCrates");
  return (
    <div className={styles.actions}>
      <select
        className={styles.input}
        aria-label={t("builder.fieldFor", { n })}
        value={condition.field}
        onChange={(event) =>
          onChange(
            newCondition(event.target.value as Field, crates[0]?.id ?? null, firstTag(groups)),
          )
        }
      >
        {FIELDS.map((field) => (
          <option key={field} value={field}>
            {t(`field.${field}`)}
          </option>
        ))}
      </select>
      <select
        className={styles.input}
        aria-label={t("builder.opFor", { n })}
        value={condition.op}
        onChange={(event) => onChange(withOp(condition, event.target.value))}
      >
        {OPS[condition.field].map((op) => (
          <option key={op} value={op}>
            {condition.field === "tagGroup" ? t(`opTagGroup.${op as "has" | "hasNot"}`) : t(`op.${op}`)}
          </option>
        ))}
      </select>
      <Value n={n} condition={condition} crates={crates} groups={groups} onChange={onChange} />
      <button
        type="button"
        className={styles.button}
        aria-label={t("builder.removeFor", { n })}
        onClick={onRemove}
      >
        {t("builder.remove")}
      </button>
    </div>
  );
}

/** The first tag, and the first tag group, a new tag condition starts on. */
function firstTag(groups: TagGroup[] | undefined) {
  const group = groups?.find((g) => g.tags.length > 0);
  return group ? { tag: group.tags[0]!.id, group: group.id } : null;
}

function Value({
  n,
  condition,
  crates,
  groups,
  onChange,
}: {
  n: number;
  condition: Condition;
  crates: { id: number; label: string }[];
  groups: TagGroup[] | undefined;
  onChange: (condition: Condition) => void;
}) {
  const { t } = useTranslation("smartCrates");
  const label = t("builder.valueFor", { n });
  switch (condition.field) {
    case "energy":
    case "bpm":
    case "rating": {
      const field = condition.field;
      const [low, high] = RANGES[field];
      const box = (value: number, aria: string, set: (v: number) => void) => (
        <input
          type="number"
          className={styles.input}
          aria-label={aria}
          min={low}
          max={high}
          value={value}
          onChange={(event) => {
            const v = clampNumber(field, event.target.value);
            if (v !== null) set(v);
          }}
        />
      );
      return (
        <>
          {box(condition.value, label, (value) => onChange({ ...condition, value }))}
          {condition.op === "between" && (
            <>
              <span>{t("builder.to")}</span>
              {box(condition.to ?? condition.value, t("builder.toFor", { n }), (to) => onChange({ ...condition, to }))}
            </>
          )}
          {condition.field === "bpm" && (
            <label>
              <input
                type="checkbox"
                checked={condition.halfDouble}
                onChange={(event) => onChange({ ...condition, halfDouble: event.target.checked })}
              />{" "}
              {t("builder.halfDouble")}
            </label>
          )}
        </>
      );
    }
    case "key":
      return (
        <select
          className={styles.input}
          aria-label={label}
          value={condition.value}
          onChange={(event) => onChange({ ...condition, value: event.target.value })}
        >
          {KEYS.map((key) => (
            <option key={key} value={key}>
              {key}
            </option>
          ))}
        </select>
      );
    case "genre":
      return (
        <input
          className={styles.input}
          aria-label={label}
          value={condition.value}
          onChange={(event) => onChange({ ...condition, value: event.target.value })}
        />
      );
    case "tag":
    case "tagGroup": {
      const isTag = condition.field === "tag";
      const current = condition.field === "tag" ? condition.tagId : condition.groupId;
      const gone = groups !== undefined && tagIsGone(condition, groups);
      const nothing = isTag ? !groups?.some((g) => g.tags.length > 0) : groups?.length === 0;
      if (groups !== undefined && nothing && !gone) return <span>{t("builder.noTags")}</span>;
      return (
        <select
          className={styles.input}
          aria-label={label}
          value={current}
          onChange={(event) => {
            const id = Number(event.target.value);
            onChange(condition.field === "tag" ? { ...condition, tagId: id } : { ...condition, groupId: id });
          }}
        >
          {gone && <option value={current}>{t(isTag ? "builder.tagGone" : "builder.groupGone")}</option>}
          {isTag
            ? (groups ?? []).map((group) => (
                <optgroup key={group.id} label={group.name}>
                  {group.tags.map((tag) => (
                    <option key={tag.id} value={tag.id}>
                      {tag.name}
                    </option>
                  ))}
                </optgroup>
              ))
            : (groups ?? []).map((group) => (
                <option key={group.id} value={group.id}>
                  {group.name}
                </option>
              ))}
        </select>
      );
    }
    case "inCrate":
      return crates.length === 0 ? (
        <span>{t("builder.noCrates")}</span>
      ) : (
        <select
          className={styles.input}
          aria-label={label}
          value={condition.crateId}
          onChange={(event) => onChange({ ...condition, crateId: Number(event.target.value) })}
        >
          {crates.map((c) => (
            <option key={c.id} value={c.id}>
              {c.label}
            </option>
          ))}
        </select>
      );
  }
}

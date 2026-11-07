import { Fragment, type ReactNode, useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import type { TimelineEntry, TimelineTransition } from "../bindings";
import { formatKey, type KeyNotation } from "../keys/keyNotation";
import { useKeyNotation } from "../keys/useKeyNotation";
import { formatTime } from "../player/formatTime";
import { shownName } from "../trackTable/columns";
import { entryName, reasonText } from "./gapReasons";
import styles from "./Timeline.module.css";
import { TimelineChart } from "./TimelineChart";
import { addPoint, MOST_POINTS, nextPoint, type Point, shapeOf } from "./timelineShape";
import { useClearTarget, useFillGap, useInsertAt, usePlaylistTimeline, useSetTarget } from "./useTimeline";

/** A gap in the playlist: after this entry (`null`: before the first), where a track inserted there lands. */
interface Gap {
  after: number | null;
  at: number;
}

/**
 * The timeline above a playlist's tracks (3C-P2b; ROADMAP 3.5): when each
 * track starts, the energy curve against the target shape, the key and BPM
 * flow with each transition's flags, and "Fill the gap" at every place a
 * track could go. Every change is one step for the top bar's Undo.
 */
export function TimelinePanel({ playlistId, onDone }: { playlistId: number; onDone: (message: string) => void }) {
  const { t } = useTranslation("playlists");
  const notation = useKeyNotation();
  const timeline = usePlaylistTimeline(playlistId);
  const set = useSetTarget();
  const clear = useClearTarget();
  const [gap, setGap] = useState<Gap | null>(null);

  if (timeline.isError) {
    return (
      <p role="alert" className={styles.error}>
        {errorMessage(timeline.error)}
      </p>
    );
  }
  const data = timeline.data;
  if (data === undefined) return null;
  const shape = shapeOf(data.target);
  const { entries, transitions } = data;
  const commit = (points: Point[]) => {
    set.reset();
    clear.reset();
    // Taking out the last point is clearing the target: one Undo step with its own name.
    if (points.length === 0) {
      return clear.mutateAsync(playlistId, { onSuccess: () => onDone(t("done.targetCleared")) }).catch(() => undefined);
    }
    return set.mutateAsync({ id: playlistId, points }, { onSuccess: () => onDone(t("done.targetSet")) }).catch(() => undefined);
  };
  const failure = [set, clear].find((m) => m.isError);
  const nameOf = (entry: TimelineEntry) => entryName(entry) || t("timeline.untitled");
  const unknown = <span className={styles.unknown}>{t("timeline.unknown")}</span>;

  /** A row holding a place where a track could go, with what its transition found. */
  const place = (key: string, at: Gap, label: string, step?: TimelineTransition) => (
    <tr key={key} className={styles.gapRow} data-gap-row={at.at}>
      <td colSpan={COLUMNS}>
        <div className={styles.gapLine}>
          {step !== undefined && <Flags step={step} />}
          <button
            type="button"
            className={styles.small}
            aria-label={label}
            aria-expanded={gap?.at === at.at}
            onClick={() => setGap(gap?.at === at.at ? null : at)}
          >
            {t("timeline.fill.button")}
          </button>
        </div>
        {gap?.at === at.at && (
          <GapSuggestions
            playlistId={playlistId}
            gap={at}
            onClose={() => setGap(null)}
            onDone={(message) => {
              setGap(null);
              onDone(message);
            }}
          />
        )}
      </td>
    </tr>
  );

  return (
    <section className={styles.panel} aria-label={t("timeline.label")}>
      <p className={styles.total}>
        {t("timeline.total", { time: formatTime(data.totalMs / 1000) })}
        {data.unknownDurations > 0 && <span className={styles.note}> {t("timeline.unknownLengths", { count: data.unknownDurations })}</span>}
      </p>
      {entries.length === 0 ? (
        <p className={styles.note}>{t("noTracks")}</p>
      ) : (
        <>
          <TimelineChart timeline={data} onCommit={commit} />
          <div className={styles.actions}>
            <button
              type="button"
              className={styles.small}
              disabled={shape.length >= MOST_POINTS || set.isPending}
              onClick={() => void commit(addPoint(shape, nextPoint(shape)))}
            >
              {t("timeline.target.add")}
            </button>
            <button
              type="button"
              className={styles.small}
              disabled={shape.length === 0 || clear.isPending}
              onClick={() => {
                set.reset();
                clear.mutate(playlistId, { onSuccess: () => onDone(t("done.targetCleared")) });
              }}
            >
              {t("timeline.target.clear")}
            </button>
            <span className={styles.note}>{shape.length === 0 ? t("timeline.target.none") : t("timeline.target.hint")}</span>
          </div>
          {failure !== undefined && (
            <p role="alert" className={styles.error}>
              {errorMessage(failure.error)}
            </p>
          )}
          <table className={styles.table}>
            <thead>
              <tr>
                <th scope="col">{t("timeline.column.position")}</th>
                <th scope="col">{t("timeline.column.start")}</th>
                <th scope="col">{t("timeline.column.track")}</th>
                <th scope="col">{t("timeline.column.length")}</th>
                <th scope="col">{t("timeline.column.bpm")}</th>
                <th scope="col">{t("timeline.column.key")}</th>
                <th scope="col">{t("timeline.column.energy")}</th>
              </tr>
            </thead>
            <tbody>
              {place("before", { after: null, at: 0 }, t("timeline.fill.before"))}
              {entries.map((entry, i) => {
                const step = transitions[i];
                const next = entries[i + 1];
                return (
                  <Fragment key={entry.entryId}>
                    <tr data-entry-row={entry.entryId}>
                      <td>{entry.position + 1}</td>
                      <td>{formatTime(entry.startMs / 1000)}</td>
                      <td>{nameOf(entry)}</td>
                      <td>{entry.durationMs === null ? unknown : formatTime(entry.durationMs / 1000)}</td>
                      <td>{entry.bpm === null ? unknown : Math.round(entry.bpm * 10) / 10}</td>
                      <KeyCell entry={entry} notation={notation} unknown={unknown} />
                      <td>
                        {entry.energy === null
                          ? unknown
                          : entry.target === null
                            ? entry.energy
                            : t("timeline.energyTarget", { energy: entry.energy, target: Math.round(entry.target * 10) / 10 })}
                      </td>
                    </tr>
                    {place(
                      `after-${entry.entryId}`,
                      { after: entry.entryId, at: entry.position + 1 },
                      next === undefined
                        ? t("timeline.fill.after", { title: nameOf(entry) })
                        : t("timeline.fill.between", { from: nameOf(entry), to: nameOf(next) }),
                      step,
                    )}
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </>
      )}
    </section>
  );
}

const COLUMNS = 7;

/** What a transition found, one mark per signal: a clash or jump, or something unknown (a different look and a different word). */
function Flags({ step }: { step: TimelineTransition }) {
  const { t } = useTranslation("playlists");
  const sign = (n: number | null) => (n === null ? "" : `${n > 0 ? "+" : ""}${Math.round(n * 10) / 10}`);
  const marks: { id: string; flow: "clash" | "unknown"; text: string }[] = [];
  if (step.key !== "fine") marks.push({ id: "key", flow: step.key, text: t(`timeline.flag.key.${step.key}`) });
  if (step.bpm !== "fine") {
    marks.push({ id: "bpm", flow: step.bpm, text: t(`timeline.flag.bpm.${step.bpm}`, { delta: sign(step.bpmDelta) }) });
  }
  if (step.energy !== "fine") {
    marks.push({ id: "energy", flow: step.energy, text: t(`timeline.flag.energy.${step.energy}`, { step: sign(step.energyStep) }) });
  }
  return (
    <span className={styles.flags}>
      {marks.map((mark) => (
        <span key={mark.id} className={styles.flag} data-flow={mark.flow} data-signal={mark.id}>
          {mark.text}
        </span>
      ))}
    </span>
  );
}

/** The tracks that fit one gap, each with what gap it fills, and "Insert here". */
function GapSuggestions({
  playlistId,
  gap,
  onClose,
  onDone,
}: {
  playlistId: number;
  gap: Gap;
  onClose: () => void;
  onDone: (message: string) => void;
}) {
  const { t } = useTranslation("playlists");
  const notation = useKeyNotation();
  const found = useFillGap(playlistId, gap.after, true);
  const insert = useInsertAt();
  // No reason, no suggestion: one whose reasons have no words is left out.
  const shown = (found.data?.suggestions ?? []).flatMap(({ row, reasons }) => {
    const why = reasons.flatMap((reason) => reasonText(t, reason, notation) ?? []);
    return why.length === 0 ? [] : [{ row, why }];
  });
  return (
    <div className={styles.gap} role="region" aria-label={t("timeline.fill.title")}>
      <div className={styles.gapLine}>
        <strong>{t("timeline.fill.title")}</strong>
        {found.data?.target != null && (
          <span className={styles.note}>{t("timeline.fill.target", { target: Math.round(found.data.target * 10) / 10 })}</span>
        )}
        <button type="button" className={styles.small} onClick={onClose}>
          {t("timeline.fill.close")}
        </button>
      </div>
      {found.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(found.error)}
        </p>
      )}
      {insert.isError && (
        <p role="alert" className={styles.error}>
          {errorMessage(insert.error)}
        </p>
      )}
      {found.data !== undefined &&
        (shown.length === 0 ? (
          <p className={styles.note}>{t("timeline.fill.none")}</p>
        ) : (
          <ul className={styles.suggestions} aria-label={t("timeline.fill.list")}>
            {shown.map(({ row, why }) => {
              const name = entryName({ artist: row.artist, title: shownName(row) }) || t("timeline.untitled");
              return (
                <li key={row.recordingId} className={styles.suggestion}>
                  <span className={styles.suggestionName}>{name}</span>
                  <span className={styles.reasons}>{why.join(", ")}</span>
                  <button
                    type="button"
                    className={styles.small}
                    disabled={row.libraryTrackId === null || insert.isPending}
                    aria-label={t("timeline.fill.insertFor", { title: name })}
                    onClick={() => {
                      if (row.libraryTrackId === null) return;
                      insert.mutate(
                        { id: playlistId, track: row.libraryTrackId, at: gap.at },
                        { onSuccess: () => onDone(t("done.inserted", { title: name, position: gap.at + 1 })) },
                      );
                    }}
                  >
                    {t("timeline.fill.insert")}
                  </button>
                </li>
              );
            })}
          </ul>
        ))}
    </div>
  );
}

/** The key cell: an estimate of the app's own is muted and says so, as in the track table (3.7). */
function KeyCell({ entry, notation, unknown }: { entry: TimelineEntry; notation: KeyNotation; unknown: ReactNode }) {
  const { t } = useTranslation("estimate");
  const key = formatKey(entry.key, notation);
  if (key === null || !entry.keyEstimated) return <td>{key ?? unknown}</td>;
  const hint = t("estimated.key");
  return (
    <td className={styles.estimate} title={`${key}. ${hint}`}>
      {key}
      <span className={styles.hidden}>{`. ${hint}`}</span>
    </td>
  );
}

import type { MutableRefObject } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../../api/errors";
import type { Crate, SmartRule } from "../../bindings";
import { libraryRow, type LibraryRow } from "../../library/libraryRow";
import { shownTitle } from "../../library/RemoveFromLibrary";
import { EmptyState } from "../../shell/StageScreen";
import { TrackTableView } from "../../trackTable/TrackTableView";
import type { RowsQuery } from "../../trackTable/useTrackRows";
import styles from "../CratesScreen.module.css";
import { CRATES_QUERY_KEY } from "../useCrates";
import { EMPTY_RULE } from "./rule";
import { RuleBuilder } from "./RuleBuilder";
import { useSmartRule } from "./useSmartCrates";

/**
 * A smart crate's side of the Crates screen (ROADMAP 3.3): its rule builder
 * with the live count, then its tracks. Removing a track never-includes it;
 * tracks dropped on it are always included.
 */
export function SmartCrateView({
  crate,
  all,
  saving,
  onSaveRule,
  onRemoveTrack,
  viewRef,
}: {
  crate: Crate;
  all: Crate[];
  saving: boolean;
  onSaveRule: (rule: SmartRule) => void;
  onRemoveTrack: (track: LibraryRow) => void;
  /** Where the table reports its query, for "start a playlist from it". */
  viewRef?: MutableRefObject<RowsQuery | null>;
}) {
  const { t } = useTranslation("smartCrates");
  const rule = useSmartRule(crate.id);
  return (
    <>
      {rule.isError ? (
        <p role="alert" className={styles.error}>
          {errorMessage(rule.error)}
        </p>
      ) : (
        rule.data !== undefined && (
          <>
            {rule.data === null && <p>{t("unreadable")}</p>}
            <RuleBuilder
              // A saved rule (or an Undo) starts the builder afresh.
              key={`${crate.id}-${JSON.stringify(rule.data)}`}
              crateId={crate.id}
              saved={rule.data ?? EMPTY_RULE}
              all={all}
              pending={saving}
              onSave={onSaveRule}
            />
          </>
        )
      )}
      <TrackTableView
        key={crate.id}
        screen="crate"
        source={{ kind: "smart", id: crate.id, rule: null }}
        keyPrefix={[...CRATES_QUERY_KEY, "table", crate.id]}
        viewRef={viewRef}
        count={(total) => t("trackCount", { count: total })}
        empty={<EmptyState>{t("noTracks")}</EmptyState>}
        actionsWidth={120}
        actions={(row) => {
          const track = libraryRow(row);
          return (
            track && (
              <button
                type="button"
                className={styles.button}
                aria-label={t("never.buttonFor", { title: shownTitle(track) })}
                onClick={() => onRemoveTrack(track)}
              >
                {t("never.button")}
              </button>
            )
          );
        }}
      />
    </>
  );
}

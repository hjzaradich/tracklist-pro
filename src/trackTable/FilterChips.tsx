import { useQuery } from "@tanstack/react-query";
import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { unwrap } from "../api/errors";
import { commands, type TableFilter, type TableSource } from "../bindings";
import type { TableScreen } from "./columns";
import {
  addFilter,
  FILTERS_BY_SCREEN,
  filterLabel,
  formatName,
  removeFilter,
  sameFilter,
} from "./filters";
import styles from "./FilterChips.module.css";

/** The filters that are a plain chip, with nothing to choose first. */
const PLAIN = new Set<TableFilter["kind"]>([
  "missing",
  "inRekordbox",
  "notInRekordbox",
  "qualityProblem",
  "inCrate",
  "inNoCrate",
  "inLibrary",
  "notInLibrary",
]);

/**
 * The filter chips above the track table (1cA-4): the filters in use, each
 * with an × to take it off, and "Add filter", which lists the ones that can
 * be added on this screen. A filter and its opposite ("In rekordbox" and
 * "Not in rekordbox") don't stand together: adding one takes the other off.
 * The format chips are alternatives (any of the formats named).
 *
 * `keyPrefix` is the screen's query key, so the formats on offer are fetched
 * again whenever its lists are.
 */
export function FilterChips({
  screen,
  source,
  keyPrefix,
  filters,
  onChange,
}: {
  screen: TableScreen;
  source: TableSource;
  keyPrefix: readonly unknown[];
  filters: readonly TableFilter[];
  onChange: (filters: TableFilter[]) => void;
}) {
  const { t } = useTranslation("trackTable");
  const [open, setOpen] = useState(false);
  const menuId = useId();
  const addButton = useRef<HTMLButtonElement>(null);
  const root = useRef<HTMLDivElement>(null);

  const offered = FILTERS_BY_SCREEN[screen];
  const formats = useQuery({
    queryKey: [...keyPrefix, "formats", source],
    queryFn: () => unwrap(commands.trackTableFormats(source)),
    enabled: open && offered.includes("format"),
  });

  // A click anywhere else closes the list.
  useEffect(() => {
    if (!open) return;
    const away = (event: MouseEvent) => {
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", away);
    return () => document.removeEventListener("mousedown", away);
  }, [open]);

  const plain: TableFilter[] = offered
    .filter((kind) => PLAIN.has(kind))
    .map((kind) => ({ kind }) as TableFilter)
    .filter((filter) => !filters.some((f) => sameFilter(f, filter)));
  const formatChoices: TableFilter[] = (offered.includes("format") ? (formats.data ?? []) : [])
    .map((format) => ({ kind: "format", format }) as TableFilter)
    .filter((filter) => !filters.some((f) => sameFilter(f, filter)));

  const choose = (filter: TableFilter) => {
    onChange(addFilter(filters, filter));
    setOpen(false);
    addButton.current?.focus();
  };

  return (
    <div className={styles.bar} ref={root}>
      {filters.length > 0 && (
        <ul className={styles.chips} aria-label={t("filters.label")}>
          {filters.map((filter) => {
            const label = filterLabel(filter, t);
            return (
              <li
                key={filter.kind === "format" ? `format:${filter.format}` : filter.kind}
                className={styles.chip}
              >
                <span>{label}</span>
                <button
                  type="button"
                  className={styles.remove}
                  aria-label={t("filters.remove", { filter: label })}
                  title={t("filters.remove", { filter: label })}
                  onClick={() => onChange(removeFilter(filters, filter))}
                >
                  <svg width="8" height="8" viewBox="0 0 8 8" aria-hidden="true" focusable="false">
                    <path d="M1 1l6 6M7 1L1 7" stroke="currentColor" strokeWidth="1.5" fill="none" />
                  </svg>
                </button>
              </li>
            );
          })}
        </ul>
      )}
      <div
        className={styles.add}
        onKeyDown={(event) => {
          if (event.key === "Escape" && open) {
            event.stopPropagation();
            setOpen(false);
            addButton.current?.focus();
          }
        }}
      >
        <button
          ref={addButton}
          type="button"
          className={styles.addButton}
          aria-expanded={open}
          aria-controls={open ? menuId : undefined}
          onClick={() => setOpen((now) => !now)}
        >
          {t("filters.add")}
        </button>
        {open && (
          <div id={menuId} role="group" aria-label={t("filters.menu")} className={styles.menu}>
            {plain.map((filter) => (
              <button
                key={filter.kind}
                type="button"
                className={styles.choice}
                onClick={() => choose(filter)}
              >
                {filterLabel(filter, t)}
              </button>
            ))}
            {formatChoices.length > 0 && (
              <>
                <p className={styles.heading}>{t("filters.formats")}</p>
                {formatChoices.map((filter) => (
                  <button
                    key={filter.kind === "format" ? filter.format : filter.kind}
                    type="button"
                    className={styles.choice}
                    onClick={() => choose(filter)}
                  >
                    {filter.kind === "format" ? formatName(filter.format, t) : ""}
                  </button>
                ))}
              </>
            )}
            {plain.length === 0 && formatChoices.length === 0 && !formats.isLoading && (
              <p className={styles.heading}>{t("filters.none")}</p>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

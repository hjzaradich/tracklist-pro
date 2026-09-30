import { Link } from "@tanstack/react-router";
import { useTranslation } from "react-i18next";
import { STAGES, type StageId } from "./stages";
import styles from "./Sidebar.module.css";

interface SidebarProps {
  className?: string;
  /** Count badges per stage. Nothing feeds them yet. */
  counts?: Partial<Record<StageId, number>>;
}

/**
 * The stages in workflow order, each with a count badge slot, then the
 * Crates section (1.14), where the crates will be listed.
 */
export function Sidebar({ className = "", counts = {} }: SidebarProps) {
  const { t } = useTranslation("shell");
  const workflow = STAGES.filter((stage) => stage.id !== "crates");

  return (
    <nav aria-label={t("stages.label")} className={`${styles.sidebar} ${className}`}>
      <ul className={styles.list}>
        {workflow.map((stage) => (
          <li key={stage.id}>
            <StageLink to={stage.path} label={t(`stages.${stage.id}`)} count={counts[stage.id]} />
          </li>
        ))}
      </ul>
      <div className={styles.section}>
        <StageLink to="/crates" label={t("stages.crates")} count={counts.crates} />
        <p className={styles.empty}>{t("sidebar.noCrates")}</p>
      </div>
    </nav>
  );
}

interface StageLinkProps {
  to: (typeof STAGES)[number]["path"];
  label: string;
  count?: number;
}

function StageLink({ to, label, count }: StageLinkProps) {
  return (
    <Link to={to} className={styles.link}>
      <span className={styles.label}>{label}</span>
      <CountBadge count={count} />
    </Link>
  );
}

/** A stage's count, e.g. how many decisions are waiting in Review. */
export function CountBadge({ count }: { count?: number }) {
  const { i18n } = useTranslation();
  if (count === undefined) return null;
  return (
    <span className={styles.badge} data-testid="count-badge">
      {new Intl.NumberFormat(i18n.language).format(count)}
    </span>
  );
}

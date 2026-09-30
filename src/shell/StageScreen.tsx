import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { StageId } from "./stages";
import styles from "./StageScreen.module.css";

interface StageScreenProps {
  stage: StageId;
  children?: ReactNode;
}

/** The frame every stage screen shares: a header with the stage's name, then its content. */
export function StageScreen({ stage, children }: StageScreenProps) {
  const { t } = useTranslation("shell");

  return (
    <div className={styles.screen}>
      <div className={styles.header}>
        <h1 className={styles.title}>{t(`stages.${stage}`)}</h1>
      </div>
      <div className={styles.body}>{children}</div>
    </div>
  );
}

/** A screen's placeholder line until its feature arrives. */
export function EmptyState({ children }: { children: ReactNode }) {
  return <p className={styles.empty}>{children}</p>;
}

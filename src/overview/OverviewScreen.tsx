import { useTranslation } from "react-i18next";
import { XmlSourcePanel } from "../rekordbox/XmlSourcePanel";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/** Empty until its feature arrives, apart from where rekordbox's collection comes from (1aB-11). */
export function OverviewScreen() {
  const { t } = useTranslation("overview");

  return (
    <StageScreen stage="overview">
      <EmptyState>{t("empty")}</EmptyState>
      <XmlSourcePanel />
    </StageScreen>
  );
}

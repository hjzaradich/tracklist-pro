import { useTranslation } from "react-i18next";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/** Empty until its feature arrives. */
export function CratesScreen() {
  const { t } = useTranslation("crates");

  return (
    <StageScreen stage="crates">
      <EmptyState>{t("empty")}</EmptyState>
    </StageScreen>
  );
}

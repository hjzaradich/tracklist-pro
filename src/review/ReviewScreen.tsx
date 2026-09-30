import { useTranslation } from "react-i18next";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/** Empty until its feature arrives. */
export function ReviewScreen() {
  const { t } = useTranslation("review");

  return (
    <StageScreen stage="review">
      <EmptyState>{t("empty")}</EmptyState>
    </StageScreen>
  );
}

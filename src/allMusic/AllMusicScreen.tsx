import { useTranslation } from "react-i18next";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/** Empty until its feature arrives. */
export function AllMusicScreen() {
  const { t } = useTranslation("allMusic");

  return (
    <StageScreen stage="allMusic">
      <EmptyState>{t("empty")}</EmptyState>
    </StageScreen>
  );
}

import { useTranslation } from "react-i18next";
import { EmptyState, StageScreen } from "../shell/StageScreen";

/** Empty until its feature arrives. */
export function LibraryScreen() {
  const { t } = useTranslation("library");

  return (
    <StageScreen stage="library">
      <EmptyState>{t("empty")}</EmptyState>
    </StageScreen>
  );
}

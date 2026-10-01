import { useTranslation } from "react-i18next";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { MissingList } from "./MissingList";
import { useAddMissingFolder } from "./useAddMissingFolder";
import { useMissingTracks } from "./useMissingTracks";

/** The decision queue (until its feature arrives) and the Missing list. */
export function ReviewScreen() {
  const { t } = useTranslation("review");
  const missing = useMissingTracks();
  const addFolder = useAddMissingFolder();

  return (
    <StageScreen stage="review">
      <EmptyState>{t("empty")}</EmptyState>
      {missing.data && <MissingList list={missing.data} onAddFolder={addFolder} />}
    </StageScreen>
  );
}

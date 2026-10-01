import { useTranslation } from "react-i18next";
import { errorMessage } from "../api/errors";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { MissingList } from "./MissingList";
import { useAddMissingFolder } from "./useAddMissingFolder";
import { useMissingTracks } from "./useMissingTracks";

/** The decision queue (until its feature arrives) and the Missing list. */
export function ReviewScreen() {
  const { t } = useTranslation("review");
  const missing = useMissingTracks();
  const add = useAddMissingFolder();
  const failure = missing.error ?? add.error;

  return (
    <StageScreen stage="review">
      <EmptyState>{t("empty")}</EmptyState>
      <MissingList
        list={missing.data}
        problem={failure ? errorMessage(failure) : undefined}
        adding={add.isPending}
        onAddFolder={(folder) => add.mutate(folder)}
      />
    </StageScreen>
  );
}

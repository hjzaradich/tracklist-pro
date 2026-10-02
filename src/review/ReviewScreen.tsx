import { useTranslation } from "react-i18next";
import { useWaitingOn } from "../activity/useWaitingOn";
import { errorMessage } from "../api/errors";
import { EmptyState, StageScreen } from "../shell/StageScreen";
import { MissingList } from "./MissingList";
import { useAddMissingFolder } from "./useAddMissingFolder";
import { useMissingTracks } from "./useMissingTracks";

/** The background work that decides which rekordbox tracks have no file. */
const DECIDES_MISSING = ["read_rekordbox", "relink"] as const;

/** The decision queue (until its feature arrives) and the Missing list. */
export function ReviewScreen() {
  const { t } = useTranslation("review");
  const missing = useMissingTracks();
  const add = useAddMissingFolder();
  const failure = missing.error ?? add.error;
  const waiting = useWaitingOn(DECIDES_MISSING);

  return (
    <StageScreen stage="review">
      <EmptyState>{t("empty")}</EmptyState>
      <MissingList
        list={missing.data}
        problem={failure ? errorMessage(failure) : undefined}
        adding={add.isPending}
        waiting={waiting}
        onAddFolder={(folder) => add.mutate(folder)}
      />
    </StageScreen>
  );
}

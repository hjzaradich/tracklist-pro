import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { unwrap } from "../api/errors";
import { commands, type AudioModelStatus, type Guesses, type SoundsLike, type YourGenres } from "../bindings";

// The audio model (ROADMAP 3.8): off until the owner turns it on in Settings.
// Everything it shows is its opinion, read from what it stored; nothing here
// loads the model or changes a track.

/** Everything the audio model asks for starts with this. */
export const AUDIO_MODEL_QUERY_KEY = ["audioModel"] as const;
const STATUS_KEY = [...AUDIO_MODEL_QUERY_KEY, "status"] as const;

/** The switch and how far the listening has got. */
export function useAudioModelStatus() {
  return useQuery({
    queryKey: STATUS_KEY,
    queryFn: (): Promise<AudioModelStatus> => unwrap(commands.audioModelStatus()),
  });
}

/** Turns the audio model on or off. The answer is the new status. */
export function useSetAudioModel() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (on: boolean) => unwrap(commands.setAudioModelEnabled(on)),
    onSuccess: (status) => queryClient.setQueryData(STATUS_KEY, status),
    onSettled: () => queryClient.invalidateQueries({ queryKey: AUDIO_MODEL_QUERY_KEY }),
  });
}

/** The model's genre and mood guesses for a track. */
export function useAudioGuesses(recordingId: number) {
  return useQuery({
    queryKey: [...AUDIO_MODEL_QUERY_KEY, "guesses", recordingId],
    queryFn: (): Promise<Guesses> => unwrap(commands.audioModelGuesses(recordingId)),
  });
}

/** The Library tracks that sound most like a track; asked for only once `enabled`. */
export function useSoundsLike(recordingId: number, enabled: boolean) {
  return useQuery({
    queryKey: [...AUDIO_MODEL_QUERY_KEY, "soundsLike", recordingId],
    queryFn: (): Promise<SoundsLike> => unwrap(commands.soundsLike(recordingId)),
    enabled,
  });
}

/**
 * Up to 3 of your own genres the model hears in a track, learned from your
 * own genres, with the reason when the list is empty.
 */
export function useYourGenres(recordingId: number) {
  return useQuery({
    queryKey: [...AUDIO_MODEL_QUERY_KEY, "yourGenres", recordingId],
    queryFn: (): Promise<YourGenres> => unwrap(commands.audioModelYourGenres(recordingId)),
  });
}

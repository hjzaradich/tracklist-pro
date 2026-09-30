import { useEffect } from "react";
import { events } from "../bindings";
import { useScannedFilesStore } from "./scannedFilesStore";

/** Adds each batch of files the walk sends to the scanned-files store. */
export function useScannedFilesSync() {
  useEffect(() => {
    const { add } = useScannedFilesStore.getState();
    let stopped = false;
    let unlisten: (() => void) | undefined;

    events.scannedFiles
      .listen((event) => add(event.payload))
      .then((stop) => {
        if (stopped) stop();
        else unlisten = stop;
      })
      // Outside the app (e.g. a plain browser) no files ever arrive.
      .catch(() => {});

    return () => {
      stopped = true;
      unlisten?.();
    };
  }, []);
}

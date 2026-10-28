import { useEffect, useRef } from "react";
import { create } from "zustand";

// Some actions end on a screen that may not be mounted yet (Send is on the
// Overview, the search box is on All music). The palette navigates, then
// leaves a request here; the screen answers it once it is on screen, with
// its own handler. A request is answered once.

export type Request = "send" | "searchAllMusic";

interface Requests {
  asked: Readonly<Record<Request, boolean>>;
  ask: (request: Request) => void;
  answered: (request: Request) => void;
}

export const useRequests = create<Requests>((set) => ({
  asked: { send: false, searchAllMusic: false },
  ask: (request) => set((state) => ({ asked: { ...state.asked, [request]: true } })),
  answered: (request) => set((state) => ({ asked: { ...state.asked, [request]: false } })),
}));

/** Calls `answer` when `request` was made (or already stands) while this is mounted. */
export function useAnswerRequest(request: Request, answer: () => void) {
  const asked = useRequests((state) => state.asked[request]);
  const latest = useRef(answer);
  useEffect(() => {
    latest.current = answer;
  });
  useEffect(() => {
    if (!asked) return;
    useRequests.getState().answered(request);
    latest.current();
  }, [asked, request]);
}

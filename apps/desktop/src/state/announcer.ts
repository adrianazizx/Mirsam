import { create } from "zustand";

/**
 * What screen readers are told about changes nobody is looking at: a save, an
 * error, the page or zoom a command moved to. `LiveRegions` renders the two
 * regions; anything may call `announce`.
 */
export type Politeness = "polite" | "assertive";

export interface Announcement {
  text: string;
  /** Grows with every announcement, so the same words said twice are spoken twice. */
  seq: number;
}

export interface AnnouncerState {
  polite: Announcement;
  assertive: Announcement;
}

export const useAnnouncer = create<AnnouncerState>(() => ({
  polite: { text: "", seq: 0 },
  assertive: { text: "", seq: 0 },
}));

let seq = 0;

/** Tells screen readers `text`: politely (after what they are saying) unless it is urgent. */
export function announce(text: string, politeness: Politeness = "polite"): void {
  if (!text) return;
  seq += 1;
  useAnnouncer.setState({ [politeness]: { text, seq } });
}

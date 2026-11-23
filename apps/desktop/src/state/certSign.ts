import { create } from "zustand";
import type {
  CertSignRequest,
  PickedCertificate,
  SignatureReport,
  SigningIdentity,
} from "../backend";

/**
 * Certificate signatures in the UI (ADR 0048): the signing dialog's form,
 * the identities to sign with, a PKCS#12 file waiting for its password, a
 * visible signature waiting to be placed, and each document's checked
 * signatures. Kept apart from the app store: nothing here is a setting, and
 * the time-stamping authority's address is stored through `set_setting`.
 */

export interface CertSignForm {
  /** The chosen identity's id. */
  identity: string | null;
  reason: string;
  location: string;
  contact: string;
  /** Time-stamp it (PAdES B-T) from `tsaUrl`. */
  timestamp: boolean;
  tsaUrl: string;
  visible: boolean;
  /** The kept signature a visible one shows, or none (the caption alone). */
  graphic: number | null;
}

export const EMPTY_FORM: CertSignForm = {
  identity: null,
  reason: "",
  location: "",
  contact: "",
  timestamp: false,
  tsaUrl: "",
  visible: false,
  graphic: null,
};

/** A document's signatures as last checked. */
export interface SignatureReports {
  status: "loading" | "done" | "error";
  list: SignatureReport[];
}

/** A visible signature waiting for a press on a page: what it will sign with. */
export interface Placing {
  documentId: number;
  request: Omit<CertSignRequest, "visible">;
  graphic: number | null;
  saveAs: boolean;
}

export interface CertSignState {
  dialogOpen: boolean;
  /** Null until listed. */
  identities: SigningIdentity[] | null;
  form: CertSignForm;
  /** A picked PKCS#12 file asking for its password. */
  picked: PickedCertificate | null;
  /** Why the last password did not open it (`certificate_password`, …). */
  unlockError: string | null;
  /** Signing runs (a time-stamp can take seconds). */
  busy: boolean;
  /** Why signing failed, shown in the dialog (a code under `errors.`). */
  error: string | null;
  placing: Placing | null;
  reports: Record<number, SignatureReports>;
  setDialogOpen(open: boolean): void;
  setIdentities(identities: SigningIdentity[]): void;
  updateForm(patch: Partial<CertSignForm>): void;
  setPicked(picked: PickedCertificate | null, error?: string | null): void;
  setBusy(busy: boolean): void;
  setError(error: string | null): void;
  setPlacing(placing: Placing | null): void;
  setReports(documentId: number, reports: SignatureReports | null): void;
}

export const useCertSign = create<CertSignState>((set) => ({
  dialogOpen: false,
  identities: null,
  form: EMPTY_FORM,
  picked: null,
  unlockError: null,
  busy: false,
  error: null,
  placing: null,
  reports: {},
  setDialogOpen(dialogOpen) {
    set({ dialogOpen });
  },
  setIdentities(identities) {
    set((state) => ({
      identities,
      // The chosen one, else the first.
      form: {
        ...state.form,
        identity: identities.some((one) => one.id === state.form.identity)
          ? state.form.identity
          : (identities[0]?.id ?? null),
      },
    }));
  },
  updateForm(patch) {
    set((state) => ({ form: { ...state.form, ...patch } }));
  },
  setPicked(picked, error = null) {
    set({ picked, unlockError: error });
  },
  setBusy(busy) {
    set({ busy });
  },
  setError(error) {
    set({ error });
  },
  setPlacing(placing) {
    set({ placing });
  },
  setReports(documentId, reports) {
    set((state) => {
      const next = { ...state.reports };
      if (reports) next[documentId] = reports;
      else delete next[documentId];
      return { reports: next };
    });
  },
}));

/** The box a visible signature takes, in points as displayed. */
export const VISIBLE_SIZE = { width: 200, height: 72 } as const;

/** A document's overall verdict: the worst of its signatures. */
export function overallStatus(list: SignatureReport[]): SignatureReport["status"] | null {
  if (list.length === 0) return null;
  if (list.some((one) => one.status === "invalid")) return "invalid";
  if (list.some((one) => one.status === "unknown")) return "unknown";
  return "valid";
}

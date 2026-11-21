import { create } from "zustand";
import type { DocumentInfo, PageSize } from "../backend";
import type { Locale } from "../i18n";
import { clampZoom } from "./zoom";

export const TOOLS = [
  "select",
  "edit",
  "comment",
  "pages",
  "forms",
  "sign",
  "protect",
  "ocr",
] as const;
export type ToolId = (typeof TOOLS)[number];

export type ThemePreference = "system" | "light" | "dark";

export interface OpenDocument extends DocumentInfo {
  /** Zero-based index of the page in view. */
  page: number;
  zoomMode: "fit-width" | "custom";
  /** Effective zoom factor (1 = 100%). In fit-width mode the viewer keeps this current. */
  zoom: number;
  /** Size of the page in view, once known. */
  pageSize: PageSize | null;
}

export interface AppState {
  locale: Locale;
  theme: ThemePreference;
  activeTool: ToolId;
  documents: OpenDocument[];
  activeDocumentId: number | null;
  /** Code of the most recent user-facing error (a key under `errors.`), or null. */
  errorCode: string | null;
  galleryOpen: boolean;

  setLocale(locale: Locale): void;
  setTheme(theme: ThemePreference): void;
  setActiveTool(tool: ToolId): void;
  addDocument(info: DocumentInfo): void;
  removeDocument(id: number): void;
  activateDocument(id: number): void;
  updateDocument(id: number, patch: Partial<Omit<OpenDocument, keyof DocumentInfo>>): void;
  setError(code: string | null): void;
  setGalleryOpen(open: boolean): void;
}

const PREFERENCES_KEY = "safha.preferences";

interface Preferences {
  locale: Locale;
  theme: ThemePreference;
}

// Interim storage: preferences move to the core's SQLite store in Milestone 1.
function loadPreferences(): Preferences {
  const fallback: Preferences = { locale: "en", theme: "system" };
  try {
    const raw = localStorage.getItem(PREFERENCES_KEY);
    if (!raw) return fallback;
    const saved = JSON.parse(raw) as Partial<Preferences>;
    return {
      locale: saved.locale === "ar" ? "ar" : "en",
      theme: saved.theme === "light" || saved.theme === "dark" ? saved.theme : "system",
    };
  } catch {
    return fallback;
  }
}

function savePreferences(preferences: Preferences) {
  try {
    localStorage.setItem(PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // Storage can be unavailable (private mode, quota); preferences then last one session.
  }
}

export const useAppStore = create<AppState>((set, get) => ({
  ...loadPreferences(),
  activeTool: "select",
  documents: [],
  activeDocumentId: null,
  errorCode: null,
  galleryOpen: false,

  setLocale(locale) {
    set({ locale });
    savePreferences({ locale, theme: get().theme });
  },
  setTheme(theme) {
    set({ theme });
    savePreferences({ locale: get().locale, theme });
  },
  setActiveTool(activeTool) {
    set({ activeTool });
  },
  addDocument(info) {
    const document: OpenDocument = {
      ...info,
      page: 0,
      zoomMode: "fit-width",
      zoom: 1,
      pageSize: null,
    };
    set((state) => ({
      documents: [...state.documents.filter((open) => open.id !== info.id), document],
      activeDocumentId: info.id,
      errorCode: null,
    }));
  },
  removeDocument(id) {
    set((state) => {
      const index = state.documents.findIndex((open) => open.id === id);
      const documents = state.documents.filter((open) => open.id !== id);
      const neighbour = documents[Math.min(index, documents.length - 1)];
      return {
        documents,
        activeDocumentId:
          state.activeDocumentId === id ? (neighbour?.id ?? null) : state.activeDocumentId,
      };
    });
  },
  activateDocument(id) {
    if (get().documents.some((open) => open.id === id)) set({ activeDocumentId: id });
  },
  updateDocument(id, patch) {
    set((state) => ({
      documents: state.documents.map((open) => {
        if (open.id !== id) return open;
        const next = { ...open, ...patch };
        next.page = Math.min(Math.max(0, next.page), Math.max(0, open.pageCount - 1));
        next.zoom = clampZoom(next.zoom);
        return next;
      }),
    }));
  },
  setError(errorCode) {
    set({ errorCode });
  },
  setGalleryOpen(galleryOpen) {
    set({ galleryOpen });
  },
}));

export function selectActiveDocument(state: AppState): OpenDocument | null {
  return state.documents.find((open) => open.id === state.activeDocumentId) ?? null;
}

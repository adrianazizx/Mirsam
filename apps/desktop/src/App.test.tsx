import { act, render, screen, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { backend } from "./backend";
import { setMockRecoveries } from "./backend/mock";
import { useAnnouncer } from "./state/announcer";
import { useAppStore } from "./state/appStore";

async function settle() {
  await act(async () => {
    for (let i = 0; i < 6; i += 1) await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  // Settings persist (the mock backend keeps them in localStorage); start clean.
  localStorage.clear();
  setMockRecoveries([]);
  useAppStore.setState({
    locale: "en",
    theme: "light",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    unlocks: [],
    recoveries: [],
    galleryOpen: false,
    activeTool: "select",
  });
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("the app's start", () => {
  it.each([
    {
      locale: "en" as const,
      dir: "ltr",
      message: "The file could not be read or written.",
      recovery: "Recover unsaved changes?",
    },
    {
      locale: "ar" as const,
      dir: "rtl",
      message: "تعذّرت قراءة الملف أو كتابته.",
      recovery: "استعادة التغييرات غير المحفوظة؟",
    },
  ])(
    "keeps the restored tabs and the recovery offer when a launch document cannot be opened ($locale)",
    async ({ locale, dir, message, recovery }) => {
      // The session's tab (the mock's sample) opened; the launch document did not.
      const restored = await backend.openDocumentDialog();
      if (restored?.kind !== "opened") throw new Error("the mock opens its sample");
      vi.spyOn(backend, "startup").mockResolvedValue({
        documents: [restored],
        active: restored.document.id,
        recoveries: [
          { token: 9, name: "draft.pdf", savedAt: 1_795_000_000, originalChanged: false },
        ],
        first: true,
        error: "io_error",
      });
      act(() => useAppStore.getState().setLocale(locale));
      render(<App />);
      await settle();

      const app = useAppStore.getState();
      expect(app.documents.map((open) => open.name)).toEqual([restored.document.name]);
      expect(app.activeDocumentId).toBe(restored.document.id);
      expect(screen.getByRole("tab", { name: restored.document.name, hidden: true })).toBeTruthy();
      expect(document.documentElement.dir).toBe(dir);
      // The error is shown and said, in the interface's language.
      expect(app.errorCode).toBe("io_error");
      expect(screen.getByTestId("error-banner").textContent).toContain(message);
      expect(useAnnouncer.getState().assertive.text).toContain(message);
      // The recovery offer still comes.
      const dialog = await screen.findByTestId("recovery");
      expect(within(dialog).getByRole("heading", { name: recovery })).toBeTruthy();
    },
  );
});

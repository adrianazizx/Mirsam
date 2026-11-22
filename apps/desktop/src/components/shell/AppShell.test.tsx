import { act, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../../App";
import { useAppStore } from "../../state/appStore";

beforeEach(() => {
  useAppStore.setState({
    locale: "en",
    theme: "light",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    galleryOpen: false,
    activeTool: "select",
  });
});

describe("app shell", () => {
  it("renders the empty state and all eight tools in English, left to right", async () => {
    render(<App />);
    expect(await screen.findByRole("heading", { name: "Open a PDF to get started" })).toBeTruthy();
    expect(document.documentElement.dir).toBe("ltr");
    expect(document.documentElement.lang).toBe("en");
    const tools = screen.getByRole("navigation", { name: "Tools" });
    expect(tools.querySelectorAll("button")).toHaveLength(8);
    expect(screen.getByRole("radio", { name: "Select" }).getAttribute("aria-checked")).toBe("true");
  });

  it("switches the whole document to Arabic and right to left", async () => {
    render(<App />);
    act(() => useAppStore.getState().setLocale("ar"));
    expect(await screen.findByRole("heading", { name: "افتح ملف PDF للبدء" })).toBeTruthy();
    expect(document.documentElement.dir).toBe("rtl");
    expect(document.documentElement.lang).toBe("ar");
    expect(screen.getByRole("navigation", { name: "الأدوات" })).toBeTruthy();
  });

  it("applies the chosen theme to the root element", () => {
    render(<App />);
    act(() => useAppStore.getState().setTheme("dark"));
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("shows open documents as tabs with document info, and a localized error banner", async () => {
    render(<App />);
    act(() => {
      useAppStore.getState().addDocument({ id: 7, name: "عقد.pdf", pageCount: 12 });
      useAppStore.getState().setError("invalid_pdf");
    });
    expect(await screen.findByRole("tab", { name: "عقد.pdf", selected: true })).toBeTruthy();
    const pageNumber = screen.getByRole("textbox", { name: "Page number" }) as HTMLInputElement;
    expect(pageNumber.value).toBe("1");
    expect(screen.getByText("of 12")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("could not be read");
  });

  it("offers view modes and page order in the status bar, in both languages", async () => {
    render(<App />);
    act(() => useAppStore.getState().addDocument({ id: 3, name: "book.pdf", pageCount: 9 }));
    expect(await screen.findByRole("region", { name: "Pages of book.pdf" })).toBeTruthy();
    expect(screen.getByTestId("view-mode").textContent).toContain("Continuous");
    expect(screen.getByTestId("page-order").textContent).toContain("Left to right");
    act(() => {
      useAppStore.getState().updateDocument(3, { viewMode: "book", pageOrder: "rtl" });
      useAppStore.getState().setLocale("ar");
    });
    expect(await screen.findByRole("textbox", { name: "رقم الصفحة" })).toBeTruthy();
    expect(screen.getByTestId("view-mode").textContent).toContain("كتاب");
    expect(screen.getByTestId("page-order").textContent).toContain("من اليمين");
    // The page area keeps its own direction whatever the UI language.
    expect(document.documentElement.dir).toBe("rtl");
  });
});

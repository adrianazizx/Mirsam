import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { isolate } from "../i18n";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    outputDialog: null,
    watermarkImage: null,
    compressed: null,
  });
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

const band = {
  edge: "bottom" as const,
  start: "",
  center: "{page}",
  end: "",
  rtl: false,
  family: "sans" as const,
  size: 10,
  color: [0, 0, 0] as [number, number, number],
  margin: 30,
  digits: "western" as const,
  firstNumber: 1,
};

describe("output commands (ADR 0050)", () => {
  it("open their dialogs, and a stamp is one undoable edit", async () => {
    await runCommand("pages.pageNumbers");
    expect(useAppStore.getState().outputDialog).toBe("pageNumbers");
    await runCommand("output.stamp", { pages: [0, 1, 2], stamp: { kind: "band", band } });
    expect(useAppStore.getState().outputDialog).toBeNull();
    expect(active()?.modified).toBe(true);
    await runCommand("edit.undo");
    expect(active()?.modified).toBe(false);
  });

  it("an image watermark needs an image picked first", async () => {
    const watermark = {
      mark: { kind: "image" as const, width: 0.5 },
      opacity: 0.3,
      rotation: 0,
      behind: false,
    };
    await runCommand("output.stamp", { pages: [0], stamp: { kind: "watermark", watermark } });
    expect(useAppStore.getState().errorCode).toBe("no_image");
    await runCommand("output.pickImage");
    expect(useAppStore.getState().watermarkImage).toBe("logo.png");
    await runCommand("output.stamp", { pages: [0], stamp: { kind: "watermark", watermark } });
    expect(active()?.modified).toBe(true);
  });

  it("exports images and text, and says where", async () => {
    await runCommand("output.exportImages", {
      pages: [0, 1],
      dpi: 150,
      imageFormat: { kind: "jpeg", quality: 85 },
    });
    expect(useAppStore.getState().notice).toEqual({
      key: "exportedImages",
      values: { count: "2" },
    });
    await runCommand("output.exportText", { pages: [0], textFormat: "markdown" });
    expect(useAppStore.getState().notice).toEqual({
      key: "exportedText",
      // The name is isolated in the sentence (either direction).
      values: { name: isolate("Sample.md") },
    });
  });

  it("compresses to a preview, then saves the copy", async () => {
    expect(isCommandEnabled("output.saveCompressed")).toBe(false);
    await runCommand("output.compress", {
      compress: { imageDpi: 150, jpegQuality: null, objectStreams: true },
    });
    expect(useAppStore.getState().compressed?.report.after).toBeLessThan(
      useAppStore.getState().compressed?.report.before ?? 0,
    );
    expect(active()?.modified).toBe(false);
    await runCommand("output.saveCompressed");
    expect(useAppStore.getState().notice?.key).toBe("compressedSaved");
  });

  it.each([
    ["en", "Page numbers", "Apply", "Page 1 of 9"],
    ["ar", "أرقام الصفحات", "تطبيق", "صفحة 1 من 9"],
  ] as const)(
    "the page numbers dialog works in %s, in its own direction",
    async (locale, title, apply, format) => {
      render(<App />);
      act(() => useAppStore.getState().setLocale(locale));
      await act(() => runCommand("pages.pageNumbers"));
      const dialog = await screen.findByTestId("output-pageNumbers");
      expect(within(dialog).getByRole("heading").textContent).toBe(title);
      expect(within(dialog).getByRole("radio", { name: format })).toBeTruthy();
      expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
      // Arabic starts on the right with Arabic-Indic digits.
      const rtl = within(dialog).getByRole("radio", {
        name: locale === "ar" ? "من اليمين إلى اليسار" : "Right to left",
      });
      expect((rtl as HTMLInputElement).checked).toBe(locale === "ar");
      // A range typed in Arabic-Indic digits is read.
      fireEvent.click(
        within(dialog).getByRole("radio", { name: locale === "ar" ? "نطاق" : "Range" }),
      );
      const field = within(dialog).getByRole("textbox", {
        name: locale === "ar" ? "نطاق الصفحات" : "Page range",
      });
      fireEvent.change(field, { target: { value: "١-٣" } });
      fireEvent.click(within(dialog).getByRole("button", { name: apply }));
      await waitFor(() => expect(useAppStore.getState().outputDialog).toBeNull());
      expect(active()?.modified).toBe(true);
    },
  );

  it("follows the file's permissions, and does not compress an encrypted file (ADR 0043)", () => {
    const permissions = {
      print: true,
      printHighQuality: true,
      modify: false,
      copy: false,
      annotate: true,
      fillForms: true,
      accessibility: true,
      assemble: true,
    };
    useAppStore.setState((state) => ({
      documents: state.documents.map((document) => ({
        ...document,
        security: { encrypted: true, cipher: "aes256", revision: 6, permissions, owner: false },
      })),
    }));
    for (const id of [
      "pages.pageNumbers",
      "pages.watermark",
      "file.exportText",
      "file.exportImages",
      "file.compress",
    ] as const) {
      expect(isCommandEnabled(id), id).toBe(false);
    }
  });

  it("the compress dialog shows the sizes before anything is saved", async () => {
    render(<App />);
    await act(() => runCommand("file.compress"));
    const dialog = await screen.findByTestId("output-compress");
    fireEvent.click(within(dialog).getByRole("button", { name: "Compress" }));
    const report = await screen.findByTestId("compress-report");
    expect(report.textContent).toContain("4.2 MB → 1.4 MB (68% smaller)");
    expect(within(dialog).getByRole("button", { name: "Save compressed copy…" })).toBeTruthy();
  });
});

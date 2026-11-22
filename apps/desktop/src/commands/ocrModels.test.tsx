import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import { failNextMockDownload, resetMockOcr } from "../backend/mockOcr";
import { useAppStore } from "../state/appStore";
import { useOcrModels } from "../state/ocrModels";
import { getCommand, isCommandEnabled, runCommand, shortcutLabel } from "./registry";

const ocr = () => useOcrModels.getState();
const app = () => useAppStore.getState();

beforeEach(() => {
  localStorage.clear();
  resetMockOcr();
  useOcrModels.setState({ models: null, jobs: {}, failures: {}, prompt: false });
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    notice: null,
    activeTool: "select",
    settingsOpen: false,
  });
});

function openDocument() {
  act(() => app().addDocument({ id: 1, name: "scan.pdf", pageCount: 3 }));
}

describe("OCR models (ADR 0034)", () => {
  it("asks to download the model the first time OCR is used, explaining its size", async () => {
    render(<App />);
    expect(isCommandEnabled("ocr.recognize")).toBe(false);
    openDocument();
    await act(() => runCommand("ocr.recognize"));
    expect(ocr().prompt).toBe(true);

    const prompt = await screen.findByTestId("ocr-prompt");
    expect(within(prompt).getByRole("heading", { name: "Download the OCR model?" })).toBeTruthy();
    expect(prompt.textContent).toContain("Nuqta-Arabic-OCR 4B");
    expect(prompt.textContent).toContain("never leave it");
    expect(prompt.textContent).toContain("one-time download of 5.5 GB");
    // Apple silicon in the mock: no slowness warning.
    expect(prompt.textContent).not.toContain("slow");

    fireEvent.click(within(prompt).getByRole("button", { name: "Download" }));
    // The prompt gives way to Settings, where the download shows.
    await waitFor(() => expect(screen.queryByTestId("ocr-prompt")).toBeNull());
    const settings = await screen.findByTestId("settings");
    const row = within(settings).getByTestId("ocr-model-nuqta-4b");
    await waitFor(() => expect(within(row).getByRole("progressbar")).toBeTruthy());
    expect(within(row).getByRole("button", { name: "Cancel" })).toBeTruthy();

    await waitFor(() => expect(within(row).getByText("Installed")).toBeTruthy());
    expect(ocr().models?.packages[0]?.state).toEqual({ kind: "installed" });
    expect(ocr().jobs).toEqual({});

    // With the model there, OCR no longer asks.
    await act(() => runCommand("ocr.recognize"));
    expect(ocr().prompt).toBe(false);
    expect(app().notice?.key).toBe("ocrNotYet");
  });

  it("warns that OCR is slow without a usable GPU, and can be put off", async () => {
    resetMockOcr({ hardware: { accelerators: [], runtimeGpu: null, os: "linux", arch: "x86_64" } });
    render(<App />);
    openDocument();
    await act(() => runCommand("ocr.recognize"));
    const prompt = await screen.findByTestId("ocr-prompt");
    expect(prompt.textContent).toContain("a few minutes a page");
    fireEvent.click(within(prompt).getByRole("button", { name: "Not now" }));
    await waitFor(() => expect(screen.queryByTestId("ocr-prompt")).toBeNull());
    expect(ocr().jobs).toEqual({});
  });

  it("cancels, resumes and deletes from Settings", async () => {
    render(<App />);
    await act(() => runCommand("app.settings"));
    const settings = await screen.findByTestId("settings");
    expect(within(settings).getByRole("heading", { name: "OCR models" })).toBeTruthy();
    expect(settings.textContent).toContain("Acceleration: the GPU, through Metal.");
    expect(settings.textContent).toContain("Memory: 36 GB.");
    const small = within(settings).getByTestId("ocr-model-nuqta-0.8b");
    expect(small.textContent).toContain("Download: 1.1 GB");
    expect(small.textContent).toContain("Optional");
    expect(small.textContent).toContain("Apache-2.0");
    expect(small.textContent).toContain("Not installed");

    fireEvent.click(within(small).getByRole("button", { name: "Download" }));
    await waitFor(() => expect(ocr().jobs["nuqta-0.8b"]?.received ?? 0).toBeGreaterThan(0));
    fireEvent.click(within(small).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(within(small).getByText(/^Paused: /)).toBeTruthy());
    const paused = ocr().models?.packages[1]?.state;
    expect(paused?.kind).toBe("partial");

    fireEvent.click(within(small).getByRole("button", { name: "Resume" }));
    await waitFor(() => expect(within(small).getByText("Installed")).toBeTruthy());

    // Deleting asks first; Keep leaves it.
    fireEvent.click(within(small).getByRole("button", { name: "Delete" }));
    expect(small.textContent).toContain("Delete ⁨Nuqta-Arabic-OCR 0.8B⁩?");
    fireEvent.click(within(small).getByRole("button", { name: "Keep" }));
    expect(within(small).getByText("Installed")).toBeTruthy();
    fireEvent.click(within(small).getByRole("button", { name: "Delete" }));
    fireEvent.click(within(small).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(within(small).getByText("Not installed")).toBeTruthy());

    await act(() => runCommand("app.closeSettings"));
    expect(screen.queryByTestId("settings")).toBeNull();
  });

  it("shows why a download failed, with the space a full disk needs", async () => {
    render(<App />);
    await act(() => runCommand("app.settings"));
    const row = (await screen.findByTestId("settings")).querySelector(
      "[data-testid='ocr-model-nuqta-4b']",
    ) as HTMLElement;
    failNextMockDownload("model_disk_full", { needed: 6_000_000_000, available: 2_000_000_000 });
    fireEvent.click(within(row).getByRole("button", { name: "Download" }));
    const alert = await within(row).findByRole("alert");
    expect(alert.textContent).toBe("Not enough disk space: it needs 6 GB free, and 2 GB is free.");

    failNextMockDownload("model_download_failed");
    fireEvent.click(within(row).getByRole("button", { name: "Download" }));
    await waitFor(() =>
      expect(within(row).getByRole("alert").textContent).toContain("Check the internet connection"),
    );
  });

  it("refuses a second download of the same model and reports it", async () => {
    render(<App />);
    await act(() => runCommand("app.settings"));
    await act(() => runCommand("ocrModels.download", { modelId: "nuqta-4b" }));
    await act(() => runCommand("ocrModels.download", { modelId: "nuqta-4b" }));
    expect(app().errorCode).toBe("model_busy");
    // Still shown as running: the listing says so.
    expect(ocr().jobs["nuqta-4b"]).toBeTruthy();
    await act(() => runCommand("ocrModels.cancel", { modelId: "nuqta-4b" }));
    await waitFor(() => expect(ocr().jobs).toEqual({}));
  });

  it("lays the panel out right to left in Arabic", async () => {
    render(<App />);
    act(() => app().setLocale("ar"));
    await act(() => runCommand("app.settings"));
    const settings = await screen.findByTestId("settings");
    expect(document.documentElement.dir).toBe("rtl");
    expect(within(settings).getByRole("heading", { name: "الإعدادات" })).toBeTruthy();
    expect(within(settings).getByRole("heading", { name: "نماذج التعرّف الضوئي" })).toBeTruthy();
    const row = within(settings).getByTestId("ocr-model-nuqta-4b");
    // The Latin model name and licence are isolated from the Arabic text around them.
    expect(row.querySelector("bdi")?.textContent).toBe("Nuqta-Arabic-OCR 4B");
    expect(row.textContent).toContain("مطلوب للتعرّف الضوئي");
    expect(row.textContent).toContain("غير مثبّت");
    expect(row.textContent).not.toMatch(/[٠-٩]/);
    expect(within(row).getByRole("button", { name: "تنزيل" })).toBeTruthy();
    expect(within(settings).getByRole("button", { name: "تم" })).toBeTruthy();
  });

  it("shows the OCR tool's state in the inspector", async () => {
    render(<App />);
    act(() => app().setActiveTool("ocr"));
    const panel = await screen.findByTestId("ocr-panel");
    await waitFor(() =>
      expect(panel.textContent).toContain(
        "The OCR model is not installed yet. It is a 5.5 GB download.",
      ),
    );
    const recognize = within(panel).getByRole("button", { name: "Recognize text" });
    expect(recognize.hasAttribute("disabled")).toBe(true);
    fireEvent.click(within(panel).getByRole("button", { name: "Manage OCR models…" }));
    expect(await screen.findByTestId("settings")).toBeTruthy();
  });

  it("opens Settings with Cmd/Ctrl+comma", () => {
    expect(getCommand("app.settings").shortcut).toEqual({ key: ",", mod: true });
    expect(shortcutLabel("app.settings", true)).toBe("⌘,");
    expect(shortcutLabel("app.settings", false)).toBe("Ctrl+,");
  });
});

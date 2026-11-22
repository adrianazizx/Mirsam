import { act, fireEvent, render, screen } from "@testing-library/react";
import { I18nextProvider } from "react-i18next";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import { runCommand } from "../../commands/registry";
import { applyLocale, i18n } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { NoteEditor } from "./NoteEditor";

beforeEach(async () => {
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    pendingNote: null,
    notice: null,
  });
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3 });
});

/** The editor for the mock page's paragraph, opened as the Edit tool opens it. */
async function open(rtl: boolean, styled = false) {
  const [block] = await backend.pageTextBlocks(1, 0);
  if (!block) throw new Error("the mock has a paragraph");
  await act(() => runCommand("text.editBlock", { page: 0, block: { ...block, rtl, styled } }));
  render(
    <I18nextProvider i18n={i18n}>
      <NoteEditor documentId={1} />
    </I18nextProvider>,
  );
  return block;
}

describe.each([
  ["en", false],
  ["ar", true],
] as const)("the paragraph editor in %s", (locale, rtl) => {
  it("holds the paragraph's text in its own direction, tells which font sets it, and saves", async () => {
    useAppStore.setState({ locale });
    applyLocale(locale);
    const block = await open(rtl);
    const field = screen.getByRole("textbox") as HTMLTextAreaElement;
    expect(field.value).toBe(block.text);
    expect(field.getAttribute("dir")).toBe(rtl ? "rtl" : "ltr");
    // The paragraph's own font, until a family is chosen.
    const note = () => screen.getByTestId("block-font").textContent ?? "";
    expect(note()).toContain(block.font);
    expect(/substituted|استبدال/i.test(note())).toBe(false);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "naskh" } });
    expect(/substituted|استبدال/i.test(note())).toBe(true);

    const edit = vi.spyOn(backend, "editTextBlock");
    fireEvent.change(field, { target: { value: `نص جديد new text ${locale}` } });
    fireEvent.keyDown(field, { key: "Enter", metaKey: true });
    await vi.waitFor(() => expect(edit).toHaveBeenCalled());
    expect(edit).toHaveBeenCalledWith(
      1,
      expect.objectContaining({ text: `نص جديد new text ${locale}`, family: "naskh" }),
    );
    expect(selectActiveDocument(useAppStore.getState())?.modified).toBe(true);
    edit.mockRestore();
  });

  it("says a styled paragraph keeps its styles, and only then", async () => {
    useAppStore.setState({ locale });
    applyLocale(locale);
    await open(rtl, true);
    const note = () => screen.getByTestId("block-font").textContent ?? "";
    expect(note()).toContain(i18n.t("block.styled"));
    expect(/italic|المائلة/.test(note())).toBe(true);
  });
});

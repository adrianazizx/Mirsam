import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { App } from "../App";
import type { SecurityInfo } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { ALL_PERMISSIONS, changeOf, initialForm } from "../state/security";
import { isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());

const userOnly: SecurityInfo = {
  encrypted: true,
  cipher: "aes256",
  revision: 6,
  owner: false,
  permissions: { ...ALL_PERMISSIONS, copy: false, print: false, printHighQuality: false },
};

beforeEach(() => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    securityPrompt: false,
  });
});

describe("the protection form (ADR 0043)", () => {
  it("asks for the same password twice, and for something to protect", () => {
    const form = initialForm();
    expect(changeOf({ ...form, userPassword: "a", confirmPassword: "b" })).toEqual({
      problem: "mismatch",
    });
    expect(changeOf(form)).toEqual({ problem: "nothing" });
    const result = changeOf({
      ...form,
      userPassword: "كلمة السر",
      confirmPassword: "كلمة السر",
      permissions: { ...ALL_PERMISSIONS, print: false },
    });
    expect(result).toMatchObject({
      change: {
        kind: "set",
        cipher: "aes256",
        userPassword: "كلمة السر",
        // High-quality printing means nothing without printing.
        permissions: { print: false, printHighQuality: false, copy: true },
      },
    });
  });

  it("keeps non-ASCII passwords out of AES-128", () => {
    const form = {
      ...initialForm(),
      cipher: "aes128" as const,
      userPassword: "سر",
      confirmPassword: "سر",
    };
    expect(changeOf(form)).toEqual({ problem: "ascii" });
    expect(changeOf({ ...form, userPassword: "pw", confirmPassword: "pw" })).toHaveProperty(
      "change",
    );
  });

  it("removing needs no password", () => {
    expect(changeOf({ ...initialForm(), mode: "remove" })).toEqual({ change: { kind: "remove" } });
  });
});

describe("permissions switch commands off, and the owner lifts them", () => {
  it("disables printing, copying and the rest the file does not allow", () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3, security: userOnly });
    expect(isCommandEnabled("file.print")).toBe(false);
    expect(isCommandEnabled("edit.copy")).toBe(false);
    // Editing, annotating, and the protection command itself stay available
    // (the backend decides on the last).
    expect(isCommandEnabled("comment.note")).toBe(true);
    expect(isCommandEnabled("pages.delete")).toBe(true);
    expect(isCommandEnabled("security.open")).toBe(true);
  });

  it("does not restrict the owner", () => {
    useAppStore
      .getState()
      .addDocument({ id: 1, name: "a.pdf", pageCount: 3, security: { ...userOnly, owner: true } });
    expect(isCommandEnabled("file.print")).toBe(true);
  });
});

describe("password and permissions dialog", () => {
  it("saves with the chosen protection and shows the new state", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3 });
    render(<App />);
    await act(async () => {
      await runCommand("security.open");
    });
    const dialog = await screen.findByTestId("security-dialog");
    expect(dialog).toBeTruthy();
    const save = screen.getByRole("button", { name: "Save with this protection" });
    expect((save as HTMLButtonElement).disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("Password to open the file (optional)"), {
      target: { value: "كلمة" },
    });
    fireEvent.change(screen.getByLabelText("Type the password again"), {
      target: { value: "كلمة" },
    });
    await waitFor(() => expect((save as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(save);
    await waitFor(() => expect(useAppStore.getState().securityPrompt).toBe(false));
    expect(active()?.security).toMatchObject({ encrypted: true, cipher: "aes256", owner: true });
    expect(active()?.canUndo).toBe(false);
  });

  it("explains that the owner password is needed when opened as a user", async () => {
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3, security: userOnly });
    render(<App />);
    await act(async () => {
      await runCommand("security.open");
    });
    expect(await screen.findByText(/owner password is needed/)).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Save with this protection" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });
});

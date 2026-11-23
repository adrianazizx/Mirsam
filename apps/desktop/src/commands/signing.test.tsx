import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { backend } from "../backend";
import { MOCK_P12_PASSWORD, resetMockSigning } from "../backend/mockSigning";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { EMPTY_FORM, useCertSign } from "../state/certSign";
import { ALL_PERMISSIONS } from "../state/security";
import { isCommandEnabled, runCommand, TSA_SETTING } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());
const sign = () => useCertSign.getState();

afterEach(() => {
  vi.restoreAllMocks();
});

beforeEach(async () => {
  localStorage.clear();
  resetMockSigning();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    activeTool: "select",
    fillTool: null,
    fillSignature: null,
    signatures: null,
    leftPanel: null,
  });
  useCertSign.setState({
    dialogOpen: false,
    identities: null,
    form: EMPTY_FORM,
    picked: null,
    unlockError: null,
    busy: false,
    error: null,
    placing: null,
    reports: {},
  });
  for (const kept of await backend.signatures()) await backend.deleteSignature(kept.id);
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

describe("signing with a certificate (ADR 0048)", () => {
  it("adds a certificate from a file, asking again after a wrong password", async () => {
    render(<App />);
    await act(async () => runCommand("certSign.open"));
    const dialog = await screen.findByTestId("cert-sign-dialog");
    // The keychain's identity, its Arabic name isolated.
    await waitFor(() => expect(dialog.querySelector("bdi")?.textContent).toBe("سارة الحارثي"));
    expect(sign().form.identity).toBe("keychain:AB");

    fireEvent.click(within(dialog).getByRole("button", { name: "Add from a file (.p12, .pfx)…" }));
    const password = await within(dialog).findByLabelText(/Password for/);
    expect(dialog.textContent).toContain("توقيعي.p12");
    fireEvent.change(password, { target: { value: "wrong" } });
    fireEvent.submit(password.closest("form") as HTMLFormElement);
    expect(await within(dialog).findByRole("alert")).toHaveProperty(
      "textContent",
      "That password does not open this certificate file. Try again.",
    );
    // The password field is emptied after each try.
    expect((password as HTMLInputElement).value).toBe("");

    fireEvent.change(password, { target: { value: MOCK_P12_PASSWORD } });
    fireEvent.submit(password.closest("form") as HTMLFormElement);
    await waitFor(() => expect(sign().picked).toBeNull());
    expect(sign().form.identity).toBe("file:1");
    expect(sign().identities?.map((identity) => identity.id)).toEqual(["file:1", "keychain:AB"]);
    expect(dialog.textContent).toContain("Self-signed");

    // Forgotten, it is no longer offered.
    await act(async () => runCommand("certSign.forget", { identity: "file:1" }));
    expect(sign().identities?.map((identity) => identity.id)).toEqual(["keychain:AB"]);
    expect(sign().form.identity).toBe("keychain:AB");
  });

  it.each([
    ["en", "Sign with a certificate", "Sign and save", "Valid: unchanged, signer verified"],
    ["ar", "التوقيع بشهادة رقمية", "وقّع واحفظ", "صالح: لم يتغيّر، وهوية الموقّع موثّقة"],
  ] as const)(
    "signs invisibly and shows the signature checked, in %s",
    async (locale, title, button, verdict) => {
      render(<App />);
      act(() => useAppStore.getState().setLocale(locale));
      await act(async () => runCommand("certSign.open"));
      const dialog = await screen.findByTestId("cert-sign-dialog");
      expect(within(dialog).getByRole("heading", { name: title })).toBeTruthy();
      await waitFor(() => expect(sign().form.identity).toBe("keychain:AB"));
      const reason = within(dialog).getAllByRole("textbox")[0] as HTMLInputElement;
      expect(reason.getAttribute("dir")).toBe("auto");
      fireEvent.change(reason, { target: { value: "موافق" } });

      const signDocument = vi.spyOn(backend, "signDocument");
      fireEvent.click(within(dialog).getByRole("button", { name: button }));
      await waitFor(() => expect(screen.queryByTestId("cert-sign-dialog")).toBeNull());
      expect(signDocument).toHaveBeenCalledWith(
        1,
        expect.objectContaining({
          identity: "keychain:AB",
          reason: "موافق",
          visible: null,
          timestamp: false,
        }),
        false,
      );
      // The signed file is what is shown: saved, signed, and undo starts over.
      expect(active()).toMatchObject({
        name: "signed.pdf",
        modified: false,
        signed: true,
        canUndo: false,
      });
      expect(useAppStore.getState().leftPanel).toBe("signatures");
      const panel = await screen.findByTestId("signatures-panel");
      await waitFor(() => expect(panel.textContent).toContain(verdict));
      expect(panel.querySelector("bdi")?.textContent).toBe("سارة الحارثي");
      expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
    },
  );

  it("places a visible signature where the page is pressed, with a caption in the UI language", async () => {
    render(<App />);
    act(() => useAppStore.getState().setLocale("ar"));
    await act(async () => runCommand("certSign.open"));
    await waitFor(() => expect(sign().form.identity).toBe("keychain:AB"));
    act(() => sign().updateForm({ visible: true }));
    await act(async () => runCommand("certSign.apply"));
    // The dialog gives way to the page: the Sign tool places it.
    expect(sign().dialogOpen).toBe(false);
    expect(sign().placing).toMatchObject({ documentId: 1, saveAs: false });
    expect(useAppStore.getState()).toMatchObject({ activeTool: "sign", fillTool: "certificate" });
    expect((await screen.findByTestId("fill-hint")).textContent).toBe(
      "انقر في الصفحة حيث تريد التوقيع الرقمي. يُوقَّع المستند ويُحفظ فورًا.",
    );

    const signDocument = vi.spyOn(backend, "signDocument");
    await act(async () => runCommand("fill.place", { page: 2, x: 300, y: 400 }));
    const request = signDocument.mock.calls[0]?.[1];
    expect(request?.visible).toMatchObject({ page: 2, x: 200, y: 364, width: 200, height: 72 });
    expect(request?.visible?.caption.split("\n").slice(0, 2)).toEqual([
      "موقّع رقميًا من",
      "سارة الحارثي",
    ]);
    expect(useAppStore.getState().fillTool).toBeNull();
    expect(sign().placing).toBeNull();
    expect(sign().reports[1]?.list[0]?.view).toMatchObject({ x: 200, y: 364 });
  });

  it("time-stamps from the authority given, which is remembered", async () => {
    render(<App />);
    await act(async () => runCommand("certSign.open"));
    const dialog = await screen.findByTestId("cert-sign-dialog");
    await waitFor(() => expect(sign().form.identity).toBe("keychain:AB"));
    fireEvent.click(
      within(dialog).getByRole("checkbox", { name: "Add a trusted time-stamp (PAdES B-T)" }),
    );
    const button = within(dialog).getByRole("button", {
      name: "Sign and save",
    }) as HTMLButtonElement;
    expect(button.disabled).toBe(true);
    expect(dialog.textContent).toContain("Enter the time-stamping server's address");
    const url = within(dialog).getByLabelText("Time-stamping server") as HTMLInputElement;
    expect(url.getAttribute("dir")).toBe("ltr");
    fireEvent.change(url, { target: { value: "http://timestamp.example" } });
    await waitFor(() => expect(button.disabled).toBe(false));
    fireEvent.click(button);
    await waitFor(() => expect(active()?.signed).toBe(true));
    expect((await backend.settings())[TSA_SETTING]).toBe("http://timestamp.example");
    expect(sign().reports[1]?.list[0]?.timestamp).toMatchObject({ intact: true, trusted: true });

    // Opened again, the authority is filled in and the time-stamp on.
    await act(async () => runCommand("certSign.open"));
    expect(sign().form).toMatchObject({ tsaUrl: "http://timestamp.example", timestamp: true });
  });

  it("keeps a failure in the open dialog, and reports one after placing in the banner", async () => {
    render(<App />);
    await act(async () => runCommand("certSign.open"));
    const dialog = await screen.findByTestId("cert-sign-dialog");
    await waitFor(() => expect(sign().form.identity).toBe("keychain:AB"));
    vi.spyOn(backend, "signDocument").mockRejectedValue({
      code: "timestamp_failed",
      detail: "down",
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Sign and save" }));
    expect((await within(dialog).findByRole("alert")).textContent).toContain(
      "The time-stamping server could not be reached",
    );
    expect(active()?.signed).toBeUndefined();

    act(() => sign().updateForm({ visible: true }));
    await act(async () => runCommand("certSign.apply"));
    await act(async () => runCommand("fill.place", { page: 0, x: 100, y: 100 }));
    expect(useAppStore.getState().errorCode).toBe("timestamp_failed");
    expect(sign().placing).toBeNull();
    expect(useAppStore.getState().fillTool).toBeNull();
  });

  it("is not offered when the file's permissions do not allow filling in fields", () => {
    useAppStore.getState().removeDocument(1);
    useAppStore.getState().addDocument({
      id: 1,
      name: "a.pdf",
      pageCount: 3,
      security: {
        encrypted: true,
        cipher: "aes256",
        revision: 6,
        permissions: { ...ALL_PERMISSIONS, fillForms: false },
        owner: false,
      },
    });
    expect(isCommandEnabled("certSign.open")).toBe(false);
    expect(isCommandEnabled("certSign.apply")).toBe(false);
    expect(isCommandEnabled("signatures.check")).toBe(true);
  });
});

describe("the signatures panel", () => {
  it("lists what changed after a signature, and says revocation is not checked", async () => {
    vi.spyOn(backend, "documentSignatures").mockResolvedValue([
      {
        field: "Signature1",
        page: 0,
        rect: null,
        view: null,
        documentTimestamp: false,
        subFilter: "ETSI.CAdES.detached",
        status: "unknown",
        integrity: "intact",
        trust: "untrusted",
        signer: {
          name: "Test Signer",
          organization: null,
          email: null,
          issuer: "Test Signer",
          notBefore: 0,
          notAfter: 2_000_000_000,
          fingerprint: "AA",
          selfSigned: true,
          key: "p256",
        },
        name: "Test Signer",
        reason: null,
        location: "مسقط",
        contact: null,
        claimedTime: 1_795_400_000,
        timestamp: null,
        revision: 2,
        revisions: 4,
        coversWholeFile: false,
        changes: {
          revisions: 2,
          pages: [0, 2],
          pageOrder: false,
          annotations: 3,
          formFields: 1,
          signatures: 0,
          other: 0,
        },
        problems: ["signer_untrusted"],
      },
    ]);
    render(<App />);
    await act(async () => runCommand("view.showSignatures"));
    const panel = await screen.findByTestId("signatures-panel");
    await waitFor(() =>
      expect(panel.textContent).toContain("Unchanged, but the signer is not verified"),
    );
    for (const text of [
      "Itself (self-signed)",
      "by the signer's own clock",
      "2 of 4",
      "Changed after signing. Later versions: 2",
      "Comments, markup or Fill & Sign items: 3",
      "Form fields filled in: 1",
      "Content changed on pages 1, 3",
      "not from an authority this computer trusts",
      "Revocation (a certificate withdrawn before it expired) is not checked yet.",
    ]) {
      expect(panel.textContent).toContain(text);
    }
    expect([...panel.querySelectorAll("bdi")].map((bdi) => bdi.textContent)).toContain("مسقط");
  });

  it("says when a document has no signatures", async () => {
    render(<App />);
    await act(async () => runCommand("view.showSignatures"));
    const panel = await screen.findByTestId("signatures-panel");
    await waitFor(() =>
      expect(panel.textContent).toContain("This document has no digital signatures."),
    );
  });
});

describe("signatures checked on open", () => {
  it.each([
    ["en", "Signed: signer not verified"],
    ["ar", "موقّع: هوية الموقّع غير موثّقة"],
  ] as const)(
    "show a signed document's verdict in the status bar at once, in %s",
    async (locale, label) => {
      const check = vi.spyOn(backend, "documentSignatures");
      useAppStore.getState().removeDocument(1);
      await backend.signDocument(
        7,
        {
          identity: "keychain:AB",
          reason: null,
          location: null,
          contact: null,
          timestamp: false,
          visible: null,
        },
        false,
      );
      // A signed file the mock cannot vouch for: the keychain one is trusted, so make it unknown.
      check.mockResolvedValueOnce([
        {
          ...(await backend.documentSignatures(7))[0],
          status: "unknown",
          trust: "untrusted",
        } as never,
      ]);
      render(<App />);
      act(() => {
        useAppStore.getState().setLocale(locale);
        useAppStore
          .getState()
          .addDocument({ id: 7, name: "signed.pdf", pageCount: 3, signed: true });
      });
      const badge = await screen.findByTestId("signature-badge");
      await waitFor(() => expect(badge.textContent).toBe(label));
      expect(check).toHaveBeenCalledWith(7);
      fireEvent.click(badge);
      await waitFor(() => expect(useAppStore.getState().leftPanel).toBe("signatures"));
    },
  );

  it("show nothing for an unsigned document", async () => {
    render(<App />);
    expect(screen.queryByTestId("signature-badge")).toBeNull();
  });
});

import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "../App";
import { backend, type PageObject } from "../backend";
import { selectActiveDocument, useAppStore } from "../state/appStore";
import { mapBetween, normaliseUrl, quarterTurn } from "../state/objects";
import { ALL_PERMISSIONS } from "../state/security";
import { commandForEvent, isCommandEnabled, runCommand } from "./registry";

const active = () => selectActiveDocument(useAppStore.getState());
const objects = () => backend.pageObjects(1, 0);
const links = () => backend.pageLinks(1, 0);

/** Selects page 0's object `index` with the Edit tool's object mode, as the viewer would. */
async function select(index: number): Promise<PageObject> {
  await runCommand("edit.objectsMode");
  const object = (await objects())[index];
  if (!object) throw new Error(`no object ${index}`);
  await runCommand("object.select", { page: 0, object });
  return object;
}

afterEach(() => {
  vi.restoreAllMocks();
});

beforeEach(async () => {
  localStorage.clear();
  useAppStore.setState({
    locale: "en",
    documents: [],
    activeDocumentId: null,
    errorCode: null,
    activeTool: "select",
    editMode: "text",
    selectedObject: null,
    objectCrop: null,
    linkDraft: null,
    linkTip: null,
    destinations: null,
    pendingNote: null,
  });
  // The mock keeps its edits per document id: start each test from the file as opened.
  while ((await backend.undoEdit(1).catch(() => null))?.canUndo) {
    /* undo everything */
  }
  useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 240 });
});

describe("the displayed-page maps (ADR 0051)", () => {
  it("move and resize one box onto another, and turn a box about its centre", () => {
    const from = { x: 10, y: 20, width: 100, height: 50 };
    const [a, b, c, d, e, f] = mapBetween(from, { x: 30, y: 40, width: 200, height: 25 });
    expect([a * from.x + c * from.y + e, b * from.x + d * from.y + f]).toEqual([30, 40]);
    expect([a, d]).toEqual([2, 0.5]);
    // A quarter turn clockwise (y down) sends the box's right edge to its bottom, about its centre.
    const [p, q, r, s, t, u] = quarterTurn(from, true);
    const at = (x: number, y: number) => [p * x + r * y + t, q * x + s * y + u];
    expect(at(60, 45)).toEqual([60, 45]);
    expect(at(110, 45)).toEqual([60, 95]);
    const [ccw] = [quarterTurn(from, false)];
    expect(ccw.slice(0, 4)).toEqual([0, -1, 1, 0]);
  });

  it("writes bare domains and e-mail addresses as links", () => {
    expect(normaliseUrl("example.com")).toBe("https://example.com");
    expect(normaliseUrl(" a@b.org ")).toBe("mailto:a@b.org");
    expect(normaliseUrl("http://x.y")).toBe("http://x.y");
  });
});

describe("page objects (ADR 0051)", () => {
  it("moves, turns, reorders and deletes the selected object, each one undo step", async () => {
    const image = await select(1);
    expect(useAppStore.getState().activeTool).toBe("edit");
    const edit = vi.spyOn(backend, "editObject");
    await runCommand("object.reshape", {
      page: 0,
      object: image,
      rect: { ...image.rect, x: image.rect.x + 20 },
    });
    expect(edit).toHaveBeenLastCalledWith(
      1,
      0,
      { index: 1, fingerprint: image.fingerprint },
      { kind: "transform", matrix: [1, 0, 0, 1, 20, 0] },
    );
    expect((await objects())[1]?.rect.x).toBe(image.rect.x + 20);
    expect(active()?.canUndo).toBe(true);

    // The edit's outcome selects it again (the viewer does, after its refresh).
    await select(1);
    await runCommand("object.rotateClockwise");
    const turned = (await objects())[1];
    expect([turned?.rect.width, turned?.rect.height]).toEqual([
      image.rect.height,
      image.rect.width,
    ]);

    await select(1);
    await runCommand("object.delete");
    expect((await objects()).map((o) => o.kind)).toEqual(["path", "text"]);
    expect(useAppStore.getState().selectedObject).toBeNull();
    await runCommand("edit.undo");
    expect((await objects()).map((o) => o.kind)).toEqual(["path", "image", "text"]);
  });

  it("crops an image from its handles and uncrops it", async () => {
    await select(1);
    expect(isCommandEnabled("object.crop")).toBe(true);
    await runCommand("object.crop");
    expect(isCommandEnabled("object.applyCrop")).toBe(true);
    // While cropping, nothing else changes the object.
    expect(isCommandEnabled("object.delete")).toBe(false);
    const crop = { x: 350, y: 100, width: 60, height: 40 };
    await runCommand("object.setCrop", { rect: crop });
    await runCommand("object.applyCrop");
    expect((await objects())[1]).toMatchObject({ cropped: true, rect: crop });
    await select(1);
    await runCommand("object.uncrop");
    expect((await objects())[1]?.cropped).toBe(false);
  });

  it("inserts and replaces images through the shell's dialog, and does nothing when cancelled", async () => {
    vi.spyOn(backend, "insertImage").mockResolvedValueOnce(null);
    await runCommand("object.insertImage");
    expect(await objects()).toHaveLength(3);
    expect(useAppStore.getState().editMode).toBe("objects");
    await runCommand("object.insertImage");
    expect((await objects()).at(-1)?.kind).toBe("image");
    await select(1);
    await runCommand("object.replaceImage");
    expect((await objects())[1]?.image).toEqual({ width: 400, height: 300 });
  });

  it("offers reordering only towards objects it overlaps", async () => {
    await select(0);
    // The viewer has not read the page's objects in this test: nothing overlaps.
    expect(isCommandEnabled("object.bringForward")).toBe(false);
    expect(isCommandEnabled("object.sendToBack")).toBe(false);
  });

  it("gives Delete, Escape and the arrows to the object while one is selected", async () => {
    await select(1);
    const key = (key: string) => new KeyboardEvent("keydown", { key });
    expect(commandForEvent(key("Delete"))?.id).toBe("object.delete");
    expect(commandForEvent(key("Escape"))?.id).toBe("object.deselect");
    expect(commandForEvent(key("ArrowLeft"))?.id).toBe("object.moveLeft");
    await runCommand("object.crop");
    expect(commandForEvent(key("Escape"))?.id).toBe("object.cancelCrop");
    expect(commandForEvent(key("Enter"))?.id).toBe("object.applyCrop");
  });

  it("are not changed when the file's permissions forbid it", () => {
    const security = {
      encrypted: true,
      cipher: "aes256" as const,
      revision: 6,
      owner: false,
      permissions: { ...ALL_PERMISSIONS, modify: false, annotate: false },
    };
    useAppStore.getState().removeDocument(1);
    useAppStore.getState().addDocument({ id: 1, name: "a.pdf", pageCount: 3, security });
    for (const command of ["object.insertImage", "link.fromSelection"] as const) {
      expect(isCommandEnabled(command), command).toBe(false);
    }
  });
});

describe("links (ADR 0051)", () => {
  it("adds a drawn link to an address, points it elsewhere, and deletes it", async () => {
    await runCommand("edit.linksMode");
    const rect = { x: 72, y: 400, width: 120, height: 16 };
    await runCommand("link.draw", { page: 0, rect });
    expect(useAppStore.getState().linkDraft).toMatchObject({ index: null, rects: [rect] });
    await runCommand("link.save", { linkTarget: { kind: "url", url: "https://example.org" } });
    const added = (await links()).at(-1);
    expect(added?.target).toEqual({ kind: "url", url: "https://example.org" });
    expect(useAppStore.getState().linkDraft).toBeNull();

    await runCommand("link.select", { page: 0, link: added ?? null });
    expect(isCommandEnabled("link.delete")).toBe(true);
    await runCommand("link.save", { linkTarget: { kind: "page", page: 4 } });
    expect((await links()).at(-1)?.target).toEqual({ kind: "page", page: 4 });
    await runCommand("link.select", { page: 0, link: (await links()).at(-1) ?? null });
    await runCommand("link.delete");
    expect(await links()).toHaveLength(2);
    expect(active()?.canUndo).toBe(true);
  });

  it("follows a page link at once, and opens a web link only from its shown address", async () => {
    const [web, page] = await links();
    if (!web || !page) throw new Error("the mock has two links");
    const open = vi.spyOn(backend, "openLink");
    await runCommand("link.follow", { page: 0, link: page });
    expect(active()?.page).toBe(1);
    expect(useAppStore.getState().linkTip).toBeNull();

    // Hovering shows the address; pressing pins it with its button; the button opens it.
    await runCommand("link.hover", { page: 0, link: web });
    expect(useAppStore.getState().linkTip?.pinned).toBe(false);
    expect(isCommandEnabled("link.open")).toBe(false);
    await runCommand("link.follow", { page: 0, link: web });
    expect(useAppStore.getState().linkTip?.pinned).toBe(true);
    expect(open).not.toHaveBeenCalled();
    await runCommand("link.open");
    expect(open).toHaveBeenCalledWith("https://example.com/مرسام");

    // An action the app does not follow is shown, never opened.
    await runCommand("link.follow", {
      page: 0,
      link: { index: 9, rects: web.rects, target: { kind: "other", action: "javascript" } },
    });
    expect(isCommandEnabled("link.open")).toBe(false);
  });
});

describe("the Edit tool's object and link panels", () => {
  it.each([
    ["en", "Objects", "Insert image…", "Image", "Bring to front"],
    ["ar", "العناصر", "إدراج صورة…", "صورة", "إحضار إلى المقدمة"],
  ] as const)(
    "show the selected object's tools in %s",
    async (locale, heading, insert, kind, front) => {
      render(<App />);
      act(() => {
        useAppStore.getState().setLocale(locale);
      });
      await act(async () => {
        await select(1);
      });
      expect(await screen.findByRole("heading", { name: heading })).toBeTruthy();
      const panel = within(await screen.findByTestId("objects-panel"));
      expect(panel.getByRole("button", { name: insert })).toBeTruthy();
      expect(panel.getByRole("heading", { name: kind })).toBeTruthy();
      expect(panel.getByRole("button", { name: front })).toBeTruthy();
      // The strip's mode switch, in the UI language.
      const modes = within(screen.getByTestId("edit-modes"));
      expect(
        modes.getAllByRole("radio").length + modes.queryAllByRole("button").length,
      ).toBeGreaterThan(0);
      expect(document.documentElement.dir).toBe(locale === "ar" ? "rtl" : "ltr");
    },
  );

  it.each([
    ["en", "New link", "Address", "Add link", "Enter a web or e-mail address without spaces."],
    ["ar", "رابط جديد", "العنوان", "إضافة الرابط", "اكتب عنوان ويب أو بريد إلكتروني بلا مسافات."],
  ] as const)(
    "asks where a new link goes in %s",
    async (locale, heading, address, add, invalid) => {
      render(<App />);
      act(() => {
        useAppStore.getState().setLocale(locale);
      });
      await act(async () => {
        await runCommand("edit.linksMode");
        await runCommand("link.draw", { page: 0, rect: { x: 72, y: 500, width: 90, height: 14 } });
      });
      const panel = within(await screen.findByTestId("link-panel"));
      expect(panel.getByRole("heading", { name: heading })).toBeTruthy();
      const input = panel.getByLabelText(address) as HTMLInputElement;
      // An address is written left to right in either language.
      expect(input.getAttribute("dir")).toBe("ltr");
      fireEvent.click(panel.getByRole("button", { name: add }));
      expect((await panel.findByRole("alert")).textContent).toBe(invalid);
      fireEvent.change(input, { target: { value: "مرسام.example" } });
      fireEvent.click(panel.getByRole("button", { name: add }));
      await waitFor(async () =>
        expect((await links()).at(-1)?.target).toEqual({
          kind: "url",
          url: "https://مرسام.example",
        }),
      );
    },
  );
});

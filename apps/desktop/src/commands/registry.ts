import {
  type BlockMark,
  backend,
  type CertSignRequest,
  type CompressOptions,
  type EditOutcome,
  FILL_KINDS,
  type FieldSpec,
  type ImageFormat,
  isBackendError,
  type LinkInfo,
  type LinkTarget,
  type MarkupKind,
  type ObjectEdit,
  type OcrAction,
  type OcrLanguage,
  type OpenOutcome,
  type PageEdit,
  type PageObject,
  type ParagraphBlock,
  type Permissions,
  type RedactPattern,
  type RedactPreset,
  type RestoreOutcome,
  type SanitizeOptions,
  type SecurityChange,
  type SplitBy,
  type StampArg,
  type StrokePoint,
  type TextFormat,
  type TextStyle,
  type ViewRect,
} from "../backend";
import { marksOfRuns, sameMarks } from "../components/comments/richText";
import { formatNumber, formatTag, i18n } from "../i18n";
import {
  type AppState,
  type CommentTool,
  type DesignTool,
  type FillTool,
  type LeftPanel,
  type OpenDocument,
  type OutputDialog,
  type ReadingTheme,
  selectActiveDocument,
  useAppStore,
} from "../state/appStore";
import { useCertSign, VISIBLE_SIZE } from "../state/certSign";
import { DOCUMENT_COLORS, type DocumentColorId, rgbOf } from "../state/colors";
import type { CommentFilter, CommentSort } from "../state/comments";
import { isOpenable, mapBetween, quarterTurn } from "../state/objects";
import {
  flaggedWords,
  nextItem,
  positionOf,
  type ReviewItem,
  reviewQueue,
  useOcrJobs,
  wordAt,
  wordRect,
} from "../state/ocrJobs";
import { missingRequired, ocrReady, useOcrModels } from "../state/ocrModels";
import {
  addMark,
  MIN_MARK_SIDE,
  marksForApply,
  redoMark,
  removeMark,
  undoMark,
} from "../state/redaction";
import { stepZoom } from "../state/zoom";
import { NOTE_SIZE, type SelectedAnnotation, TEXT_BOX_WIDTH } from "../viewer/controller";
import type { PageOrder, ViewMode } from "../viewer/layout";
import { getViewer } from "../viewer/registry";
import {
  applyDesign,
  defaultSpec,
  designOf,
  detectedEdits,
  drawnRect,
  firstWidget,
  loadFormDesign,
  moved,
  newFieldName,
  nextButtonRect,
  pageFields,
  rowOrder,
  selectedField,
} from "./formDesign";
import { isMac } from "./platform";

/**
 * Command registry: every user action is a command (docs/SPEC.md section 9).
 * Buttons, keyboard shortcuts, and the command palette all run the same
 * entries, so an action has one implementation, one title, and one shortcut
 * everywhere.
 */

export interface Shortcut {
  /** `KeyboardEvent.key`, lower-cased for letters. */
  key: string;
  /** Another key that does the same (Backspace for Delete: a Mac's delete key). */
  alternate?: string;
  /** Cmd on macOS, Ctrl elsewhere. */
  mod?: boolean;
  /** The Control key itself on every platform (Ctrl+Tab). */
  ctrl?: boolean;
  shift?: boolean;
  /**
   * Leave the key to a focused text field (Cmd+C, Cmd+A and Escape mean
   * something there already).
   */
  notInFields?: boolean;
}

export interface Command {
  /** Translation key suffix under `commands.`. */
  id: CommandId;
  shortcut?: Shortcut;
  isEnabled?(state: AppState): boolean;
  /** `arg` comes from the control that ran the command (e.g. a typed page number); shortcuts pass none. */
  run(arg?: CommandArg): void | Promise<void>;
  /** Hidden from the command palette (needs an argument only a control can give). */
  paletteHidden?: boolean;
}

/** Arguments a control can pass to a command. */
export interface CommandArg {
  /** `view.goToPage`: zero-based page index. */
  page?: number;
  /** `file.openRecent`, `file.forgetRecent`: the recent file's token. */
  token?: number;
  /** `file.unlock`. Used once, never stored. */
  password?: string;
  /** `search.run`. */
  query?: string;
  /** `search.goTo`: index into the hits. `tabs.move`: the new position in tab order. */
  index?: number;
  /** `file.close`, `tabs.move`: a tab other than the active one. */
  documentId?: number;
  /** `file.close`: close even with unsaved changes (the user chose not to save). */
  force?: boolean;
  /** `pages.move`: the pages to move (zero-based, ascending). */
  pages?: number[];
  /** `pages.move`: the index the first moved page takes in the result. */
  to?: number;
  /** `pages.splitInto`: how to split. */
  split?: SplitBy;
  /** `note.save`: the note's text. */
  text?: string;
  /** `note.place`, `textBox.place`: the top-left corner, in displayed-page points. */
  x?: number;
  y?: number;
  /** `textBox.place`: the box's width, in points. */
  width?: number;
  /** `ink.add`: the pen's strokes, in displayed-page points. */
  strokes?: StrokePoint[][];
  /** `ink.erase`: the ink annotations touched, by index. */
  indices?: number[];
  /** `recovery.restore`, `recovery.discard`: the recovery file's token (`token` above). */
  /** `comment.show`: the annotation to scroll to and select (`page`, `index` as for replies). */
  annotation?: SelectedAnnotation;
  /** `comment.resolve`: resolved (true) or open again (false). */
  resolved?: boolean;
  /** `comments.filter`: the filters to change. */
  filter?: Partial<CommentFilter>;
  /** `comments.sort`. */
  sort?: CommentSort;
  /** `redact.addArea`: the area, on the displayed page (`page` above). */
  rect?: ViewRect;
  /** `redact.findPreset`. */
  preset?: RedactPreset;
  /** `redact.removeMark`. */
  markId?: number;
  /** `security.apply`: what the save does to the protection, and where it saves. */
  securityChange?: SecurityChange;
  saveAs?: boolean;
  /** `sanitize.setOption`. */
  option?: keyof SanitizeOptions;
  value?: boolean;
  /** `text.setStyle`: what to change in how new page text is set. */
  textStyle?: Partial<TextStyle>;
  /** `text.editBlock`: the paragraph to edit in place (ADR 0023). */
  block?: ParagraphBlock;
  /** `note.save` of a paragraph: the bundled family to set it in; absent, its own font where it can. */
  blockFamily?: "sans" | "naskh";
  /** `output.stamp`: the header, footer, page numbers, or watermark (`pages` above). */
  stamp?: StampArg;
  /** `output.exportImages`: resolution and format (`pages` above). */
  dpi?: number;
  imageFormat?: ImageFormat;
  /** `output.exportText` (`pages` above). */
  textFormat?: TextFormat;
  /** `output.compress`. */
  compress?: CompressOptions;
  /** `note.save` of a paragraph: bold and italic as its field set them (ADR 0025). */
  blockMarks?: BlockMark[];
  /** `form.setValue`: the field (its id) and its new value (as `FormField.values`). */
  field?: number;
  values?: string[];
  /** `formDesign.update`: the selected field's new properties (ADR 0047). */
  spec?: FieldSpec;
  /** `certSign.forget`: a signing identity's id. */
  identity?: string;
  /** `fill.signature`, `signature.delete`: a kept signature's id. */
  signature?: number;
  /** `signature.saveTyped`: the family it is set in. */
  family?: "sans" | "naskh";
  /** `fill.color`: one of the document colours (`state/colors.ts`). */
  colorId?: string;
  /** `ocrModels.download`, `.cancel`, `.delete`: the OCR package (ADR 0034). */
  modelId?: string;
  /** `ocr.pauseJob`, `.resumeJob`, `.cancelJob`, `.removeJob`: the job (ADR 0035). */
  jobId?: number;
  /** `ocr.showWord`: a flagged word of the review list, `[page index, word index]`. */
  reviewAt?: [number, number];
  /** `ocr.reviewConfirm`, `ocr.reviewChoose`: the word's text as typed or chosen (ADR 0037). */
  reviewText?: string;
  /** `ocr.setRegionOptions`: the language an area is read in, and "numbers only" (ADR 0038). */
  language?: OcrLanguage;
  numbers?: boolean;
  /** `ocr.setConvertOptions` (ADR 0039): keep the scan behind the text, and the family. */
  keepScan?: boolean;
  convertFamily?: "naskh" | "sans";
  /** `object.select`, `object.reshape`: a page object (ADR 0051) (`page`, and `rect` its new box). */
  object?: PageObject | null;
  /** `link.select`, `link.reshape`, `link.follow`, `link.hover`: a link (`page`, `rect`). */
  link?: LinkInfo | null;
  /** `link.save`: where the link goes. */
  linkTarget?: LinkTarget;
}

/** A new mark's side, in points (the engine's `Mark::DEFAULT_SIZE`). */
export const FILL_MARK_SIZE = 14;
/** A new signature's width, and new initials', in points. */
export const SIGNATURE_WIDTH = 150;
export const INITIALS_WIDTH = 60;
/** How much one step of "larger" or "smaller" scales a Fill & Sign item. */
const FILL_STEP = 1.25;
/** How far an arrow key moves a Fill & Sign item, and Shift with it, in points. */
const NUDGE = 1;
const NUDGE_MORE = 10;
/** The pen width drawn signatures are kept with, in pad pixels. */
export const SIGNATURE_PEN = 3;

/** DOM id of the status bar's page number field, which `view.goToPage` focuses. */
export const PAGE_NUMBER_INPUT_ID = "page-number-input";
/** DOM id of the search panel's query field, which `search.focus` focuses. */
export const SEARCH_INPUT_ID = "search-input";

export type CommandId =
  | "file.open"
  | "file.close"
  | "file.print"
  | "file.saveAndClose"
  | "window.closeDiscarding"
  | "app.quitDiscarding"
  | "app.cancelClose"
  | "file.save"
  | "file.saveAs"
  | "file.openRecent"
  | "file.forgetRecent"
  | "file.clearRecents"
  | "file.unlock"
  | "file.cancelUnlock"
  | "tabs.next"
  | "tabs.previous"
  | "tabs.moveEarlier"
  | "tabs.moveLater"
  | "tabs.move"
  | "tabs.moveToNewWindow"
  | "window.new"
  | "edit.copy"
  | "markup.highlight"
  | "markup.underline"
  | "markup.strikeOut"
  | "markup.squiggly"
  | "annotation.delete"
  | "redact.openTool"
  | "redact.addArea"
  | "redact.markSelection"
  | "redact.find"
  | "redact.findPreset"
  | "redact.toggleRegex"
  | "redact.markHit"
  | "redact.markAllHits"
  | "redact.removeMark"
  | "redact.clearMarks"
  | "redact.undoMark"
  | "redact.redoMark"
  | "redact.apply"
  | "redact.confirm"
  | "redact.cancel"
  | "sanitize.setOption"
  | "sanitize.run"
  | "security.open"
  | "security.apply"
  | "security.cancel"
  | "certSign.open"
  | "certSign.cancel"
  | "certSign.importFile"
  | "certSign.unlockFile"
  | "certSign.cancelFile"
  | "certSign.forget"
  | "certSign.apply"
  | "certSign.place"
  | "signatures.check"
  | "signatures.reveal"
  | "view.showSignatures"
  | "recovery.restore"
  | "recovery.discard"
  | "recovery.later"
  | "forms.openTool"
  | "form.setValue"
  | "forms.fillMode"
  | "forms.designMode"
  | "formDesign.select"
  | "formDesign.text"
  | "formDesign.checkbox"
  | "formDesign.radio"
  | "formDesign.combo"
  | "formDesign.list"
  | "formDesign.button"
  | "formDesign.draw"
  | "formDesign.selectField"
  | "formDesign.move"
  | "formDesign.update"
  | "formDesign.addButton"
  | "formDesign.removeButton"
  | "formDesign.delete"
  | "formDesign.tabEarlier"
  | "formDesign.tabLater"
  | "formDesign.tabRowsLeftToRight"
  | "formDesign.tabRowsRightToLeft"
  | "formDesign.detect"
  | "formDesign.toggleDetected"
  | "formDesign.createDetected"
  | "formDesign.cancelDetected"
  | "formDesign.moveLeft"
  | "formDesign.moveRight"
  | "formDesign.moveUp"
  | "formDesign.moveDown"
  | "formDesign.moveLeftMore"
  | "formDesign.moveRightMore"
  | "formDesign.moveUpMore"
  | "formDesign.moveDownMore"
  | "sign.openTool"
  | "fill.select"
  | "fill.text"
  | "fill.date"
  | "fill.check"
  | "fill.cross"
  | "fill.dot"
  | "fill.signature"
  | "fill.place"
  | "fill.color"
  | "fill.move"
  | "fill.toPreviousPage"
  | "fill.toNextPage"
  | "fill.moveLeft"
  | "fill.moveRight"
  | "fill.moveUp"
  | "fill.moveDown"
  | "fill.moveLeftMore"
  | "fill.moveRightMore"
  | "fill.moveUpMore"
  | "fill.moveDownMore"
  | "fill.larger"
  | "fill.smaller"
  | "signatures.load"
  | "signature.new"
  | "signature.newInitials"
  | "signature.cancel"
  | "signature.saveDrawn"
  | "signature.saveTyped"
  | "signature.importImage"
  | "signature.delete"
  | "comment.note"
  | "comment.textBox"
  | "textBox.place"
  | "comment.pen"
  | "comment.eraser"
  | "note.place"
  | "note.save"
  | "note.cancel"
  | "ink.add"
  | "ink.erase"
  | "edit.undo"
  | "edit.redo"
  | "pages.rotateClockwise"
  | "pages.rotateCounterclockwise"
  | "pages.delete"
  | "pages.duplicate"
  | "pages.insertBlank"
  | "pages.moveEarlier"
  | "pages.moveLater"
  | "pages.move"
  | "pages.open"
  | "pages.insertFromFile"
  | "pages.extract"
  | "pages.split"
  | "pages.splitInto"
  | "pages.headerFooter"
  | "pages.pageNumbers"
  | "pages.watermark"
  | "file.exportImages"
  | "file.exportText"
  | "file.compress"
  | "output.stamp"
  | "output.pickImage"
  | "output.exportImages"
  | "output.exportText"
  | "output.compress"
  | "output.saveCompressed"
  | "output.cancel"
  | "edit.selectAll"
  | "edit.deselect"
  | "view.zoomIn"
  | "view.zoomOut"
  | "view.actualSize"
  | "view.fitWidth"
  | "view.nextPage"
  | "view.previousPage"
  | "view.goToPage"
  | "view.modeContinuous"
  | "view.modeSingle"
  | "view.modeSpread"
  | "view.modeBook"
  | "view.pagesLeftToRight"
  | "view.pagesRightToLeft"
  | "view.toggleLeftPanel"
  | "view.showThumbnails"
  | "view.showOutline"
  | "view.showComments"
  | "comment.show"
  | "comment.reply"
  | "comment.resolve"
  | "comments.filter"
  | "comments.sort"
  | "comments.setAuthor"
  | "text.place"
  | "text.editBlock"
  | "text.bold"
  | "text.italic"
  | "text.setStyle"
  | "view.readingNormal"
  | "view.readingSepia"
  | "view.readingNight"
  | "app.digitsArabicIndic"
  | "app.digitsWestern"
  | "search.focus"
  | "search.run"
  | "search.next"
  | "search.previous"
  | "search.goTo"
  | "search.toggleCase"
  | "search.toggleWholeWord"
  | "search.toggleRegex"
  | "search.toggleArabic"
  | "search.toggleTaMarbuta"
  | "app.toggleTheme"
  | "app.toggleLanguage"
  | "app.toggleGallery"
  | "app.commandPalette"
  | "app.settings"
  | "app.closeSettings"
  | "ocr.recognize"
  | "ocr.recognizePage"
  | "ocr.pauseJob"
  | "ocr.resumeJob"
  | "ocr.cancelJob"
  | "ocr.removeJob"
  | "ocr.review"
  | "ocr.showWord"
  | "ocr.nextFlag"
  | "ocr.previousFlag"
  | "ocr.startReview"
  | "ocr.stopReview"
  | "ocr.reviewConfirm"
  | "ocr.reviewChoose"
  | "ocr.reviewCheck"
  | "ocr.reviewCheckPage"
  | "ocr.reviewNumbers"
  | "ocr.drawRegion"
  | "ocr.setRegion"
  | "ocr.setRegionOptions"
  | "ocr.readRegion"
  | "ocr.cancelRegion"
  | "ocr.checkLayers"
  | "ocr.replaceLayers"
  | "ocr.setConvertOptions"
  | "ocr.convertPage"
  | "ocr.convertDocument"
  | "ocrModels.downloadRequired"
  | "ocrModels.dismissPrompt"
  | "ocrModels.download"
  | "ocrModels.cancel"
  | "ocrModels.delete"
  | "edit.textMode"
  | "edit.objectsMode"
  | "edit.linksMode"
  | "object.select"
  | "object.reshape"
  | "object.insertImage"
  | "object.replaceImage"
  | "object.delete"
  | "object.rotateClockwise"
  | "object.rotateCounterclockwise"
  | "object.bringToFront"
  | "object.bringForward"
  | "object.sendBackward"
  | "object.sendToBack"
  | "object.crop"
  | "object.setCrop"
  | "object.applyCrop"
  | "object.cancelCrop"
  | "object.uncrop"
  | "object.deselect"
  | "object.moveLeft"
  | "object.moveRight"
  | "object.moveUp"
  | "object.moveDown"
  | "object.moveLeftMore"
  | "object.moveRightMore"
  | "object.moveUpMore"
  | "object.moveDownMore"
  | "link.fromSelection"
  | "link.draw"
  | "link.select"
  | "link.save"
  | "link.reshape"
  | "link.delete"
  | "link.cancel"
  | "link.hover"
  | "link.follow"
  | "link.open"
  | "link.dismiss";

const store = () => useAppStore.getState();
const hasDocument = (state: AppState) => selectActiveDocument(state) !== null;
/** An open document that is not encrypted. */
const isUnencrypted = (state: AppState) => {
  const active = selectActiveDocument(state);
  return active !== null && !active.security?.encrypted;
};
const hasSelection = (state: AppState) => selectActiveDocument(state)?.hasSelection ?? false;
/** The Forms tool is making fields for the active document (ADR 0047). */
const designing = (state: AppState) =>
  hasDocument(state) && state.activeTool === "forms" && state.formsMode === "design";
/** A field is selected in the form designer. */
const fieldSelected = (state: AppState) => {
  const active = selectActiveDocument(state);
  return (
    designing(state) &&
    active !== null &&
    state.formDesign?.documentId === active.id &&
    state.formDesign.selected !== null
  );
};

/** Opens the Forms tool to fill fields in or to make them (ADR 0047). */
async function openForms(mode: "fill" | "design"): Promise<void> {
  store().setActiveTool("forms");
  if (store().formsMode !== mode) store().setFormsMode(mode);
  const active = selectActiveDocument(store());
  if (mode === "design" && active) await loadFormDesign(active.id);
}

/** Arms the form designer to draw `tool` (null: select). */
async function chooseDesignTool(tool: DesignTool | null): Promise<void> {
  if (!designing(store())) await openForms("design");
  store().setDesignTool(tool);
}

/** The selected field in the active document, with its document. */
function selection() {
  const active = selectActiveDocument(store());
  const design = active ? designOf(active.id) : null;
  const field = selectedField(design);
  if (!active || !design?.selected || !field) return null;
  return { active, design, field, widget: design.selected.widget };
}

/** Moves the selected field `step` places in its page's tab order. */
async function stepTabOrder(step: number): Promise<void> {
  const chosen = selection();
  const on = chosen ? firstWidget(chosen.field) : null;
  if (!chosen || !on) return;
  const ids = pageFields(chosen.design.fields, on.page).map((field) => field.id);
  const index = ids.indexOf(chosen.field.id);
  if (index === -1) return;
  await applyDesign(chosen.active.id, [
    { op: "tabOrder", page: on.page, fields: moved(ids, index, step) },
  ]);
  store().updateFormDesign({ selected: { field: chosen.field.id, widget: chosen.widget } });
}

/** Orders the current page's fields by rows (ADR 0047). */
async function tabByRows(rtl: boolean): Promise<void> {
  const active = selectActiveDocument(store());
  const design = active ? designOf(active.id) : null;
  if (!active || !design) return;
  const page = active.page;
  const fields = rowOrder(design.fields, page, rtl);
  if (fields.length === 0) return;
  const selected = design.selected;
  await applyDesign(active.id, [{ op: "tabOrder", page, fields }]);
  store().updateFormDesign({ selected });
}

/** Moves the selected field's widget by (`dx`, `dy`) points, on its page. */
async function nudgeField(dx: number, dy: number): Promise<void> {
  const chosen = selection();
  const widget = chosen?.field.widgets[chosen.widget];
  if (!chosen || !widget?.view || widget.page === null) return;
  const size = chosen.active.pageSize;
  const { x, y, width, height } = widget.view;
  const rect = {
    x: Math.max(0, Math.min((size?.width ?? Number.POSITIVE_INFINITY) - width, x + dx)),
    y: Math.max(0, Math.min((size?.height ?? Number.POSITIVE_INFINITY) - height, y + dy)),
    width,
    height,
  };
  await applyDesign(
    chosen.active.id,
    [{ op: "setRect", field: chosen.field.id, widget: chosen.widget, page: widget.page, rect }],
    chosen.widget,
  );
}

/** The form designer's tools, and the command that arms each. */
const DESIGN_TOOLS: [CommandId, DesignTool | null][] = [
  ["formDesign.select", null],
  ["formDesign.text", "text"],
  ["formDesign.checkbox", "checkbox"],
  ["formDesign.radio", "radio"],
  ["formDesign.combo", "combo"],
  ["formDesign.list", "list"],
  ["formDesign.button", "pushButton"],
];
const hasHits = (state: AppState) => (selectActiveDocument(state)?.search.hits.length ?? 0) > 0;
const hasAnnotationSelected = (state: AppState) =>
  Boolean(selectActiveDocument(state)?.selectedAnnotation);
/** A note is being written: a new one, or the selected one. */
/** A paragraph of page text is open in the editor (ADR 0023). */
const isEditingBlock = (state: AppState) =>
  state.pendingNote?.kind === "block" && state.pendingNote.documentId === state.activeDocumentId;

/**
 * Bold or italic on the paragraph field's selection, by the browser's own
 * editing command, so the field's undo covers it (ADR 0025). The field keeps
 * focus: the editor's buttons do not take it.
 */
function formatBlock(kind: "bold" | "italic") {
  const field = document.querySelector<HTMLElement>('[data-testid="block-field"]');
  if (!field) return;
  if (!field.contains(document.activeElement)) field.focus();
  document.execCommand(kind);
  field.dispatchEvent(new Event("input", { bubbles: true }));
}
const isWritingNote = (state: AppState) =>
  (state.pendingNote !== null && state.pendingNote.documentId === state.activeDocumentId) ||
  ["note", "textBox", "fillText"].includes(
    selectActiveDocument(state)?.selectedAnnotation?.kind ?? "",
  );
/** A Fill & Sign item is selected (ADR 0045). */
const hasFillSelected = (state: AppState) => {
  const kind = selectActiveDocument(state)?.selectedAnnotation?.kind;
  return kind !== undefined && FILL_KINDS.includes(kind);
};
/** While marking areas to redact, undo and redo step through the marks first (ADR 0042). */
const stepsMarks = (state: AppState, direction: "undo" | "redo") => {
  const redaction = selectActiveDocument(state)?.redaction;
  if (state.activeTool !== "protect" || !redaction) return false;
  return direction === "undo" ? redaction.marks.length > 0 : redaction.undone.length > 0;
};
const canUndo = (state: AppState) =>
  stepsMarks(state, "undo") || (selectActiveDocument(state)?.canUndo ?? false);
const canRedo = (state: AppState) =>
  stepsMarks(state, "redo") || (selectActiveDocument(state)?.canRedo ?? false);
const hasMarks = (state: AppState) =>
  (selectActiveDocument(state)?.redaction.marks.length ?? 0) > 0;

/**
 * The pages a page command acts on: the organizer's selection when it has
 * one, else the page being read. Ascending.
 */
export function targetPages(state: AppState = store()): number[] {
  const active = selectActiveDocument(state);
  if (!active) return [];
  if (state.activeTool === "pages" && active.selectedPages.length > 0) return active.selectedPages;
  return [active.page];
}

/** Opens an output dialog (ADR 0050). */
function openOutput(dialog: OutputDialog): void {
  store().setOutputDialog(dialog);
}

/**
 * Applies a page edit to the active document (ADR 0014). `select`: the pages
 * to select afterwards, in the edited document.
 */
async function editActive(
  edit: (active: OpenDocument, pages: number[]) => PageEdit,
  select: (pages: number[], edit: PageEdit) => number[] = () => [],
): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active) return;
  const pages = targetPages();
  const change = edit(active, pages);
  store().applyEdit(active.id, await backend.editPages(active.id, change));
  store().setSelectedPages(active.id, select(pages, change));
}

/** Where moved pages land: the first at `to` of the result, the rest after it. */
function landed(edit: PageEdit, count: number): number[] {
  return edit.kind === "move" ? Array.from({ length: count }, (_, i) => edit.to + i) : [];
}

/**
 * `to` for moving `pages` one step earlier or later: the index the first of
 * them takes in the result. Null when they cannot move that way.
 */
export function moveTarget(pages: number[], pageCount: number, step: -1 | 1): number | null {
  const first = pages[0];
  if (first === undefined) return null;
  const contiguous = pages.every((page, i) => page === first + i);
  if (step < 0) return first === 0 && contiguous ? null : Math.max(0, first - 1);
  const last = pageCount - pages.length;
  return first >= last && contiguous ? null : Math.min(last, first + 1);
}

/**
 * `to` for dropping `pages` before or after `target` (an index before the
 * move): how many unmoved pages precede the drop point.
 */
export function dropTarget(pages: number[], target: number, after: boolean): number {
  const point = target + (after ? 1 : 0);
  const moving = new Set(pages);
  let to = 0;
  for (let page = 0; page < point; page += 1) if (!moving.has(page)) to += 1;
  return to;
}

/** Marks the selected text (ADR 0017): one annotation per page, in one undo step. */
async function markSelection(kind: MarkupKind): Promise<void> {
  const active = selectActiveDocument(store());
  const viewer = active ? getViewer(active.id) : undefined;
  if (!active || !viewer) return;
  const pages = viewer.selectionMarks();
  if (pages.length === 0) return;
  const outcome = await backend.addMarkup(active.id, kind, pages);
  viewer.clearSelection();
  store().applyEdit(active.id, outcome);
}

/** Switches to the Sign tool, placing `tool` (null: selecting) (ADR 0045). */
function chooseFill(tool: FillTool | null, signature?: number | null) {
  if (store().activeTool !== "sign") store().setActiveTool("sign");
  store().setFill(tool, signature);
}

/** Today's date as the UI language writes it, for Fill & Sign. */
export function fillDate(locale: AppState["locale"], date = new Date()): string {
  return new Intl.DateTimeFormat(formatTag(locale), { dateStyle: "long" }).format(date);
}

/** The setting holding the time-stamping authority's address (ADR 0048). */
export const TSA_SETTING = "signing.tsaUrl";

/** Errors opening a PKCS#12 file that the dialog shows itself. */
const CERTIFICATE_ERRORS = ["certificate_password", "certificate_unreadable", "key_unsupported"];

/** Errors signing that the dialog shows itself, while it is open. */
const SIGNING_ERRORS = [
  "certificate_cannot_sign",
  "timestamp_failed",
  "timestamp_url",
  "signing_failed",
  "signature_too_large",
  "keychain_unavailable",
  "certificate_unreadable",
  "save_as_required",
];

/**
 * Signs `documentId` and shows the signed file and its signatures; false
 * when the file picker was cancelled. Failures the dialog can show stay in
 * it (it is open for an invisible signature); others reach the banner.
 */
async function signNow(
  documentId: number,
  request: CertSignRequest,
  saveAs: boolean,
): Promise<boolean> {
  const sign = useCertSign.getState();
  sign.setBusy(true);
  sign.setError(null);
  try {
    const outcome = await backend.signDocument(documentId, request, saveAs);
    if (!outcome) return false;
    store().applySigned(documentId, outcome);
    sign.setReports(documentId, { status: "done", list: outcome.signatures });
    sign.setDialogOpen(false);
    sign.setPlacing(null);
    if (store().fillTool === "certificate") store().setFill(null);
    showPanel("signatures");
    return true;
  } catch (error) {
    if (sign.dialogOpen && isBackendError(error) && SIGNING_ERRORS.includes(error.code)) {
      sign.setError(error.code);
      return false;
    }
    // A visible signature: placing it again would fail the same way.
    sign.setPlacing(null);
    if (store().fillTool === "certificate") store().setFill(null);
    throw error;
  } finally {
    useCertSign.getState().setBusy(false);
  }
}

/** Checks a document's signatures and keeps the reports (ADR 0048). */
async function checkSignatures(documentId: number): Promise<void> {
  const sign = useCertSign.getState();
  const previous = sign.reports[documentId]?.list ?? [];
  sign.setReports(documentId, { status: "loading", list: previous });
  try {
    sign.setReports(documentId, {
      status: "done",
      list: await backend.documentSignatures(documentId),
    });
  } catch (error) {
    sign.setReports(documentId, { status: "error", list: [] });
    throw error;
  }
}

/** Loads the kept signatures (ADR 0045). */
async function loadSignatures(): Promise<void> {
  store().setSignatures(await backend.signatures());
}

/** Scales the selected Fill & Sign item, its top-left corner kept, and keeps it selected. */
async function resizeFill(factor: number): Promise<void> {
  const selected = selectActiveDocument(store())?.selectedAnnotation;
  if (!selected) return;
  const { rect } = selected;
  const next = { x: rect.x, y: rect.y, width: rect.width * factor, height: rect.height * factor };
  if (next.width < 4 || next.height < 4 || next.width > 2000 || next.height > 2000) return;
  await placeFill(selected, next);
}

/** Moves the selected Fill & Sign item by (`dx`, `dy`) points, kept on the page. */
async function nudgeFill(dx: number, dy: number): Promise<void> {
  const selected = selectActiveDocument(store())?.selectedAnnotation;
  if (!selected) return;
  const { rect } = selected;
  await placeFill(selected, { ...rect, x: Math.max(0, rect.x + dx), y: Math.max(0, rect.y + dy) });
}

/**
 * Gives a Fill & Sign item a new box on the displayed page (moved or
 * resized, ADR 0045) as one undo step, and keeps it selected.
 */
async function placeFill(item: SelectedAnnotation, rect: ViewRect): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active || !FILL_KINDS.includes(item.kind)) return;
  const outcome = await backend.setAnnotationRect(active.id, item.page, item.index, rect);
  getViewer(active.id)?.reselectAfterRefresh({ ...item, rect });
  store().applyEdit(active.id, outcome);
}

/**
 * Moves a Fill & Sign item to `rect` on another page (ADR 0045) as one undo
 * step. It goes last in that page's annotations, and stays selected there.
 */
async function moveFillToPage(
  item: SelectedAnnotation,
  page: number,
  rect: ViewRect,
): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active || !FILL_KINDS.includes(item.kind)) return;
  const outcome = await backend.moveAnnotationToPage(active.id, item.page, item.index, page, rect);
  const landed = (await backend.pageAnnotations(active.id, page))
    .filter((candidate) => candidate.kind === item.kind)
    .at(-1);
  if (landed)
    getViewer(active.id)?.reselectAfterRefresh({ ...item, page, index: landed.index, rect });
  store().applyEdit(active.id, outcome);
}

/** The page before or after the selected Fill & Sign item's, if there is one. */
function fillPageTarget(state: AppState, step: -1 | 1): number | null {
  const active = selectActiveDocument(state);
  const selected = active?.selectedAnnotation;
  if (!active || !selected) return null;
  const page = selected.page + step;
  return page >= 0 && page < active.pageCount ? page : null;
}

/**
 * Moves the selected Fill & Sign item to the page before or after its own,
 * where it was on its page (kept on the new one), and shows that page.
 */
async function stepFillPage(step: -1 | 1): Promise<void> {
  const active = selectActiveDocument(store());
  const selected = active?.selectedAnnotation;
  const page = fillPageTarget(store(), step);
  if (!active || !selected || page === null) return;
  const size = getViewer(active.id)?.pageSize(page) ?? null;
  const { rect } = selected;
  const clamp = (value: number, max: number | null) =>
    Math.max(0, max === null ? value : Math.min(max, value));
  const next = {
    ...rect,
    x: clamp(rect.x, size ? size.width - rect.width : null),
    y: clamp(rect.y, size ? size.height - rect.height : null),
  };
  await moveFillToPage(selected, page, next);
  navigateTo(active, page);
}

/**
 * Arrow keys move the selected Fill & Sign item only while the page view
 * has the focus: elsewhere (a toolbar, a list) they keep their own meaning.
 */
const nudgesFill = (state: AppState) =>
  hasFillSelected(state) &&
  (typeof document === "undefined" ||
    document.activeElement === null ||
    document.activeElement === document.body ||
    document.activeElement.closest("[data-testid='viewer']") !== null);

/** Keeps a new signature, places it next, and closes the dialog. */
function keepSignature(kept: { id: number }) {
  void loadSignatures();
  store().setSignatureDialog(null);
  chooseFill("signature", kept.id);
}

/** Switches to the Comment tool in one of its modes (ADR 0018). */
function chooseCommentTool(tool: CommentTool) {
  store().setActiveTool("comment");
  store().setCommentTool(tool);
}

async function stepActive(step: "undo" | "redo"): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active) return;
  if (stepsMarks(store(), step)) {
    store().updateRedaction(active.id, step === "undo" ? undoMark : redoMark);
    return;
  }
  const outcome =
    step === "undo" ? await backend.undoEdit(active.id) : await backend.redoEdit(active.id);
  store().applyEdit(active.id, outcome);
  // A correction undone or redone moves the review with it (ADR 0037).
  if (outcome.textChanged && ocrJobs().review[active.id]) await refreshReview(active.id);
}

/** Makes `page` current and scrolls the viewer to it. */
function navigateTo(active: OpenDocument, page: number) {
  const target = Math.min(Math.max(0, Math.trunc(page)), active.pageCount - 1);
  store().updateDocument(active.id, { page: target });
  getViewer(active.id)?.goToPage(target);
}

/**
 * The page one step away: the next or previous row of the layout when a viewer
 * is showing the document (so spreads step by two), otherwise one page.
 */
function stepTarget(active: OpenDocument, direction: 1 | -1): number | null {
  const viewer = getViewer(active.id);
  if (viewer) return viewer.stepTarget(direction);
  const next = active.page + direction;
  return next >= 0 && next < active.pageCount ? next : null;
}

function setViewMode(viewMode: ViewMode) {
  const active = selectActiveDocument(store());
  if (active) store().updateDocument(active.id, { viewMode });
}

function setPageOrder(pageOrder: PageOrder) {
  const active = selectActiveDocument(store());
  if (active) store().updateDocument(active.id, { pageOrder });
}

function showPanel(panel: LeftPanel) {
  store().setLeftPanel(panel);
}

function setReadingTheme(theme: ReadingTheme) {
  store().setReadingTheme(theme);
}

// ------------------------------------------------------------ objects and links (ADR 0051)

/** The Edit tool in one of its modes, on a document. */
const editingIn = (state: AppState, mode: "objects" | "links") =>
  hasDocument(state) && state.activeTool === "edit" && state.editMode === mode;
/** An object of the active document is selected in the object mode. */
const objectSelected = (state: AppState) =>
  editingIn(state, "objects") &&
  state.selectedObject !== null &&
  state.selectedObject.documentId === state.activeDocumentId;
/** The selected object can be changed (some text cannot be split out of its text object). */
const objectChangeable = (state: AppState) =>
  objectSelected(state) &&
  state.selectedObject?.object.refusal === null &&
  state.objectCrop === null;
const isCropping = (state: AppState) => objectSelected(state) && state.objectCrop !== null;
const croppable = (state: AppState) =>
  objectChangeable(state) && state.selectedObject?.object.kind !== "text";
/** Arrow keys move the selected object only while the page view has the focus. */
const nudgesObject = (state: AppState) =>
  objectChangeable(state) &&
  (typeof document === "undefined" ||
    document.activeElement === null ||
    document.activeElement === document.body ||
    document.activeElement.closest("[data-testid='viewer']") !== null);
/** A link of the active document is being added or edited. */
const linkDrafted = (state: AppState) =>
  editingIn(state, "links") &&
  state.linkDraft !== null &&
  state.linkDraft.documentId === state.activeDocumentId;
const linkSelected = (state: AppState) => linkDrafted(state) && state.linkDraft?.index !== null;

/** Whether an object the selected one overlaps is drawn above it (`1`) or below it (`-1`). */
function hasOverlapping(state: AppState, direction: 1 | -1): boolean {
  const selected = state.selectedObject;
  if (!objectChangeable(state) || !selected) return false;
  const list = getViewer(selected.documentId)?.objectsOf(selected.page) ?? [];
  const r = selected.object.rect;
  const overlaps = (o: PageObject) =>
    o.rect.x < r.x + r.width &&
    r.x < o.rect.x + o.rect.width &&
    o.rect.y < r.y + r.height &&
    r.y < o.rect.y + o.rect.height;
  return list.some((o) =>
    direction === 1
      ? o.index > selected.object.index && overlaps(o)
      : o.index < selected.object.index && overlaps(o),
  );
}

/** Changes the selected object as one undo step, and keeps it selected where it went. */
async function editObject(edit: ObjectEdit): Promise<void> {
  const active = selectActiveDocument(store());
  const selected = store().selectedObject;
  if (!active || !selected || selected.documentId !== active.id) return;
  const outcome = await backend.editObject(
    active.id,
    selected.page,
    { index: selected.object.index, fingerprint: selected.object.fingerprint },
    edit,
  );
  afterObjectEdit(active.id, selected.page, outcome);
}

function afterObjectEdit(
  documentId: number,
  page: number,
  outcome: { index: number | null } & EditOutcome,
) {
  if (outcome.index !== null)
    getViewer(documentId)?.reselectObjectAfterRefresh(page, outcome.index);
  store().setSelectedObject(null);
  store().applyEdit(documentId, outcome);
}

/** Moves the selected object by (`dx`, `dy`) points of the displayed page. */
function nudgeObject(dx: number, dy: number): Promise<void> {
  return editObject({ kind: "transform", matrix: [1, 0, 0, 1, dx, dy] });
}

/** The Edit tool in `mode`, on the active document. */
function chooseEditMode(mode: "text" | "objects" | "links") {
  if (store().activeTool !== "edit") store().setActiveTool("edit");
  store().setEditMode(mode);
}

/** Reads the active document's named destinations, once, for links to them. */
async function loadDestinations(documentId: number): Promise<void> {
  if (store().destinations?.documentId === documentId) return;
  const list = await backend.namedDestinations(documentId);
  store().setDestinations({ documentId, list });
}

/** Saves the link being added or edited, as one undo step. */
async function saveLink(target: LinkTarget): Promise<void> {
  const active = selectActiveDocument(store());
  const draft = store().linkDraft;
  if (!active || !draft || draft.documentId !== active.id) return;
  const outcome = await backend.editLink(
    active.id,
    draft.index === null
      ? { kind: "add", page: draft.page, rects: draft.rects, target }
      : { kind: "setTarget", page: draft.page, index: draft.index, target },
  );
  store().setLinkDraft(null);
  store().applyEdit(active.id, outcome);
}

/** Error codes with their own message under `errors.`; anything else is "generic". */
const KNOWN_ERRORS = [
  "invalid_pdf",
  "engine_unavailable",
  "file_missing",
  "print_failed",
  "save_failed",
  "io_error",
  "edit_unsupported",
  "invalid_edit",
  "text_failed",
  "no_bookmarks",
  "redaction_failed",
  "no_image",
  "image_unreadable",
  "compress_stale",
  "store_unavailable",
  "password_not_encodable",
  "not_permitted",
  "model_unknown",
  "model_busy",
  "model_unavailable",
  "model_download_failed",
  "model_checksum_mismatch",
  "model_disk_full",
  "model_archive_invalid",
  "model_install_failed",
  "certificate_password",
  "certificate_unreadable",
  "key_unsupported",
  "certificate_cannot_sign",
  "timestamp_failed",
  "timestamp_url",
  "signing_failed",
  "signature_too_large",
  "keychain_unavailable",
  "save_as_required",
  "image_too_large",
  "link_refused",
  "ocr_unavailable",
  "ocr_model_missing",
  "ocr_signed_document",
  "ocr_job_running",
  "ocr_unknown_job",
  "ocr_review_no_word",
  "ocr_review_too_long",
  "ocr_review_converted",
  "ocr_review_stale",
  "ocr_convert_unrecognized",
];

/** Why an OCR job stopped (`OcrJob.error`): codes with their own message under `errors.`. */
export const OCR_JOB_ERRORS = [
  "ocr_engine_missing",
  "ocr_model_missing",
  "ocr_engine_failed",
  "ocr_engine_stopped",
  "ocr_timeout",
  "ocr_bad_response",
  "ocr_image_invalid",
  "ocr_document_changed",
  "ocr_region_converted",
  "ocr_region_unknown_layer",
  "ocr_region_draft",
];

function reportError(error: unknown) {
  const code = isBackendError(error) && KNOWN_ERRORS.includes(error.code) ? error.code : "generic";
  console.error("command failed", error);
  store().setError(code);
}

/**
 * Puts a restored recovery file in a tab (ADR 0040), dropping the tab of its
 * original that it replaces, or queues its password prompt.
 */
function acceptRestore(restored: RestoreOutcome) {
  if (restored.replaced !== null) store().removeDocument(restored.replaced);
  const { outcome } = restored;
  if (outcome.kind === "opened") {
    store().addDocument(outcome.document);
  } else {
    store().pushUnlock({
      token: outcome.token,
      name: outcome.name,
      retry: outcome.retry,
      recovery: true,
    });
  }
}

/**
 * Saves a document to its file. A recovered document whose original changed
 * since (ADR 0040) asks where instead. False when the user cancelled that.
 */
async function saveDocument(id: number): Promise<boolean> {
  try {
    store().applySave(id, await backend.saveDocument(id));
    return true;
  } catch (error) {
    if (!isBackendError(error) || error.code !== "save_as_required") throw error;
    const outcome = await backend.saveDocumentAs(id);
    if (outcome) store().applySave(id, outcome);
    return outcome !== null;
  }
}

/** Pages inserted from files: the new revision, with the inserted pages selected. */
function applyInsert(
  insert: { documentId: number; at: number; pageCount: number },
  edit: EditOutcome,
) {
  store().applyEdit(insert.documentId, edit);
  const added = Math.max(0, edit.pageCount - insert.pageCount);
  store().setSelectedPages(
    insert.documentId,
    Array.from({ length: added }, (_, i) => insert.at + i),
  );
}

/** The recovery file `token` is dealt with: it leaves the list. */
function settleRecovery(token: number) {
  store().setRecoveries(store().recoveries.filter((entry) => entry.token !== token));
}

/** Puts an opened file in a tab, or queues its password prompt. */
export function acceptOutcome(outcome: OpenOutcome, options?: { activate?: boolean }) {
  if (outcome.kind === "opened") {
    store().addDocument(outcome.document, options);
  } else {
    store().pushUnlock({ token: outcome.token, name: outcome.name, retry: outcome.retry });
  }
}

/** Reloads the start screen's recent files (after opening, forgetting, clearing). */
export async function refreshRecents() {
  try {
    store().setRecents(await backend.recentFiles(12));
  } catch (error) {
    console.warn("recent files unavailable", error);
    store().setRecents([]);
  }
}

function moveTab(direction: 1 | -1) {
  const { documents, activeDocumentId } = store();
  if (documents.length < 2) return;
  const index = documents.findIndex((open) => open.id === activeDocumentId);
  const next = documents[(index + direction + documents.length) % documents.length];
  if (next) store().activateDocument(next.id);
}

function shiftTab(direction: 1 | -1) {
  const { documents, activeDocumentId } = store();
  const index = documents.findIndex((open) => open.id === activeDocumentId);
  if (index >= 0 && activeDocumentId !== null)
    store().moveDocument(activeDocumentId, index + direction);
}

/** Copies text: through a `copy` event first (works inside the key press), then the async API. */
async function writeClipboard(text: string) {
  let copied = false;
  const onCopy = (event: ClipboardEvent) => {
    event.clipboardData?.setData("text/plain", text);
    event.preventDefault();
    copied = true;
  };
  document.addEventListener("copy", onCopy, { capture: true, once: true });
  try {
    document.execCommand("copy");
  } finally {
    document.removeEventListener("copy", onCopy, { capture: true });
  }
  if (!copied) await navigator.clipboard.writeText(text);
}

async function runSearch(active: OpenDocument, query: string) {
  const serial = active.search.serial + 1;
  const viewer = getViewer(active.id);
  if (!query.trim()) {
    store().updateSearch(active.id, {
      query,
      status: "idle",
      hits: [],
      truncated: false,
      active: -1,
      error: null,
      serial,
    });
    return;
  }
  store().updateSearch(active.id, { query, status: "searching", error: null, serial });
  try {
    const results = await backend.searchDocument(active.id, query, store().searchOptions);
    const current = store().documents.find((open) => open.id === active.id);
    if (!current || current.search.serial !== serial) return;
    // Start from the reader's position rather than jumping back to page 1.
    const from = results.hits.findIndex((hit) => hit.page >= current.page);
    const first = results.hits.length === 0 ? -1 : Math.max(0, from);
    store().updateSearch(active.id, {
      status: "done",
      hits: results.hits,
      truncated: results.truncated,
      active: first,
    });
    const hit = results.hits[first];
    if (hit) viewer?.revealHit(hit);
  } catch (error) {
    const current = store().documents.find((open) => open.id === active.id);
    if (!current || current.search.serial !== serial) return;
    const code =
      isBackendError(error) && error.code === "invalid_query" ? "searchInvalid" : "searchFailed";
    if (!isBackendError(error) || error.code !== "cancelled") {
      store().updateSearch(active.id, { status: "error", error: code, hits: [], active: -1 });
    }
  }
}

function stepHit(direction: 1 | -1) {
  const active = selectActiveDocument(store());
  if (!active || active.search.hits.length === 0) return;
  const count = active.search.hits.length;
  const current = active.search.active;
  const index =
    current < 0 ? (direction === 1 ? 0 : count - 1) : (current + direction + count) % count;
  goToHit(active, index);
}

function goToHit(active: OpenDocument, index: number) {
  const hit = active.search.hits[index];
  if (!hit) return;
  store().updateSearch(active.id, { active: index });
  getViewer(active.id)?.revealHit(hit);
}

function toggleSearchOption(key: keyof AppState["searchOptions"]) {
  const options = store().searchOptions;
  store().setSearchOptions({ ...options, [key]: !options[key] });
  const active = selectActiveDocument(store());
  if (active?.search.query) void runSearch(active, active.search.query);
}

/** The theme currently on screen, with "system" resolved. */
export function resolvedTheme(theme: AppState["theme"]): "light" | "dark" {
  if (theme !== "system") return theme;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

/** Runs a search for things to redact, keeping the newest answer (ADR 0042). */
async function runRedactFind(
  active: OpenDocument,
  pattern: RedactPattern,
  field: { query: string; preset: RedactPreset | null },
) {
  const serial = active.redaction.find.serial + 1;
  store().updateRedaction(active.id, (redaction) => ({
    ...redaction,
    ...field,
    find: { ...redaction.find, status: "searching", error: null, serial },
  }));
  const current = () => store().documents.find((open) => open.id === active.id);
  try {
    const results = await backend.findRedactions(active.id, pattern);
    if (current()?.redaction.find.serial !== serial) return;
    store().updateRedaction(active.id, (redaction) => ({
      ...redaction,
      find: {
        status: "done",
        hits: results.hits,
        truncated: results.truncated,
        error: null,
        serial,
      },
    }));
  } catch (error) {
    if (current()?.redaction.find.serial !== serial) return;
    const code =
      isBackendError(error) && error.code === "invalid_query" ? "searchInvalid" : "searchFailed";
    store().updateRedaction(active.id, (redaction) => ({
      ...redaction,
      find: { status: "error", hits: [], truncated: false, error: code, serial },
    }));
  }
}

/** Marks hits for redaction: those not marked already. */
function markHits(active: OpenDocument, indices: number[]) {
  store().updateRedaction(active.id, (redaction) => {
    let next = redaction;
    for (const index of indices) {
      const hit = redaction.find.hits[index];
      if (!hit) continue;
      next = addMark(next, {
        page: hit.page,
        rects: hit.rects,
        text: hit.text,
        source: "search",
      }).state;
    }
    return next;
  });
}

const ocr = () => useOcrModels.getState();
const ocrJobs = () => useOcrJobs.getState();

/** Loads a document's OCR jobs, review list and layer checks from the shell (ADR 0035, 0038). */
export async function refreshOcrJobs(documentId: number): Promise<void> {
  const [jobs, review, checks] = await Promise.all([
    backend.ocrJobs(documentId),
    backend.ocrReview(documentId, ocrJobs().numbers),
    backend.ocrLayerChecks(documentId),
  ]);
  ocrJobs().setJobs(jobs);
  ocrJobs().setReview(documentId, review);
  ocrJobs().setChecks(documentId, checks);
  refocus(documentId);
}

/** Reloads a document's review list (every number too when asked), keeping the word shown. */
export async function refreshReview(documentId: number): Promise<void> {
  ocrJobs().setReview(documentId, await backend.ocrReview(documentId, ocrJobs().numbers));
  refocus(documentId);
}

/** After the list changed, the shown word's place in it is found again (or it is let go). */
function refocus(documentId: number) {
  const focus = ocrJobs().focus;
  if (!focus || focus.documentId !== documentId || focus.word === null) return;
  const pages = ocrJobs().review[documentId] ?? [];
  const at = positionOf(pages, focus.page, focus.word);
  ocrJobs().setFocus(at ? { ...focus, at } : null);
}

/** Whether the models OCR needs are in; otherwise asks to download them. */
async function modelsReady(): Promise<boolean> {
  if (!ocr().models) await refreshOcrModels();
  const { models } = ocr();
  if (!models?.available) {
    store().setError("model_unavailable");
    return false;
  }
  if (!ocrReady(models)) {
    ocr().setPrompt(true);
    return false;
  }
  return true;
}

/** Starts OCR of the active document's `pages` (all when absent), once its models are in. */
async function recognize(pages?: number[], replace = false): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active || !(await modelsReady())) return;
  ocrJobs().setJob(await backend.startOcr(active.id, pages, replace));
  // The OCR panel shows the job's progress and, as pages finish, what to review.
  store().setActiveTool("ocr");
}

/** Shows a review item: a word (its page, its box marked) or a page to check. */
function showItem(item: ReviewItem | null) {
  const active = selectActiveDocument(store());
  if (!active) return;
  if (!item) {
    ocrJobs().setFocus(null);
    return;
  }
  if (item.at) {
    showFlag(item.at);
    return;
  }
  ocrJobs().setFocus({ documentId: active.id, page: item.page, word: null, at: null, rect: null });
  store().updateDocument(active.id, { page: item.page });
  getViewer(active.id)?.goToPage(item.page);
}

/** The word (or page) review mode is on, in the active document. */
function reviewed(): { documentId: number; page: number; word: number | null } | null {
  const active = selectActiveDocument(store());
  const focus = ocrJobs().focus;
  if (!active || !focus || focus.documentId !== active.id) return null;
  return { documentId: active.id, page: focus.page, word: focus.word };
}

/**
 * Takes a review decision on the word (or page) shown, then, in review mode,
 * shows the next item still open (ADR 0037).
 */
async function decide(action: OcrAction): Promise<void> {
  const current = reviewed();
  if (!current) return;
  const result = await backend.ocrDecide(
    current.documentId,
    current.page,
    action.kind === "checkPage" ? null : current.word,
    action,
    ocrJobs().numbers,
  );
  if (result.outcome) store().applyEdit(current.documentId, result.outcome);
  ocrJobs().setReview(current.documentId, result.review);
  if (ocrJobs().reviewing) showItem(nextItem(reviewQueue(result.review), current, result.review));
  else refocus(current.documentId);
}

/** The shown word as the review list has it. */
function reviewedWord() {
  const current = reviewed();
  if (!current || current.word === null) return undefined;
  const pages = ocrJobs().review[current.documentId] ?? [];
  const at = positionOf(pages, current.page, current.word);
  return at ? wordAt(pages, at) : undefined;
}

/** Converts recognised pages of the active document into page text (ADR 0039). */
async function convert(pages: number[] | null): Promise<void> {
  const active = selectActiveDocument(store());
  if (!active) return;
  const { keepScan, family } = ocrJobs();
  try {
    const outcome = await backend.ocrConvert(active.id, pages, keepScan, family);
    store().applyEdit(active.id, outcome);
  } catch (error) {
    if (isBackendError(error) && error.code === "ocr_convert_unreviewed") {
      const locale = store().locale;
      const list = error.detail
        .split(",")
        .map((page) => formatNumber(Number(page), locale))
        .join(i18n.t("ocrPanel.convert.listSeparator"));
      store().setNotice({ key: "ocrConvertUnreviewed", values: { pages: list } });
      return;
    }
    throw error;
  }
  await refreshReview(active.id);
}

async function controlJob(arg: CommandArg | undefined, action: "pause" | "resume" | "cancel") {
  if (arg?.jobId === undefined) return;
  ocrJobs().setJob(await backend.controlOcrJob(arg.jobId, action));
}

/** Shows a flagged word of the active document's review list: its page, its box marked. */
function showFlag(at: [number, number]) {
  const active = selectActiveDocument(store());
  if (!active) return;
  const pages = ocrJobs().review[active.id] ?? [];
  const word = wordAt(pages, at);
  const page = pages[at[0]];
  if (!word || !page) return;
  const rect = wordRect(word);
  ocrJobs().setFocus({ documentId: active.id, page: page.page, word: word.index, at, rect });
  const viewer = getViewer(active.id);
  store().updateDocument(active.id, { page: page.page });
  if (viewer && rect) viewer.revealHit({ page: page.page, rects: [rect] });
  else viewer?.goToPage(page.page);
}

/** The next or previous flagged word after the one shown (in review mode: still open). */
function stepFlag(direction: 1 | -1) {
  const active = selectActiveDocument(store());
  if (!active) return;
  if (ocrJobs().reviewing) {
    const pages = ocrJobs().review[active.id] ?? [];
    const queue = reviewQueue(pages);
    if (direction === 1) {
      showItem(nextItem(queue, reviewed(), pages));
    } else {
      const current = reviewed();
      const at = current
        ? queue.findIndex((item) => item.page === current.page && item.word === current.word)
        : -1;
      // Before the current item (wrapping), or the last when the current one was decided.
      const index = at < 0 ? queue.length - 1 : (at - 1 + queue.length) % queue.length;
      showItem(queue[index] ?? null);
    }
    return;
  }
  const flags = flaggedWords(ocrJobs().review[active.id] ?? []);
  if (flags.length === 0) return;
  const focus = ocrJobs().focus;
  const current =
    focus?.documentId === active.id && focus.at
      ? flags.findIndex(([p, w]) => p === focus.at?.[0] && w === focus.at?.[1])
      : -1;
  const next =
    current < 0
      ? direction === 1
        ? 0
        : flags.length - 1
      : (current + direction + flags.length) % flags.length;
  const at = flags[next];
  if (at) showFlag(at);
}

const hasFlags = (state: AppState) => {
  const active = selectActiveDocument(state);
  return active !== null && flaggedWords(useOcrJobs.getState().review[active.id] ?? []).length > 0;
};

/** Review mode is on, with a word (or a page) shown in the active document. */
const reviewingItem = (state: AppState) => {
  const active = selectActiveDocument(state);
  const { reviewing, focus } = useOcrJobs.getState();
  return reviewing && active !== null && focus?.documentId === active.id;
};

/** An area is drawn on the active document. */
const hasRegion = (state: AppState) => {
  const active = selectActiveDocument(state);
  return active !== null && useOcrJobs.getState().region?.documentId === active.id;
};

/** Pages of the active document whose existing text layer the check found poor. */
function poorPages(state: AppState): number[] {
  const active = selectActiveDocument(state);
  if (!active) return [];
  return (useOcrJobs.getState().checks[active.id] ?? [])
    .filter((check) => check.verdict === "poor")
    .map((check) => check.page);
}

/** Loads the OCR models' listing from the shell (ADR 0034). */
export async function refreshOcrModels(): Promise<void> {
  ocr().setModels(await backend.ocrModels());
}

/** Starts a download; progress then arrives as events (App listens). */
async function downloadOcrModel(id: string): Promise<void> {
  ocr().startJob(id);
  try {
    await backend.downloadOcrModel(id);
  } catch (error) {
    // The listing says what is really running (e.g. already downloading).
    await refreshOcrModels().catch(() => {});
    throw error;
  }
}

const DECLARED: Command[] = [
  {
    id: "file.open",
    shortcut: { key: "o", mod: true },
    async run() {
      const outcome = await backend.openDocumentDialog();
      if (outcome) acceptOutcome(outcome);
      void refreshRecents();
    },
  },
  {
    id: "file.close",
    shortcut: { key: "w", mod: true },
    isEnabled: hasDocument,
    async run(arg) {
      const id = arg?.documentId ?? selectActiveDocument(store())?.id;
      if (id === undefined) return;
      const open = store().documents.find((candidate) => candidate.id === id);
      if (open?.modified && !arg?.force) {
        store().setClosePrompt({ kind: "tab", documentId: id, name: open.name });
        return;
      }
      store().setClosePrompt(null);
      store().removeDocument(id);
      await backend.closeDocument(id);
    },
  },
  {
    id: "file.saveAndClose",
    paletteHidden: true,
    async run(arg) {
      const id = arg?.documentId;
      if (id === undefined) return;
      if (await saveDocument(id)) await runCommand("file.close", { documentId: id, force: true });
    },
  },
  {
    id: "window.closeDiscarding",
    paletteHidden: true,
    async run() {
      store().setClosePrompt(null);
      await backend.closeWindowDiscarding();
    },
  },
  {
    id: "app.quitDiscarding",
    paletteHidden: true,
    async run() {
      store().setClosePrompt(null);
      await backend.quitDiscarding();
    },
  },
  {
    id: "app.cancelClose",
    paletteHidden: true,
    run() {
      store().setClosePrompt(null);
    },
  },
  {
    id: "file.save",
    shortcut: { key: "s", mod: true },
    isEnabled: (state) => selectActiveDocument(state)?.modified ?? false,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await saveDocument(active.id);
    },
  },
  {
    id: "file.saveAs",
    shortcut: { key: "s", mod: true, shift: true },
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const outcome = await backend.saveDocumentAs(active.id);
      if (outcome) store().applySave(active.id, outcome);
    },
  },
  {
    id: "edit.undo",
    shortcut: { key: "z", mod: true },
    isEnabled: canUndo,
    run: () => stepActive("undo"),
  },
  {
    id: "edit.redo",
    shortcut: { key: "z", mod: true, shift: true },
    isEnabled: canRedo,
    run: () => stepActive("redo"),
  },
  {
    id: "pages.rotateClockwise",
    isEnabled: hasDocument,
    run: () =>
      editActive(
        (_, pages) => ({ kind: "rotate", pages, quarterTurns: 1 }),
        (pages) => pages,
      ),
  },
  {
    id: "pages.rotateCounterclockwise",
    isEnabled: hasDocument,
    run: () =>
      editActive(
        (_, pages) => ({ kind: "rotate", pages, quarterTurns: -1 }),
        (pages) => pages,
      ),
  },
  {
    id: "pages.delete",
    // A document keeps at least one page.
    isEnabled: (state) =>
      targetPages(state).length > 0 &&
      targetPages(state).length < (selectActiveDocument(state)?.pageCount ?? 0),
    run: () => editActive((_, pages) => ({ kind: "delete", pages })),
  },
  {
    id: "pages.duplicate",
    isEnabled: hasDocument,
    // Each copy lands right after its page; the copies are selected.
    run: () =>
      editActive(
        (_, pages) => ({ kind: "duplicate", pages }),
        (pages) => pages.map((page, i) => page + i + 1),
      ),
  },
  {
    id: "pages.insertBlank",
    isEnabled: hasDocument,
    // After the last target page, the size of the current page (A4 until it is known).
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "insertBlank",
          at: (pages[pages.length - 1] ?? active.page) + 1,
          width: active.pageSize?.width ?? 595,
          height: active.pageSize?.height ?? 842,
        }),
        (_, edit) => (edit.kind === "insertBlank" ? [edit.at] : []),
      ),
  },
  {
    id: "pages.moveEarlier",
    isEnabled: (state) =>
      moveTarget(targetPages(state), selectActiveDocument(state)?.pageCount ?? 0, -1) !== null,
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "move",
          pages,
          to: moveTarget(pages, active.pageCount, -1) ?? 0,
        }),
        (pages, edit) => landed(edit, pages.length),
      ),
  },
  {
    id: "pages.moveLater",
    isEnabled: (state) =>
      moveTarget(targetPages(state), selectActiveDocument(state)?.pageCount ?? 0, 1) !== null,
    run: () =>
      editActive(
        (active, pages) => ({
          kind: "move",
          pages,
          to: moveTarget(pages, active.pageCount, 1) ?? active.pageCount,
        }),
        (pages, edit) => landed(edit, pages.length),
      ),
  },
  {
    // Drag and drop in the page organizer: `pages` land at `to` of the result.
    id: "pages.move",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const pages = arg?.pages;
      if (!active || !pages?.length || arg?.to === undefined) return;
      const to = arg.to;
      store().applyEdit(active.id, await backend.editPages(active.id, { kind: "move", pages, to }));
      store().setSelectedPages(
        active.id,
        pages.map((_, i) => to + i),
      );
    },
  },
  {
    // Merging is inserting at the end; in the organizer, after the selection.
    id: "pages.insertFromFile",
    isEnabled: hasDocument,
    async run() {
      const state = store();
      const active = selectActiveDocument(state);
      if (!active) return;
      const inOrganizer = state.activeTool === "pages" && active.selectedPages.length > 0;
      const last = active.selectedPages[active.selectedPages.length - 1] ?? 0;
      const at = inOrganizer ? last + 1 : active.pageCount;
      const outcome = await backend.insertPagesFromFiles(active.id, at);
      if (!outcome) return;
      const insert = { documentId: active.id, at, pageCount: active.pageCount };
      if (outcome.kind === "inserted") {
        applyInsert(insert, outcome.edit);
      } else {
        store().pushUnlock({ ...outcome, insert });
      }
    },
  },
  {
    id: "pages.extract",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const name = await backend.extractPages(active.id, targetPages());
      if (name) store().setNotice({ key: "extracted", values: { name } });
    },
  },
  {
    // Asks how to split, then `pages.splitInto` writes the files.
    id: "pages.split",
    isEnabled: (state) => (selectActiveDocument(state)?.pageCount ?? 0) > 1,
    run() {
      store().setSplitPrompt(true);
    },
  },
  {
    id: "pages.splitInto",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      store().setSplitPrompt(false);
      const active = selectActiveDocument(store());
      const split = arg?.split;
      if (!active || !split) return;
      if (split.kind === "pages" && (!Number.isInteger(split.size) || split.size < 1)) return;
      if (split.kind === "bytes" && !(split.limit >= 1)) return;
      const count = await backend.splitDocument(active.id, split);
      if (count !== null) store().setNotice({ key: "split", values: { count: String(count) } });
    },
  },
  // Output (ADR 0050): each opens its dialog; the dialog runs `output.*` with what was chosen.
  ...(
    [
      ["pages.headerFooter", "headerFooter"],
      ["pages.pageNumbers", "pageNumbers"],
      ["pages.watermark", "watermark"],
      ["file.exportImages", "exportImages"],
      ["file.exportText", "exportText"],
      ["file.compress", "compress"],
    ] as const
  ).map(
    ([id, dialog]): Command => ({
      id,
      // Compression rewrites the file, which this app does not do to an encrypted one (ADR 0050).
      isEnabled: id === "file.compress" ? isUnencrypted : hasDocument,
      run: () => openOutput(dialog),
    }),
  ),
  {
    id: "output.stamp",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.stamp || !arg.pages || arg.pages.length === 0) return;
      const outcome = await backend.stampPages(active.id, arg.pages, arg.stamp);
      store().setOutputDialog(null);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "output.pickImage",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run() {
      const name = await backend.pickWatermarkImage();
      if (name) store().setWatermarkImage(name);
    },
  },
  {
    id: "output.exportImages",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.pages || arg.pages.length === 0 || !arg.imageFormat || !arg.dpi) return;
      const count = await backend.exportPageImages(active.id, arg.pages, arg.dpi, arg.imageFormat);
      if (count === null) return;
      store().setOutputDialog(null);
      store().setNotice({ key: "exportedImages", values: { count: String(count) } });
    },
  },
  {
    id: "output.exportText",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.pages || arg.pages.length === 0 || !arg.textFormat) return;
      const name = await backend.exportText(active.id, arg.pages, arg.textFormat);
      if (name === null) return;
      store().setOutputDialog(null);
      store().setNotice({ key: "exportedText", values: { name } });
    },
  },
  {
    // Compresses in memory and shows the sizes; nothing is written yet.
    id: "output.compress",
    paletteHidden: true,
    isEnabled: isUnencrypted,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.compress) return;
      store().setCompressed(null);
      const report = await backend.compressDocument(active.id, arg.compress);
      store().setCompressed({ documentId: active.id, report });
    },
  },
  {
    id: "output.saveCompressed",
    paletteHidden: true,
    isEnabled: (state) =>
      state.compressed !== null && state.compressed.documentId === state.activeDocumentId,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const name = await backend.saveCompressed(active.id);
      if (name === null) return;
      store().setOutputDialog(null);
      store().setNotice({ key: "compressedSaved", values: { name } });
    },
  },
  {
    id: "output.cancel",
    paletteHidden: true,
    run: () => store().setOutputDialog(null),
  },
  {
    // Opens a page from the organizer in the page view.
    id: "pages.open",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      if (arg?.page === undefined) return;
      store().setActiveTool("select");
      await runCommand("view.goToPage", { page: arg.page });
    },
  },
  {
    id: "file.print",
    shortcut: { key: "p", mod: true },
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await backend.printDocument(active.id);
    },
  },
  {
    id: "file.openRecent",
    paletteHidden: true,
    async run(arg) {
      if (arg?.token === undefined) return;
      acceptOutcome(await backend.openRecent(arg.token));
      void refreshRecents();
    },
  },
  {
    id: "file.forgetRecent",
    paletteHidden: true,
    async run(arg) {
      if (arg?.token === undefined) return;
      await backend.forgetRecent(arg.token);
      await refreshRecents();
    },
  },
  {
    id: "file.clearRecents",
    isEnabled: (state) => (state.recents?.length ?? 0) > 0,
    async run() {
      await backend.clearRecents();
      await refreshRecents();
    },
  },
  {
    id: "file.unlock",
    paletteHidden: true,
    isEnabled: (state) => state.unlocks.length > 0,
    async run(arg) {
      const pending = store().unlocks[0];
      if (!pending || arg?.password === undefined) return;
      if (pending.insert) {
        const outcome = await backend.unlockInsert(pending.token, arg.password);
        if (outcome.kind === "inserted") {
          store().resolveUnlock(null);
          applyInsert(pending.insert, outcome.edit);
        } else {
          store().resolveUnlock({ ...outcome, insert: pending.insert });
        }
        return;
      }
      if (pending.recovery) {
        const restored = await backend.unlockRecovery(pending.token, arg.password);
        const { outcome } = restored;
        if (outcome.kind === "opened") {
          store().resolveUnlock(null);
          if (restored.replaced !== null) store().removeDocument(restored.replaced);
          store().addDocument(outcome.document);
        } else {
          store().resolveUnlock({ ...outcome, recovery: true });
        }
        return;
      }
      const outcome = await backend.unlockDocument(pending.token, arg.password);
      if (outcome.kind === "opened") {
        store().resolveUnlock(null);
        store().addDocument(outcome.document);
        void refreshRecents();
      } else {
        store().resolveUnlock({ token: outcome.token, name: outcome.name, retry: outcome.retry });
      }
    },
  },
  {
    id: "file.cancelUnlock",
    paletteHidden: true,
    isEnabled: (state) => state.unlocks.length > 0,
    async run() {
      const pending = store().unlocks[0];
      if (!pending) return;
      store().resolveUnlock(null);
      await backend.cancelUnlock(pending.token);
    },
  },
  {
    id: "tabs.next",
    shortcut: { key: "Tab", ctrl: true },
    isEnabled: (state) => state.documents.length > 1,
    run: () => moveTab(1),
  },
  {
    id: "tabs.previous",
    shortcut: { key: "Tab", ctrl: true, shift: true },
    isEnabled: (state) => state.documents.length > 1,
    run: () => moveTab(-1),
  },
  {
    // Tab order is reading order: "earlier" is to the left in English and to
    // the right in Arabic, so the commands are named by order, not side.
    id: "tabs.moveEarlier",
    shortcut: { key: "PageUp", ctrl: true, shift: true },
    isEnabled: (state) => state.documents.findIndex((d) => d.id === state.activeDocumentId) > 0,
    run: () => shiftTab(-1),
  },
  {
    id: "tabs.moveLater",
    shortcut: { key: "PageDown", ctrl: true, shift: true },
    isEnabled: (state) => {
      const index = state.documents.findIndex((d) => d.id === state.activeDocumentId);
      return index >= 0 && index < state.documents.length - 1;
    },
    run: () => shiftTab(1),
  },
  {
    id: "tabs.move",
    paletteHidden: true,
    isEnabled: (state) => state.documents.length > 1,
    run(arg) {
      if (arg?.documentId !== undefined && arg.index !== undefined) {
        store().moveDocument(arg.documentId, arg.index);
      }
    },
  },
  {
    id: "tabs.moveToNewWindow",
    isEnabled: (state) => state.documents.length > 1,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      await backend.moveToNewWindow(active.id);
      // The document stays open; it now belongs to the new window.
      store().removeDocument(active.id);
    },
  },
  {
    id: "window.new",
    shortcut: { key: "n", mod: true },
    run: () => backend.newWindow(),
  },
  {
    id: "edit.copy",
    shortcut: { key: "c", mod: true, notInFields: true },
    isEnabled: hasSelection,
    async run() {
      const active = selectActiveDocument(store());
      const text = active ? (getViewer(active.id)?.selectedText() ?? "") : "";
      if (text) await writeClipboard(text);
    },
  },
  {
    id: "markup.highlight",
    isEnabled: hasSelection,
    run: () => markSelection("highlight"),
  },
  {
    id: "markup.underline",
    isEnabled: hasSelection,
    run: () => markSelection("underline"),
  },
  {
    id: "markup.strikeOut",
    isEnabled: hasSelection,
    run: () => markSelection("strikeOut"),
  },
  {
    id: "markup.squiggly",
    isEnabled: hasSelection,
    run: () => markSelection("squiggly"),
  },
  {
    id: "annotation.delete",
    shortcut: { key: "Delete", alternate: "Backspace", notInFields: true },
    isEnabled: hasAnnotationSelected,
    async run() {
      const active = selectActiveDocument(store());
      const selected = active?.selectedAnnotation;
      if (!active || !selected) return;
      const outcome = await backend.removeAnnotations(active.id, selected.page, [selected.index]);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "redact.openTool",
    isEnabled: hasDocument,
    run: () => store().setActiveTool("protect"),
  },
  {
    id: "redact.addArea",
    isEnabled: hasDocument,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      const { page, rect } = arg ?? {};
      if (!active || page === undefined || !rect) return;
      if (rect.width < MIN_MARK_SIDE || rect.height < MIN_MARK_SIDE) return;
      store().updateRedaction(
        active.id,
        (redaction) =>
          addMark(redaction, { page, rects: [rect], text: null, source: "area" }).state,
      );
    },
  },
  {
    id: "redact.markSelection",
    isEnabled: hasSelection,
    run() {
      const active = selectActiveDocument(store());
      const viewer = active ? getViewer(active.id) : undefined;
      if (!active || !viewer) return;
      const pages = viewer.selectionMarks();
      const text = viewer.selectedText().trim();
      store().updateRedaction(active.id, (redaction) => {
        let next = redaction;
        for (const { page, rects } of pages) {
          next = addMark(next, {
            page,
            rects,
            text: text.length > 0 ? text : null,
            source: "selection",
          }).state;
        }
        return next;
      });
      viewer.clearSelection();
      store().setActiveTool("protect");
    },
  },
  {
    id: "redact.find",
    isEnabled: hasDocument,
    paletteHidden: true,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      const query = arg?.query ?? active.redaction.query;
      if (!query.trim()) {
        store().updateRedaction(active.id, (redaction) => ({
          ...redaction,
          query,
          preset: null,
          find: { ...redaction.find, status: "idle", hits: [], error: null },
        }));
        return;
      }
      const regex = active.redaction.regex;
      await runRedactFind(
        active,
        { kind: "text", query, options: { ...store().searchOptions, regex } },
        { query, preset: null },
      );
    },
  },
  {
    id: "redact.findPreset",
    isEnabled: hasDocument,
    paletteHidden: true,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.preset) return;
      await runRedactFind(
        active,
        { kind: "preset", preset: arg.preset },
        { query: "", preset: arg.preset },
      );
    },
  },
  {
    id: "redact.toggleRegex",
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, { regex: !active.redaction.regex });
    },
  },
  {
    id: "redact.markHit",
    isEnabled: hasDocument,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.index !== undefined) markHits(active, [arg.index]);
    },
  },
  {
    id: "redact.markAllHits",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.find.hits.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active)
        markHits(
          active,
          active.redaction.find.hits.map((_, i) => i),
        );
    },
  },
  {
    id: "redact.removeMark",
    isEnabled: hasMarks,
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      const id = arg?.markId;
      if (active && id !== undefined) {
        store().updateRedaction(active.id, (redaction) => removeMark(redaction, id));
      }
    },
  },
  {
    id: "redact.clearMarks",
    isEnabled: hasMarks,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateRedaction(active.id, (redaction) => ({
          ...redaction,
          marks: [],
          undone: [],
        }));
      }
    },
  },
  {
    id: "redact.undoMark",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.marks.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, undoMark);
    },
  },
  {
    id: "redact.redoMark",
    isEnabled: (state) => (selectActiveDocument(state)?.redaction.undone.length ?? 0) > 0,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateRedaction(active.id, redoMark);
    },
  },
  {
    // Asks first: applying removes content from the file.
    id: "redact.apply",
    isEnabled: hasMarks,
    run: () => store().setRedactPrompt(true),
  },
  {
    id: "redact.confirm",
    isEnabled: hasMarks,
    paletteHidden: true,
    async run() {
      const active = selectActiveDocument(store());
      store().setRedactPrompt(false);
      if (!active || active.redaction.marks.length === 0) return;
      const outcome = await backend.applyRedactions(active.id, marksForApply(active.redaction));
      // The edit starts the document over (the marks are spent); then the summary.
      store().applyEdit(active.id, outcome.edit);
      store().updateRedaction(active.id, { applied: outcome.report, sanitized: null });
    },
  },
  {
    id: "redact.cancel",
    paletteHidden: true,
    run: () => store().setRedactPrompt(false),
  },
  {
    id: "sanitize.setOption",
    paletteHidden: true,
    run(arg) {
      if (!arg?.option || arg.value === undefined) return;
      store().setSanitizeOptions({ ...store().sanitizeOptions, [arg.option]: arg.value });
    },
  },
  {
    id: "sanitize.run",
    isEnabled: (state) => hasDocument(state) && Object.values(state.sanitizeOptions).some(Boolean),
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const outcome = await backend.sanitizeDocument(active.id, store().sanitizeOptions);
      store().applyEdit(active.id, outcome.edit);
      store().updateRedaction(active.id, { sanitized: outcome.report, applied: null });
    },
  },
  {
    id: "security.open",
    isEnabled: hasDocument,
    run: () => store().setSecurityPrompt(true),
  },
  {
    id: "security.apply",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || !arg?.securityChange) return;
      const outcome = await backend.secureDocument(
        active.id,
        arg.securityChange,
        arg.saveAs ?? false,
      );
      // Cancelled in the file picker: the dialog stays for another try.
      if (!outcome) return;
      store().applySecured(active.id, outcome);
      store().setSecurityPrompt(false);
    },
  },
  {
    id: "security.cancel",
    paletteHidden: true,
    run: () => store().setSecurityPrompt(false),
  },
  {
    // Sign with a certificate (ADR 0048): the dialog, with the identities
    // available now and the time-stamping authority used last.
    id: "certSign.open",
    isEnabled: hasDocument,
    async run() {
      const sign = useCertSign.getState();
      sign.setPlacing(null);
      sign.setError(null);
      sign.setDialogOpen(true);
      const settings = await backend.settings();
      const tsaUrl = settings[TSA_SETTING] ?? "";
      sign.updateForm({ tsaUrl, timestamp: tsaUrl.trim() !== "" });
      sign.setIdentities(await backend.signingIdentities());
      if (store().signatures === null) await loadSignatures();
    },
  },
  {
    id: "certSign.cancel",
    paletteHidden: true,
    async run() {
      const sign = useCertSign.getState();
      if (sign.picked) await backend.cancelCertificate(sign.picked.token);
      sign.setPicked(null);
      sign.setPlacing(null);
      sign.setError(null);
      sign.setDialogOpen(false);
      if (store().fillTool === "certificate") store().setFill(null);
    },
  },
  {
    // A PKCS#12 file, picked through the native dialog; its password comes next.
    id: "certSign.importFile",
    paletteHidden: true,
    async run() {
      const picked = await backend.pickCertificateFile();
      if (picked) useCertSign.getState().setPicked(picked);
    },
  },
  {
    id: "certSign.unlockFile",
    paletteHidden: true,
    async run(arg) {
      const sign = useCertSign.getState();
      const { picked } = sign;
      if (!picked || arg?.password === undefined) return;
      try {
        const identity = await backend.unlockCertificate(picked.token, arg.password);
        sign.setPicked(null);
        sign.setIdentities(await backend.signingIdentities());
        sign.updateForm({ identity: identity.id });
      } catch (error) {
        // A wrong password asks again, in the dialog.
        if (isBackendError(error) && CERTIFICATE_ERRORS.includes(error.code)) {
          sign.setPicked(picked, error.code);
          return;
        }
        throw error;
      }
    },
  },
  {
    id: "certSign.cancelFile",
    paletteHidden: true,
    async run() {
      const sign = useCertSign.getState();
      if (sign.picked) await backend.cancelCertificate(sign.picked.token);
      sign.setPicked(null);
    },
  },
  {
    id: "certSign.forget",
    paletteHidden: true,
    async run(arg) {
      if (!arg?.identity) return;
      await backend.forgetIdentity(arg.identity);
      useCertSign.getState().setIdentities(await backend.signingIdentities());
    },
  },
  {
    // Signs with what the dialog holds. An invisible signature is made at
    // once; a visible one waits for a press on a page (certSign.place).
    id: "certSign.apply",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const sign = useCertSign.getState();
      const { form } = sign;
      if (!active || !form.identity || sign.busy) return;
      const text = (value: string) => (value.trim() === "" ? null : value.trim());
      const request = {
        identity: form.identity,
        reason: text(form.reason),
        location: text(form.location),
        contact: text(form.contact),
        timestamp: form.timestamp,
      };
      const saveAs = arg?.saveAs ?? false;
      if (form.timestamp) await backend.setSetting(TSA_SETTING, form.tsaUrl.trim());
      if (form.visible) {
        sign.setPlacing({ documentId: active.id, request, graphic: form.graphic, saveAs });
        sign.setDialogOpen(false);
        store().setActiveTool("sign");
        store().setFill("certificate");
        return;
      }
      await signNow(active.id, { ...request, visible: null }, saveAs);
    },
  },
  {
    // A press on a page placed the visible signature: centred on it.
    id: "certSign.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const { placing, identities } = useCertSign.getState();
      if (!active || !placing || placing.documentId !== active.id) return;
      if (arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      const signer = identities?.find((one) => one.id === placing.request.identity);
      const { width, height } = VISIBLE_SIZE;
      const caption = [
        i18n.t("certSign.captionSigned", { lng: store().locale }),
        signer?.certificate.name ?? "",
        fillDate(store().locale),
      ]
        .filter((line) => line.trim() !== "")
        .join("\n");
      const visible = {
        page: arg.page,
        x: Math.max(0, arg.x - width / 2),
        y: Math.max(0, arg.y - height / 2),
        width,
        height,
        signature: placing.graphic,
        caption,
      };
      await signNow(active.id, { ...placing.request, visible }, placing.saveAs);
    },
  },
  {
    // Checks the document's signatures again (ADR 0048).
    id: "signatures.check",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await checkSignatures(active.id);
    },
  },
  {
    // From the signatures panel: the page a visible signature is on.
    id: "signatures.reveal",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      if (arg?.page !== undefined) await runCommand("view.goToPage", { page: arg.page });
    },
  },
  {
    id: "view.showSignatures",
    async run() {
      showPanel("signatures");
      const active = selectActiveDocument(store());
      if (active && !useCertSign.getState().reports[active.id]) await checkSignatures(active.id);
    },
  },
  {
    id: "recovery.restore",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    async run(arg) {
      if (arg?.token === undefined) return;
      const restored = await backend.restoreRecovery(arg.token);
      settleRecovery(arg.token);
      acceptRestore(restored);
    },
  },
  {
    id: "recovery.discard",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    async run(arg) {
      if (arg?.token === undefined) return;
      await backend.discardRecovery(arg.token);
      settleRecovery(arg.token);
    },
  },
  {
    // Offered again at the next start.
    id: "recovery.later",
    paletteHidden: true,
    isEnabled: (state) => state.recoveries.length > 0,
    run: () => store().setRecoveries([]),
  },
  {
    id: "forms.openTool",
    isEnabled: hasDocument,
    run: () => store().setActiveTool("forms"),
  },
  {
    // A field's new value (ADR 0045): one undo step.
    id: "form.setValue",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.field === undefined || !arg.values) return;
      const outcome = await backend.setFormField(active.id, arg.field, arg.values);
      store().applyEdit(active.id, outcome);
    },
  },
  { id: "forms.fillMode", isEnabled: hasDocument, run: () => openForms("fill") },
  { id: "forms.designMode", isEnabled: hasDocument, run: () => openForms("design") },
  ...DESIGN_TOOLS.map(
    ([id, tool]): Command => ({ id, isEnabled: hasDocument, run: () => chooseDesignTool(tool) }),
  ),
  {
    // A drag (or a click) with a field tool makes a field there (ADR 0047).
    id: "formDesign.draw",
    paletteHidden: true,
    isEnabled: designing,
    async run(arg) {
      const active = selectActiveDocument(store());
      const tool = store().designTool;
      if (!active || !tool || arg?.page === undefined || !arg.rect) return;
      const fields = designOf(active.id)?.fields ?? [];
      const rect = drawnRect(tool, arg.rect, active.pageSize);
      const spec = defaultSpec(tool, newFieldName(tool, fields));
      await applyDesign(active.id, [{ op: "add", page: arg.page, rect, spec }]);
    },
  },
  {
    // A press on a field's widget selects it; elsewhere, nothing is selected.
    id: "formDesign.selectField",
    paletteHidden: true,
    isEnabled: designing,
    run(arg) {
      const field = arg?.field;
      store().updateFormDesign({
        selected: field === undefined ? null : { field, widget: arg?.index ?? 0 },
      });
    },
  },
  {
    // A widget dragged or resized on its page (ADR 0047).
    id: "formDesign.move",
    paletteHidden: true,
    isEnabled: designing,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.field === undefined || arg.page === undefined || !arg.rect) return;
      const widget = arg.index ?? 0;
      await applyDesign(
        active.id,
        [{ op: "setRect", field: arg.field, widget, page: arg.page, rect: arg.rect }],
        widget,
      );
    },
  },
  {
    // The inspector's changes to the selected field: one undo step each.
    id: "formDesign.update",
    paletteHidden: true,
    isEnabled: fieldSelected,
    async run(arg) {
      const chosen = selection();
      if (!chosen || !arg?.spec) return;
      await applyDesign(
        chosen.active.id,
        [{ op: "update", field: chosen.field.id, spec: arg.spec }],
        chosen.widget,
      );
    },
  },
  {
    id: "formDesign.addButton",
    isEnabled: (state) => fieldSelected(state) && selection()?.field.kind === "radio",
    async run() {
      const chosen = selection();
      if (chosen?.field.kind !== "radio") return;
      const place = nextButtonRect(chosen.field, chosen.active.pageSize);
      if (!place) return;
      const exports = new Set(chosen.field.options.map((option) => option.label));
      let n = chosen.field.widgets.length + 1;
      let value = i18n.t("formDesign.choice", { n });
      while (exports.has(value)) {
        n += 1;
        value = i18n.t("formDesign.choice", { n });
      }
      await applyDesign(
        chosen.active.id,
        [{ op: "addButton", field: chosen.field.id, page: place.page, rect: place.rect, value }],
        "last",
      );
    },
  },
  {
    id: "formDesign.removeButton",
    paletteHidden: true,
    isEnabled: (state) =>
      fieldSelected(state) &&
      selection()?.field.kind === "radio" &&
      (selection()?.field.widgets.length ?? 0) > 1,
    async run(arg) {
      const chosen = selection();
      if (!chosen) return;
      const widget = arg?.index ?? chosen.widget;
      await applyDesign(chosen.active.id, [{ op: "removeButton", field: chosen.field.id, widget }]);
    },
  },
  {
    id: "formDesign.delete",
    shortcut: { key: "Delete", alternate: "Backspace", notInFields: true },
    isEnabled: fieldSelected,
    async run() {
      const chosen = selection();
      if (!chosen) return;
      await applyDesign(chosen.active.id, [{ op: "remove", field: chosen.field.id }]);
    },
  },
  { id: "formDesign.tabEarlier", isEnabled: fieldSelected, run: () => stepTabOrder(-1) },
  { id: "formDesign.tabLater", isEnabled: fieldSelected, run: () => stepTabOrder(1) },
  { id: "formDesign.tabRowsLeftToRight", isEnabled: designing, run: () => tabByRows(false) },
  { id: "formDesign.tabRowsRightToLeft", isEnabled: designing, run: () => tabByRows(true) },
  {
    // Fields the current page seems to ask for, for review before any is made.
    id: "formDesign.detect",
    isEnabled: hasDocument,
    async run() {
      if (!designing(store())) await openForms("design");
      const active = selectActiveDocument(store());
      if (!active) return;
      const page = active.page;
      const found = await backend.detectFormFields(active.id, page);
      if (!designOf(active.id)) await loadFormDesign(active.id);
      store().updateFormDesign({
        detected: { page, fields: found.map((field) => ({ ...field, include: true })) },
      });
    },
  },
  {
    id: "formDesign.toggleDetected",
    paletteHidden: true,
    isEnabled: (state) => state.formDesign?.detected != null,
    run(arg) {
      const index = arg?.index;
      if (index === undefined) return;
      store().updateFormDesign((current) =>
        current?.detected
          ? {
              ...current,
              detected: {
                ...current.detected,
                fields: current.detected.fields.map((field, i) =>
                  i === index ? { ...field, include: !field.include } : field,
                ),
              },
            }
          : current,
      );
    },
  },
  {
    id: "formDesign.createDetected",
    isEnabled: (state) =>
      state.formDesign?.detected?.fields.some((field) => field.include) ?? false,
    async run() {
      const active = selectActiveDocument(store());
      const design = active ? designOf(active.id) : null;
      if (!active || !design) return;
      const edits = detectedEdits(design);
      if (edits.length === 0) return;
      await applyDesign(active.id, edits);
      store().updateFormDesign({ detected: null });
      store().setNotice({
        key: "fieldsMade",
        values: { n: formatNumber(edits.length, store().locale) },
      });
    },
  },
  {
    id: "formDesign.cancelDetected",
    isEnabled: (state) => state.formDesign?.detected != null,
    run: () => store().updateFormDesign({ detected: null }),
  },
  ...(
    [
      ["formDesign.moveLeft", "ArrowLeft", -1, 0],
      ["formDesign.moveRight", "ArrowRight", 1, 0],
      ["formDesign.moveUp", "ArrowUp", 0, -1],
      ["formDesign.moveDown", "ArrowDown", 0, 1],
    ] as const
  ).flatMap(([id, key, dx, dy]): Command[] => [
    {
      id,
      shortcut: { key, notInFields: true },
      isEnabled: fieldSelected,
      run: () => nudgeField(dx * NUDGE, dy * NUDGE),
    },
    {
      id: `${id}More` as CommandId,
      shortcut: { key, shift: true, notInFields: true },
      isEnabled: fieldSelected,
      run: () => nudgeField(dx * NUDGE_MORE, dy * NUDGE_MORE),
    },
  ]),
  { id: "sign.openTool", isEnabled: hasDocument, run: () => chooseFill(null) },
  { id: "fill.select", isEnabled: hasDocument, run: () => chooseFill(null) },
  { id: "fill.text", isEnabled: hasDocument, run: () => chooseFill("text") },
  { id: "fill.date", isEnabled: hasDocument, run: () => chooseFill("date") },
  { id: "fill.check", isEnabled: hasDocument, run: () => chooseFill("check") },
  { id: "fill.cross", isEnabled: hasDocument, run: () => chooseFill("cross") },
  { id: "fill.dot", isEnabled: hasDocument, run: () => chooseFill("dot") },
  {
    id: "fill.signature",
    paletteHidden: true,
    isEnabled: (state) => hasDocument(state) && (state.signatures?.length ?? 0) > 0,
    run(arg) {
      const id = arg?.signature ?? store().signatures?.[0]?.id;
      if (id !== undefined) chooseFill("signature", id);
    },
  },
  {
    // A press with the Sign tool places what it places (ADR 0045).
    id: "fill.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      const { page, x, y } = arg;
      const { fillTool, fillSignature, signatures, locale, fillColor } = store();
      const color = rgbOf(fillColor);
      getViewer(active.id)?.clearSelection();
      switch (fillTool) {
        case "text":
          store().setPendingNote({
            documentId: active.id,
            kind: "fillText",
            page,
            x,
            y,
            width: TEXT_BOX_WIDTH,
          });
          return;
        case "date": {
          store().applyEdit(
            active.id,
            await backend.addFillText(active.id, page, x, y, fillDate(locale), undefined, color),
          );
          store().setFill(null);
          return;
        }
        case "check":
        case "cross":
        case "dot": {
          // Centred on the press; the tool stays, to tick several boxes.
          const half = FILL_MARK_SIZE / 2;
          const outcome = await backend.addFillMark(
            active.id,
            page,
            x - half,
            y - half,
            fillTool,
            undefined,
            color,
          );
          store().applyEdit(active.id, outcome);
          return;
        }
        case "certificate":
          await runCommand("certSign.place", { page, x, y });
          return;
        case "signature": {
          const kept = signatures?.find((signature) => signature.id === fillSignature);
          if (!kept) return;
          const width = kept.initials ? INITIALS_WIDTH : SIGNATURE_WIDTH;
          const height = width / kept.aspect;
          const outcome = await backend.placeSignature(
            active.id,
            page,
            x - width / 2,
            y - height / 2,
            width,
            kept.id,
          );
          store().applyEdit(active.id, outcome);
          store().setFill(null);
          return;
        }
        default:
      }
    },
  },
  {
    // A Fill & Sign item dragged on its page, or onto another (`page`) (ADR 0045).
    id: "fill.move",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const item = arg?.annotation;
      if (!item || !arg.rect) return;
      if (arg.page !== undefined && arg.page !== item.page)
        await moveFillToPage(item, arg.page, arg.rect);
      else await placeFill(item, arg.rect);
    },
  },
  {
    id: "fill.toPreviousPage",
    isEnabled: (state) => hasFillSelected(state) && fillPageTarget(state, -1) !== null,
    async run() {
      await stepFillPage(-1);
    },
  },
  {
    id: "fill.toNextPage",
    isEnabled: (state) => hasFillSelected(state) && fillPageTarget(state, 1) !== null,
    async run() {
      await stepFillPage(1);
    },
  },
  ...(
    [
      ["fill.moveLeft", "ArrowLeft", -1, 0],
      ["fill.moveRight", "ArrowRight", 1, 0],
      ["fill.moveUp", "ArrowUp", 0, -1],
      ["fill.moveDown", "ArrowDown", 0, 1],
    ] as const
  ).flatMap(([id, key, dx, dy]): Command[] => [
    {
      id,
      shortcut: { key, notInFields: true },
      isEnabled: nudgesFill,
      run: () => nudgeFill(dx * NUDGE, dy * NUDGE),
    },
    {
      id: `${id}More` as CommandId,
      shortcut: { key, shift: true, notInFields: true },
      isEnabled: nudgesFill,
      run: () => nudgeFill(dx * NUDGE_MORE, dy * NUDGE_MORE),
    },
  ]),
  {
    // The colour new Fill & Sign text and marks take; the selected one is recoloured (ADR 0045).
    id: "fill.color",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const choice = DOCUMENT_COLORS.find((color) => color.id === arg?.colorId);
      if (!choice) return;
      store().setFillColor(choice.id as DocumentColorId);
      const active = selectActiveDocument(store());
      const selected = active?.selectedAnnotation;
      if (!active || !selected || (selected.kind !== "fillText" && selected.kind !== "mark"))
        return;
      const outcome = await backend.setAnnotationColor(
        active.id,
        selected.page,
        selected.index,
        rgbOf(choice.id),
      );
      getViewer(active.id)?.reselectAfterRefresh(selected);
      store().applyEdit(active.id, outcome);
    },
  },
  { id: "fill.larger", isEnabled: hasFillSelected, run: () => resizeFill(FILL_STEP) },
  { id: "fill.smaller", isEnabled: hasFillSelected, run: () => resizeFill(1 / FILL_STEP) },
  { id: "signatures.load", paletteHidden: true, run: loadSignatures },
  { id: "signature.new", run: () => store().setSignatureDialog({ initials: false }) },
  { id: "signature.newInitials", run: () => store().setSignatureDialog({ initials: true }) },
  {
    id: "signature.cancel",
    paletteHidden: true,
    isEnabled: (state) => state.signatureDialog !== null,
    run: () => store().setSignatureDialog(null),
  },
  {
    id: "signature.saveDrawn",
    paletteHidden: true,
    isEnabled: (state) => state.signatureDialog !== null,
    async run(arg) {
      const dialog = store().signatureDialog;
      if (!dialog || !arg?.strokes?.length) return;
      keepSignature(await backend.saveDrawnSignature(dialog.initials, arg.strokes, SIGNATURE_PEN));
    },
  },
  {
    id: "signature.saveTyped",
    paletteHidden: true,
    isEnabled: (state) => state.signatureDialog !== null,
    async run(arg) {
      const dialog = store().signatureDialog;
      const text = arg?.text?.trim() ?? "";
      if (!dialog || text === "") return;
      keepSignature(
        await backend.saveTypedSignature(dialog.initials, text, arg?.family ?? "naskh"),
      );
    },
  },
  {
    id: "signature.importImage",
    paletteHidden: true,
    isEnabled: (state) => state.signatureDialog !== null,
    async run() {
      const dialog = store().signatureDialog;
      if (!dialog) return;
      const kept = await backend.importSignatureImage(dialog.initials);
      if (kept) keepSignature(kept);
    },
  },
  {
    id: "signature.delete",
    paletteHidden: true,
    async run(arg) {
      if (arg?.signature === undefined) return;
      await backend.deleteSignature(arg.signature);
      await loadSignatures();
    },
  },
  { id: "comment.note", isEnabled: hasDocument, run: () => chooseCommentTool("note") },
  { id: "comment.textBox", isEnabled: hasDocument, run: () => chooseCommentTool("textBox") },
  { id: "comment.pen", isEnabled: hasDocument, run: () => chooseCommentTool("pen") },
  { id: "comment.eraser", isEnabled: hasDocument, run: () => chooseCommentTool("eraser") },
  {
    // A press with the note tool: the note is added once its text is saved.
    id: "note.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "note",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: NOTE_SIZE,
      });
    },
  },
  {
    // A press with the text box tool: the box is added once its text is saved.
    id: "textBox.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "textBox",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: arg.width ?? TEXT_BOX_WIDTH,
      });
    },
  },
  {
    // The Edit tool: a press places new page text there (ADR 0021).
    id: "text.place",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.x === undefined || arg.y === undefined) return;
      getViewer(active.id)?.clearSelection();
      store().setPendingNote({
        documentId: active.id,
        kind: "pageText",
        page: arg.page,
        x: arg.x,
        y: arg.y,
        width: arg.width ?? TEXT_BOX_WIDTH,
      });
    },
  },
  {
    // The Edit tool pressed a paragraph of the page's own text (ADR 0023).
    id: "text.editBlock",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.block) return;
      getViewer(active.id)?.clearSelection();
      const { block } = arg;
      store().setPendingNote({
        documentId: active.id,
        kind: "block",
        page: arg.page,
        x: block.x,
        y: block.y,
        width: block.width,
        height: block.height,
        block,
      });
    },
  },
  {
    id: "text.bold",
    isEnabled: isEditingBlock,
    shortcut: { key: "b", mod: true },
    run() {
      formatBlock("bold");
    },
  },
  {
    id: "text.italic",
    isEnabled: isEditingBlock,
    shortcut: { key: "i", mod: true },
    run() {
      formatBlock("italic");
    },
  },
  {
    id: "text.setStyle",
    paletteHidden: true,
    run(arg) {
      if (arg?.textStyle) store().setTextStyle(arg.textStyle);
    },
  },
  {
    id: "note.save",
    paletteHidden: true,
    isEnabled: isWritingNote,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      const text = arg?.text ?? "";
      const pending = store().pendingNote;
      if (pending && pending.documentId === active.id) {
        store().setPendingNote(null);
        if (pending.kind === "block" && pending.block) {
          // Unchanged text and looks are no edit; emptied text removes the paragraph (undoable).
          const { block } = pending;
          const marks = arg?.blockMarks;
          const restyled = marks !== undefined && !sameMarks(text, marks, marksOfRuns(block.runs));
          if (text === block.text && !restyled && !arg?.blockFamily) return;
          const done = await backend.editTextBlock(active.id, {
            page: pending.page,
            index: block.index,
            fingerprint: block.fingerprint,
            text,
            ...(arg?.blockFamily ? { family: arg.blockFamily } : {}),
            ...(marks ? { marks } : {}),
          });
          store().applyEdit(active.id, done);
          if (done.substituted) {
            // Font names come from the document: isolated, as <bdi> would.
            const isolate = (name: string) => `\u2068${name}\u2069`;
            const { from, font } = done.substituted;
            store().setNotice({
              key: from ? "fontSubstituted" : "fontSubstitutedUnnamed",
              values: { from: isolate(from), to: isolate(font) },
            });
          }
          return;
        }
        // An empty new note or text box is none.
        if (text.trim() === "") return;
        const { page, x, y, width } = pending;
        const outcome =
          pending.kind === "pageText"
            ? await backend.addText(active.id, page, x, y, width, text, store().textStyle)
            : pending.kind === "fillText"
              ? await backend.addFillText(
                  active.id,
                  page,
                  x,
                  y,
                  text,
                  undefined,
                  rgbOf(store().fillColor),
                )
              : pending.kind === "textBox"
                ? await backend.addTextBox(active.id, page, x, y, width, text)
                : await backend.addNote(active.id, page, x, y, text);
        store().applyEdit(active.id, outcome);
        return;
      }
      const note = active.selectedAnnotation;
      if (note?.kind !== "note" && note?.kind !== "textBox" && note?.kind !== "fillText") return;
      getViewer(active.id)?.clearSelection();
      if ((note.contents ?? "") === text) return;
      // A text box (or Fill & Sign text) emptied of its text goes.
      const outcome =
        note.kind !== "note" && text.trim() === ""
          ? await backend.removeAnnotations(active.id, note.page, [note.index])
          : await backend.setAnnotationContents(active.id, note.page, note.index, text);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "note.cancel",
    paletteHidden: true,
    isEnabled: isWritingNote,
    run() {
      store().setPendingNote(null);
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.clearSelection();
    },
  },
  {
    id: "ink.add",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const page = arg?.page;
      if (!active || page === undefined || !arg?.strokes?.length) return;
      try {
        store().applyEdit(active.id, await backend.addInk(active.id, page, arg.strokes));
      } catch (error) {
        // Not added: the stroke on screen must not stay as if it were.
        getViewer(active.id)?.clearInkPreview(page);
        throw error;
      }
    },
  },
  {
    id: "ink.erase",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.indices?.length) return;
      store().applyEdit(
        active.id,
        await backend.removeAnnotations(active.id, arg.page, arg.indices),
      );
    },
  },
  {
    id: "edit.selectAll",
    shortcut: { key: "a", mod: true, notInFields: true },
    isEnabled: (state) => hasDocument(state) && state.activeTool === "select",
    run() {
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.selectAllOnPage();
    },
  },
  {
    id: "edit.deselect",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: (state) => hasSelection(state) || hasAnnotationSelected(state),
    run() {
      const active = selectActiveDocument(store());
      if (active) getViewer(active.id)?.clearSelection();
    },
  },
  {
    id: "view.zoomIn",
    shortcut: { key: "=", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, 1) });
      }
    },
  },
  {
    id: "view.zoomOut",
    shortcut: { key: "-", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) {
        store().updateDocument(active.id, { zoomMode: "custom", zoom: stepZoom(active.zoom, -1) });
      }
    },
  },
  {
    id: "view.actualSize",
    shortcut: { key: "1", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "custom", zoom: 1 });
    },
  },
  {
    id: "view.fitWidth",
    shortcut: { key: "2", mod: true },
    isEnabled: hasDocument,
    run() {
      const active = selectActiveDocument(store());
      if (active) store().updateDocument(active.id, { zoomMode: "fit-width" });
    },
  },
  {
    id: "view.nextPage",
    shortcut: { key: "PageDown" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && stepTarget(active, 1) !== null;
    },
    run() {
      const active = selectActiveDocument(store());
      const target = active && stepTarget(active, 1);
      if (active && target !== null && target !== undefined) navigateTo(active, target);
    },
  },
  {
    id: "view.previousPage",
    shortcut: { key: "PageUp" },
    isEnabled: (state) => {
      const active = selectActiveDocument(state);
      return active !== null && stepTarget(active, -1) !== null;
    },
    run() {
      const active = selectActiveDocument(store());
      const target = active && stepTarget(active, -1);
      if (active && target !== null && target !== undefined) navigateTo(active, target);
    },
  },
  {
    // With a page argument (the status bar field, the outline, thumbnails),
    // goes there; from the shortcut, puts the cursor in that field.
    id: "view.goToPage",
    shortcut: { key: "n", mod: true, shift: true },
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      if (arg?.page !== undefined && Number.isFinite(arg.page)) {
        navigateTo(active, arg.page);
        return;
      }
      const field = document.getElementById(PAGE_NUMBER_INPUT_ID);
      if (field instanceof HTMLInputElement) {
        field.focus();
        field.select();
      }
    },
  },
  { id: "view.modeContinuous", isEnabled: hasDocument, run: () => setViewMode("continuous") },
  { id: "view.modeSingle", isEnabled: hasDocument, run: () => setViewMode("single") },
  { id: "view.modeSpread", isEnabled: hasDocument, run: () => setViewMode("spread") },
  { id: "view.modeBook", isEnabled: hasDocument, run: () => setViewMode("book") },
  { id: "view.pagesLeftToRight", isEnabled: hasDocument, run: () => setPageOrder("ltr") },
  { id: "view.pagesRightToLeft", isEnabled: hasDocument, run: () => setPageOrder("rtl") },
  {
    id: "view.toggleLeftPanel",
    // Acrobat's key for the navigation pane.
    shortcut: { key: "F4" },
    run() {
      store().setLeftPanel(store().leftPanel ? null : "thumbnails");
    },
  },
  { id: "view.showThumbnails", run: () => showPanel("thumbnails") },
  { id: "view.showOutline", run: () => showPanel("outline") },
  { id: "view.showComments", run: () => showPanel("comments") },
  {
    // From the comments panel: the comment on its page, selected (ADR 0041).
    id: "comment.show",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      const annotation = arg?.annotation;
      if (!active || !annotation) return;
      // The organizer shows no page view: back to the pages.
      if (store().activeTool === "pages") store().setActiveTool("select");
      store().updateDocument(active.id, { page: annotation.page });
      getViewer(active.id)?.showAnnotation(annotation);
    },
  },
  {
    id: "comment.reply",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const text = arg?.text ?? "";
      if (!active || arg?.page === undefined || arg.index === undefined || text.trim() === "") {
        return;
      }
      const outcome = await backend.replyToComment(active.id, arg.page, arg.index, text);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "comment.resolve",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || arg.index === undefined) return;
      const outcome = await backend.setCommentResolved(
        active.id,
        arg.page,
        arg.index,
        arg.resolved ?? true,
      );
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "comments.filter",
    paletteHidden: true,
    run(arg) {
      if (arg?.filter) store().setCommentFilter(arg.filter);
    },
  },
  {
    id: "comments.sort",
    paletteHidden: true,
    run(arg) {
      if (arg?.sort) store().setCommentSort(arg.sort);
    },
  },
  {
    // The name written on new comments; the shell reads the stored setting.
    id: "comments.setAuthor",
    paletteHidden: true,
    run(arg) {
      store().setCommentAuthor(arg?.text ?? "");
    },
  },
  { id: "view.readingNormal", run: () => setReadingTheme("normal") },
  { id: "view.readingSepia", run: () => setReadingTheme("sepia") },
  { id: "view.readingNight", run: () => setReadingTheme("night") },
  // Digits of the Arabic UI's own numbers (owner decision, 2026-11-23).
  { id: "app.digitsArabicIndic", run: () => store().setUiDigits("arabicIndic") },
  { id: "app.digitsWestern", run: () => store().setUiDigits("western") },
  {
    id: "search.focus",
    shortcut: { key: "f", mod: true },
    isEnabled: hasDocument,
    run() {
      showPanel("search");
      // The field mounts with the panel; focus it once it exists.
      requestAnimationFrame(() => {
        const field = document.getElementById(SEARCH_INPUT_ID);
        if (field instanceof HTMLInputElement) {
          field.focus();
          field.select();
        }
      });
    },
  },
  {
    id: "search.run",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.query !== undefined) await runSearch(active, arg.query);
    },
  },
  {
    id: "search.next",
    shortcut: { key: "g", mod: true },
    isEnabled: hasHits,
    run: () => stepHit(1),
  },
  {
    id: "search.previous",
    shortcut: { key: "g", mod: true, shift: true },
    isEnabled: hasHits,
    run: () => stepHit(-1),
  },
  {
    id: "search.goTo",
    paletteHidden: true,
    isEnabled: hasHits,
    run(arg) {
      const active = selectActiveDocument(store());
      if (active && arg?.index !== undefined) goToHit(active, arg.index);
    },
  },
  { id: "search.toggleCase", run: () => toggleSearchOption("caseSensitive") },
  { id: "search.toggleWholeWord", run: () => toggleSearchOption("wholeWord") },
  { id: "search.toggleRegex", run: () => toggleSearchOption("regex") },
  { id: "search.toggleArabic", run: () => toggleSearchOption("arabicNormalize") },
  { id: "search.toggleTaMarbuta", run: () => toggleSearchOption("taMarbutaAsHa") },
  {
    id: "app.toggleTheme",
    run() {
      store().setTheme(resolvedTheme(store().theme) === "dark" ? "light" : "dark");
    },
  },
  {
    id: "app.toggleLanguage",
    run() {
      store().setLocale(store().locale === "ar" ? "en" : "ar");
    },
  },
  {
    id: "app.toggleGallery",
    // Not Shift+Cmd+G: that is "previous search result" everywhere.
    shortcut: { key: "d", mod: true, shift: true },
    run() {
      store().setGalleryOpen(!store().galleryOpen);
    },
  },
  {
    id: "app.commandPalette",
    shortcut: { key: "k", mod: true },
    run() {
      store().setPaletteOpen(!store().paletteOpen);
    },
  },
  {
    id: "app.settings",
    shortcut: { key: ",", mod: true },
    async run() {
      store().setSettingsOpen(true);
      await refreshOcrModels();
    },
  },
  {
    id: "app.closeSettings",
    paletteHidden: true,
    isEnabled: (state) => state.settingsOpen,
    run: () => store().setSettingsOpen(false),
  },
  {
    // The first use of OCR without its model asks to download it (spec §8);
    // with it, the whole document is read in the background (ADR 0035).
    id: "ocr.recognize",
    isEnabled: hasDocument,
    run: () => recognize(),
  },
  {
    id: "ocr.recognizePage",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await recognize([active.page]);
    },
  },
  {
    id: "ocr.pauseJob",
    paletteHidden: true,
    run: (arg) => controlJob(arg, "pause"),
  },
  {
    id: "ocr.resumeJob",
    paletteHidden: true,
    run: (arg) => controlJob(arg, "resume"),
  },
  {
    id: "ocr.cancelJob",
    paletteHidden: true,
    run: (arg) => controlJob(arg, "cancel"),
  },
  {
    id: "ocr.removeJob",
    paletteHidden: true,
    async run(arg) {
      if (arg?.jobId === undefined) return;
      await backend.removeOcrJob(arg.jobId);
      ocrJobs().forgetJob(arg.jobId);
      const active = selectActiveDocument(store());
      if (active) ocrJobs().setReview(active.id, await backend.ocrReview(active.id));
    },
  },
  {
    // The pages-needing-review list (spec §5.8), in the OCR panel.
    id: "ocr.review",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      store().setActiveTool("ocr");
      await refreshOcrJobs(active.id);
    },
  },
  {
    id: "ocr.showWord",
    paletteHidden: true,
    run(arg) {
      if (arg?.reviewAt) showFlag(arg.reviewAt);
    },
  },
  {
    id: "ocr.nextFlag",
    shortcut: { key: "F8" },
    isEnabled: hasFlags,
    run: () => stepFlag(1),
  },
  {
    id: "ocr.previousFlag",
    shortcut: { key: "F8", shift: true },
    isEnabled: hasFlags,
    run: () => stepFlag(-1),
  },
  {
    // Review mode (ADR 0037): the card steps through what is still open.
    id: "ocr.startReview",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      store().setActiveTool("ocr");
      ocrJobs().setDrawingRegion(false);
      await refreshOcrJobs(active.id);
      ocrJobs().setReviewing(true);
      const pages = ocrJobs().review[active.id] ?? [];
      showItem(nextItem(reviewQueue(pages), null, pages));
    },
  },
  {
    id: "ocr.stopReview",
    isEnabled: () => useOcrJobs.getState().reviewing,
    run: () => ocrJobs().setReviewing(false),
  },
  {
    // Enter in the card: the text as recognised is right, or as typed.
    id: "ocr.reviewConfirm",
    paletteHidden: true,
    isEnabled: reviewingItem,
    async run(arg) {
      const word = reviewedWord();
      if (!word) return;
      const text = arg?.reviewText;
      await decide(
        text !== undefined && text.trim() !== word.text
          ? { kind: "edit", text }
          : { kind: "accept" },
      );
    },
  },
  {
    // One of the other readings, chosen.
    id: "ocr.reviewChoose",
    paletteHidden: true,
    isEnabled: reviewingItem,
    async run(arg) {
      if (arg?.reviewText !== undefined) await decide({ kind: "edit", text: arg.reviewText });
    },
  },
  {
    id: "ocr.reviewCheck",
    shortcut: { key: "Enter", mod: true },
    isEnabled: (state) => reviewingItem(state) && useOcrJobs.getState().focus?.word !== null,
    run: () => decide({ kind: "check" }),
  },
  {
    id: "ocr.reviewCheckPage",
    shortcut: { key: "Enter", mod: true, shift: true },
    isEnabled: reviewingItem,
    run: () => decide({ kind: "checkPage" }),
  },
  {
    // Every number, date and ID on the list too, not only the flagged ones.
    id: "ocr.reviewNumbers",
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      if (!active) return;
      ocrJobs().setNumbers(arg?.numbers ?? !ocrJobs().numbers);
      await refreshReview(active.id);
    },
  },
  {
    // An area read again (ADR 0038): drawn on the page first.
    id: "ocr.drawRegion",
    isEnabled: hasDocument,
    run() {
      const drawing = !ocrJobs().drawingRegion;
      store().setActiveTool("ocr");
      ocrJobs().setReviewing(false);
      ocrJobs().setDrawingRegion(drawing);
      if (drawing) ocrJobs().setRegion(null);
    },
  },
  {
    id: "ocr.setRegion",
    paletteHidden: true,
    isEnabled: hasDocument,
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.rect) return;
      if (arg.rect.width < 2 || arg.rect.height < 2) return;
      ocrJobs().setRegion({ documentId: active.id, page: arg.page, rect: arg.rect });
      ocrJobs().setDrawingRegion(false);
    },
  },
  {
    id: "ocr.setRegionOptions",
    paletteHidden: true,
    run(arg) {
      ocrJobs().setRegionOptions({ language: arg?.language, numbers: arg?.numbers });
    },
  },
  {
    id: "ocr.readRegion",
    isEnabled: hasRegion,
    async run() {
      const region = ocrJobs().region;
      if (!region || !(await modelsReady())) return;
      const { x, y, width, height } = region.rect;
      const { regionLanguage, regionNumbers } = ocrJobs();
      ocrJobs().setJob(
        await backend.startOcrRegion(
          region.documentId,
          region.page,
          [x, y, x + width, y + height],
          regionLanguage,
          regionNumbers,
        ),
      );
      ocrJobs().setRegion(null);
    },
  },
  {
    id: "ocr.cancelRegion",
    isEnabled: () => useOcrJobs.getState().drawingRegion || useOcrJobs.getState().region !== null,
    run() {
      ocrJobs().setRegion(null);
      ocrJobs().setDrawingRegion(false);
    },
  },
  {
    // Another tool's text layers checked against a quick draft (ADR 0038); no models needed.
    id: "ocr.checkLayers",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      store().setActiveTool("ocr");
      ocrJobs().setJob(await backend.checkTextLayers(active.id));
    },
  },
  {
    // The pages found poor (or `pages`) read again, their old layer removed.
    id: "ocr.replaceLayers",
    isEnabled: (state) => poorPages(state).length > 0,
    async run(arg) {
      const pages = arg?.pages ?? poorPages(store());
      if (pages.length > 0) await recognize(pages, true);
    },
  },
  {
    id: "ocr.setConvertOptions",
    paletteHidden: true,
    run(arg) {
      ocrJobs().setConvertOptions({ keepScan: arg?.keepScan, family: arg?.convertFamily });
    },
  },
  {
    // Scan to editable text (ADR 0039): this page, or every recognised page.
    id: "ocr.convertPage",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (active) await convert([active.page]);
    },
  },
  {
    id: "ocr.convertDocument",
    isEnabled: hasDocument,
    run: () => convert(null),
  },
  {
    // The first-use prompt's answer: whatever OCR still needs, shown downloading in Settings.
    id: "ocrModels.downloadRequired",
    isEnabled: () => {
      const { models, jobs } = ocr();
      return models === null || missingRequired(models).some((pkg) => !jobs[pkg.id]);
    },
    async run() {
      ocr().setPrompt(false);
      if (!ocr().models) await refreshOcrModels();
      const missing = missingRequired(ocr().models).filter((pkg) => !ocr().jobs[pkg.id]);
      if (missing.length === 0) return;
      store().setSettingsOpen(true);
      for (const pkg of missing) await downloadOcrModel(pkg.id);
    },
  },
  {
    id: "ocrModels.dismissPrompt",
    paletteHidden: true,
    run: () => ocr().setPrompt(false),
  },
  {
    id: "ocrModels.download",
    paletteHidden: true,
    async run(arg) {
      if (arg?.modelId) await downloadOcrModel(arg.modelId);
    },
  },
  {
    id: "ocrModels.cancel",
    paletteHidden: true,
    async run(arg) {
      if (arg?.modelId) await backend.cancelOcrModelDownload(arg.modelId);
    },
  },
  {
    id: "ocrModels.delete",
    paletteHidden: true,
    async run(arg) {
      if (!arg?.modelId) return;
      await backend.deleteOcrModel(arg.modelId);
      await refreshOcrModels();
    },
  },
  // ---- The Edit tool's modes, page objects and links (ADR 0051).
  { id: "edit.textMode", isEnabled: hasDocument, run: () => chooseEditMode("text") },
  { id: "edit.objectsMode", isEnabled: hasDocument, run: () => chooseEditMode("objects") },
  { id: "edit.linksMode", isEnabled: hasDocument, run: () => chooseEditMode("links") },
  {
    // The viewer selected an object, or nothing.
    id: "object.select",
    paletteHidden: true,
    isEnabled: (state) => editingIn(state, "objects"),
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined) return;
      store().setSelectedObject(
        arg.object ? { documentId: active.id, page: arg.page, object: arg.object } : null,
      );
    },
  },
  {
    // The viewer dragged or resized the selected object to `rect`.
    id: "object.reshape",
    paletteHidden: true,
    isEnabled: objectChangeable,
    async run(arg) {
      const selected = store().selectedObject;
      if (!selected || !arg?.rect || (arg.object && arg.object.index !== selected.object.index))
        return;
      await editObject({ kind: "transform", matrix: mapBetween(selected.object.rect, arg.rect) });
    },
  },
  {
    id: "object.insertImage",
    isEnabled: hasDocument,
    async run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      chooseEditMode("objects");
      const outcome = await backend.insertImage(active.id, active.page, null);
      if (outcome) afterObjectEdit(active.id, active.page, outcome);
    },
  },
  {
    id: "object.replaceImage",
    isEnabled: (state) => objectChangeable(state) && state.selectedObject?.object.kind === "image",
    async run() {
      const active = selectActiveDocument(store());
      const selected = store().selectedObject;
      if (!active || !selected) return;
      const outcome = await backend.replaceImage(active.id, selected.page, {
        index: selected.object.index,
        fingerprint: selected.object.fingerprint,
      });
      if (outcome) afterObjectEdit(active.id, selected.page, outcome);
    },
  },
  {
    id: "object.delete",
    shortcut: { key: "Delete", alternate: "Backspace", notInFields: true },
    isEnabled: objectChangeable,
    run: () => editObject({ kind: "delete" }),
  },
  {
    id: "object.rotateClockwise",
    isEnabled: objectChangeable,
    run() {
      const selected = store().selectedObject;
      if (selected)
        return editObject({ kind: "transform", matrix: quarterTurn(selected.object.rect, true) });
    },
  },
  {
    id: "object.rotateCounterclockwise",
    isEnabled: objectChangeable,
    run() {
      const selected = store().selectedObject;
      if (selected)
        return editObject({ kind: "transform", matrix: quarterTurn(selected.object.rect, false) });
    },
  },
  {
    id: "object.bringToFront",
    shortcut: { key: "]", mod: true, shift: true },
    isEnabled: (state) => hasOverlapping(state, 1),
    run: () => editObject({ kind: "arrange", to: "front" }),
  },
  {
    id: "object.bringForward",
    shortcut: { key: "]", mod: true },
    isEnabled: (state) => hasOverlapping(state, 1),
    run: () => editObject({ kind: "arrange", to: "forward" }),
  },
  {
    id: "object.sendBackward",
    shortcut: { key: "[", mod: true },
    isEnabled: (state) => hasOverlapping(state, -1),
    run: () => editObject({ kind: "arrange", to: "backward" }),
  },
  {
    id: "object.sendToBack",
    shortcut: { key: "[", mod: true, shift: true },
    isEnabled: (state) => hasOverlapping(state, -1),
    run: () => editObject({ kind: "arrange", to: "back" }),
  },
  {
    // Shows the crop handles on the selected object; Enter applies, Escape cancels.
    id: "object.crop",
    isEnabled: croppable,
    run() {
      const selected = store().selectedObject;
      if (selected) store().setObjectCrop(selected.object.rect);
    },
  },
  {
    id: "object.setCrop",
    paletteHidden: true,
    isEnabled: isCropping,
    run(arg) {
      if (arg?.rect) store().setObjectCrop(arg.rect);
    },
  },
  {
    id: "object.applyCrop",
    shortcut: { key: "Enter", notInFields: true },
    isEnabled: isCropping,
    async run() {
      const crop = store().objectCrop;
      if (!crop) return;
      store().setObjectCrop(null);
      await editObject({ kind: "crop", rect: crop });
    },
  },
  {
    id: "object.cancelCrop",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: isCropping,
    run: () => store().setObjectCrop(null),
  },
  {
    id: "object.uncrop",
    isEnabled: (state) => objectChangeable(state) && state.selectedObject?.object.cropped === true,
    run: () => editObject({ kind: "crop", rect: null }),
  },
  {
    id: "object.deselect",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: (state) => objectSelected(state) && state.objectCrop === null,
    run: () => store().setSelectedObject(null),
  },
  ...(
    [
      ["object.moveLeft", "ArrowLeft", -1, 0],
      ["object.moveRight", "ArrowRight", 1, 0],
      ["object.moveUp", "ArrowUp", 0, -1],
      ["object.moveDown", "ArrowDown", 0, 1],
    ] as const
  ).flatMap(([id, key, dx, dy]): Command[] => [
    {
      id,
      shortcut: { key, notInFields: true },
      isEnabled: nudgesObject,
      run: () => nudgeObject(dx * NUDGE, dy * NUDGE),
    },
    {
      id: `${id}More` as CommandId,
      shortcut: { key, shift: true, notInFields: true },
      isEnabled: nudgesObject,
      run: () => nudgeObject(dx * NUDGE_MORE, dy * NUDGE_MORE),
    },
  ]),
  {
    // A link over the selected text: the link mode opens with it, to say where it goes.
    id: "link.fromSelection",
    isEnabled: hasSelection,
    run() {
      const active = selectActiveDocument(store());
      if (!active) return;
      const viewer = getViewer(active.id);
      const first = viewer?.selectionMarks()[0];
      if (!first || first.rects.length === 0) return;
      viewer?.clearSelection();
      chooseEditMode("links");
      store().setLinkDraft({
        documentId: active.id,
        page: first.page,
        index: null,
        rects: first.rects,
        target: { kind: "url", url: "" },
      });
      void loadDestinations(active.id);
    },
  },
  {
    // The viewer drew a rectangle for a new link.
    id: "link.draw",
    paletteHidden: true,
    isEnabled: (state) => editingIn(state, "links"),
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined || !arg.rect) return;
      store().setLinkDraft({
        documentId: active.id,
        page: arg.page,
        index: null,
        rects: [arg.rect],
        target: { kind: "url", url: "" },
      });
      void loadDestinations(active.id);
    },
  },
  {
    // The viewer selected a link (to edit it), or nothing.
    id: "link.select",
    paletteHidden: true,
    isEnabled: (state) => editingIn(state, "links"),
    run(arg) {
      const active = selectActiveDocument(store());
      if (!active || arg?.page === undefined) return;
      const link = arg.link;
      store().setLinkDraft(
        link
          ? {
              documentId: active.id,
              page: arg.page,
              index: link.index,
              rects: link.rects,
              target: link.target,
            }
          : null,
      );
      if (link) void loadDestinations(active.id);
    },
  },
  {
    id: "link.save",
    paletteHidden: true,
    isEnabled: linkDrafted,
    async run(arg) {
      if (arg?.linkTarget) await saveLink(arg.linkTarget);
    },
  },
  {
    // The viewer dragged or resized the selected link to `rect`.
    id: "link.reshape",
    paletteHidden: true,
    isEnabled: linkSelected,
    async run(arg) {
      const active = selectActiveDocument(store());
      const draft = store().linkDraft;
      if (!active || !draft || draft.index === null || !arg?.rect) return;
      const outcome = await backend.editLink(active.id, {
        kind: "setRect",
        page: draft.page,
        index: draft.index,
        rect: arg.rect,
      });
      store().applyEdit(active.id, outcome);
      // Still the link being edited, where it went.
      store().setLinkDraft({ ...draft, rects: [arg.rect] });
    },
  },
  {
    id: "link.delete",
    shortcut: { key: "Delete", alternate: "Backspace", notInFields: true },
    isEnabled: linkSelected,
    async run() {
      const active = selectActiveDocument(store());
      const draft = store().linkDraft;
      if (!active || !draft || draft.index === null) return;
      const outcome = await backend.editLink(active.id, {
        kind: "remove",
        page: draft.page,
        index: draft.index,
      });
      store().setLinkDraft(null);
      store().applyEdit(active.id, outcome);
    },
  },
  {
    id: "link.cancel",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: linkDrafted,
    run: () => store().setLinkDraft(null),
  },
  {
    // The pointer came over a link, or left it: its target is shown (ADR 0051).
    id: "link.hover",
    paletteHidden: true,
    run(arg) {
      const active = selectActiveDocument(store());
      const tip = store().linkTip;
      if (!active || tip?.pinned) return;
      store().setLinkTip(
        arg?.link && arg.page !== undefined
          ? { documentId: active.id, page: arg.page, link: arg.link, pinned: false }
          : null,
      );
    },
  },
  {
    // A link was pressed: a page of this document is shown at once; a web
    // address only once the user, having seen it, chooses to open it.
    id: "link.follow",
    paletteHidden: true,
    isEnabled: hasDocument,
    async run(arg) {
      const active = selectActiveDocument(store());
      const link = arg?.link;
      if (!active || !link || arg.page === undefined) return;
      const target = link.target;
      if (target.kind === "page") {
        store().setLinkTip(null);
        navigateTo(active, target.page);
      } else if (target.kind === "named" && target.page !== undefined && target.page !== null) {
        store().setLinkTip(null);
        navigateTo(active, target.page);
      } else {
        store().setLinkTip({ documentId: active.id, page: arg.page, link, pinned: true });
      }
    },
  },
  {
    // From the link tip's own button, after the address was shown.
    id: "link.open",
    paletteHidden: true,
    isEnabled: (state) => state.linkTip?.pinned === true && isOpenable(state.linkTip.link.target),
    async run() {
      const tip = store().linkTip;
      if (!tip || !isOpenable(tip.link.target)) return;
      store().setLinkTip(null);
      await backend.openLink(tip.link.target.url);
    },
  },
  {
    id: "link.dismiss",
    shortcut: { key: "Escape", notInFields: true },
    isEnabled: (state) => state.linkTip?.pinned === true,
    run: () => store().setLinkTip(null),
  },
];

/**
 * What a file's permissions switch off (ADR 0043): a viewer honours the flags
 * of a file opened with its user password. The owner password lifts them.
 */
const PERMISSION_GATES: Partial<Record<CommandId, keyof Permissions>> = {
  "file.print": "print",
  "edit.copy": "copy",
  "pages.rotateClockwise": "assemble",
  "pages.rotateCounterclockwise": "assemble",
  "pages.delete": "assemble",
  "pages.duplicate": "assemble",
  "pages.insertBlank": "assemble",
  "pages.moveEarlier": "assemble",
  "pages.moveLater": "assemble",
  "pages.move": "assemble",
  "pages.insertFromFile": "assemble",
  "pages.extract": "assemble",
  "text.place": "modify",
  "text.editBlock": "modify",
  "redact.apply": "modify",
  "sanitize.run": "modify",
  "markup.highlight": "annotate",
  "markup.underline": "annotate",
  "markup.strikeOut": "annotate",
  "markup.squiggly": "annotate",
  "comment.note": "annotate",
  "comment.textBox": "annotate",
  "comment.pen": "annotate",
  "annotation.delete": "annotate",
  // Output (ADR 0050): stamps change pages; exports take their content out.
  "pages.headerFooter": "modify",
  "pages.pageNumbers": "modify",
  "pages.watermark": "modify",
  "output.stamp": "modify",
  "file.exportImages": "copy",
  "output.exportImages": "copy",
  "file.exportText": "copy",
  "output.exportText": "copy",
  // Form filling and Fill & Sign (ADR 0045).
  "form.setValue": "fillForms",
  // Signing fills in a signature field (ISO 32000-2, table 22, bit 9).
  "certSign.open": "fillForms",
  "certSign.apply": "fillForms",
  "certSign.place": "fillForms",
  // Making form fields (ADR 0047): ISO 32000-2 asks for "modify" as well as "annotate".
  "formDesign.text": "modify",
  "formDesign.checkbox": "modify",
  "formDesign.radio": "modify",
  "formDesign.combo": "modify",
  "formDesign.list": "modify",
  "formDesign.button": "modify",
  "formDesign.draw": "modify",
  "formDesign.move": "modify",
  "formDesign.update": "modify",
  "formDesign.addButton": "modify",
  "formDesign.removeButton": "modify",
  "formDesign.delete": "modify",
  "formDesign.tabEarlier": "modify",
  "formDesign.tabLater": "modify",
  "formDesign.tabRowsLeftToRight": "modify",
  "formDesign.tabRowsRightToLeft": "modify",
  "formDesign.createDetected": "modify",
  "formDesign.moveLeft": "modify",
  "formDesign.moveRight": "modify",
  "formDesign.moveUp": "modify",
  "formDesign.moveDown": "modify",
  "formDesign.moveLeftMore": "modify",
  "formDesign.moveRightMore": "modify",
  "formDesign.moveUpMore": "modify",
  "formDesign.moveDownMore": "modify",
  "fill.text": "annotate",
  "fill.date": "annotate",
  "fill.check": "annotate",
  "fill.cross": "annotate",
  "fill.dot": "annotate",
  "fill.signature": "annotate",
  "fill.place": "annotate",
  "fill.color": "annotate",
  "fill.move": "annotate",
  "fill.toPreviousPage": "annotate",
  "fill.toNextPage": "annotate",
  "fill.moveLeft": "annotate",
  "fill.moveRight": "annotate",
  "fill.moveUp": "annotate",
  "fill.moveDown": "annotate",
  "fill.moveLeftMore": "annotate",
  "fill.moveRightMore": "annotate",
  "fill.moveUpMore": "annotate",
  "fill.moveDownMore": "annotate",
  "fill.larger": "annotate",
  "fill.smaller": "annotate",
  // Page objects change the page's content; links are annotations (ADR 0051).
  "object.reshape": "modify",
  "object.insertImage": "modify",
  "object.replaceImage": "modify",
  "object.delete": "modify",
  "object.rotateClockwise": "modify",
  "object.rotateCounterclockwise": "modify",
  "object.bringToFront": "modify",
  "object.bringForward": "modify",
  "object.sendBackward": "modify",
  "object.sendToBack": "modify",
  "object.crop": "modify",
  "object.applyCrop": "modify",
  "object.uncrop": "modify",
  "object.moveLeft": "modify",
  "object.moveRight": "modify",
  "object.moveUp": "modify",
  "object.moveDown": "modify",
  "object.moveLeftMore": "modify",
  "object.moveRightMore": "modify",
  "object.moveUpMore": "modify",
  "object.moveDownMore": "modify",
  "link.fromSelection": "annotate",
  "link.draw": "annotate",
  "link.save": "annotate",
  "link.reshape": "annotate",
  "link.delete": "annotate",
};

/** Whether the active document's protection lets its reader do what `flag` names. */
export function permits(state: AppState, flag: keyof Permissions): boolean {
  const security = selectActiveDocument(state)?.security;
  return !security || security.owner || security.permissions[flag];
}

const COMMANDS: Command[] = DECLARED.map((command) => {
  const flag = PERMISSION_GATES[command.id];
  if (!flag) return command;
  const own = command.isEnabled;
  return { ...command, isEnabled: (state) => permits(state, flag) && (own?.(state) ?? true) };
});

const BY_ID = new Map(COMMANDS.map((command) => [command.id, command]));

export function getCommand(id: CommandId): Command {
  const command = BY_ID.get(id);
  if (!command) throw new Error(`unknown command: ${id}`);
  return command;
}

/** Every command, in registry order (the palette lists them). */
export function allCommands(): readonly Command[] {
  return COMMANDS;
}

export function isCommandEnabled(id: CommandId, state: AppState = store()): boolean {
  return getCommand(id).isEnabled?.(state) ?? true;
}

/** Runs a command if it is enabled. Failures surface as a localized error banner. */
export async function runCommand(id: CommandId, arg?: CommandArg): Promise<void> {
  if (!isCommandEnabled(id)) return;
  try {
    await getCommand(id).run(arg);
  } catch (error) {
    reportError(error);
  }
}

export function matchesShortcut(shortcut: Shortcut, event: KeyboardEvent, mac = isMac): boolean {
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  // "+" is Shift+"=" on most layouts; accept either for zoom in.
  const keyMatches =
    key === shortcut.key || key === shortcut.alternate || (shortcut.key === "=" && key === "+");
  const shiftMatches = shortcut.key === "=" || event.shiftKey === Boolean(shortcut.shift);
  if (shortcut.ctrl) {
    return keyMatches && event.ctrlKey && !event.metaKey && !event.altKey && shiftMatches;
  }
  const mod = mac ? event.metaKey : event.ctrlKey;
  const otherMod = mac ? event.ctrlKey : event.metaKey;
  return keyMatches && mod === Boolean(shortcut.mod) && !otherMod && !event.altKey && shiftMatches;
}

/** True when keyboard focus is somewhere that edits text. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  if (target instanceof HTMLTextAreaElement) return true;
  if (target instanceof HTMLInputElement) {
    return !["button", "checkbox", "radio", "range", "submit", "reset"].includes(target.type);
  }
  return false;
}

/** The command bound to this key event, if any. */
export function commandForEvent(event: KeyboardEvent, mac = isMac): Command | undefined {
  const inField = isEditableTarget(event.target);
  const matching = COMMANDS.filter(
    (command) =>
      command.shortcut &&
      !(inField && command.shortcut.notInFields) &&
      matchesShortcut(command.shortcut, event, mac),
  );
  // A key several commands share (Delete, Escape, the arrows) runs the one that applies now.
  return matching.find((command) => isCommandEnabled(command.id)) ?? matching[0];
}

/** Human-readable shortcut, e.g. "⌘O" on macOS and "Ctrl+O" elsewhere. */
export function shortcutLabel(id: CommandId, mac = isMac): string | null {
  const shortcut = getCommand(id).shortcut;
  if (!shortcut) return null;
  const names: Record<string, string> = {
    "=": "+",
    PageDown: "PgDn",
    PageUp: "PgUp",
    Escape: "Esc",
    Enter: mac ? "↩" : "Enter",
    // A Mac's delete key sends Backspace.
    Delete: mac ? "⌫" : "Del",
    Tab: mac ? "⇥" : "Tab",
    ArrowLeft: "←",
    ArrowRight: "→",
    ArrowUp: "↑",
    ArrowDown: "↓",
  };
  const key = names[shortcut.key] ?? shortcut.key.toUpperCase();
  if (mac) {
    return `${shortcut.ctrl ? "⌃" : ""}${shortcut.shift ? "⇧" : ""}${shortcut.mod ? "⌘" : ""}${key}`;
  }
  return [shortcut.mod || shortcut.ctrl ? "Ctrl" : null, shortcut.shift ? "Shift" : null, key]
    .filter(Boolean)
    .join("+");
}

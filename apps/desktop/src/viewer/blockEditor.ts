/**
 * The in-place text editor's surface (ADR 0022, 0024).
 *
 * The engine lays the block's text out exactly as it will be written and
 * returns its glyphs' outlines; this draws them on a canvas over the page,
 * with the original text masked by a paper-coloured box. A hidden text field
 * takes the keyboard, IME composition, the clipboard, and undo of typing; the
 * caret and selection drawn come from the engine's caret stops, so what is
 * seen while typing is what the page will show.
 *
 * Both elements live inside the page's own element, so they scroll and zoom
 * with it and take the reading theme's tint as the page does. Positions are
 * percentages of the page; only the canvas's pixel size follows the zoom.
 */
import type { Backend, BlockEditStart, BlockLayout, PageSize } from "../backend/types";
import {
  caretFor,
  lineEdge,
  lineOf,
  offsetAt,
  selectionRects,
  verticalMove,
  wordAround,
} from "./blockLayout";

export interface BlockEditorCallbacks {
  /** A press outside the editor, or Cmd/Ctrl+Enter: keep the text. */
  commit(): void;
  /** Escape: drop the changes. */
  cancel(): void;
  /** The engine could not lay the text out. */
  failed(error: unknown): void;
}

export interface BlockEditorClassNames {
  editor: string;
  mask: string;
  canvas: string;
  field: string;
}

/** Room around the text box for glyphs that overhang it (marks, swashes), in ems. */
const OVERHANG_EMS = 0.6;
/** The mask covers the original block with this much to spare, in points. */
const MASK_MARGIN = 1;

export class BlockEditorSurface {
  /** The canvas and the hidden field, over the page. */
  readonly element: HTMLDivElement;
  /** Hides the original text while it is edited. */
  readonly mask: HTMLDivElement;
  private readonly canvas: HTMLCanvasElement;
  private readonly field: HTMLTextAreaElement;
  private layout: BlockLayout;
  /** Outlines by "font:id", kept for the whole session (each path made when first drawn). */
  private readonly paths = new Map<
    string,
    { d: string; path: Path2D | null; unitsPerEm: number }
  >();
  /** CSS pixels per point, and device pixels per CSS pixel. */
  private scale = 1;
  private devicePixelRatio = 1;
  private requesting = false;
  private stale = false;
  /** The caret's x before a run of Up/Down presses. */
  private goalX: number | null = null;
  private composing: [number, number] | null = null;
  private dragging: number | null = null;
  /** Shown but no longer taking input: waiting for the page to repaint. */
  private closing = false;

  constructor(
    readonly page: number,
    readonly start: BlockEditStart,
    private readonly pageSize: PageSize,
    private readonly backend: Backend,
    private readonly callbacks: BlockEditorCallbacks,
    classNames: BlockEditorClassNames,
    label: string,
  ) {
    this.layout = start.layout;
    this.remember(start.layout);

    this.mask = document.createElement("div");
    this.mask.className = classNames.mask;
    this.mask.setAttribute("aria-hidden", "true");
    const { original } = start;
    this.placeIn(this.mask, {
      x: original.x - MASK_MARGIN,
      y: original.y - MASK_MARGIN,
      width: original.width + 2 * MASK_MARGIN,
      height: original.height + 2 * MASK_MARGIN,
    });

    this.element = document.createElement("div");
    this.element.className = classNames.editor;
    this.element.dataset.testid = "block-editor";
    this.canvas = document.createElement("canvas");
    this.canvas.className = classNames.canvas;
    this.canvas.setAttribute("aria-hidden", "true");
    this.field = document.createElement("textarea");
    this.field.className = classNames.field;
    this.field.value = start.text;
    this.field.dir = start.rtl ? "rtl" : "ltr";
    // One line, so its own Left/Right (visual, by the field's bidi) never wraps.
    this.field.wrap = "off";
    this.field.spellcheck = false;
    this.field.setAttribute("autocapitalize", "off");
    this.field.setAttribute("aria-label", label);
    this.field.setAttribute("aria-multiline", "true");
    this.element.append(this.canvas, this.field);
    // How much of the field's text the drawing shows (the self-test times keystrokes by it).
    this.element.dataset.laidOutLength = String(start.text.length);

    this.field.addEventListener("input", this.onInput);
    this.field.addEventListener("keydown", this.onKeyDown);
    this.field.addEventListener("compositionstart", this.onCompositionStart);
    this.field.addEventListener("compositionend", this.onCompositionEnd);
    this.field.addEventListener("blur", this.onFocusChange);
    this.field.addEventListener("focus", this.onFocusChange);
    document.addEventListener("selectionchange", this.onSelectionChange);
    this.canvas.addEventListener("pointerdown", this.onCanvasPointerDown);
    this.canvas.addEventListener("pointermove", this.onCanvasPointerMove);
    this.canvas.addEventListener("pointerup", this.onCanvasPointerUp);
    this.canvas.addEventListener("pointercancel", this.onCanvasPointerUp);
    this.canvas.addEventListener("dblclick", this.onCanvasDoubleClick);
    window.addEventListener("pointerdown", this.onWindowPointerDown, true);
    this.place();
  }

  /** Puts both elements on the page's element (again, after it was mounted anew). */
  attach(pageElement: HTMLElement): void {
    pageElement.append(this.mask, this.element);
  }

  /** CSS pixels per point and the device pixel ratio: redraws at that resolution. */
  setScale(scale: number, devicePixelRatio: number): void {
    if (scale === this.scale && devicePixelRatio === this.devicePixelRatio) return;
    this.scale = scale;
    this.devicePixelRatio = devicePixelRatio;
    this.draw();
  }

  focus(): void {
    this.field.focus({ preventScroll: true });
    this.draw();
  }

  /** The text as edited. */
  text(): string {
    return this.field.value;
  }

  /** The press is on the editor's own elements. */
  contains(target: EventTarget | null): boolean {
    return target instanceof Node && (this.element.contains(target) || this.mask.contains(target));
  }

  /** Stops taking input but stays drawn, until `destroy` (once the page shows the new text). */
  freeze(): void {
    this.closing = true;
    this.stopListening();
    this.field.blur();
    this.draw();
  }

  destroy(): void {
    this.stopListening();
    this.mask.remove();
    this.element.remove();
  }

  private stopListening(): void {
    window.removeEventListener("pointerdown", this.onWindowPointerDown, true);
    document.removeEventListener("selectionchange", this.onSelectionChange);
  }

  // ------------------------------------------------------------ geometry

  private placeIn(
    element: HTMLElement,
    rect: { x: number; y: number; width: number; height: number },
  ) {
    const { width, height } = this.pageSize;
    // Percentages of the page: zoom moves nothing. The page is laid out left to right.
    element.style.insetInlineStart = `${(100 * rect.x) / width}%`;
    element.style.insetBlockStart = `${(100 * rect.y) / height}%`;
    element.style.inlineSize = `${(100 * rect.width) / width}%`;
    element.style.blockSize = `${(100 * rect.height) / height}%`;
  }

  /** The canvas's box in page points: the text box with room for overhangs. */
  private frame() {
    const pad = this.start.size * OVERHANG_EMS;
    return {
      x: this.start.x - pad,
      y: this.layout.top - pad,
      width: this.start.width + 2 * pad,
      height: this.layout.height + 2 * pad,
      pad,
    };
  }

  private place(): void {
    this.placeIn(this.element, this.frame());
  }

  /** A client point in the layout's coordinates. */
  private layoutPoint(clientX: number, clientY: number): [number, number] {
    const bounds = this.canvas.getBoundingClientRect();
    const frame = this.frame();
    const perPoint = bounds.width / frame.width || 1;
    return [
      (clientX - bounds.left) / perPoint - frame.pad,
      (clientY - bounds.top) / perPoint - frame.pad,
    ];
  }

  // ------------------------------------------------------------ layout

  private remember(layout: BlockLayout): void {
    for (const outline of layout.outlines) {
      this.paths.set(`${outline.font}:${outline.id}`, {
        d: outline.path,
        path: null,
        unitsPerEm: outline.unitsPerEm,
      });
    }
  }

  private known(): [number, number][] {
    return [...this.paths.keys()].map((key) => {
      const [font, id] = key.split(":").map(Number);
      return [font ?? 0, id ?? 0];
    });
  }

  /** Lays out the field's text; a change made meanwhile is laid out next. */
  private request(): void {
    if (this.requesting) {
      this.stale = true;
      return;
    }
    this.requesting = true;
    this.stale = false;
    const text = this.field.value;
    this.backend.layoutBlockEdit(this.start.token, text, this.known()).then(
      (layout) => {
        this.requesting = false;
        if (this.closing) return;
        this.remember(layout);
        this.layout = layout;
        this.place();
        this.draw();
        this.element.dataset.laidOutLength = String(text.length);
        if (this.stale) this.request();
      },
      (error: unknown) => {
        this.requesting = false;
        if (!this.closing) this.callbacks.failed(error);
      },
    );
  }

  // ------------------------------------------------------------ drawing

  private draw(): void {
    const frame = this.frame();
    const ratio = this.scale * this.devicePixelRatio;
    const width = Math.max(1, Math.ceil(frame.width * ratio));
    const height = Math.max(1, Math.ceil(frame.height * ratio));
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width;
      this.canvas.height = height;
    }
    const context = this.canvas.getContext("2d");
    if (!context) return;
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width, height);
    const origin = (x: number, y: number) =>
      [(x + frame.pad) * ratio, (y + frame.pad) * ratio] as const;

    const style = getComputedStyle(this.element);
    const [start, end] = this.selection();
    const focused = !this.closing && document.activeElement === this.field;
    if (focused && end > start) {
      context.fillStyle = style.getPropertyValue("--color-text-selection") || "highlight";
      for (const rect of selectionRects(this.layout, start, end)) {
        const [x, y] = origin(rect.x, rect.y);
        context.fillRect(x, y, rect.width * ratio, rect.height * ratio);
      }
    }

    // The block's own colour: the document's, not the interface's.
    const [r, g, b] = this.start.color.map((channel) => Math.round(channel * 255));
    const ink = `rgb(${r} ${g} ${b})`;
    context.fillStyle = ink;
    for (const glyph of this.layout.glyphs) {
      const outline = this.paths.get(`${glyph.font}:${glyph.id}`);
      if (!outline) continue;
      outline.path ??= new Path2D(outline.d);
      const k = (glyph.size / outline.unitsPerEm) * ratio;
      const [x, y] = origin(glyph.x, glyph.y);
      context.setTransform(k, 0, 0, -k, x, y);
      context.fill(outline.path);
    }
    context.setTransform(1, 0, 0, 1, 0, 0);

    // IME composition: underlined, as text fields show it.
    if (this.composing) {
      const [from, to] = this.composing;
      context.fillStyle = ink;
      for (const rect of selectionRects(this.layout, from, to)) {
        const [x, y] = origin(rect.x, rect.y + rect.height - 1);
        context.fillRect(x, y, rect.width * ratio, Math.max(1, ratio));
      }
    }

    if (focused && start === end) {
      const caret = caretFor(this.layout, this.field.selectionEnd);
      const line = caret ? this.layout.lines[caret.line] : undefined;
      if (caret && line) {
        context.fillStyle = ink;
        const [x, y] = origin(caret.x, line.top);
        context.fillRect(x - ratio / 2, y, Math.max(1, ratio), line.height * ratio);
        this.placeField(caret.x, line.top, line.height);
      }
    }
  }

  /** Keeps the hidden field at the caret, so an IME's candidate window opens there. */
  private placeField(x: number, top: number, height: number): void {
    const frame = this.frame();
    this.field.style.insetInlineStart = `${(100 * (x + frame.pad)) / frame.width}%`;
    this.field.style.insetBlockStart = `${(100 * (top + frame.pad)) / frame.height}%`;
    this.field.style.blockSize = `${(100 * height) / frame.height}%`;
  }

  private selection(): [number, number] {
    return [this.field.selectionStart, this.field.selectionEnd];
  }

  // ------------------------------------------------------------ input

  private readonly onInput = () => {
    this.goalX = null;
    this.request();
  };

  private readonly onSelectionChange = () => {
    if (document.activeElement === this.field) this.draw();
  };

  private readonly onFocusChange = () => this.draw();

  private readonly onCompositionStart = () => {
    const at = this.field.selectionStart;
    this.composing = [at, at];
    const update = () => {
      if (this.composing) this.composing = [this.composing[0], this.field.selectionEnd];
    };
    this.field.addEventListener("input", update);
    this.field.addEventListener(
      "compositionend",
      () => this.field.removeEventListener("input", update),
      {
        once: true,
      },
    );
  };

  private readonly onCompositionEnd = () => {
    this.composing = null;
    this.draw();
  };

  private moveTo(offset: number, extend: boolean): void {
    const anchor =
      this.field.selectionDirection === "backward"
        ? this.field.selectionEnd
        : this.field.selectionStart;
    if (extend) {
      const [from, to] = offset < anchor ? [offset, anchor] : [anchor, offset];
      this.field.setSelectionRange(from, to, offset < anchor ? "backward" : "forward");
    } else {
      this.field.setSelectionRange(offset, offset);
    }
    this.draw();
  }

  /** The end of the selection that moves (its focus). */
  private focusOffset(): number {
    return this.field.selectionDirection === "backward"
      ? this.field.selectionStart
      : this.field.selectionEnd;
  }

  private readonly onKeyDown = (event: KeyboardEvent) => {
    if (event.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      this.callbacks.cancel();
      return;
    }
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      event.stopPropagation();
      this.callbacks.commit();
      return;
    }
    // Up, Down, Home and End follow the engine's lines, not the field's.
    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      event.preventDefault();
      const focus = this.focusOffset();
      const caret = caretFor(this.layout, focus);
      this.goalX ??= caret?.x ?? 0;
      const target = verticalMove(this.layout, focus, event.key === "ArrowUp" ? -1 : 1, this.goalX);
      this.moveTo(target, event.shiftKey);
      return;
    }
    if ((event.key === "Home" || event.key === "End") && !event.metaKey && !event.ctrlKey) {
      event.preventDefault();
      this.goalX = null;
      this.moveTo(
        lineEdge(this.layout, this.focusOffset(), event.key === "Home" ? "start" : "end"),
        event.shiftKey,
      );
      return;
    }
    this.goalX = null;
    // Shortcuts of the app (undo, save, zoom) are the field's own while it has focus.
    event.stopPropagation();
  };

  private readonly onWindowPointerDown = (event: PointerEvent) => {
    if (this.contains(event.target)) return;
    // The bar beside the editor (Done, Cancel) runs its own commands.
    if (event.target instanceof Element && event.target.closest("[data-block-edit-bar]")) return;
    this.callbacks.commit();
  };

  private readonly onCanvasPointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || this.closing) return;
    event.preventDefault();
    event.stopPropagation();
    const [x, y] = this.layoutPoint(event.clientX, event.clientY);
    const offset = offsetAt(this.layout, x, y);
    this.field.focus({ preventScroll: true });
    this.goalX = null;
    this.moveTo(offset, event.shiftKey);
    this.dragging = event.pointerId;
    this.canvas.setPointerCapture?.(event.pointerId);
  };

  private readonly onCanvasPointerMove = (event: PointerEvent) => {
    if (this.dragging !== event.pointerId) return;
    const [x, y] = this.layoutPoint(event.clientX, event.clientY);
    this.moveTo(offsetAt(this.layout, x, y), true);
  };

  private readonly onCanvasPointerUp = (event: PointerEvent) => {
    if (this.dragging !== event.pointerId) return;
    this.dragging = null;
    this.canvas.releasePointerCapture?.(event.pointerId);
  };

  private readonly onCanvasDoubleClick = (event: MouseEvent) => {
    const [x, y] = this.layoutPoint(event.clientX, event.clientY);
    const [from, to] = wordAround(this.field.value, offsetAt(this.layout, x, y));
    this.field.setSelectionRange(from, to);
    this.draw();
  };

  /** For tests: the line the caret is on. */
  caretLine(): number {
    return lineOf(this.layout, this.focusOffset());
  }
}

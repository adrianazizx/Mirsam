import type { DesignEdit, FieldOption, FieldSpec, FormField, ViewRect } from "../backend";
import { backend } from "../backend";
import { i18n } from "../i18n";
import { type DesignTool, type FormDesign, useAppStore } from "../state/appStore";

/**
 * The form designer (ADR 0047): what the registry's `formDesign.*`
 * commands share. Every change goes to the engine as one `designForm` call
 * (one undo step), and the form is read again after it.
 */

const store = () => useAppStore.getState();

/** A field placed by a click (no drag) gets this size, in points. */
export const DEFAULT_SIZES: Record<DesignTool, { width: number; height: number }> = {
  text: { width: 160, height: 22 },
  checkbox: { width: 14, height: 14 },
  radio: { width: 14, height: 14 },
  combo: { width: 160, height: 22 },
  list: { width: 160, height: 60 },
  pushButton: { width: 80, height: 24 },
};

/** A drag shorter than this (points) either way is a click. */
const CLICK = 4;
/** How far apart a radio group's new button is placed from its last, in points. */
const BUTTON_GAP = 8;

/** The new field's box: the drag, or (a click) the default size at the press, kept on the page. */
export function drawnRect(
  tool: DesignTool,
  rect: ViewRect,
  page: { width: number; height: number } | null,
): ViewRect {
  if (rect.width >= CLICK && rect.height >= CLICK) return rect;
  const { width, height } = DEFAULT_SIZES[tool];
  const maxX = Math.max(0, (page?.width ?? Number.POSITIVE_INFINITY) - width);
  const maxY = Math.max(0, (page?.height ?? Number.POSITIVE_INFINITY) - height);
  return {
    x: Math.max(0, Math.min(maxX, rect.x)),
    y: Math.max(0, Math.min(maxY, rect.y)),
    width,
    height,
  };
}

/** "Text 1", "Text 2", …: the first name of its kind no field has. */
export function newFieldName(kind: DesignTool | "default", fields: FormField[]): string {
  const taken = new Set(fields.map((field) => field.name));
  for (let n = 1; ; n += 1) {
    const name = i18n.t(`formDesign.names.${kind}`, { n });
    if (!taken.has(name)) return name;
  }
}

/** A new field's properties: its kind's defaults, under `name`. */
export function defaultSpec(kind: DesignTool, name: string): FieldSpec {
  const choice = (n: number): FieldOption => ({
    value: i18n.t("formDesign.choice", { n }),
    label: "",
  });
  return {
    kind,
    name,
    tooltip: "",
    required: false,
    readOnly: false,
    multiline: false,
    maxLen: null,
    defaultValues: [],
    align: "start",
    font: "sans",
    fontSize: 0,
    options:
      kind === "radio"
        ? [{ value: "0", label: i18n.t("formDesign.choice", { n: 1 }) }]
        : kind === "combo" || kind === "list"
          ? [choice(1), choice(2)]
          : [],
    multiSelect: false,
    editable: false,
    bordered: true,
    caption: kind === "pushButton" ? i18n.t("formDesign.defaultCaption") : "",
  };
}

/** A field's properties as the designer edits them. */
export function specOf(field: FormField): FieldSpec {
  return {
    kind: field.kind,
    name: field.partialName ?? field.name,
    tooltip: field.tooltip ?? "",
    required: field.required,
    readOnly: field.readOnly,
    multiline: field.multiline,
    maxLen: field.maxLen,
    defaultValues: field.defaultValues ?? [],
    align: field.align ?? "start",
    font: field.font ?? null,
    fontSize: field.fontSize ?? 0,
    options:
      field.kind === "checkbox"
        ? []
        : field.kind === "radio"
          ? // One per button, in button order: what the engine renames by position.
            field.widgets.map((widget, i) => ({
              value: widget.onState ?? String(i),
              label:
                field.options.find((o) => o.value === widget.onState)?.label ??
                widget.onState ??
                "",
            }))
          : field.options,
    multiSelect: field.multiSelect,
    editable: field.editable,
    bordered: field.bordered ?? true,
    caption: field.caption ?? "",
  };
}

/** The designer's state for `documentId`, if it shows that document. */
export function designOf(documentId: number): FormDesign | null {
  const design = store().formDesign;
  return design?.documentId === documentId ? design : null;
}

/** The selected field, as last read. */
export function selectedField(design: FormDesign | null): FormField | null {
  const selected = design?.selected;
  if (!design || !selected) return null;
  return design.fields.find((field) => field.id === selected.field) ?? null;
}

/** Reads the form again; a selection that is gone is dropped. */
export async function loadFormDesign(documentId: number): Promise<void> {
  const info = await backend.formFields(documentId);
  store().updateFormDesign((current) => {
    if (current?.documentId !== documentId) {
      return { documentId, fields: info.fields, selected: null, detected: null };
    }
    const selected = current.selected;
    const kept = selected
      ? info.fields.find((field) => field.id === selected.field)?.widgets[selected.widget]
      : undefined;
    return { ...current, fields: info.fields, selected: kept ? selected : null };
  });
}

/**
 * Applies `edits` as one undo step, reads the form again, and selects the
 * field the engine names (`widget` of it; its last widget for `"last"`).
 */
export async function applyDesign(
  documentId: number,
  edits: DesignEdit[],
  widget: number | "last" = 0,
): Promise<void> {
  const outcome = await backend.designForm(documentId, edits);
  store().applyEdit(documentId, outcome.edit);
  await loadFormDesign(documentId);
  const field = outcome.field;
  store().updateFormDesign((current) => {
    if (!current) return current;
    if (field === null) return { ...current, selected: null };
    const found = current.fields.find((candidate) => candidate.id === field);
    if (!found) return { ...current, selected: null };
    const index =
      widget === "last" ? found.widgets.length - 1 : Math.min(widget, found.widgets.length - 1);
    return { ...current, selected: { field, widget: Math.max(0, index) } };
  });
}

/** The page a field is first on, and its box there. */
export function firstWidget(field: FormField): { page: number; view: ViewRect } | null {
  for (const widget of field.widgets) {
    if (widget.page !== null && widget.view) return { page: widget.page, view: widget.view };
  }
  return null;
}

/** A field's place in `page`'s tab order: its first widget's position there. */
function tabPosition(field: FormField, page: number): number {
  const positions = field.widgets
    .filter((widget) => widget.page === page)
    .map((widget) => widget.annotIndex ?? Number.MAX_SAFE_INTEGER);
  return positions.length > 0 ? Math.min(...positions) : Number.MAX_SAFE_INTEGER;
}

/** The fields shown on `page`, in its tab order. */
export function pageFields(fields: FormField[], page: number): FormField[] {
  return fields
    .filter((field) => field.widgets.some((widget) => widget.page === page))
    .sort((a, b) => tabPosition(a, page) - tabPosition(b, page));
}

/** `ids` with the one at `index` moved by `step` (kept inside the list). */
export function moved(ids: number[], index: number, step: number): number[] {
  const to = Math.max(0, Math.min(ids.length - 1, index + step));
  const next = [...ids];
  const [id] = next.splice(index, 1);
  if (id !== undefined) next.splice(to, 0, id);
  return next;
}

/**
 * The fields on `page` in rows, top to bottom; within a row from the right
 * (`rtl`) or from the left. Fields whose boxes share most of their height
 * share a row.
 */
export function rowOrder(fields: FormField[], page: number, rtl: boolean): number[] {
  const boxes = pageFields(fields, page).flatMap((field) => {
    const widget = field.widgets.find((candidate) => candidate.page === page && candidate.view);
    return widget?.view ? [{ id: field.id, view: widget.view }] : [];
  });
  boxes.sort((a, b) => a.view.y - b.view.y);
  const rows: (typeof boxes)[] = [];
  for (const box of boxes) {
    const row = rows[rows.length - 1];
    const first = row?.[0];
    const middle = box.view.y + box.view.height / 2;
    if (row && first && middle < first.view.y + first.view.height) row.push(box);
    else rows.push([box]);
  }
  return rows.flatMap((row) =>
    [...row]
      .sort((a, b) =>
        rtl ? b.view.x + b.view.width - (a.view.x + a.view.width) : a.view.x - b.view.x,
      )
      .map((box) => box.id),
  );
}

/** Where a radio group's next button goes: after its last one, on the same line, else below. */
export function nextButtonRect(
  field: FormField,
  page: { width: number } | null,
): { page: number; rect: ViewRect } | null {
  const last = [...field.widgets].reverse().find((widget) => widget.page !== null && widget.view);
  if (!last?.view || last.page === null) return null;
  const { x, y, width, height } = last.view;
  const right = x + width + BUTTON_GAP + width;
  if (page === null || right <= page.width) {
    return { page: last.page, rect: { x: x + width + BUTTON_GAP, y, width, height } };
  }
  return { page: last.page, rect: { x, y: y + height + BUTTON_GAP, width, height } };
}

/** The ticked suggestions as new fields, named after their labels (unique), one undo step. */
export function detectedEdits(design: FormDesign): DesignEdit[] {
  const detected = design.detected;
  if (!detected) return [];
  const taken = new Set(design.fields.map((field) => field.name));
  const edits: DesignEdit[] = [];
  for (const found of detected.fields) {
    if (!found.include || !found.view) continue;
    const kind = found.kind === "checkbox" ? "checkbox" : "text";
    const named = found.name.trim();
    let name = named;
    if (named === "" || taken.has(named)) {
      for (let n = named === "" ? 1 : 2; ; n += 1) {
        const candidate =
          named === "" ? i18n.t(`formDesign.names.${kind}`, { n }) : `${named} ${n}`;
        if (!taken.has(candidate)) {
          name = candidate;
          break;
        }
      }
    }
    taken.add(name);
    edits.push({
      op: "add",
      page: detected.page,
      rect: found.view,
      spec: { ...defaultSpec(kind, name), multiline: found.multiline, bordered: false },
    });
  }
  return edits;
}

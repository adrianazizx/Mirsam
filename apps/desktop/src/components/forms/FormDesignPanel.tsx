import {
  ArrowDown,
  ArrowUp,
  CircleDot,
  List,
  type LucideIcon,
  MousePointer2,
  Plus,
  RectangleHorizontal,
  ScanSearch,
  SquareCheck,
  SquareChevronDown,
  TextAlignCenter,
  TextAlignEnd,
  TextAlignStart,
  TextCursorInput,
  Trash2,
} from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { Checkbox, type Key, ToggleButton, ToggleButtonGroup } from "react-aria-components";
import { useTranslation } from "react-i18next";
import type {
  FieldAlign,
  FieldFont,
  FieldKind,
  FieldOption,
  FieldSpec,
  FormField,
} from "../../backend";
import { loadFormDesign, pageFields, specOf } from "../../commands/formDesign";
import { type CommandId, isCommandEnabled, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { type DesignTool, useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { IconButton } from "../ui/IconButton";
import { Tooltip } from "../ui/Tooltip";
import styles from "./FormDesignPanel.module.css";
import fieldStyles from "./FormsPanel.module.css";

/** What a drag on the page draws, in a fixed order; "select" draws nothing. */
const TOOLS: { id: DesignTool | "select"; command: CommandId; icon: LucideIcon }[] = [
  { id: "select", command: "formDesign.select", icon: MousePointer2 },
  { id: "text", command: "formDesign.text", icon: TextCursorInput },
  { id: "checkbox", command: "formDesign.checkbox", icon: SquareCheck },
  { id: "radio", command: "formDesign.radio", icon: CircleDot },
  { id: "combo", command: "formDesign.combo", icon: SquareChevronDown },
  { id: "list", command: "formDesign.list", icon: List },
  { id: "pushButton", command: "formDesign.button", icon: RectangleHorizontal },
];

const KIND_ICONS: Record<FieldKind, LucideIcon> = {
  text: TextCursorInput,
  checkbox: SquareCheck,
  radio: CircleDot,
  combo: SquareChevronDown,
  list: List,
  pushButton: RectangleHorizontal,
  signature: RectangleHorizontal,
};

/** Start and end follow the text's own direction; their icons mirror in the Arabic UI. */
const ALIGNS: { id: FieldAlign; icon: ReactNode; mirror: boolean }[] = [
  { id: "start", icon: <TextAlignStart />, mirror: true },
  { id: "center", icon: <TextAlignCenter />, mirror: false },
  { id: "end", icon: <TextAlignEnd />, mirror: true },
];

/** Text sizes offered; 0 fits the text to the box. */
const SIZES = [0, 8, 9, 10, 11, 12, 14, 16, 18, 24];

/**
 * The form designer (ADR 0047), in the Forms tool's panel: pick what a drag
 * on the page draws, select a field (on the page or in the tab order) to
 * change its properties, order Tab's way through the page, and find the
 * blanks of a flat form, reviewed before any field is made. Every control
 * runs a registry command; each change is one undo step.
 */
export function FormDesignPanel({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const designTool = useAppStore((state) => state.designTool);
  const design = useAppStore((state) =>
    state.formDesign?.documentId === documentId ? state.formDesign : null,
  );
  const page = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.page ?? 0,
  );
  const revision = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.revision ?? 0,
  );
  const paint = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.paint ?? 0,
  );
  // A file opened with its user password may not allow changing its form (ADR 0043).
  const permitted = useAppStore((state) => isCommandEnabled("formDesign.text", state));

  // biome-ignore lint/correctness/useExhaustiveDependencies: every new revision (an undo too) is read again
  useEffect(() => {
    loadFormDesign(documentId).catch((error: unknown) => console.warn("form unavailable", error));
  }, [documentId, revision, paint]);

  const current = designTool ?? "select";
  const selected = design?.selected ?? null;
  const field = selected
    ? design?.fields.find((candidate) => candidate.id === selected.field)
    : null;
  const onPage = pageFields(design?.fields ?? [], page);

  return (
    <div className={styles.panel} data-testid="form-design-panel">
      <section className={styles.section} aria-labelledby="design-tools-heading">
        <h3 id="design-tools-heading" className={styles.heading}>
          {t("formDesign.toolsHeading")}
        </h3>
        <ToggleButtonGroup
          aria-label={t("formDesign.tools")}
          selectionMode="single"
          selectedKeys={[current]}
          className={styles.tools}
          onSelectionChange={(keys: Set<Key>) => {
            const [next] = keys;
            const tool = TOOLS.find((candidate) => candidate.id === next);
            if (tool) void runCommand(tool.command);
          }}
        >
          {TOOLS.map(({ id, command, icon: Icon }) => (
            <Tooltip key={id} label={t(`commands.${command}`)}>
              <ToggleButton
                id={id}
                className={styles.tool}
                aria-label={t(`commands.${command}`)}
                isDisabled={id !== "select" && !permitted}
              >
                <Icon aria-hidden="true" />
              </ToggleButton>
            </Tooltip>
          ))}
        </ToggleButtonGroup>
        <p className={styles.hint} data-testid="design-hint">
          {t(permitted ? `formDesign.hint.${current}` : "formDesign.notPermitted")}
        </p>
      </section>

      <section className={styles.section} aria-labelledby="design-field-heading">
        <h3 id="design-field-heading" className={styles.heading}>
          {t("formDesign.fieldHeading")}
        </h3>
        {field && selected ? (
          <FieldInspector
            key={field.id}
            field={field}
            widget={selected.widget}
            permitted={permitted}
          />
        ) : (
          <p className={styles.hint}>{t("formDesign.noSelection")}</p>
        )}
      </section>

      <section className={styles.section} aria-labelledby="design-tabs-heading">
        <h3 id="design-tabs-heading" className={styles.heading}>
          {t("formDesign.tabsHeading", { page: formatNumber(page + 1, locale) })}
        </h3>
        {onPage.length === 0 ? (
          <p className={styles.hint}>{t("formDesign.noFieldsOnPage")}</p>
        ) : (
          <ol className={styles.order} data-testid="tab-order">
            {onPage.map((candidate, i) => (
              <li key={candidate.id}>
                <button
                  type="button"
                  className={styles.orderItem}
                  aria-current={candidate.id === selected?.field ? "true" : undefined}
                  onClick={() =>
                    void runCommand("formDesign.selectField", { field: candidate.id, index: 0 })
                  }
                >
                  <span className={styles.orderNumber}>{formatNumber(i + 1, locale)}</span>
                  <KindIcon kind={candidate.kind} />
                  <bdi className={styles.orderName}>{candidate.name}</bdi>
                </button>
              </li>
            ))}
          </ol>
        )}
        <div className={styles.row}>
          <CommandButton command="formDesign.tabEarlier">
            <ArrowUp />
          </CommandButton>
          <CommandButton command="formDesign.tabLater">
            <ArrowDown />
          </CommandButton>
        </div>
        <div className={styles.row}>
          <Button
            variant="secondary"
            isDisabled={!permitted || onPage.length < 2}
            onPress={() => void runCommand("formDesign.tabRowsRightToLeft")}
          >
            {t("commands.formDesign.tabRowsRightToLeft")}
          </Button>
          <Button
            variant="secondary"
            isDisabled={!permitted || onPage.length < 2}
            onPress={() => void runCommand("formDesign.tabRowsLeftToRight")}
          >
            {t("commands.formDesign.tabRowsLeftToRight")}
          </Button>
        </div>
      </section>

      <DetectSection permitted={permitted} />
    </div>
  );
}

function KindIcon({ kind }: { kind: FieldKind }) {
  const { t } = useTranslation();
  const Icon = KIND_ICONS[kind];
  return (
    <span className={styles.kindIcon} title={t(`formDesign.kinds.${kind}`)}>
      <Icon aria-hidden="true" />
    </span>
  );
}

/** The blanks of a flat page, for review: each ticked one becomes a field. */
function DetectSection({ permitted }: { permitted: boolean }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const detected = useAppStore((state) => state.formDesign?.detected ?? null);
  const included = detected?.fields.filter((field) => field.include).length ?? 0;
  return (
    <section className={styles.section} aria-labelledby="design-detect-heading">
      <h3 id="design-detect-heading" className={styles.heading}>
        {t("formDesign.detectHeading")}
      </h3>
      {detected === null ? (
        <>
          <p className={styles.hint}>{t("formDesign.detectHint")}</p>
          <Button
            variant="secondary"
            isDisabled={!permitted}
            onPress={() => void runCommand("formDesign.detect")}
          >
            <ScanSearch aria-hidden="true" className={styles.buttonIcon} />
            {t("commands.formDesign.detect")}
          </Button>
        </>
      ) : detected.fields.length === 0 ? (
        <>
          <p className={styles.hint} data-testid="detect-none">
            {t("formDesign.detectNone", { page: formatNumber(detected.page + 1, locale) })}
          </p>
          <Button variant="secondary" onPress={() => void runCommand("formDesign.cancelDetected")}>
            {t("formDesign.detectClose")}
          </Button>
        </>
      ) : (
        <>
          <p className={styles.hint}>
            {t("formDesign.detectReview", {
              count: detected.fields.length,
              n: formatNumber(detected.fields.length, locale),
              page: formatNumber(detected.page + 1, locale),
            })}
          </p>
          <ul className={styles.detected} data-testid="detected">
            {detected.fields.map((found, index) => (
              <li
                // biome-ignore lint/suspicious/noArrayIndexKey: suggestions have no ids and do not move
                key={index}
              >
                <Checkbox
                  className={fieldStyles.check}
                  isSelected={found.include}
                  onChange={() => void runCommand("formDesign.toggleDetected", { index })}
                >
                  <span className={fieldStyles.box} aria-hidden="true" />
                  <KindIcon kind={found.kind} />
                  {found.name ? (
                    <bdi>{found.name}</bdi>
                  ) : (
                    <span className={styles.unnamed}>{t("formDesign.unnamed")}</span>
                  )}
                </Checkbox>
              </li>
            ))}
          </ul>
          <div className={styles.row}>
            <Button
              isDisabled={included === 0 || !permitted}
              onPress={() => void runCommand("formDesign.createDetected")}
            >
              {t("formDesign.detectCreate", { count: included, n: formatNumber(included, locale) })}
            </Button>
            <Button
              variant="secondary"
              onPress={() => void runCommand("formDesign.cancelDetected")}
            >
              {t("formDesign.detectCancel")}
            </Button>
          </div>
        </>
      )}
    </section>
  );
}

/** Sends the selected field's changed properties: one undo step. */
function update(spec: FieldSpec, patch: Partial<FieldSpec>) {
  void runCommand("formDesign.update", { spec: { ...spec, ...patch } });
}

/** The selected field's properties (ADR 0047). */
function FieldInspector({
  field,
  widget,
  permitted,
}: {
  field: FormField;
  widget: number;
  permitted: boolean;
}) {
  const { t } = useTranslation();
  const spec = specOf(field);
  const set = (patch: Partial<FieldSpec>) => update(spec, patch);
  const disabled = !permitted;
  const id = (part: string) => `design-${field.id}-${part}`;
  const text = field.kind === "text";
  const choices = field.kind === "combo" || field.kind === "list";
  return (
    <div className={styles.inspector} data-testid="field-inspector">
      <p className={styles.kind}>
        <KindIcon kind={field.kind} />
        {t(`formDesign.kinds.${field.kind}`)}
      </p>
      <Labelled id={id("name")} label={t("formDesign.name")} hint={t("formDesign.nameHint")}>
        <CommitInput
          id={id("name")}
          value={spec.name}
          disabled={disabled}
          onCommit={(name) => set({ name })}
        />
      </Labelled>
      <Labelled id={id("tooltip")} label={t("formDesign.tooltip")}>
        <CommitInput
          id={id("tooltip")}
          value={spec.tooltip}
          disabled={disabled}
          onCommit={(tooltip) => set({ tooltip })}
        />
      </Labelled>
      <div className={styles.checks}>
        <Check
          label={t("formDesign.required")}
          checked={spec.required}
          disabled={disabled || field.kind === "pushButton"}
          onChange={(required) => set({ required })}
        />
        <Check
          label={t("formDesign.readOnly")}
          checked={spec.readOnly}
          disabled={disabled}
          onChange={(readOnly) => set({ readOnly })}
        />
        {text ? (
          <Check
            label={t("formDesign.multiline")}
            checked={spec.multiline}
            disabled={disabled}
            onChange={(multiline) => set({ multiline })}
          />
        ) : null}
        {field.kind === "combo" ? (
          <Check
            label={t("formDesign.editable")}
            checked={spec.editable}
            disabled={disabled}
            onChange={(editable) => set({ editable })}
          />
        ) : null}
        {field.kind === "list" ? (
          <Check
            label={t("formDesign.multiSelect")}
            checked={spec.multiSelect}
            disabled={disabled}
            onChange={(multiSelect) => set({ multiSelect })}
          />
        ) : null}
        <Check
          label={t("formDesign.bordered")}
          checked={spec.bordered}
          disabled={disabled}
          onChange={(bordered) => set({ bordered })}
        />
      </div>

      {text ? (
        <>
          <Labelled id={id("default")} label={t("formDesign.defaultValue")}>
            <CommitInput
              id={id("default")}
              value={spec.defaultValues[0] ?? ""}
              multiline={spec.multiline}
              disabled={disabled}
              maxLength={spec.maxLen ?? undefined}
              onCommit={(value) => set({ defaultValues: value === "" ? [] : [value] })}
            />
          </Labelled>
          <Labelled id={id("max")} label={t("formDesign.maxLen")} hint={t("formDesign.maxLenHint")}>
            <CommitInput
              id={id("max")}
              value={spec.maxLen === null ? "" : String(spec.maxLen)}
              numeric
              disabled={disabled}
              onCommit={(value) => {
                const n = Number.parseInt(value, 10);
                set({ maxLen: Number.isFinite(n) && n > 0 ? n : null });
              }}
            />
          </Labelled>
        </>
      ) : null}

      {field.kind === "checkbox" ? (
        <Check
          label={t("formDesign.checkedByDefault")}
          checked={spec.defaultValues.length > 0}
          disabled={disabled}
          onChange={(checked) => set({ defaultValues: checked ? ["Yes"] : [] })}
        />
      ) : null}

      {field.kind === "pushButton" ? (
        <Labelled id={id("caption")} label={t("formDesign.caption")}>
          <CommitInput
            id={id("caption")}
            value={spec.caption}
            disabled={disabled}
            onCommit={(caption) => set({ caption })}
          />
        </Labelled>
      ) : null}

      {text || choices || field.kind === "pushButton" ? (
        <TextLook
          spec={spec}
          disabled={disabled}
          id={id}
          set={set}
          align={field.kind !== "pushButton"}
        />
      ) : null}

      {choices ? <Choices field={field} spec={spec} disabled={disabled} set={set} /> : null}
      {field.kind === "radio" ? (
        <RadioButtons field={field} spec={spec} widget={widget} disabled={disabled} set={set} />
      ) : null}

      <div className={styles.row}>
        <Button
          variant="secondary"
          isDisabled={disabled}
          onPress={() => void runCommand("formDesign.delete")}
        >
          <Trash2 aria-hidden="true" className={styles.buttonIcon} />
          {t("commands.formDesign.delete")}
        </Button>
      </div>
    </div>
  );
}

/** Font, size, and (`align`) alignment. */
function TextLook({
  spec,
  disabled,
  id,
  set,
  align,
}: {
  spec: FieldSpec;
  disabled: boolean;
  id: (part: string) => string;
  set: (patch: Partial<FieldSpec>) => void;
  align: boolean;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  return (
    <>
      {align ? (
        <div className={styles.field}>
          <span className={fieldStyles.label} id={id("align")}>
            {t("formDesign.align")}
          </span>
          <ToggleButtonGroup
            aria-labelledby={id("align")}
            selectionMode="single"
            disallowEmptySelection
            isDisabled={disabled}
            selectedKeys={[spec.align]}
            className={styles.tools}
            onSelectionChange={(keys: Set<Key>) => {
              const [next] = keys;
              const choice = ALIGNS.find((candidate) => candidate.id === next);
              if (choice) set({ align: choice.id });
            }}
          >
            {ALIGNS.map(({ id: key, icon, mirror }) => (
              <Tooltip key={key} label={t(`formDesign.aligns.${key}`)}>
                <ToggleButton
                  id={key}
                  aria-label={t(`formDesign.aligns.${key}`)}
                  className={styles.tool}
                >
                  <span className={mirror ? styles.mirrored : styles.glyph} aria-hidden="true">
                    {icon}
                  </span>
                </ToggleButton>
              </Tooltip>
            ))}
          </ToggleButtonGroup>
          <p className={styles.hint}>{t("formDesign.alignHint")}</p>
        </div>
      ) : null}
      <div className={styles.pair}>
        <Labelled id={id("font")} label={t("formDesign.font")}>
          <select
            id={id("font")}
            className={fieldStyles.input}
            value={spec.font ?? "own"}
            disabled={disabled}
            onChange={(event) => {
              const value = event.target.value;
              set({ font: value === "own" ? null : (value as FieldFont) });
            }}
          >
            {spec.font === null ? <option value="own">{t("formDesign.fonts.own")}</option> : null}
            <option value="sans">{t("formDesign.fonts.sans")}</option>
            <option value="naskh">{t("formDesign.fonts.naskh")}</option>
          </select>
        </Labelled>
        <Labelled id={id("size")} label={t("formDesign.size")}>
          <select
            id={id("size")}
            className={fieldStyles.input}
            value={String(spec.fontSize)}
            disabled={disabled}
            onChange={(event) => set({ fontSize: Number(event.target.value) })}
          >
            {[...new Set([...SIZES, spec.fontSize])]
              .sort((a, b) => a - b)
              .map((size) => (
                <option key={size} value={String(size)}>
                  {size === 0 ? t("formDesign.sizeAuto") : formatNumber(size, locale)}
                </option>
              ))}
          </select>
        </Labelled>
      </div>
    </>
  );
}

/** A drop-down's or list's choices, and its default. */
function Choices({
  field,
  spec,
  disabled,
  set,
}: {
  field: FormField;
  spec: FieldSpec;
  disabled: boolean;
  set: (patch: Partial<FieldSpec>) => void;
}) {
  const { t } = useTranslation();
  const options = spec.options;
  const replace = (index: number, patch: Partial<FieldOption>) => {
    const next = options.map((option, i) => (i === index ? { ...option, ...patch } : option));
    const was = options[index]?.value;
    const now = next[index]?.value;
    // The default follows a renamed choice.
    const defaults = spec.defaultValues.map((value) => (value === was && now ? now : value));
    set({ options: next, defaultValues: defaults });
  };
  const remove = (index: number) => {
    const gone = options[index]?.value;
    set({
      options: options.filter((_, i) => i !== index),
      defaultValues: spec.defaultValues.filter((value) => value !== gone),
    });
  };
  const add = () => {
    const values = new Set(options.map((option) => option.value));
    let n = options.length + 1;
    while (values.has(t("formDesign.choice", { n }))) n += 1;
    set({ options: [...options, { value: t("formDesign.choice", { n }), label: "" }] });
  };
  return (
    <fieldset className={styles.fieldset}>
      <legend className={fieldStyles.label}>{t("formDesign.choices")}</legend>
      <p className={styles.hint}>{t("formDesign.choicesHint")}</p>
      <ol className={styles.choices}>
        {options.map((option, index) => (
          <li
            // biome-ignore lint/suspicious/noArrayIndexKey: choices are edited in place by position
            key={index}
            className={styles.choice}
          >
            <CommitInput
              value={option.label === option.value ? "" : option.label}
              placeholder={t("formDesign.choiceLabel")}
              label={t("formDesign.choiceLabelOf", { n: index + 1 })}
              disabled={disabled}
              onCommit={(label) => replace(index, { label })}
            />
            <CommitInput
              value={option.value}
              placeholder={t("formDesign.choiceValue")}
              label={t("formDesign.choiceValueOf", { n: index + 1 })}
              disabled={disabled}
              onCommit={(value) => {
                if (value.trim() !== "") replace(index, { value: value.trim() });
              }}
            />
            <IconButton
              label={t("formDesign.removeChoice")}
              isDisabled={disabled || options.length < 2}
              onPress={() => remove(index)}
            >
              <Trash2 />
            </IconButton>
          </li>
        ))}
      </ol>
      <div className={styles.row}>
        <Button variant="secondary" isDisabled={disabled} onPress={add}>
          <Plus aria-hidden="true" className={styles.buttonIcon} />
          {t("formDesign.addChoice")}
        </Button>
      </div>
      <Labelled id={`design-${field.id}-choice-default`} label={t("formDesign.defaultValue")}>
        <select
          id={`design-${field.id}-choice-default`}
          className={
            field.kind === "list" && spec.multiSelect ? fieldStyles.list : fieldStyles.input
          }
          multiple={field.kind === "list" && spec.multiSelect}
          value={
            field.kind === "list" && spec.multiSelect
              ? spec.defaultValues
              : (spec.defaultValues[0] ?? "")
          }
          disabled={disabled}
          onChange={(event) => {
            const chosen = [...event.target.selectedOptions]
              .map((o) => o.value)
              .filter((v) => v !== "");
            set({ defaultValues: chosen });
          }}
        >
          {field.kind === "list" && spec.multiSelect ? null : (
            <option value="">{t("forms.noChoice")}</option>
          )}
          {options.map((option) => (
            <option key={option.value} value={option.value} dir="auto">
              {option.label || option.value}
            </option>
          ))}
        </select>
      </Labelled>
    </fieldset>
  );
}

/** A radio group's buttons: each one's export value, and the default. */
function RadioButtons({
  field,
  spec,
  widget,
  disabled,
  set,
}: {
  field: FormField;
  spec: FieldSpec;
  widget: number;
  disabled: boolean;
  set: (patch: Partial<FieldSpec>) => void;
}) {
  const { t } = useTranslation();
  // One choice per button, in button order (the engine's FormField options are per state).
  const buttons = field.widgets.map((candidate, i) => {
    const option = field.options.find((o) => o.value === candidate.onState);
    return {
      state: candidate.onState ?? String(i),
      label: option?.label ?? candidate.onState ?? "",
    };
  });
  const options = buttons.map((button) => ({ value: button.state, label: button.label }));
  return (
    <fieldset className={styles.fieldset}>
      <legend className={fieldStyles.label}>{t("formDesign.buttons")}</legend>
      <p className={styles.hint}>{t("formDesign.buttonsHint")}</p>
      <ol className={styles.choices}>
        {buttons.map((button, index) => (
          <li
            key={button.state}
            className={styles.choice}
            aria-current={index === widget ? "true" : undefined}
          >
            <button
              type="button"
              className={styles.choiceSelect}
              onClick={() => void runCommand("formDesign.selectField", { field: field.id, index })}
              aria-label={t("formDesign.selectButton", { n: index + 1 })}
            >
              {index + 1}
            </button>
            <CommitInput
              value={button.label}
              label={t("formDesign.buttonValueOf", { n: index + 1 })}
              disabled={disabled}
              onCommit={(label) => {
                if (label.trim() === "") return;
                set({
                  options: options.map((option, i) =>
                    i === index ? { ...option, label: label.trim() } : option,
                  ),
                });
              }}
            />
            <IconButton
              label={t("commands.formDesign.removeButton")}
              isDisabled={disabled || buttons.length < 2}
              onPress={() => void runCommand("formDesign.removeButton", { index })}
            >
              <Trash2 />
            </IconButton>
          </li>
        ))}
      </ol>
      <div className={styles.row}>
        <Button
          variant="secondary"
          isDisabled={disabled}
          onPress={() => void runCommand("formDesign.addButton")}
        >
          <Plus aria-hidden="true" className={styles.buttonIcon} />
          {t("commands.formDesign.addButton")}
        </Button>
      </div>
      <Labelled id={`design-${field.id}-radio-default`} label={t("formDesign.defaultValue")}>
        <select
          id={`design-${field.id}-radio-default`}
          className={fieldStyles.input}
          value={spec.defaultValues[0] ?? ""}
          disabled={disabled}
          onChange={(event) =>
            set({ options, defaultValues: event.target.value === "" ? [] : [event.target.value] })
          }
        >
          <option value="">{t("forms.noChoice")}</option>
          {options.map((option) => (
            <option key={option.value} value={option.value} dir="auto">
              {option.label}
            </option>
          ))}
        </select>
      </Labelled>
    </fieldset>
  );
}

function Labelled({
  id,
  label,
  hint,
  children,
}: {
  id: string;
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <div className={styles.field}>
      <label className={fieldStyles.label} htmlFor={id}>
        {label}
      </label>
      {children}
      {hint ? <p className={styles.hint}>{hint}</p> : null}
    </div>
  );
}

function Check({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <Checkbox
      className={fieldStyles.check}
      isSelected={checked}
      isDisabled={disabled}
      onChange={onChange}
    >
      <span className={fieldStyles.box} aria-hidden="true" />
      <span>{label}</span>
    </Checkbox>
  );
}

/**
 * A text box whose value is sent when it is left or Enter is pressed (in a
 * one-line box), so typing a name is one step; Escape restores it. Text
 * takes its direction from what is typed.
 */
function CommitInput({
  id,
  value,
  onCommit,
  disabled,
  multiline = false,
  numeric = false,
  maxLength,
  placeholder,
  label,
}: {
  id?: string;
  value: string;
  onCommit: (value: string) => void;
  disabled: boolean;
  multiline?: boolean;
  numeric?: boolean;
  maxLength?: number;
  placeholder?: string;
  label?: string;
}) {
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) onCommit(draft);
  };
  const common = {
    id,
    dir: "auto" as const,
    value: draft,
    disabled,
    maxLength,
    placeholder,
    "aria-label": label,
    onBlur: commit,
  };
  return multiline ? (
    <textarea
      {...common}
      className={fieldStyles.textarea}
      onChange={(event) => setDraft(event.target.value)}
    />
  ) : (
    <input
      {...common}
      className={fieldStyles.input}
      inputMode={numeric ? "numeric" : undefined}
      onChange={(event) => setDraft(event.target.value)}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          event.preventDefault();
          commit();
        } else if (event.key === "Escape") {
          setDraft(value);
        }
      }}
    />
  );
}

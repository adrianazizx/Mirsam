import { useEffect, useState } from "react";
import { Checkbox, Radio, RadioGroup } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { backend, type FormField, type FormInfo } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./FormsPanel.module.css";

/** Sets a field's value through the command registry. */
function setValue(field: FormField, values: string[]) {
  void runCommand("form.setValue", { field: field.id, values });
}

/** The page a field is first shown on, for grouping. */
function pageOf(field: FormField): number | null {
  return field.widgets.find((widget) => widget.page !== null)?.page ?? null;
}

/**
 * The Forms tool (ADR 0045): every field of the document's form, page by
 * page, filled in here. Each change is one undo step; the engine draws the
 * field's new appearance (shaped Arabic, right to left) on the page. Text
 * fields take their direction from what is typed (`dir="auto"`), so Arabic
 * and English values read correctly in either UI language.
 */
export function FormsPanel({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const revision = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.revision ?? 0,
  );
  const paint = useAppStore(
    (state) => state.documents.find((open) => open.id === documentId)?.paint ?? 0,
  );
  const [form, setForm] = useState<FormInfo | null>(null);
  const [failed, setFailed] = useState(false);

  // biome-ignore lint/correctness/useExhaustiveDependencies: every new revision is read again
  useEffect(() => {
    let live = true;
    backend.formFields(documentId).then(
      (info) => {
        if (!live) return;
        setForm(info);
        setFailed(false);
      },
      (error: unknown) => {
        console.warn("form fields unavailable", error);
        if (live) setFailed(true);
      },
    );
    return () => {
      live = false;
    };
  }, [documentId, revision, paint]);

  if (failed) return <p className={styles.hint}>{t("forms.unreadable")}</p>;
  if (!form) return <p className={styles.hint}>{t("forms.loading")}</p>;
  const fillable = form.fields.filter(
    (field) => field.kind !== "pushButton" && field.kind !== "signature",
  );
  if (fillable.length === 0) {
    return (
      <div className={styles.panel} data-testid="forms-panel">
        <p className={styles.hint}>{t("forms.none")}</p>
        <Button variant="secondary" onPress={() => void runCommand("sign.openTool")}>
          {t("forms.useFillAndSign")}
        </Button>
      </div>
    );
  }

  const pages = [...new Set(fillable.map(pageOf))];
  return (
    <div className={styles.panel} data-testid="forms-panel">
      <p className={styles.hint}>
        {t("forms.count", { count: fillable.length, n: formatNumber(fillable.length, locale) })}
      </p>
      {form.xfa ? <p className={styles.warning}>{t("forms.xfa")}</p> : null}
      {form.needAppearances ? <p className={styles.hint}>{t("forms.needAppearances")}</p> : null}
      {pages.map((page) => (
        <section key={page ?? "none"} className={styles.page}>
          <h3 className={styles.pageHeading}>
            {page === null
              ? t("forms.noPage")
              : t("forms.page", { page: formatNumber(page + 1, locale) })}
          </h3>
          {fillable
            .filter((field) => pageOf(field) === page)
            .map((field) => (
              <FieldControl key={field.id} field={field} />
            ))}
        </section>
      ))}
    </div>
  );
}

/** One field's control, by its kind. */
function FieldControl({ field }: { field: FormField }) {
  const { t } = useTranslation();
  const label = (
    <>
      <bdi>{field.name}</bdi>
      {field.required ? <span className={styles.required}>{t("forms.required")}</span> : null}
    </>
  );
  const id = `form-field-${field.id}`;
  switch (field.kind) {
    case "text":
      return (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={id}>
            {label}
          </label>
          <TextValue id={id} field={field} />
        </div>
      );
    case "checkbox": {
      const on = field.options[0]?.value ?? "Yes";
      return (
        <Checkbox
          className={styles.check}
          isSelected={field.values.length > 0}
          isDisabled={field.readOnly}
          onChange={(checked) => setValue(field, checked ? [on] : [])}
        >
          <span className={styles.box} aria-hidden="true" />
          <span className={styles.label}>{label}</span>
        </Checkbox>
      );
    }
    case "radio":
      return (
        <RadioGroup
          className={styles.field}
          value={field.values[0] ?? null}
          isDisabled={field.readOnly}
          onChange={(value) => setValue(field, [value])}
          aria-label={field.name}
        >
          <span className={styles.label}>{label}</span>
          {field.options.map((option) => (
            <Radio key={option.value} value={option.value} className={styles.check}>
              <span className={`${styles.box} ${styles.round}`} aria-hidden="true" />
              <bdi>{option.label}</bdi>
            </Radio>
          ))}
        </RadioGroup>
      );
    case "combo":
      return (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={id}>
            {label}
          </label>
          <select
            id={id}
            className={styles.input}
            dir="auto"
            value={field.values[0] ?? ""}
            disabled={field.readOnly}
            onChange={(event) => setValue(field, [event.target.value])}
          >
            <option value="">{t("forms.noChoice")}</option>
            {field.options.map((option) => (
              <option key={option.value} value={option.value} dir="auto">
                {option.label}
              </option>
            ))}
          </select>
        </div>
      );
    case "list":
      return (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={id}>
            {label}
          </label>
          <select
            id={id}
            className={styles.list}
            multiple={field.multiSelect}
            size={Math.min(6, Math.max(2, field.options.length))}
            value={field.multiSelect ? field.values : (field.values[0] ?? "")}
            disabled={field.readOnly}
            onChange={(event) =>
              setValue(
                field,
                [...event.target.selectedOptions].map((option) => option.value),
              )
            }
          >
            {field.options.map((option) => (
              <option key={option.value} value={option.value} dir="auto">
                {option.label}
              </option>
            ))}
          </select>
        </div>
      );
    default:
      return null;
  }
}

/**
 * A text field's value: written as it is typed, set when the field is left
 * (or Enter is pressed in a one-line field), so typing a name is one step.
 */
function TextValue({ id, field }: { id: string; field: FormField }) {
  const value = field.values[0] ?? "";
  const [draft, setDraft] = useState(value);
  useEffect(() => setDraft(value), [value]);
  const commit = () => {
    if (draft !== value) setValue(field, [draft]);
  };
  const common = {
    id,
    className: field.multiline ? styles.textarea : styles.input,
    dir: "auto" as const,
    value: draft,
    readOnly: field.readOnly,
    maxLength: field.maxLen ?? undefined,
    onBlur: commit,
  };
  return field.multiline ? (
    <textarea {...common} onChange={(event) => setDraft(event.target.value)} />
  ) : (
    <input
      {...common}
      type={field.password ? "password" : "text"}
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

import { Check, Trash2, X } from "lucide-react";
import { type FormEvent, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LinkTarget } from "../../backend";
import { isCommandEnabled, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { type LinkDraft, normaliseUrl } from "../../state/objects";
import { Button } from "../ui/Button";
import styles from "./EditPanels.module.css";
import { describeTarget } from "./linkText";

type Kind = "url" | "page" | "named";

/**
 * The Edit tool's link mode (ADR 0051): draw a rectangle on the page, or
 * select text and choose Link, then say where the link goes — a web or
 * e-mail address, a page, or a named destination. Press a link to change
 * where it goes, drag it or its corners to move or resize it, or delete it.
 */
export function LinkPanel({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const draft = useAppStore((state) =>
    state.linkDraft?.documentId === documentId ? state.linkDraft : null,
  );
  return (
    <div className={styles.panel} data-testid="link-panel">
      {draft ? (
        <LinkForm
          // A new draft starts the form afresh.
          key={`${draft.page}:${draft.index ?? "new"}:${draft.rects.map((r) => `${r.x},${r.y}`).join(";")}`}
          draft={draft}
        />
      ) : (
        <p className={styles.hint}>{t("links.hint")}</p>
      )}
    </div>
  );
}

function LinkForm({ draft }: { draft: LinkDraft }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const pageCount = useAppStore((state) => selectActiveDocument(state)?.pageCount ?? 1);
  const destinations = useAppStore((state) =>
    state.destinations?.documentId === draft.documentId ? state.destinations.list : null,
  );
  const permitted = useAppStore((state) => isCommandEnabled("link.save", state));
  const target = draft.target;
  const [kind, setKind] = useState<Kind>(target.kind === "other" ? "url" : target.kind);
  const [url, setUrl] = useState(target.kind === "url" ? target.url : "");
  const [page, setPage] = useState(
    String(target.kind === "page" ? target.page + 1 : Math.min(draft.page + 2, pageCount)),
  );
  const [name, setName] = useState(target.kind === "named" ? target.name : "");
  const [invalid, setInvalid] = useState(false);
  const id = useId();

  const chosen = (): LinkTarget | null => {
    if (kind === "url") {
      const text = url.trim();
      if (text === "" || /\s/.test(text)) return null;
      return { kind: "url", url: normaliseUrl(text) };
    }
    if (kind === "page") {
      // Arabic-Indic digits typed in an Arabic UI count as digits.
      const western = page.replace(/[٠-٩]/g, (d) => String(d.charCodeAt(0) - 0x0660));
      const number = Number.parseInt(western, 10);
      return Number.isInteger(number) && number >= 1 && number <= pageCount
        ? { kind: "page", page: number - 1 }
        : null;
    }
    return name !== "" && destinations?.some((d) => d.name === name)
      ? { kind: "named", name }
      : null;
  };

  const save = (event: FormEvent) => {
    event.preventDefault();
    const next = chosen();
    setInvalid(next === null);
    if (next) void runCommand("link.save", { linkTarget: next });
  };

  return (
    <form className={styles.section} onSubmit={save} aria-labelledby={`${id}-heading`}>
      <h3 id={`${id}-heading`} className={styles.heading}>
        {t(draft.index === null ? "links.newHeading" : "links.editHeading")}
      </h3>
      {target.kind === "other" ? (
        <p className={styles.note} role="note">
          {describeTarget(target, t, locale)} {t("links.otherReplace")}
        </p>
      ) : null}
      <fieldset className={styles.kinds} aria-label={t("links.kind")}>
        {(["url", "page", "named"] as const).map((choice) => (
          <label key={choice} className={styles.kind}>
            <input
              type="radio"
              name={`${id}-kind`}
              value={choice}
              checked={kind === choice}
              disabled={choice === "named" && (destinations?.length ?? 0) === 0}
              onChange={() => {
                setKind(choice);
                setInvalid(false);
              }}
            />
            {t(`links.kinds.${choice}`)}
          </label>
        ))}
      </fieldset>
      {kind === "url" ? (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={`${id}-url`}>
            {t("links.url")}
          </label>
          <input
            id={`${id}-url`}
            className={`${styles.input} ${styles.url}`}
            type="text"
            inputMode="url"
            dir="ltr"
            autoComplete="off"
            spellCheck={false}
            value={url}
            placeholder="https://"
            onChange={(event) => setUrl(event.target.value)}
            data-testid="link-url"
          />
        </div>
      ) : kind === "page" ? (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={`${id}-page`}>
            {t("links.page", { last: formatNumber(pageCount, locale) })}
          </label>
          <input
            id={`${id}-page`}
            className={styles.input}
            type="text"
            inputMode="numeric"
            value={page}
            onChange={(event) => setPage(event.target.value)}
            data-testid="link-page"
          />
        </div>
      ) : (
        <div className={styles.field}>
          <label className={styles.label} htmlFor={`${id}-named`}>
            {t("links.named")}
          </label>
          <select
            id={`${id}-named`}
            className={styles.input}
            value={name}
            onChange={(event) => setName(event.target.value)}
            data-testid="link-named"
          >
            <option value="">{t("links.chooseNamed")}</option>
            {(destinations ?? []).map((destination) => (
              <option key={destination.name} value={destination.name}>
                {destination.page === null
                  ? destination.name
                  : `${destination.name} · ${t("links.toPage", { page: formatNumber(destination.page + 1, locale) })}`}
              </option>
            ))}
          </select>
        </div>
      )}
      {invalid ? (
        <p className={styles.error} role="alert">
          {t(`links.invalid.${kind}`)}
        </p>
      ) : null}
      <div className={styles.row}>
        <Button type="submit" variant="primary" icon={<Check />} isDisabled={!permitted}>
          {t(draft.index === null ? "links.add" : "links.save")}
        </Button>
        <Button icon={<X />} onPress={() => void runCommand("link.cancel")}>
          {t("commands.link.cancel")}
        </Button>
        {draft.index !== null ? (
          <Button
            variant="ghost"
            icon={<Trash2 />}
            isDisabled={!permitted}
            onPress={() => void runCommand("link.delete")}
          >
            {t("commands.link.delete")}
          </Button>
        ) : null}
      </div>
    </form>
  );
}

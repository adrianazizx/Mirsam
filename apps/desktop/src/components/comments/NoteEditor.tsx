import { Bold, Italic, Trash2 } from "lucide-react";
import { type KeyboardEvent, useEffect, useRef, useState } from "react";
import { ToggleButton, Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand, shortcutLabel } from "../../commands/registry";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { getViewer } from "../../viewer/registry";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { Tooltip } from "../ui/Tooltip";
import styles from "./NoteEditor.module.css";
import { marksOfRuns, type StyledValue } from "./richText";
import { StyledField } from "./StyledField";

/** Whether the selection in the paragraph field is bold and italic (for the buttons). */
function useFormatState(active: boolean): { bold: boolean; italic: boolean } {
  const [state, setState] = useState({ bold: false, italic: false });
  useEffect(() => {
    if (!active) return;
    const update = () => {
      const field = document.querySelector('[data-testid="block-field"]');
      const node = document.getSelection()?.anchorNode ?? null;
      if (!field || !node || !field.contains(node)) return;
      const query = (kind: string) => {
        try {
          return document.queryCommandState(kind);
        } catch {
          return false;
        }
      };
      setState({ bold: query("bold"), italic: query("italic") });
    };
    document.addEventListener("selectionchange", update);
    document.addEventListener("input", update, true);
    return () => {
      document.removeEventListener("selectionchange", update);
      document.removeEventListener("input", update, true);
    };
  }, [active]);
  return state;
}

/**
 * Writes a sticky note's or a text box's text (ADRs 0018, 0019): a new one's,
 * or the selected one's. The text field takes its direction from what is typed (`dir="auto"`),
 * so Arabic and English notes each read correctly in either UI language.
 *
 * Pressing anywhere outside it keeps what was written, as other editors do
 * with notes; Escape cancels (on a placed one, it drops what was typed and
 * returns to the page, the item still selected, so the arrow keys move it),
 * Cmd/Ctrl+Enter saves.
 */
export function NoteEditor({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const note = useAppStore((state) => {
    const selected = state.documents.find((open) => open.id === documentId)?.selectedAnnotation;
    return selected?.kind === "note" ||
      selected?.kind === "textBox" ||
      selected?.kind === "fillText"
      ? selected
      : null;
  });
  const pending = useAppStore((state) =>
    state.pendingNote?.documentId === documentId ? state.pendingNote : null,
  );
  const block = pending?.kind === "block" ? (pending.block ?? null) : null;
  const initial = block?.text ?? note?.contents ?? "";
  const kind =
    pending?.kind ??
    (note?.kind === "textBox" ? "textBox" : note?.kind === "fillText" ? "fillText" : "note");
  const [text, setText] = useState(initial);
  // A paragraph of page text: its own font where it can, or a bundled family (ADR 0023).
  const [family, setFamily] = useState<"auto" | "sans" | "naskh">("auto");
  const blockFamily = family === "auto" ? undefined : family;
  const which = pending
    ? `new@${pending.page}:${pending.x}:${pending.y}`
    : `${note?.page}:${note?.index}`;
  // A paragraph keeps its bold and italic in a rich field (ADR 0025).
  const [styled, setStyled] = useState<StyledValue>({ text: initial, marks: [] });
  const format = useFormatState(block !== null);
  const field = useRef<HTMLTextAreaElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: another note starts from its own text
  useEffect(() => {
    setText(initial);
    setFamily("auto");
    if (block) return;
    // On the next frame: the viewer shows the editor's anchor after this renders,
    // and a field in a hidden element cannot take focus.
    const frame = requestAnimationFrame(() => field.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [which]);

  /** What saving sends: a paragraph's text and marks, or a note's text. */
  const saved = () =>
    block
      ? {
          text: styled.text,
          blockMarks: styled.marks,
          ...(blockFamily ? { blockFamily } : {}),
        }
      : { text, ...(blockFamily ? { blockFamily } : {}) };
  const card = useRef<HTMLDivElement>(null);
  const latest = useRef(saved);
  latest.current = saved;
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && card.current?.contains(event.target)) return;
      // A press on the item itself, or on its handle, moves or resizes it: it stays selected.
      const keepSelected =
        getViewer(documentId)?.grabsSelected(event.clientX, event.clientY) ?? false;
      void runCommand("note.save", { ...latest.current(), keepSelected });
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => window.removeEventListener("pointerdown", onPointerDown, true);
  }, [documentId]);
  const onKeys = (event: KeyboardEvent<HTMLElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      if ((note && !pending) || block) {
        // A placed one: what was typed is dropped, and the page takes the keys, the
        // item still selected, so the arrow keys move it; Escape again deselects it.
        setText(initial);
        void runCommand("note.toPage");
      } else {
        void runCommand("note.cancel");
      }
    } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
      event.preventDefault();
      void runCommand("note.save", saved());
    }
  };

  return (
    <div className={styles.band}>
      <div
        ref={card}
        className={styles.card}
        role="dialog"
        aria-label={t(`${kind}.label`)}
        dir={directionOf(locale)}
        data-testid="note-editor"
      >
        {block ? (
          <>
            <Toolbar className={styles.formatting} aria-label={t("block.formatting")}>
              {(["bold", "italic"] as const).map((kind) => (
                <Tooltip
                  key={kind}
                  label={t(`commands.text.${kind}`)}
                  shortcut={shortcutLabel(`text.${kind}`)}
                >
                  <ToggleButton
                    className={styles.format}
                    isSelected={format[kind]}
                    // The field keeps focus and its selection.
                    preventFocusOnPress
                    onChange={() => void runCommand(`text.${kind}`)}
                    aria-label={t(`commands.text.${kind}`)}
                    data-testid={`block-${kind}`}
                  >
                    <span className={styles.glyph} aria-hidden="true">
                      {kind === "bold" ? <Bold /> : <Italic />}
                    </span>
                  </ToggleButton>
                </Tooltip>
              ))}
            </Toolbar>
            <StyledField
              className={`${styles.text} ${styles.styled}`}
              initial={{ text: block.text, marks: marksOfRuns(block.runs) }}
              resetKey={which}
              dir={block.rtl ? "rtl" : "ltr"}
              label={t(`${kind}.text`)}
              placeholder={t(`${kind}.placeholder`)}
              onChange={setStyled}
              onKeyDown={onKeys}
            />
          </>
        ) : (
          <textarea
            ref={field}
            className={styles.text}
            value={text}
            dir="auto"
            aria-label={t(`${kind}.text`)}
            placeholder={t(`${kind}.placeholder`)}
            onChange={(event) => setText(event.target.value)}
            onKeyDown={onKeys}
          />
        )}
        {block ? (
          <div className={styles.font} data-testid="block-font">
            <p className={styles.fontNote}>
              {blockFamily || !block.fontReusable
                ? t("block.substituted", { font: block.font })
                : t("block.ownFont", { font: block.font })}
            </p>
            {block.styled ? <p className={styles.fontNote}>{t("block.styled")}</p> : null}
            <label className={styles.family}>
              {t("textStyle.family")}
              <select
                value={family}
                onChange={(event) => setFamily(event.target.value as typeof family)}
              >
                <option value="auto">{t("block.familyAuto")}</option>
                <option value="sans">{t("textStyle.sans")}</option>
                <option value="naskh">{t("textStyle.naskh")}</option>
              </select>
            </label>
          </div>
        ) : null}
        <div className={styles.actions}>
          {note ? (
            <CommandButton command="annotation.delete" tooltipPlacement="bottom">
              <Trash2 />
            </CommandButton>
          ) : null}
          <span className={styles.spacer} />
          <Button variant="ghost" onPress={() => void runCommand("note.cancel")}>
            {t("commands.note.cancel")}
          </Button>
          <Button variant="primary" onPress={() => void runCommand("note.save", saved())}>
            {t("commands.note.save")}
          </Button>
        </div>
      </div>
    </div>
  );
}

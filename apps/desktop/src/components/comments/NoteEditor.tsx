import { Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import styles from "./NoteEditor.module.css";

/**
 * Writes a sticky note's text (ADR 0018): a new note's, or the selected
 * note's. The text field takes its direction from what is typed (`dir="auto"`),
 * so Arabic and English notes each read correctly in either UI language.
 *
 * Pressing anywhere outside it keeps what was written, as other editors do
 * with notes; Escape cancels, Cmd/Ctrl+Enter saves.
 */
export function NoteEditor({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const note = useAppStore((state) => {
    const selected = state.documents.find((open) => open.id === documentId)?.selectedAnnotation;
    return selected?.kind === "note" ? selected : null;
  });
  const pending = useAppStore((state) =>
    state.pendingNote?.documentId === documentId ? state.pendingNote : null,
  );
  const initial = note?.contents ?? "";
  const [text, setText] = useState(initial);
  const which = pending
    ? `new@${pending.page}:${pending.x}:${pending.y}`
    : `${note?.page}:${note?.index}`;
  const field = useRef<HTMLTextAreaElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: another note starts from its own text
  useEffect(() => {
    setText(initial);
    // On the next frame: the viewer shows the editor's anchor after this renders,
    // and a field in a hidden element cannot take focus.
    const frame = requestAnimationFrame(() => field.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [which]);

  const card = useRef<HTMLDivElement>(null);
  const latest = useRef(text);
  latest.current = text;
  useEffect(() => {
    const onPointerDown = (event: PointerEvent) => {
      if (event.target instanceof Node && card.current?.contains(event.target)) return;
      void runCommand("note.save", { text: latest.current });
    };
    window.addEventListener("pointerdown", onPointerDown, true);
    return () => window.removeEventListener("pointerdown", onPointerDown, true);
  }, []);

  return (
    <div className={styles.band}>
      <div
        ref={card}
        className={styles.card}
        role="dialog"
        aria-label={t("note.label")}
        dir={directionOf(locale)}
        data-testid="note-editor"
      >
        <textarea
          ref={field}
          className={styles.text}
          value={text}
          dir="auto"
          aria-label={t("note.text")}
          placeholder={t("note.placeholder")}
          onChange={(event) => setText(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              void runCommand("note.cancel");
            } else if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
              event.preventDefault();
              void runCommand("note.save", { text });
            }
          }}
        />
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
          <Button variant="primary" onPress={() => void runCommand("note.save", { text })}>
            {t("commands.note.save")}
          </Button>
        </div>
      </div>
    </div>
  );
}

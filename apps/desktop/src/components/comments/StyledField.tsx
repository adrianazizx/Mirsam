import { type KeyboardEvent, useEffect, useRef } from "react";
import { fillField, readField, type StyledValue } from "./richText";

/**
 * The paragraph editor's field (ADR 0025): the browser's own rich-text
 * editing, held to plain text with bold and italic. Line breaks are `<br>`,
 * pasted text arrives plain, and formatting goes through the browser's own
 * commands (`text.bold`, `text.italic`), so its undo covers them.
 */
export function StyledField({
  initial,
  resetKey,
  dir,
  label,
  placeholder,
  className,
  onChange,
  onKeyDown,
}: {
  initial: StyledValue;
  /** Changes when the field should start again from `initial`. */
  resetKey: string;
  dir: "ltr" | "rtl";
  label: string;
  placeholder: string;
  className?: string;
  onChange(value: StyledValue): void;
  onKeyDown(event: KeyboardEvent<HTMLDivElement>): void;
}) {
  const field = useRef<HTMLDivElement>(null);
  const latestInitial = useRef(initial);
  latestInitial.current = initial;
  const latestChange = useRef(onChange);
  latestChange.current = onChange;

  // biome-ignore lint/correctness/useExhaustiveDependencies: another paragraph starts from its own text
  useEffect(() => {
    const element = field.current;
    if (!element) return;
    fillField(element, latestInitial.current);
    latestChange.current(readField(element));
    // On the next frame: the viewer shows the editor's anchor after this renders,
    // and a field in a hidden element cannot take focus.
    const frame = requestAnimationFrame(() => element.focus());
    return () => cancelAnimationFrame(frame);
  }, [resetKey]);

  const changed = () => {
    if (field.current) onChange(readField(field.current));
  };

  return (
    // biome-ignore lint/a11y/useSemanticElements: a rich-text field has no native element; role and label make it one
    <div
      ref={field}
      className={className}
      contentEditable
      suppressContentEditableWarning
      role="textbox"
      aria-multiline="true"
      aria-label={label}
      data-placeholder={placeholder}
      dir={dir}
      tabIndex={0}
      data-testid="block-field"
      onInput={changed}
      onKeyDown={(event) => {
        onKeyDown(event);
        if (event.defaultPrevented) return;
        // A line break, not a new block element.
        if (event.key === "Enter" && !event.metaKey && !event.ctrlKey && !event.altKey) {
          event.preventDefault();
          document.execCommand("insertLineBreak");
          changed();
        }
      }}
      onPaste={(event) => {
        // Pasted text arrives plain: its own fonts and colours are not the paragraph's.
        event.preventDefault();
        document.execCommand("insertText", false, event.clipboardData.getData("text/plain"));
        changed();
      }}
      onDrop={(event) => event.preventDefault()}
    />
  );
}

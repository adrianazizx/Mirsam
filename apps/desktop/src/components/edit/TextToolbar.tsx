import {
  Link,
  Pilcrow,
  PilcrowLeft,
  PilcrowRight,
  Shapes,
  TextAlignCenter,
  TextAlignEnd,
  TextAlignJustify,
  TextAlignStart,
  Type,
} from "lucide-react";
import type { CSSProperties, ReactNode } from "react";
import {
  Button as AriaButton,
  type Key,
  ListBox,
  ListBoxItem,
  Popover,
  Radio,
  RadioGroup,
  Select,
  SelectValue,
  ToggleButton,
  ToggleButtonGroup,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { TextStyle } from "../../backend";
import { type CommandId, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { DOCUMENT_COLORS } from "../../state/colors";
import type { EditMode } from "../../state/objects";
import selectStyles from "../ui/CommandSelect.module.css";
import { Tooltip } from "../ui/Tooltip";
import styles from "./TextToolbar.module.css";

/** Sizes offered, in points. */
const SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72];

/** Colours offered for new text (shared with Fill & Sign). */
export const TEXT_COLORS = DOCUMENT_COLORS;

const ALIGNS: { id: TextStyle["align"]; icon: ReactNode; mirror: boolean }[] = [
  { id: "start", icon: <TextAlignStart />, mirror: true },
  { id: "center", icon: <TextAlignCenter />, mirror: false },
  { id: "end", icon: <TextAlignEnd />, mirror: true },
  { id: "justify", icon: <TextAlignJustify />, mirror: false },
];

// Physical directions: these icons never mirror.
const DIRECTIONS: { id: TextStyle["direction"]; icon: ReactNode }[] = [
  { id: "auto", icon: <Pilcrow /> },
  { id: "rtl", icon: <PilcrowLeft /> },
  { id: "ltr", icon: <PilcrowRight /> },
];

/** The Edit tool's modes (ADR 0051): text, page objects, links. */
const MODES: { id: EditMode; command: CommandId; icon: ReactNode }[] = [
  { id: "text", command: "edit.textMode", icon: <Type /> },
  { id: "objects", command: "edit.objectsMode", icon: <Shapes /> },
  { id: "links", command: "edit.linksMode", icon: <Link /> },
];

const setStyle = (textStyle: Partial<TextStyle>) => void runCommand("text.setStyle", { textStyle });
const same = (a: readonly number[], b: readonly number[]) =>
  a.every((value, i) => Math.abs(value - (b[i] ?? 0)) < 1e-6);

/**
 * The Edit tool's strip (ADR 0021, ADR 0051): what a press on the page does —
 * text, page objects, or links — and, for text, how new page text is set:
 * the font, size, colour, alignment and direction apply to text added next.
 */
export function TextToolbar() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const style = useAppStore((state) => state.textStyle);
  const editMode = useAppStore((state) => state.editMode);
  const color = TEXT_COLORS.find((choice) => same(choice.rgb, style.color))?.id ?? "black";
  const single = (keys: Set<Key>) => [...keys][0];

  const modes = (
    <ToggleButtonGroup
      className={styles.group}
      selectionMode="single"
      disallowEmptySelection
      aria-label={t("editMode.label")}
      selectedKeys={[editMode]}
      onSelectionChange={(keys: Set<Key>) => {
        const next = MODES.find((mode) => mode.id === single(keys));
        if (next) void runCommand(next.command);
      }}
      data-testid="edit-modes"
    >
      {MODES.map(({ id, command, icon }) => (
        <ToggleButton key={id} id={id} className={`${styles.tool} ${styles.word}`}>
          <span className={styles.glyph} aria-hidden="true">
            {icon}
          </span>
          {t(`commands.${command}`)}
        </ToggleButton>
      ))}
    </ToggleButtonGroup>
  );
  if (editMode !== "text") {
    return (
      <div className={styles.bar} data-testid="text-toolbar">
        {modes}
        <p className={styles.hint}>{t(`editMode.hint.${editMode}`)}</p>
      </div>
    );
  }

  return (
    <div className={styles.bar} data-testid="text-toolbar">
      {modes}
      <ToggleButtonGroup
        className={styles.group}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("textStyle.family")}
        selectedKeys={[style.family]}
        onSelectionChange={(keys: Set<Key>) => {
          const next = single(keys);
          if (next === "sans" || next === "naskh") setStyle({ family: next });
        }}
      >
        {(["sans", "naskh"] as const).map((family) => (
          <ToggleButton key={family} id={family} className={`${styles.tool} ${styles.word}`}>
            {t(`textStyle.${family}`)}
          </ToggleButton>
        ))}
      </ToggleButtonGroup>
      <Select
        aria-label={t("textStyle.size")}
        value={style.size}
        onChange={(key: Key | null) => {
          if (typeof key === "number") setStyle({ size: key });
        }}
        className={selectStyles.select}
      >
        <AriaButton className={`${selectStyles.button} ${styles.size}`}>
          <SelectValue>
            {() => t("textStyle.points", { size: formatNumber(style.size, locale) })}
          </SelectValue>
        </AriaButton>
        <Popover placement="bottom" className={selectStyles.popover}>
          <ListBox className={selectStyles.list}>
            {SIZES.map((size) => (
              <ListBoxItem key={size} id={size} className={selectStyles.item}>
                {formatNumber(size, locale)}
              </ListBoxItem>
            ))}
          </ListBox>
        </Popover>
      </Select>
      <RadioGroup
        aria-label={t("textStyle.color")}
        orientation="horizontal"
        value={color}
        onChange={(id) => {
          const choice = TEXT_COLORS.find((candidate) => candidate.id === id);
          if (choice) setStyle({ color: [...choice.rgb] });
        }}
        className={styles.group}
      >
        {TEXT_COLORS.map((choice) => (
          <Radio
            key={choice.id}
            value={choice.id}
            aria-label={t(`textStyle.colors.${choice.id}`)}
            className={styles.swatch}
            style={
              {
                "--swatch": `rgb(${choice.rgb.map((c) => Math.round(c * 255)).join(" ")})`,
              } as CSSProperties
            }
          />
        ))}
      </RadioGroup>
      <ToggleButtonGroup
        className={styles.group}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("textStyle.align")}
        selectedKeys={[style.align]}
        onSelectionChange={(keys: Set<Key>) => {
          const next = ALIGNS.find((align) => align.id === single(keys));
          if (next) setStyle({ align: next.id });
        }}
      >
        {ALIGNS.map(({ id, icon, mirror }) => (
          <Tooltip key={id} label={t(`textStyle.aligns.${id}`)} placement="bottom">
            <ToggleButton id={id} aria-label={t(`textStyle.aligns.${id}`)} className={styles.tool}>
              <span className={mirror ? styles.mirrored : styles.glyph} aria-hidden="true">
                {icon}
              </span>
            </ToggleButton>
          </Tooltip>
        ))}
      </ToggleButtonGroup>
      <ToggleButtonGroup
        className={styles.group}
        selectionMode="single"
        disallowEmptySelection
        aria-label={t("textStyle.direction")}
        selectedKeys={[style.direction]}
        onSelectionChange={(keys: Set<Key>) => {
          const next = DIRECTIONS.find((direction) => direction.id === single(keys));
          if (next) setStyle({ direction: next.id });
        }}
      >
        {DIRECTIONS.map(({ id, icon }) => (
          <Tooltip key={id} label={t(`textStyle.directions.${id}`)} placement="bottom">
            <ToggleButton
              id={id}
              aria-label={t(`textStyle.directions.${id}`)}
              className={styles.tool}
            >
              <span className={styles.glyph} aria-hidden="true">
                {icon}
              </span>
            </ToggleButton>
          </Tooltip>
        ))}
      </ToggleButtonGroup>
    </div>
  );
}

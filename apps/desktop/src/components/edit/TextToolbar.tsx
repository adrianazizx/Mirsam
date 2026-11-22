import {
  Pilcrow,
  PilcrowLeft,
  PilcrowRight,
  TextAlignCenter,
  TextAlignEnd,
  TextAlignJustify,
  TextAlignStart,
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
import { runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import selectStyles from "../ui/CommandSelect.module.css";
import { Tooltip } from "../ui/Tooltip";
import styles from "./TextToolbar.module.css";

/** Sizes offered, in points. */
const SIZES = [8, 9, 10, 11, 12, 14, 16, 18, 20, 24, 28, 32, 36, 48, 72];

/**
 * Colours offered for new text. They are the document's colours (written into
 * the PDF), not the UI's, so they are data here rather than design tokens.
 */
export const TEXT_COLORS: { id: string; rgb: [number, number, number] }[] = [
  { id: "black", rgb: [0, 0, 0] },
  { id: "blue", rgb: [0.05, 0.28, 0.63] },
  { id: "red", rgb: [0.72, 0.11, 0.11] },
  { id: "green", rgb: [0.1, 0.45, 0.22] },
];

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

const setStyle = (textStyle: Partial<TextStyle>) => void runCommand("text.setStyle", { textStyle });
const same = (a: [number, number, number], b: [number, number, number]) =>
  a.every((value, i) => Math.abs(value - (b[i] ?? 0)) < 1e-6);

/**
 * The Edit tool's strip (ADR 0021): how new page text is set. A press on the
 * page places it; the font, size, colour, alignment and direction here apply
 * to text added next.
 */
export function TextToolbar() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const style = useAppStore((state) => state.textStyle);
  const color = TEXT_COLORS.find((choice) => same(choice.rgb, style.color))?.id ?? "black";
  const single = (keys: Set<Key>) => [...keys][0];

  return (
    <div className={styles.bar} data-testid="text-toolbar">
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
          if (choice) setStyle({ color: choice.rgb });
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

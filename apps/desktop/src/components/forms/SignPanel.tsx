import {
  ArrowDownToLine,
  ArrowUpToLine,
  CalendarDays,
  Check,
  Circle,
  type LucideIcon,
  MousePointer2,
  Trash2,
  Type,
  X,
} from "lucide-react";
import { type CSSProperties, useEffect } from "react";
import {
  type Key,
  Radio,
  RadioGroup,
  ToggleButton,
  ToggleButtonGroup,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { FILL_KINDS, type SignatureInfo } from "../../backend";
import { type CommandId, isCommandEnabled, runCommand } from "../../commands/registry";
import { type FillTool, type OpenDocument, useAppStore } from "../../state/appStore";
import { cssOf, DOCUMENT_COLORS } from "../../state/colors";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import { IconButton } from "../ui/IconButton";
import { Tooltip } from "../ui/Tooltip";
import { SignaturePreview } from "./SignaturePreview";
import styles from "./SignPanel.module.css";

/** The Fill & Sign tools, in a fixed order; "select" places nothing. */
const TOOLS: { id: FillTool | "select"; command: CommandId; icon: LucideIcon }[] = [
  { id: "select", command: "fill.select", icon: MousePointer2 },
  { id: "text", command: "fill.text", icon: Type },
  { id: "date", command: "fill.date", icon: CalendarDays },
  { id: "check", command: "fill.check", icon: Check },
  { id: "cross", command: "fill.cross", icon: X },
  { id: "dot", command: "fill.dot", icon: Circle },
];

/**
 * The Sign tool (ADR 0045): Fill & Sign on any page, interactive form or
 * not. Pick what a press on the page places (text, today's date, a tick, a
 * cross, a dot, or a kept signature); press an item placed before to select
 * it, then make it smaller or larger, change its text, or delete it.
 * Signatures and initials are drawn, typed, or taken from an image, and
 * kept on this device. Every control runs a command.
 */
export function SignPanel({ document }: { document: OpenDocument }) {
  const { t } = useTranslation();
  const fillTool = useAppStore((state) => state.fillTool);
  const fillSignature = useAppStore((state) => state.fillSignature);
  const fillColor = useAppStore((state) => state.fillColor);
  const signatures = useAppStore((state) => state.signatures);
  const selected = document.selectedAnnotation;
  // A file opened with its user password may not allow annotating (ADR 0043).
  const permitted = useAppStore((state) => isCommandEnabled("fill.text", state));
  // Certificate signatures need the right to fill in fields (ADR 0047).
  const certificatePermitted = useAppStore((state) => isCommandEnabled("certSign.open", state));
  const fillSelected = selected !== null && FILL_KINDS.includes(selected.kind);

  useEffect(() => {
    if (useAppStore.getState().signatures === null) void runCommand("signatures.load");
  }, []);

  const current = fillTool === "signature" ? null : (fillTool ?? "select");
  const full = (signatures ?? []).filter((signature) => !signature.initials);
  const initials = (signatures ?? []).filter((signature) => signature.initials);

  return (
    <div className={styles.panel} data-testid="sign-panel">
      <section className={styles.section} aria-labelledby="fill-heading">
        <h3 id="fill-heading" className={styles.heading}>
          {t("sign.fillHeading")}
        </h3>
        <ToggleButtonGroup
          aria-label={t("sign.tools")}
          selectionMode="single"
          selectedKeys={current ? [current] : []}
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
        <p className={styles.hint} data-testid="fill-hint">
          {t(permitted ? `sign.hint.${fillTool ?? "select"}` : "sign.notPermitted")}
        </p>
        <RadioGroup
          aria-label={t("sign.color")}
          aria-describedby="fill-color-hint"
          orientation="horizontal"
          value={fillColor}
          isDisabled={!permitted}
          onChange={(colorId) => void runCommand("fill.color", { colorId })}
          className={styles.tools}
          data-testid="fill-colors"
        >
          {DOCUMENT_COLORS.map((choice) => (
            <Radio
              key={choice.id}
              value={choice.id}
              aria-label={t(`textStyle.colors.${choice.id}`)}
              className={styles.swatch}
              style={{ "--swatch": cssOf(choice.rgb) } as CSSProperties}
            />
          ))}
        </RadioGroup>
        <p id="fill-color-hint" className={styles.hint}>
          {t("sign.colorHint")}
        </p>
      </section>

      {fillSelected ? (
        <section className={styles.section} aria-labelledby="fill-selected-heading">
          <h3 id="fill-selected-heading" className={styles.heading}>
            {t("sign.selectedHeading")}
          </h3>
          <div className={styles.row}>
            <Button onPress={() => void runCommand("fill.smaller")}>
              {t("commands.fill.smaller")}
            </Button>
            <Button onPress={() => void runCommand("fill.larger")}>
              {t("commands.fill.larger")}
            </Button>
            <CommandButton command="annotation.delete">
              <Trash2 />
            </CommandButton>
          </div>
          <div className={styles.row}>
            <CommandButton command="fill.toPreviousPage">
              <ArrowUpToLine />
            </CommandButton>
            <CommandButton command="fill.toNextPage">
              <ArrowDownToLine />
            </CommandButton>
          </div>
        </section>
      ) : null}

      <SignatureList
        heading={t("sign.signatures")}
        empty={t("sign.noSignatures")}
        add={{ command: "signature.new", label: t("commands.signature.new") }}
        list={full}
        loading={signatures === null}
        chosen={fillTool === "signature" ? fillSignature : null}
      />
      <SignatureList
        heading={t("sign.initials")}
        empty={t("sign.noInitials")}
        add={{ command: "signature.newInitials", label: t("commands.signature.newInitials") }}
        list={initials}
        loading={signatures === null}
        chosen={fillTool === "signature" ? fillSignature : null}
      />
      <p className={styles.hint}>{t("sign.keptLocally")}</p>

      <section className={styles.section} aria-labelledby="cert-sign-heading">
        <h3 id="cert-sign-heading" className={styles.heading}>
          {t("sign.certificateHeading")}
        </h3>
        <p className={styles.hint}>{t("sign.certificateHint")}</p>
        <div className={styles.row}>
          <Button
            isDisabled={!certificatePermitted}
            onPress={() => void runCommand("certSign.open")}
          >
            {t("commands.certSign.open")}
          </Button>
          <Button variant="ghost" onPress={() => void runCommand("view.showSignatures")}>
            {t("commands.view.showSignatures")}
          </Button>
        </div>
      </section>
    </div>
  );
}

function SignatureList({
  heading,
  empty,
  add,
  list,
  loading,
  chosen,
}: {
  heading: string;
  empty: string;
  add: { command: CommandId; label: string };
  list: SignatureInfo[];
  loading: boolean;
  chosen: number | null;
}) {
  const { t } = useTranslation();
  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>{heading}</h3>
      {list.length === 0 ? (
        <p className={styles.hint}>{loading ? t("sign.loading") : empty}</p>
      ) : (
        <ul className={styles.list}>
          {list.map((signature) => (
            <li key={signature.id} className={styles.item}>
              <button
                type="button"
                className={styles.signature}
                aria-pressed={chosen === signature.id}
                aria-label={t("sign.place", {
                  kind: t(`sign.kind.${signature.kind}`),
                })}
                onClick={() => void runCommand("fill.signature", { signature: signature.id })}
              >
                <SignaturePreview signature={signature} />
              </button>
              <IconButton
                label={t("commands.signature.delete")}
                onPress={() => void runCommand("signature.delete", { signature: signature.id })}
              >
                <Trash2 />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
      <Button onPress={() => void runCommand(add.command)}>{add.label}</Button>
    </section>
  );
}

import {
  ArrowDown,
  ArrowUp,
  BringToFront,
  Check,
  Crop,
  ImagePlus,
  Replace,
  RotateCcw,
  RotateCw,
  SendToBack,
  Trash2,
  Undo2,
  X,
} from "lucide-react";
import { Toolbar } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { isCommandEnabled, runCommand } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import { CommandButton } from "../ui/CommandButton";
import styles from "./EditPanels.module.css";

/**
 * The Edit tool's object mode (ADR 0051): place an image, and change the
 * object pressed on the page — move, resize and turn it, put it over or
 * under others, crop or replace an image, or delete it. Dragging and the
 * corner handles do the same on the page. Every control runs a command.
 */
export function ObjectsPanel({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const selected = useAppStore((state) =>
    state.selectedObject?.documentId === documentId ? state.selectedObject : null,
  );
  const cropping = useAppStore((state) => state.objectCrop !== null);
  const canInsert = useAppStore((state) => isCommandEnabled("object.insertImage", state));
  const number = (value: number) => formatNumber(value, locale, { maximumFractionDigits: 0 });
  const object = selected?.object ?? null;

  return (
    <div className={styles.panel} data-testid="objects-panel">
      <section className={styles.section}>
        <p className={styles.hint}>{t(object ? "objects.selectedHint" : "objects.hint")}</p>
        <Button
          icon={<ImagePlus />}
          isDisabled={!canInsert}
          onPress={() => void runCommand("object.insertImage")}
        >
          {t("commands.object.insertImage")}
        </Button>
      </section>
      {object ? (
        <section className={styles.section} aria-labelledby="object-heading">
          <h3 id="object-heading" className={styles.heading}>
            {t(`objects.kind.${object.kind}`)}
          </h3>
          <dl className={styles.facts}>
            <dt>{t("objects.position")}</dt>
            <dd>
              {t("objects.positionPair", { a: number(object.rect.x), b: number(object.rect.y) })}
            </dd>
            <dt>{t("objects.size")}</dt>
            <dd>
              {t("objects.pointsPair", {
                a: number(object.rect.width),
                b: number(object.rect.height),
              })}
            </dd>
            {object.image ? (
              <>
                <dt>{t("objects.pixels")}</dt>
                <dd>
                  {t("objects.pixelsValue", {
                    width: number(object.image.width),
                    height: number(object.image.height),
                  })}
                </dd>
              </>
            ) : null}
          </dl>
          {object.refusal ? (
            <p className={styles.note} role="note">
              {t(`objects.refusal.${object.refusal}`)}
            </p>
          ) : null}
          {cropping ? (
            <>
              <p className={styles.hint}>{t("objects.cropHint")}</p>
              <div className={styles.row}>
                <Button
                  variant="primary"
                  icon={<Check />}
                  onPress={() => void runCommand("object.applyCrop")}
                >
                  {t("commands.object.applyCrop")}
                </Button>
                <Button icon={<X />} onPress={() => void runCommand("object.cancelCrop")}>
                  {t("commands.object.cancelCrop")}
                </Button>
              </div>
            </>
          ) : (
            <>
              <Toolbar className={styles.tools} aria-label={t("objects.arrange")}>
                <CommandButton command="object.bringToFront">
                  <BringToFront />
                </CommandButton>
                <CommandButton command="object.bringForward">
                  <ArrowUp />
                </CommandButton>
                <CommandButton command="object.sendBackward">
                  <ArrowDown />
                </CommandButton>
                <CommandButton command="object.sendToBack">
                  <SendToBack />
                </CommandButton>
              </Toolbar>
              <Toolbar className={styles.tools} aria-label={t("objects.change")}>
                {/* Turning is physical: these icons do not mirror. */}
                <CommandButton command="object.rotateCounterclockwise">
                  <RotateCcw />
                </CommandButton>
                <CommandButton command="object.rotateClockwise">
                  <RotateCw />
                </CommandButton>
                {object.kind !== "text" ? (
                  <CommandButton command="object.crop">
                    <Crop />
                  </CommandButton>
                ) : null}
                {object.cropped ? (
                  <CommandButton command="object.uncrop">
                    <Undo2 />
                  </CommandButton>
                ) : null}
                {object.kind === "image" ? (
                  <CommandButton command="object.replaceImage">
                    <Replace />
                  </CommandButton>
                ) : null}
                <CommandButton command="object.delete">
                  <Trash2 />
                </CommandButton>
              </Toolbar>
            </>
          )}
        </section>
      ) : null}
    </div>
  );
}

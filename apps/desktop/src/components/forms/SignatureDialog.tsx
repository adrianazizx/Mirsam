import { useRef, useState } from "react";
import {
  Dialog,
  Heading,
  Modal,
  ModalOverlay,
  Tab,
  TabList,
  TabPanel,
  Tabs,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { StrokePoint } from "../../backend";
import { runCommand, SIGNATURE_PEN } from "../../commands/registry";
import { directionOf } from "../../i18n";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./SignatureDialog.module.css";
import { strokePaths } from "./SignaturePreview";

/** Pressure when the pointer reports none (a mouse). */
const MOUSE_PRESSURE = 0.5;

/**
 * Makes a new signature or initials (ADR 0045): drawn on a pad with a
 * mouse, pen, or finger; typed and set in a bundled family; or taken from a
 * PNG or JPEG picked through the system's file dialog. It is kept on this
 * device and becomes what the Sign tool places next.
 */
export function SignatureDialog() {
  const { t } = useTranslation();
  const dialog = useAppStore((state) => state.signatureDialog);
  if (!dialog) return null;
  const title = t(dialog.initials ? "signature.titleInitials" : "signature.title");
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) void runCommand("signature.cancel");
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="signature-dialog">
          <Heading slot="title" className={styles.title}>
            {title}
          </Heading>
          <Tabs className={styles.tabs}>
            <TabList aria-label={t("signature.how")} className={styles.tabList}>
              <Tab id="draw" className={styles.tab}>
                {t("signature.draw")}
              </Tab>
              <Tab id="type" className={styles.tab}>
                {t("signature.type")}
              </Tab>
              <Tab id="image" className={styles.tab}>
                {t("signature.image")}
              </Tab>
            </TabList>
            <TabPanel id="draw" className={styles.panel}>
              <DrawPad />
            </TabPanel>
            <TabPanel id="type" className={styles.panel}>
              <TypeField initials={dialog.initials} />
            </TabPanel>
            <TabPanel id="image" className={styles.panel}>
              <p className={styles.hint}>{t("signature.imageHint")}</p>
              <div className={styles.actions}>
                <Button variant="primary" onPress={() => void runCommand("signature.importImage")}>
                  {t("commands.signature.importImage")}
                </Button>
                <Button variant="ghost" onPress={() => void runCommand("signature.cancel")}>
                  {t("signature.cancel")}
                </Button>
              </div>
            </TabPanel>
          </Tabs>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** A pad to draw on: strokes in its own pixels, y down, with the pen's pressure. */
function DrawPad() {
  const { t } = useTranslation();
  const pad = useRef<SVGSVGElement>(null);
  const [strokes, setStrokes] = useState<StrokePoint[][]>([]);
  const drawing = useRef<number | null>(null);

  const pointAt = (event: React.PointerEvent): StrokePoint | null => {
    const box = pad.current?.getBoundingClientRect();
    if (!box) return null;
    return {
      x: Math.round((event.clientX - box.left) * 10) / 10,
      y: Math.round((event.clientY - box.top) * 10) / 10,
      pressure: event.pointerType === "mouse" || !event.pressure ? MOUSE_PRESSURE : event.pressure,
    };
  };

  return (
    <div className={styles.draw}>
      <svg
        ref={pad}
        className={styles.pad}
        role="img"
        aria-label={t("signature.pad")}
        data-testid="signature-pad"
        onPointerDown={(event) => {
          const point = pointAt(event);
          if (!point) return;
          event.preventDefault();
          drawing.current = event.pointerId;
          event.currentTarget.setPointerCapture?.(event.pointerId);
          setStrokes((current) => [...current, [point]]);
        }}
        onPointerMove={(event) => {
          if (drawing.current !== event.pointerId) return;
          const point = pointAt(event);
          if (!point) return;
          setStrokes((current) => {
            const last = current.at(-1);
            return last ? [...current.slice(0, -1), [...last, point]] : current;
          });
        }}
        onPointerUp={(event) => {
          if (drawing.current === event.pointerId) drawing.current = null;
        }}
        onPointerCancel={() => {
          drawing.current = null;
        }}
      >
        <line className={styles.baseline} x1="5%" x2="95%" y1="78%" y2="78%" />
        {strokePaths(strokes.map((stroke) => stroke.map((p): [number, number] => [p.x, p.y]))).map(
          (d) => (
            <path key={d} d={d} className={styles.stroke} strokeWidth={SIGNATURE_PEN} />
          ),
        )}
      </svg>
      <p className={styles.hint}>{t("signature.drawHint")}</p>
      <div className={styles.actions}>
        <Button
          variant="primary"
          isDisabled={strokes.length === 0}
          onPress={() => void runCommand("signature.saveDrawn", { strokes })}
        >
          {t("signature.save")}
        </Button>
        <Button isDisabled={strokes.length === 0} onPress={() => setStrokes([])}>
          {t("signature.clear")}
        </Button>
        <Button variant="ghost" onPress={() => void runCommand("signature.cancel")}>
          {t("signature.cancel")}
        </Button>
      </div>
    </div>
  );
}

/** A typed signature: its text (either script) and the family it is set in. */
function TypeField({ initials }: { initials: boolean }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [text, setText] = useState("");
  const [family, setFamily] = useState<"naskh" | "sans">("naskh");
  const save = () => {
    if (text.trim() !== "") void runCommand("signature.saveTyped", { text, family });
  };
  return (
    <form
      className={styles.type}
      dir={directionOf(locale)}
      onSubmit={(event) => {
        event.preventDefault();
        save();
      }}
    >
      <label className={styles.label} htmlFor="typed-signature">
        {t(initials ? "signature.typedInitials" : "signature.typedName")}
      </label>
      <input
        id="typed-signature"
        className={styles.input}
        dir="auto"
        value={text}
        maxLength={200}
        // biome-ignore lint/a11y/noAutofocus: the dialog opens to type into it
        autoFocus
        onChange={(event) => setText(event.target.value)}
      />
      <label className={styles.label} htmlFor="typed-family">
        {t("textStyle.family")}
      </label>
      <select
        id="typed-family"
        className={styles.input}
        value={family}
        onChange={(event) => setFamily(event.target.value as "naskh" | "sans")}
      >
        <option value="naskh">{t("textStyle.naskh")}</option>
        <option value="sans">{t("textStyle.sans")}</option>
      </select>
      <p
        className={`${styles.typedPreview} ${family === "naskh" ? styles.naskh : ""}`}
        dir="auto"
        aria-hidden="true"
      >
        {text}
      </p>
      <div className={styles.actions}>
        <Button variant="primary" type="submit" isDisabled={text.trim() === ""}>
          {t("signature.save")}
        </Button>
        <Button variant="ghost" onPress={() => void runCommand("signature.cancel")}>
          {t("signature.cancel")}
        </Button>
      </div>
    </form>
  );
}

import { type ReactNode, useState } from "react";
import {
  Checkbox,
  Dialog,
  Heading,
  Input,
  Label,
  Modal,
  ModalOverlay,
  NumberField,
  Radio,
  RadioGroup,
  Text,
  TextField,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { Band, CompressReport, StampDigits, WatermarkMark } from "../../backend";
import { parsePageRanges } from "../../commands/pageRanges";
import { runCommand } from "../../commands/registry";
import { formatNumber, type Locale } from "../../i18n";
import {
  type OpenDocument,
  type OutputDialog as OutputDialogKind,
  selectActiveDocument,
  useAppStore,
} from "../../state/appStore";
import { TEXT_COLORS } from "../edit/TextToolbar";
import { Button } from "../ui/Button";
import styles from "./OutputDialog.module.css";

/** Light grey for watermarks: a document colour, kept as data like TEXT_COLORS. */
const WATERMARK_GREY: [number, number, number] = [0.6, 0.6, 0.6];
const WATERMARK_COLORS = [{ id: "grey", rgb: WATERMARK_GREY }, ...TEXT_COLORS];

/** Headers, footers, page numbers, watermarks, exports, and compression (ADR 0050). */
export function OutputDialog() {
  const kind = useAppStore((state) => state.outputDialog);
  const active = useAppStore((state) => selectActiveDocument(state));
  if (!kind || !active) return null;
  return <OutputForm key={`${kind}-${active.id}`} kind={kind} document={active} />;
}

type PageChoice = "all" | "current" | "range";

function OutputForm({ kind, document }: { kind: OutputDialogKind; document: OpenDocument }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const [pageChoice, setPageChoice] = useState<PageChoice>("all");
  const [range, setRange] = useState("");
  const cancel = () => void runCommand("output.cancel");

  const pages =
    pageChoice === "all"
      ? Array.from({ length: document.pageCount }, (_, i) => i)
      : pageChoice === "current"
        ? [document.page]
        : parsePageRanges(range, document.pageCount);

  const pagesField = (
    <PagesField
      choice={pageChoice}
      onChoice={setPageChoice}
      range={range}
      onRange={setRange}
      invalid={pageChoice === "range" && pages === null}
      pageCount={document.pageCount}
      locale={locale}
    />
  );

  let body: ReactNode;
  switch (kind) {
    case "headerFooter":
    case "pageNumbers":
      body = <BandForm kind={kind} pages={pages} pagesField={pagesField} />;
      break;
    case "watermark":
      body = <WatermarkForm pages={pages} pagesField={pagesField} />;
      break;
    case "exportImages":
      body = <ImagesForm pages={pages} pagesField={pagesField} />;
      break;
    case "exportText":
      body = <TextForm pages={pages} pagesField={pagesField} />;
      break;
    case "compress":
      body = <CompressForm documentId={document.id} />;
      break;
  }

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) cancel();
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid={`output-${kind}`}>
          <Heading slot="title" className={styles.title}>
            {t(`output.${kind}.title`)}
          </Heading>
          {body}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function PagesField(props: {
  choice: PageChoice;
  onChoice(choice: PageChoice): void;
  range: string;
  onRange(range: string): void;
  invalid: boolean;
  pageCount: number;
  locale: Locale;
}) {
  const { t } = useTranslation();
  return (
    <div className={styles.group}>
      <RadioGroup
        value={props.choice}
        onChange={(value) => props.onChoice(value as PageChoice)}
        className={styles.choices}
        orientation="horizontal"
      >
        <Label className={styles.label}>{t("output.pages")}</Label>
        <Radio value="all" className={styles.method}>
          {t("output.allPages", { count: props.pageCount })}
        </Radio>
        <Radio value="current" className={styles.method}>
          {t("output.currentPage")}
        </Radio>
        <Radio value="range" className={styles.method}>
          {t("output.range")}
        </Radio>
      </RadioGroup>
      {props.choice === "range" && (
        <TextField
          className={styles.field}
          value={props.range}
          onChange={props.onRange}
          isInvalid={props.invalid && props.range !== ""}
          autoFocus
        >
          <Label className={styles.label}>{t("output.rangeLabel")}</Label>
          {/* Digits and separators read left to right in either language. */}
          <Input className={styles.input} dir="ltr" placeholder={t("output.rangePlaceholder")} />
          <Text slot="description" className={styles.help}>
            {t("output.rangeHelp", {
              last: formatNumber(props.pageCount, props.locale),
            })}
          </Text>
        </TextField>
      )}
    </div>
  );
}

function Actions({ confirm, valid }: { confirm: string; valid: boolean }) {
  const { t } = useTranslation();
  return (
    <div className={styles.actions}>
      <Button variant="primary" type="submit" isDisabled={!valid}>
        {confirm}
      </Button>
      <Button variant="ghost" onPress={() => void runCommand("output.cancel")}>
        {t("output.cancel")}
      </Button>
    </div>
  );
}

function Choices<T extends string>(props: {
  label: string;
  value: T;
  onChange(value: NoInfer<T>): void;
  options: { id: NoInfer<T>; label: string }[];
}) {
  return (
    <RadioGroup
      value={props.value}
      onChange={(value) => props.onChange(value as T)}
      className={styles.choices}
      orientation="horizontal"
    >
      <Label className={styles.label}>{props.label}</Label>
      {props.options.map((option) => (
        <Radio key={option.id} value={option.id} className={styles.method}>
          {option.label}
        </Radio>
      ))}
    </RadioGroup>
  );
}

function NumberInput(props: {
  label: string;
  value: number;
  onChange(value: number): void;
  min: number;
  max: number;
  step?: number;
  percent?: boolean;
}) {
  const locale = useAppStore((state) => state.locale);
  const valid =
    Number.isFinite(props.value) && props.value >= props.min && props.value <= props.max;
  return (
    <NumberField
      className={styles.field}
      value={props.value}
      onChange={props.onChange}
      minValue={props.min}
      maxValue={props.max}
      step={props.step ?? 1}
      isInvalid={!valid}
      formatOptions={
        props.percent
          ? { style: "percent", maximumFractionDigits: 0 }
          : { maximumFractionDigits: 1 }
      }
    >
      <Label className={styles.label}>{props.label}</Label>
      <Input className={styles.input} lang={locale} />
    </NumberField>
  );
}

const inRange = (value: number, min: number, max: number) =>
  Number.isFinite(value) && value >= min && value <= max;

type Slot = "start" | "center" | "end";

function BandForm({
  kind,
  pages,
  pagesField,
}: {
  kind: "headerFooter" | "pageNumbers";
  pages: number[] | null;
  pagesField: ReactNode;
}) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const numbers = kind === "pageNumbers";
  const [edge, setEdge] = useState<Band["edge"]>(numbers ? "bottom" : "top");
  const [slots, setSlots] = useState<Record<Slot, string>>({ start: "", center: "", end: "" });
  const [slot, setSlot] = useState<Slot>("center");
  const [format, setFormat] = useState<"number" | "ofTotal" | "slash">("ofTotal");
  const [digits, setDigits] = useState<StampDigits>(locale === "ar" ? "arabicIndic" : "western");
  const [rtl, setRtl] = useState(locale === "ar");
  const [family, setFamily] = useState<Band["family"]>(locale === "ar" ? "naskh" : "sans");
  const [size, setSize] = useState(10);
  const [first, setFirst] = useState(1);

  // Page numbers fill one slot with the chosen format, in the UI's language.
  const numberText = {
    number: "{page}",
    ofTotal: t("output.pageNumbers.ofTotalText"),
    slash: "{page} / {pages}",
  }[format];
  const texts: Record<Slot, string> = numbers
    ? { start: "", center: "", end: "", [slot]: numberText }
    : slots;
  const band: Band = {
    edge,
    ...texts,
    rtl,
    family,
    size,
    color: [0, 0, 0],
    margin: 30,
    digits,
    firstNumber: first,
  };
  const valid =
    pages !== null &&
    Object.values(texts).some((text) => text.trim() !== "") &&
    inRange(size, 4, 72) &&
    inRange(first, 0, 1_000_000);

  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (valid && pages)
          void runCommand("output.stamp", { pages, stamp: { kind: "band", band } });
      }}
    >
      <Choices
        label={t("output.edge")}
        value={edge}
        onChange={setEdge}
        options={[
          { id: "top", label: t("output.top") },
          { id: "bottom", label: t("output.bottom") },
        ]}
      />
      {numbers ? (
        <>
          <Choices
            label={t("output.position")}
            value={slot}
            onChange={setSlot}
            options={[
              { id: "start", label: t("output.start") },
              { id: "center", label: t("output.center") },
              { id: "end", label: t("output.end") },
            ]}
          />
          <Choices
            label={t("output.pageNumbers.format")}
            value={format}
            onChange={setFormat}
            options={[
              { id: "number", label: t("output.pageNumbers.number") },
              { id: "ofTotal", label: t("output.pageNumbers.ofTotal") },
              { id: "slash", label: t("output.pageNumbers.slash") },
            ]}
          />
          <NumberInput
            label={t("output.firstNumber")}
            value={first}
            onChange={setFirst}
            min={0}
            max={1_000_000}
          />
        </>
      ) : (
        <div className={styles.slots}>
          {(["start", "center", "end"] as const).map((id) => (
            <TextField
              key={id}
              className={styles.field}
              value={slots[id]}
              onChange={(value) => setSlots({ ...slots, [id]: value })}
            >
              <Label className={styles.label}>{t(`output.${id}`)}</Label>
              {/* Each piece takes the direction of its own text. */}
              <Input className={styles.input} dir="auto" />
            </TextField>
          ))}
          <p className={styles.help}>{t("output.headerFooter.tokens")}</p>
        </div>
      )}
      <Choices
        label={t("output.direction")}
        value={rtl ? "rtl" : "ltr"}
        onChange={(value) => setRtl(value === "rtl")}
        options={[
          { id: "ltr", label: t("output.ltr") },
          { id: "rtl", label: t("output.rtl") },
        ]}
      />
      <Choices
        label={t("output.digits")}
        value={digits}
        onChange={setDigits}
        options={[
          { id: "western", label: t("output.western") },
          { id: "arabicIndic", label: t("output.arabicIndic") },
        ]}
      />
      <div className={styles.row}>
        <Choices
          label={t("output.font")}
          value={family}
          onChange={setFamily}
          options={[
            { id: "sans", label: t("textStyle.sans") },
            { id: "naskh", label: t("textStyle.naskh") },
          ]}
        />
        <NumberInput label={t("output.size")} value={size} onChange={setSize} min={4} max={72} />
      </div>
      {pagesField}
      <Actions confirm={t("output.apply")} valid={valid} />
    </form>
  );
}

function WatermarkForm({ pages, pagesField }: { pages: number[] | null; pagesField: ReactNode }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const image = useAppStore((state) => state.watermarkImage);
  const [kind, setKind] = useState<WatermarkMark["kind"]>("text");
  const [text, setText] = useState(t("output.watermark.default"));
  const [size, setSize] = useState(60);
  const [color, setColor] = useState("grey");
  const [width, setWidth] = useState(0.5);
  const [opacity, setOpacity] = useState(0.3);
  const [rotation, setRotation] = useState(45);
  const [behind, setBehind] = useState(false);
  const rgb = WATERMARK_COLORS.find((choice) => choice.id === color)?.rgb ?? WATERMARK_GREY;
  const mark: WatermarkMark =
    kind === "text"
      ? { kind, text, family: locale === "ar" ? "naskh" : "sans", size, color: rgb }
      : { kind, width };
  const valid =
    pages !== null &&
    inRange(opacity, 0.05, 1) &&
    inRange(rotation, -180, 180) &&
    (kind === "text"
      ? text.trim() !== "" && inRange(size, 8, 300)
      : image !== null && inRange(width, 0.05, 1));

  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (valid && pages) {
          void runCommand("output.stamp", {
            pages,
            stamp: { kind: "watermark", watermark: { mark, opacity, rotation, behind } },
          });
        }
      }}
    >
      <Choices
        label={t("output.watermark.kind")}
        value={kind}
        onChange={setKind}
        options={[
          { id: "text", label: t("output.watermark.text") },
          { id: "image", label: t("output.watermark.image") },
        ]}
      />
      {kind === "text" ? (
        <>
          <TextField className={styles.field} value={text} onChange={setText} autoFocus>
            <Label className={styles.label}>{t("output.watermark.textLabel")}</Label>
            <Input className={styles.input} dir="auto" />
          </TextField>
          <div className={styles.row}>
            <NumberInput
              label={t("output.size")}
              value={size}
              onChange={setSize}
              min={8}
              max={300}
            />
            <Choices
              label={t("output.color")}
              value={color}
              onChange={setColor}
              options={WATERMARK_COLORS.map((choice) => ({
                id: choice.id,
                label: t(`output.colors.${choice.id}`),
              }))}
            />
          </div>
        </>
      ) : (
        <div className={styles.group}>
          <div className={styles.row}>
            <Button variant="secondary" onPress={() => void runCommand("output.pickImage")}>
              {t("output.watermark.choose")}
            </Button>
            <span className={styles.help} data-testid="watermark-image">
              {image ? <bdi>{image}</bdi> : t("output.watermark.noImage")}
            </span>
          </div>
          <NumberInput
            label={t("output.watermark.width")}
            value={width}
            onChange={setWidth}
            min={0.05}
            max={1}
            step={0.05}
            percent
          />
        </div>
      )}
      <div className={styles.row}>
        <NumberInput
          label={t("output.watermark.opacity")}
          value={opacity}
          onChange={setOpacity}
          min={0.05}
          max={1}
          step={0.05}
          percent
        />
        <NumberInput
          label={t("output.watermark.rotation")}
          value={rotation}
          onChange={setRotation}
          min={-180}
          max={180}
          step={15}
        />
      </div>
      <Checkbox className={styles.check} isSelected={behind} onChange={setBehind}>
        <span className={styles.box} aria-hidden="true" />
        {t("output.watermark.behind")}
      </Checkbox>
      {pagesField}
      <Actions confirm={t("output.apply")} valid={valid} />
    </form>
  );
}

function ImagesForm({ pages, pagesField }: { pages: number[] | null; pagesField: ReactNode }) {
  const { t } = useTranslation();
  const [format, setFormat] = useState<"png" | "jpeg">("png");
  const [dpi, setDpi] = useState(150);
  const [quality, setQuality] = useState(85);
  const valid = pages !== null && inRange(dpi, 36, 600) && inRange(quality, 1, 100);
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid || !pages) return;
        void runCommand("output.exportImages", {
          pages,
          dpi,
          imageFormat: format === "png" ? { kind: "png" } : { kind: "jpeg", quality },
        });
      }}
    >
      <Choices
        label={t("output.exportImages.format")}
        value={format}
        onChange={setFormat}
        options={[
          { id: "png", label: "PNG" },
          { id: "jpeg", label: "JPEG" },
        ]}
      />
      <div className={styles.row}>
        <NumberInput
          label={t("output.exportImages.dpi")}
          value={dpi}
          onChange={setDpi}
          min={36}
          max={600}
          step={1}
        />
        {format === "jpeg" && (
          <NumberInput
            label={t("output.exportImages.quality")}
            value={quality}
            onChange={setQuality}
            min={1}
            max={100}
            step={1}
          />
        )}
      </div>
      {pagesField}
      <Actions confirm={t("output.export")} valid={valid} />
    </form>
  );
}

function TextForm({ pages, pagesField }: { pages: number[] | null; pagesField: ReactNode }) {
  const { t } = useTranslation();
  const [format, setFormat] = useState<"plain" | "markdown">("plain");
  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (pages) void runCommand("output.exportText", { pages, textFormat: format });
      }}
    >
      <Choices
        label={t("output.exportText.format")}
        value={format}
        onChange={setFormat}
        options={[
          { id: "plain", label: t("output.exportText.plain") },
          { id: "markdown", label: t("output.exportText.markdown") },
        ]}
      />
      <p className={styles.help}>{t("output.exportText.help")}</p>
      {pagesField}
      <Actions confirm={t("output.export")} valid={pages !== null} />
    </form>
  );
}

function CompressForm({ documentId }: { documentId: number }) {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const compressed = useAppStore((state) =>
    state.compressed?.documentId === documentId ? state.compressed.report : null,
  );
  const [reduce, setReduce] = useState(true);
  const [dpi, setDpi] = useState(150);
  const [jpeg, setJpeg] = useState(false);
  const [objectStreams, setObjectStreams] = useState(true);
  const [busy, setBusy] = useState(false);
  const valid = !busy && (!reduce || inRange(dpi, 36, 1200));
  const changed = () => useAppStore.getState().setCompressed(null);

  return (
    <form
      className={styles.form}
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid) return;
        setBusy(true);
        void runCommand("output.compress", {
          compress: {
            imageDpi: reduce ? dpi : null,
            jpegQuality: reduce && jpeg ? 75 : null,
            objectStreams,
          },
        }).finally(() => setBusy(false));
      }}
    >
      <Checkbox
        className={styles.check}
        isSelected={reduce}
        onChange={(value) => {
          setReduce(value);
          changed();
        }}
      >
        <span className={styles.box} aria-hidden="true" />
        {t("output.compress.reduce")}
      </Checkbox>
      {reduce && (
        <div className={styles.row}>
          <NumberInput
            label={t("output.compress.dpi")}
            value={dpi}
            onChange={(value) => {
              setDpi(value);
              changed();
            }}
            min={36}
            max={1200}
            step={1}
          />
          <Checkbox
            className={styles.check}
            isSelected={jpeg}
            onChange={(value) => {
              setJpeg(value);
              changed();
            }}
          >
            <span className={styles.box} aria-hidden="true" />
            {t("output.compress.jpeg")}
          </Checkbox>
        </div>
      )}
      <Checkbox
        className={styles.check}
        isSelected={objectStreams}
        onChange={(value) => {
          setObjectStreams(value);
          changed();
        }}
      >
        <span className={styles.box} aria-hidden="true" />
        {t("output.compress.objectStreams")}
      </Checkbox>
      <p className={styles.help}>{t("output.compress.help")}</p>
      {compressed && <CompressResult report={compressed} locale={locale} />}
      <div className={styles.actions}>
        {compressed ? (
          <Button variant="primary" onPress={() => void runCommand("output.saveCompressed")}>
            {t("output.compress.save")}
          </Button>
        ) : (
          <Button variant="primary" type="submit" isDisabled={!valid}>
            {busy ? t("output.compress.working") : t("output.compress.run")}
          </Button>
        )}
        <Button variant="ghost" onPress={() => void runCommand("output.cancel")}>
          {t("output.cancel")}
        </Button>
      </div>
    </form>
  );
}

function CompressResult({ report, locale }: { report: CompressReport; locale: Locale }) {
  const { t } = useTranslation();
  const size = (bytes: number) =>
    bytes >= 1_000_000
      ? formatNumber(bytes / 1_000_000, locale, {
          style: "unit",
          unit: "megabyte",
          maximumFractionDigits: 1,
        })
      : formatNumber(bytes / 1_000, locale, {
          style: "unit",
          unit: "kilobyte",
          maximumFractionDigits: 0,
        });
  const saved = report.before > 0 ? Math.max(0, 1 - report.after / report.before) : 0;
  return (
    <div className={styles.report} role="status" data-testid="compress-report">
      <p>
        {t("output.compress.sizes", {
          before: size(report.before),
          after: size(report.after),
          saved: formatNumber(saved, locale, { style: "percent", maximumFractionDigits: 0 }),
        })}
      </p>
      <p className={styles.help}>
        {t("output.compress.details", {
          images: formatNumber(report.imagesReduced, locale),
          objects: formatNumber(report.objectsRemoved, locale),
        })}
      </p>
      {report.imagesKept > 0 && (
        <p className={styles.help}>{t("output.compress.kept", { count: report.imagesKept })}</p>
      )}
    </div>
  );
}

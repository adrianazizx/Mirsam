import {
  Command,
  Download,
  FolderOpen,
  History,
  Languages,
  Laptop,
  type LucideIcon,
  PanelTop,
  ScanText,
  ShieldCheck,
} from "lucide-react";
import { type ReactNode, useEffect, useId, useRef } from "react";
import {
  Dialog,
  Heading,
  Label,
  Modal,
  ModalOverlay,
  Radio,
  RadioGroup,
  Text,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand, shortcutLabel } from "../../commands/registry";
import { formatNumber } from "../../i18n";
import { TOOLS, useAppStore } from "../../state/appStore";
import { GUIDE_STEPS, type GuideStep, useOnboarding } from "../../state/onboarding";
import { TOOL_ICONS } from "../shell/ToolRail";
import { Button } from "../ui/Button";
import { useKeyboardScrollable } from "../ui/useKeyboardScrollable";
import styles from "./OnboardingGuide.module.css";

/**
 * The first-run guide (M7): four short steps on opening a PDF, the tool rail,
 * Arabic and English with the digits setting, and OCR staying on this
 * computer. It opens by itself once, on an empty window, until finished or
 * skipped (Escape skips); Help → Welcome guide in the command palette opens it
 * again. Every button runs a registry command.
 *
 * Moving between steps keeps focus on the button pressed and announces the new
 * step (`onboarding.next`/`.back`), so pressing Enter on "Next" walks through.
 */
export function OnboardingGuide() {
  const { t } = useTranslation();
  const open = useOnboarding((state) => state.guideOpen);
  const step = useOnboarding((state) => state.step);
  const locale = useAppStore((state) => state.locale);
  const titleId = useId();
  const stepTitleId = useId();
  const primary = useRef<HTMLButtonElement>(null);
  const dialog = useRef<HTMLElement>(null);

  // A step change that removed the focused button ("Back" on the first step,
  // "Skip" on the last) hands focus to the step's main button.
  // biome-ignore lint/correctness/useExhaustiveDependencies: runs on each step change
  useEffect(() => {
    if (!open) return;
    const focused = document.activeElement;
    if (!focused || focused === document.body || !dialog.current?.contains(focused)) {
      primary.current?.focus();
    }
  }, [open, step]);

  if (!open) return null;
  const current: GuideStep = GUIDE_STEPS[step] ?? "open";
  const last = step === GUIDE_STEPS.length - 1;

  return (
    <ModalOverlay
      isOpen
      // Escape skips; a click outside does nothing (it is too easy to do by accident).
      isDismissable={false}
      onOpenChange={(isOpen) => {
        if (!isOpen) void runCommand("onboarding.skip");
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog
          ref={dialog}
          className={styles.dialog}
          aria-labelledby={`${titleId} ${stepTitleId}`}
          data-testid="onboarding-guide"
        >
          <div className={styles.header}>
            <Heading slot="title" id={titleId} className={styles.title}>
              {t("onboarding.title")}
            </Heading>
            <p className={styles.progress}>
              {t("onboarding.stepOf", {
                step: formatNumber(step + 1, locale),
                count: formatNumber(GUIDE_STEPS.length, locale),
              })}
            </p>
          </div>
          <ol className={styles.dots} aria-hidden="true">
            {GUIDE_STEPS.map((id, index) => (
              <li key={id} className={styles.dot} data-current={index === step ? "" : undefined} />
            ))}
          </ol>
          <StepSection key={current} step={current} titleId={stepTitleId} />
          <div className={styles.footer}>
            {last ? null : (
              <Button variant="ghost" onPress={() => void runCommand("onboarding.skip")}>
                {t("onboarding.skip")}
              </Button>
            )}
            <span className={styles.spacer} />
            {step > 0 ? (
              <Button onPress={() => void runCommand("onboarding.back")}>
                {t("onboarding.back")}
              </Button>
            ) : null}
            {last ? (
              <Button
                onPress={async () => {
                  await runCommand("onboarding.finish");
                  await runCommand("file.open");
                }}
              >
                {t("commands.file.open")}
              </Button>
            ) : null}
            <Button
              ref={primary}
              variant="primary"
              onPress={() => void runCommand(last ? "onboarding.finish" : "onboarding.next")}
            >
              {last ? t("onboarding.done") : t("onboarding.next")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/**
 * One step. At a large interface size it may scroll, and some steps have no
 * control inside: it then joins the tab order so the keyboard can scroll it
 * (WCAG 2.1.1; found by the Chrome check at interface size 2).
 */
function StepSection({ step, titleId }: { step: GuideStep; titleId: string }) {
  const { t } = useTranslation();
  const ref = useRef<HTMLElement>(null);
  useKeyboardScrollable(ref);
  return (
    <section ref={ref} className={styles.step} aria-labelledby={titleId}>
      <h3 id={titleId} className={styles.stepTitle}>
        {t(`onboarding.steps.${step}.title`)}
      </h3>
      <StepBody step={step} />
    </section>
  );
}

function StepBody({ step }: { step: GuideStep }) {
  switch (step) {
    case "open":
      return <OpenStep />;
    case "tools":
      return <ToolsStep />;
    case "language":
      return <LanguageStep />;
    case "ocr":
      return <OcrStep />;
  }
}

/** A point of a step: a decorative icon and its sentence. */
function Point({ icon: Icon, children }: { icon: LucideIcon; children: ReactNode }) {
  return (
    <li className={styles.point}>
      <Icon className={styles.pointIcon} aria-hidden="true" />
      <span>{children}</span>
    </li>
  );
}

function OpenStep() {
  const { t } = useTranslation();
  return (
    <ul className={styles.points}>
      <Point icon={FolderOpen}>
        {t("onboarding.steps.open.open", { shortcut: shortcutLabel("file.open") ?? "" })}
      </Point>
      <Point icon={History}>{t("onboarding.steps.open.recent")}</Point>
      <Point icon={PanelTop}>{t("onboarding.steps.open.tabs")}</Point>
    </ul>
  );
}

function ToolsStep() {
  const { t } = useTranslation();
  return (
    <>
      <p className={styles.text}>{t("onboarding.steps.tools.body")}</p>
      <ul className={styles.tools}>
        {TOOLS.map((tool) => {
          const Icon = TOOL_ICONS[tool];
          return (
            <li key={tool} className={styles.tool}>
              <Icon className={styles.toolIcon} aria-hidden="true" />
              <span>{t(`tools.${tool}`)}</span>
            </li>
          );
        })}
      </ul>
      <ul className={styles.points}>
        <Point icon={Command}>
          {t("onboarding.steps.tools.palette", {
            shortcut: shortcutLabel("app.commandPalette") ?? "",
          })}
        </Point>
      </ul>
    </>
  );
}

function LanguageStep() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const uiDigits = useAppStore((state) => state.uiDigits);
  return (
    <>
      <ul className={styles.points}>
        <Point icon={Languages}>{t("onboarding.steps.language.body")}</Point>
      </ul>
      <RadioGroup
        className={styles.group}
        orientation="horizontal"
        value={locale}
        onChange={(value) =>
          void runCommand(value === "ar" ? "app.languageArabic" : "app.languageEnglish")
        }
      >
        <Label className={styles.legend}>{t("onboarding.steps.language.language")}</Label>
        <div className={styles.choices}>
          <Radio value="en" className={styles.radio}>
            <span className={styles.radioDot} aria-hidden="true" />
            <span lang="en">English</span>
          </Radio>
          <Radio value="ar" className={styles.radio}>
            <span className={styles.radioDot} aria-hidden="true" />
            <span lang="ar">العربية</span>
          </Radio>
        </div>
      </RadioGroup>
      <RadioGroup
        className={styles.group}
        value={uiDigits}
        onChange={(value) =>
          void runCommand(value === "western" ? "app.digitsWestern" : "app.digitsArabicIndic")
        }
      >
        <Label className={styles.legend}>{t("onboarding.steps.language.digits")}</Label>
        <Radio value="arabicIndic" className={styles.radio}>
          <span className={styles.radioDot} aria-hidden="true" />
          {t("settings.digitsArabicIndic")}
        </Radio>
        <Radio value="western" className={styles.radio}>
          <span className={styles.radioDot} aria-hidden="true" />
          {t("settings.digitsWestern")}
        </Radio>
        <Text slot="description" className={styles.hint}>
          {t("onboarding.steps.language.later", {
            shortcut: shortcutLabel("app.settings") ?? "",
          })}
        </Text>
      </RadioGroup>
    </>
  );
}

function OcrStep() {
  const { t } = useTranslation();
  return (
    <ul className={styles.points}>
      <Point icon={ScanText}>{t("onboarding.steps.ocr.what")}</Point>
      <Point icon={Download}>{t("onboarding.steps.ocr.download")}</Point>
      <Point icon={Laptop}>{t("onboarding.steps.ocr.local")}</Point>
      <Point icon={ShieldCheck}>{t("onboarding.steps.ocr.private")}</Point>
    </ul>
  );
}

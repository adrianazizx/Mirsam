import { ChevronLeft, ChevronRight, FolderOpen, ZoomIn } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../components/ui/Button";
import { IconButton } from "../components/ui/IconButton";
import styles from "./Gallery.module.css";

const COLOURS = [
  "bg-app",
  "bg-surface",
  "bg-canvas",
  "bg-hover",
  "bg-pressed",
  "bg-selected",
  "border",
  "border-strong",
  "text",
  "text-muted",
  "accent",
  "accent-hover",
  "danger",
];

const SPACES = ["1", "2", "3", "4", "6", "8", "12"];
const SIZES = ["xs", "sm", "md", "lg", "xl"];

/**
 * Living reference for the design tokens and base components (Cmd/Ctrl+Shift+G).
 * It renders in whatever language, direction, and theme the app is in, so it
 * doubles as the review surface for LTR/RTL and light/dark.
 */
export function Gallery() {
  const { t } = useTranslation();
  return (
    <div className={styles.gallery} data-testid="gallery">
      <h1 className={styles.heading}>{t("gallery.title")}</h1>

      <section className={styles.section}>
        <h2>{t("gallery.colours")}</h2>
        <div className={styles.swatches}>
          {COLOURS.map((name) => (
            <div key={name} className={styles.swatch}>
              <span className={styles.chip} style={{ background: `var(--color-${name})` }} />
              <code dir="ltr">{name}</code>
            </div>
          ))}
        </div>
      </section>

      <section className={styles.section}>
        <h2>{t("gallery.type")}</h2>
        {SIZES.map((size) => (
          <p key={size} style={{ fontSize: `var(--font-size-${size})` }}>
            <code dir="ltr">{size}</code> · {t("gallery.typeSample")}
          </p>
        ))}
      </section>

      <section className={styles.section}>
        <h2>{t("gallery.buttons")}</h2>
        <div className={styles.row}>
          <Button variant="primary" icon={<FolderOpen />}>
            {t("gallery.primary")}
          </Button>
          <Button variant="secondary">{t("gallery.secondary")}</Button>
          <Button variant="ghost">{t("gallery.ghost")}</Button>
          <Button variant="primary" isDisabled>
            {t("gallery.disabled")}
          </Button>
        </div>
      </section>

      <section className={styles.section}>
        <h2>{t("gallery.iconButtons")}</h2>
        <div className={styles.row}>
          <IconButton label={t("commands.view.previousPage")} mirrorInRtl>
            <ChevronLeft />
          </IconButton>
          <IconButton label={t("commands.view.nextPage")} mirrorInRtl>
            <ChevronRight />
          </IconButton>
          <IconButton label={t("commands.view.zoomIn")}>
            <ZoomIn />
          </IconButton>
          <IconButton label={t("gallery.disabled")} isDisabled>
            <ZoomIn />
          </IconButton>
        </div>
      </section>

      <section className={styles.section}>
        <h2>{t("gallery.spacing")}</h2>
        <div className={styles.spaces}>
          {SPACES.map((step) => (
            <div key={step} className={styles.space}>
              <span className={styles.bar} style={{ inlineSize: `var(--space-${step})` }} />
              <code dir="ltr">space-{step}</code>
            </div>
          ))}
        </div>
      </section>
    </div>
  );
}

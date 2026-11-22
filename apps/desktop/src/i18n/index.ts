import i18next from "i18next";
import { initReactI18next } from "react-i18next";
import ar from "./locales/ar.json";
import en from "./locales/en.json";

export const LOCALES = ["en", "ar"] as const;
export type Locale = (typeof LOCALES)[number];

export type Direction = "ltr" | "rtl";

export function directionOf(locale: Locale): Direction {
  return locale === "ar" ? "rtl" : "ltr";
}

/**
 * BCP 47 tag used for number and date formatting.
 *
 * The Arabic UI uses Western digits (nu-latn) for chrome such as page counts and
 * zoom. This is a product default awaiting confirmation (see PROGRESS.md);
 * digits *inside documents* are a separate, per-feature user choice (spec 6.3).
 */
export function formatTag(locale: Locale): string {
  return locale === "ar" ? "ar-u-nu-latn" : "en";
}

export function formatNumber(value: number, locale: Locale, options?: Intl.NumberFormatOptions) {
  return new Intl.NumberFormat(formatTag(locale), options).format(value);
}

/**
 * A size for people, in decimal units as file managers show them: "5.5 GB",
 * "205 MB".
 */
export function formatBytes(bytes: number, locale: Locale): string {
  if (bytes >= 1e9) {
    return formatNumber(bytes / 1e9, locale, {
      style: "unit",
      unit: "gigabyte",
      maximumFractionDigits: 1,
    });
  }
  const megabytes = bytes / 1e6;
  return formatNumber(megabytes, locale, {
    style: "unit",
    unit: "megabyte",
    maximumFractionDigits: megabytes < 10 ? 1 : 0,
  });
}

/** Installed memory, in the binary gigabytes it is sold in: "16 GB". */
export function formatMemory(bytes: number, locale: Locale): string {
  return formatNumber(bytes / 2 ** 30, locale, {
    style: "unit",
    unit: "gigabyte",
    maximumFractionDigits: 0,
  });
}

/**
 * Isolates text of either direction inside a translated sentence (U+2068 …
 * U+2069), the in-string equivalent of `<bdi>`: a Latin model name stays
 * whole, and its punctuation does not jump, in an Arabic sentence.
 */
export function isolate(text: string): string {
  return `\u2068${text}\u2069`;
}

export const i18n = i18next.createInstance();

void i18n.use(initReactI18next).init({
  resources: { en: { translation: en }, ar: { translation: ar } },
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
  returnNull: false,
});

/** Applies a locale to i18next and to the document root (lang + dir). */
export function applyLocale(locale: Locale) {
  void i18n.changeLanguage(locale);
  document.documentElement.lang = locale;
  document.documentElement.dir = directionOf(locale);
}

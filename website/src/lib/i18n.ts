import ar from "../i18n/ar.json";
import en from "../i18n/en.json";

export type Lang = "en" | "ar";
export type Strings = typeof en;

export const LANGS: Lang[] = ["en", "ar"];
export const strings: Record<Lang, Strings> = { en, ar: ar as Strings };

/** Fills `{name}` holes in a string. */
export function fill(text: string, vars: Record<string, string>): string {
  return text.replace(/\{(\w+)\}/g, (hole, name: string) => vars[name] ?? hole);
}

export const homePath = (lang: Lang): string => (lang === "ar" ? "/ar/" : "/");
export const dirOf = (lang: Lang): "rtl" | "ltr" => (lang === "ar" ? "rtl" : "ltr");

/** Numbers as each page writes them: Arabic-Indic digits on the Arabic page, like the app. */
export function formatNumber(lang: Lang, value: number, digits = 1): string {
  return new Intl.NumberFormat(lang === "ar" ? "ar-u-nu-arab" : "en", { maximumFractionDigits: digits }).format(value);
}

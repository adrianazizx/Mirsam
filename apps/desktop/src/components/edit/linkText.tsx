import type { TFunction } from "i18next";
import type { ReactNode } from "react";
import type { LinkTarget } from "../../backend";
import { formatNumber, type Locale } from "../../i18n";

/** Actions a link may run that the app names (ADR 0051); anything else is "unknown". */
const KNOWN_ACTIONS = ["javascript", "launch", "remote", "named", "destination", "none"];

/**
 * Where a link goes, in words: a web address as it is (left to right, isolated
 * from the sentence around it), a page, a named destination, or an action
 * the app does not follow.
 */
export function describeTarget(target: LinkTarget, t: TFunction, locale: Locale): ReactNode {
  switch (target.kind) {
    case "url":
      return <bdi dir="ltr">{target.url}</bdi>;
    case "page":
      return t("links.toPage", { page: formatNumber(target.page + 1, locale) });
    case "named":
      return target.page === null || target.page === undefined ? (
        <>
          {t("links.toNamedNowhere")} <bdi>{target.name}</bdi>
        </>
      ) : (
        <>
          <bdi>{target.name}</bdi> ·{" "}
          {t("links.toPage", { page: formatNumber(target.page + 1, locale) })}
        </>
      );
    case "other":
      return t(
        `links.actions.${KNOWN_ACTIONS.includes(target.action) ? target.action : "unknown"}`,
      );
  }
}

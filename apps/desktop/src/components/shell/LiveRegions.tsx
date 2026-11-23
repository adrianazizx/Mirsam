import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { formatNumber } from "../../i18n";
import { announce, useAnnouncer } from "../../state/announcer";
import { selectActiveDocument, useAppStore } from "../../state/appStore";

/** Page and zoom changes are told this long after they stop, so scrolling is not narrated. */
const SETTLE_MS = 700;

/**
 * The window's two live regions, present from the start so screen readers
 * watch them (a region inserted together with its text is often not spoken).
 * `data-live-announcer` keeps them audible while a modal dialog hides the rest
 * of the window (React Aria's `ariaHideOutside` leaves such elements alone).
 *
 * Besides `announce()` calls, it tells errors and notices from the store, and,
 * once they settle, the current page and zoom of the active document when they
 * change in place (not when a document opens or another tab is shown, nor when
 * a fitted zoom follows the window's size).
 */
export function LiveRegions() {
  const { t } = useTranslation();
  const polite = useAnnouncer((state) => state.polite);
  const assertive = useAnnouncer((state) => state.assertive);
  const errorCode = useAppStore((state) => state.errorCode);
  const notice = useAppStore((state) => state.notice);

  useEffect(() => {
    if (errorCode) {
      announce(t(`errors.${errorCode}`, { defaultValue: t("errors.generic") }), "assertive");
    }
  }, [errorCode, t]);

  useEffect(() => {
    if (notice) announce(t(`notices.${notice.key}`, notice.values ?? {}));
  }, [notice, t]);

  usePageAndZoomAnnouncements();

  return (
    <div className="visually-hidden" data-live-announcer="">
      <div role="status" aria-live="polite" aria-atomic="true" data-testid="live-polite">
        {polite.text ? <span key={polite.seq}>{polite.text}</span> : null}
      </div>
      <div role="alert" aria-live="assertive" aria-atomic="true" data-testid="live-assertive">
        {assertive.text ? <span key={assertive.seq}>{assertive.text}</span> : null}
      </div>
    </div>
  );
}

function usePageAndZoomAnnouncements() {
  const { t } = useTranslation();
  const last = useRef<{ id: number; page: number; zoom: number; zoomMode: string } | null>(null);
  const timer = useRef<number | null>(null);
  // What changed since the last announcement, for one document.
  const pending = useRef({ id: -1, page: false, zoom: false });

  useEffect(() => {
    const unsubscribe = useAppStore.subscribe((state) => {
      const open = selectActiveDocument(state);
      const previous = last.current;
      last.current = open
        ? { id: open.id, page: open.page, zoom: open.zoom, zoomMode: open.zoomMode }
        : null;
      // A document opened or another one shown: its tab says which, so nothing to add.
      if (!open || !previous || previous.id !== open.id) {
        if (timer.current !== null) window.clearTimeout(timer.current);
        timer.current = null;
        return;
      }
      const pageChanged = previous.page !== open.page;
      // A fitted zoom follows the window (opening, resizing, a panel shown): not news.
      // A zoom the reader chose, or a change of fit, is.
      const zoomChanged =
        previous.zoomMode !== open.zoomMode ||
        (open.zoomMode === "custom" &&
          Math.round(previous.zoom * 100) !== Math.round(open.zoom * 100));
      if (!pageChanged && !zoomChanged) return;
      if (timer.current !== null) window.clearTimeout(timer.current);
      else pending.current = { id: open.id, page: false, zoom: false };
      pending.current.page ||= pageChanged;
      pending.current.zoom ||= zoomChanged;
      timer.current = window.setTimeout(() => {
        timer.current = null;
        const { id, page: pageChanged, zoom: zoomChanged } = pending.current;
        const now = useAppStore.getState();
        const current = selectActiveDocument(now);
        if (!current || current.id !== id) return;
        const locale = now.locale;
        const parts: string[] = [];
        if (pageChanged) {
          parts.push(
            t("a11y.page", {
              page: current.labels?.[current.page] ?? formatNumber(current.page + 1, locale),
              count: formatNumber(current.pageCount, locale),
            }),
          );
        }
        if (zoomChanged) {
          parts.push(
            t("status.zoom", { percent: formatNumber(Math.round(current.zoom * 100), locale) }),
          );
        }
        announce(parts.join(t("a11y.separator")));
      }, SETTLE_MS);
    });
    return () => {
      unsubscribe();
      if (timer.current !== null) window.clearTimeout(timer.current);
    };
  }, [t]);
}

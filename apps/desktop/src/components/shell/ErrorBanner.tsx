import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../../state/appStore";
import { IconButton } from "../ui/IconButton";
import styles from "./ErrorBanner.module.css";

/**
 * Inline, non-modal error message (docs/SPEC.md section 4, interaction rules),
 * or, when there is no error, a notice about something done elsewhere. Screen
 * readers hear both through `LiveRegions`, which is always present; a role
 * here as well would have them said twice.
 */
export function ErrorBanner() {
  const { t } = useTranslation();
  const errorCode = useAppStore((state) => state.errorCode);
  const setError = useAppStore((state) => state.setError);
  const notice = useAppStore((state) => state.notice);
  const setNotice = useAppStore((state) => state.setNotice);
  if (!errorCode) {
    if (!notice) return null;
    return (
      <div className={`${styles.banner} ${styles.notice}`} data-testid="notice">
        <p className={styles.message}>{t(`notices.${notice.key}`, notice.values ?? {})}</p>
        <IconButton label={t("errors.dismiss")} onPress={() => setNotice(null)}>
          <X />
        </IconButton>
      </div>
    );
  }

  return (
    <div className={styles.banner} data-testid="error-banner">
      <p className={styles.message}>
        {t(`errors.${errorCode}`, { defaultValue: t("errors.generic") })}
      </p>
      <IconButton label={t("errors.dismiss")} onPress={() => setError(null)}>
        <X />
      </IconButton>
    </div>
  );
}

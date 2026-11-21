import { X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../../state/appStore";
import { IconButton } from "../ui/IconButton";
import styles from "./ErrorBanner.module.css";

/** Inline, non-modal error message (docs/SPEC.md section 4, interaction rules). */
export function ErrorBanner() {
  const { t } = useTranslation();
  const errorCode = useAppStore((state) => state.errorCode);
  const setError = useAppStore((state) => state.setError);
  if (!errorCode) return null;

  return (
    <div className={styles.banner} role="alert">
      <p className={styles.message}>
        {t(`errors.${errorCode}`, { defaultValue: t("errors.generic") })}
      </p>
      <IconButton label={t("errors.dismiss")} onPress={() => setError(null)}>
        <X />
      </IconButton>
    </div>
  );
}

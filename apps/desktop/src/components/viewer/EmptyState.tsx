import { FileText, FolderOpen } from "lucide-react";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { Button } from "../ui/Button";
import styles from "./EmptyState.module.css";

export function EmptyState() {
  const { t } = useTranslation();
  return (
    <div className={styles.empty}>
      <FileText className={styles.icon} aria-hidden="true" />
      <h1 className={styles.title}>{t("viewer.emptyTitle")}</h1>
      <p className={styles.body}>{t("viewer.emptyBody")}</p>
      <Button variant="primary" icon={<FolderOpen />} onPress={() => void runCommand("file.open")}>
        {t("commands.file.open")}
      </Button>
    </div>
  );
}

import { LockKeyhole } from "lucide-react";
import { useState } from "react";
import { FieldError, Form, Input, Label, TextField } from "react-aria-components";
import { useTranslation } from "react-i18next";
import { runCommand } from "../../commands/registry";
import { useAppStore } from "../../state/appStore";
import { Button } from "../ui/Button";
import styles from "./UnlockPanel.module.css";

/**
 * Asks for the password of an encrypted PDF, inline in the page area (spec
 * section 4: no modal dialogs for routine actions). The password goes to the
 * engine once and is not kept anywhere.
 */
export function UnlockPanel() {
  const { t } = useTranslation();
  const pending = useAppStore((state) => state.unlocks[0] ?? null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  if (!pending) return null;

  return (
    <section
      className={styles.panel}
      aria-labelledby="unlock-title"
      data-testid="unlock"
      // Escape always backs out (spec section 4), here as the Cancel button does.
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        void runCommand("file.cancelUnlock");
      }}
    >
      <LockKeyhole className={styles.icon} aria-hidden="true" />
      <h1 id="unlock-title" className={styles.title}>
        {t("unlock.title")}
      </h1>
      <p className={styles.body}>
        {t(pending.insert ? "unlock.insertBody" : "unlock.body")}{" "}
        <bdi className={styles.name}>{pending.name}</bdi>
      </p>
      <Form
        className={styles.form}
        // The message is shown, but does not block trying another password.
        validationBehavior="aria"
        validationErrors={pending.retry ? { password: t("unlock.wrong") } : undefined}
        onSubmit={(event: React.FormEvent<HTMLFormElement>) => {
          event.preventDefault();
          setBusy(true);
          void runCommand("file.unlock", { password }).finally(() => {
            setBusy(false);
            setPassword("");
          });
        }}
      >
        <TextField
          name="password"
          type="password"
          value={password}
          onChange={setPassword}
          autoFocus
          isRequired
          className={styles.field}
        >
          <Label className={styles.label}>{t("unlock.password")}</Label>
          <Input className={styles.input} autoComplete="off" data-testid="unlock-password" />
          <FieldError className={styles.error} />
        </TextField>
        <div className={styles.actions}>
          <Button type="submit" variant="primary" isDisabled={busy || password === ""}>
            {t(pending.insert ? "unlock.insert" : "unlock.open")}
          </Button>
          <Button variant="ghost" onPress={() => void runCommand("file.cancelUnlock")}>
            {t("unlock.cancel")}
          </Button>
        </div>
      </Form>
    </section>
  );
}

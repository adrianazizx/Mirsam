import { useState } from "react";
import {
  Checkbox,
  Dialog,
  Heading,
  Modal,
  ModalOverlay,
  Radio,
  RadioGroup,
} from "react-aria-components";
import { useTranslation } from "react-i18next";
import type { Cipher, Permissions } from "../../backend";
import { runCommand } from "../../commands/registry";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { changeOf, initialForm, RESTRICTABLE, type SecurityForm } from "../../state/security";
import { Button } from "../ui/Button";
import styles from "./SecurityDialog.module.css";

/**
 * Passwords and permissions (ADR 0043). The form's state, passwords
 * included, lives here only while the dialog is open. Saving runs
 * `security.apply`; the file on disk is rewritten with the new protection.
 */
export function SecurityDialog() {
  const open = useAppStore((state) => state.securityPrompt);
  const documentId = useAppStore((state) => selectActiveDocument(state)?.id ?? null);
  if (!open || documentId === null) return null;
  // Keyed by document: a form never carries passwords to another tab.
  return <SecurityEditor key={documentId} />;
}

function SecurityEditor() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const security = useAppStore((state) => selectActiveDocument(state)?.security);
  const encrypted = security?.encrypted ?? false;
  // Without the owner password the protection cannot be changed.
  const locked = encrypted && !security?.owner;
  const [form, setForm] = useState<SecurityForm>(() => initialForm(security?.permissions));
  const [busy, setBusy] = useState(false);
  const result = changeOf(form);
  const problem = "problem" in result ? result.problem : null;
  const set = (patch: Partial<SecurityForm>) => setForm((current) => ({ ...current, ...patch }));
  const flag = (name: keyof Permissions, value: boolean) =>
    setForm((current) => ({ ...current, permissions: { ...current.permissions, [name]: value } }));

  const apply = async (saveAs: boolean) => {
    if (!("change" in result) || busy) return;
    setBusy(true);
    try {
      await runCommand("security.apply", { securityChange: result.change, saveAs });
    } finally {
      setBusy(false);
    }
  };
  const canSave = "change" in result && !locked && !busy;

  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(isOpen) => {
        if (!isOpen) void runCommand("security.cancel");
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="security-dialog">
          <Heading slot="title" className={styles.title}>
            {t("securityDialog.title")}
          </Heading>
          <p className={styles.hint} role="status">
            {encrypted
              ? t("securityDialog.currentProtected", {
                  cipher: t(`securityDialog.cipherName.${security?.cipher ?? "unknown"}`),
                })
              : t("securityDialog.currentOpen")}
          </p>
          {locked ? (
            <p className={styles.problem}>{t("securityDialog.needsOwner")}</p>
          ) : (
            <>
              {encrypted ? (
                <RadioGroup
                  className={styles.group}
                  value={form.mode}
                  onChange={(mode) => set({ mode: mode === "remove" ? "remove" : "set" })}
                  aria-label={t("securityDialog.mode")}
                >
                  <Radio value="set" className={styles.check}>
                    <span className={styles.box} aria-hidden="true" />
                    {t("securityDialog.modeSet")}
                  </Radio>
                  <Radio value="remove" className={styles.check}>
                    <span className={styles.box} aria-hidden="true" />
                    {t("securityDialog.modeRemove")}
                  </Radio>
                </RadioGroup>
              ) : null}
              {form.mode === "set" ? (
                <>
                  <div className={styles.field}>
                    <label className={styles.label} htmlFor="security-cipher">
                      {t("securityDialog.cipher")}
                    </label>
                    <select
                      id="security-cipher"
                      className={styles.select}
                      value={form.cipher}
                      onChange={(event) => set({ cipher: event.target.value as Cipher })}
                    >
                      <option value="aes256">{t("securityDialog.aes256")}</option>
                      <option value="aes128">{t("securityDialog.aes128")}</option>
                    </select>
                  </div>
                  <PasswordField
                    id="security-user"
                    label={t("securityDialog.userPassword")}
                    value={form.userPassword}
                    locale={locale}
                    onChange={(userPassword) => set({ userPassword })}
                  />
                  <PasswordField
                    id="security-confirm"
                    label={t("securityDialog.confirmPassword")}
                    value={form.confirmPassword}
                    locale={locale}
                    onChange={(confirmPassword) => set({ confirmPassword })}
                  />
                  <PasswordField
                    id="security-owner"
                    label={t("securityDialog.ownerPassword")}
                    value={form.ownerPassword}
                    locale={locale}
                    onChange={(ownerPassword) => set({ ownerPassword })}
                  />
                  <p className={styles.hint}>{t("securityDialog.ownerHint")}</p>
                  <fieldset className={styles.group}>
                    <legend className={styles.legend}>{t("securityDialog.permissions")}</legend>
                    {RESTRICTABLE.map((name) => (
                      <Checkbox
                        key={name}
                        className={styles.check}
                        isSelected={form.permissions[name]}
                        isDisabled={name === "printHighQuality" && !form.permissions.print}
                        onChange={(value) => flag(name, value)}
                      >
                        <span className={styles.box} aria-hidden="true" />
                        {t(`securityDialog.allow.${name}`)}
                      </Checkbox>
                    ))}
                  </fieldset>
                </>
              ) : null}
              {problem ? (
                <p className={styles.problem} role="alert">
                  {t(`securityDialog.problem.${problem}`)}
                </p>
              ) : null}
              <p className={styles.hint}>{t("securityDialog.saveNote")}</p>
            </>
          )}
          <div className={styles.actions}>
            <Button variant="primary" isDisabled={!canSave} onPress={() => void apply(false)}>
              {t("securityDialog.save")}
            </Button>
            <Button variant="secondary" isDisabled={!canSave} onPress={() => void apply(true)}>
              {t("securityDialog.saveAs")}
            </Button>
            <Button variant="ghost" onPress={() => void runCommand("security.cancel")}>
              {t("securityDialog.cancel")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function PasswordField(props: {
  id: string;
  label: string;
  value: string;
  locale: string;
  onChange(value: string): void;
}) {
  return (
    <div className={styles.field}>
      <label className={styles.label} htmlFor={props.id}>
        {props.label}
      </label>
      <input
        id={props.id}
        className={styles.input}
        type="password"
        autoComplete="new-password"
        value={props.value}
        lang={props.locale}
        dir="auto"
        onChange={(event) => props.onChange(event.target.value)}
      />
    </div>
  );
}

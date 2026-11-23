import { KeyRound, Trash2 } from "lucide-react";
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
import type { SigningIdentity } from "../../backend";
import { runCommand } from "../../commands/registry";
import { formatTag } from "../../i18n";
import { selectActiveDocument, useAppStore } from "../../state/appStore";
import { useCertSign } from "../../state/certSign";
import { SignaturePreview } from "../forms/SignaturePreview";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
import styles from "./CertSignDialog.module.css";

/**
 * Sign with a certificate (ADR 0047): who signs (a PKCS#12 file read this
 * run, or the macOS keychain), what the signature says, whether it shows on
 * a page (a kept signature and a caption, placed by a press afterwards) or
 * not, and whether a time-stamping authority vouches for its time. Signing
 * saves the file. Every action runs a command; a PKCS#12 password lives in
 * this dialog only until it is used.
 */
export function CertSignDialog() {
  const open = useCertSign((state) => state.dialogOpen);
  const documentId = useAppStore((state) => selectActiveDocument(state)?.id ?? null);
  if (!open || documentId === null) return null;
  return <CertSignEditor />;
}

function CertSignEditor() {
  const { t } = useTranslation();
  const locale = useAppStore((state) => state.locale);
  const signatures = useAppStore((state) => state.signatures);
  const encrypted = useAppStore(
    (state) => selectActiveDocument(state)?.security?.encrypted ?? false,
  );
  const { identities, form, picked, unlockError, busy, error } = useCertSign();
  const update = useCertSign((state) => state.updateForm);
  const kept = (signatures ?? []).filter((signature) => !signature.initials);
  const tsaMissing = form.timestamp && form.tsaUrl.trim() === "";
  const canSign = form.identity !== null && !busy && !tsaMissing && picked === null;
  const sign = (saveAs: boolean) => void runCommand("certSign.apply", { saveAs });

  return (
    <ModalOverlay
      isOpen
      isDismissable={!busy}
      onOpenChange={(isOpen) => {
        if (!isOpen) void runCommand("certSign.cancel");
      }}
      className={styles.overlay}
    >
      <Modal className={styles.modal}>
        <Dialog className={styles.dialog} data-testid="cert-sign-dialog">
          <Heading slot="title" className={styles.title}>
            {t("certSign.title")}
          </Heading>

          <section className={styles.section} aria-labelledby="cert-sign-who">
            <h3 id="cert-sign-who" className={styles.heading}>
              {t("certSign.who")}
            </h3>
            {identities === null ? (
              <p className={styles.hint} role="status">
                {t("certSign.looking")}
              </p>
            ) : identities.length === 0 ? (
              <p className={styles.hint}>{t("certSign.noIdentities")}</p>
            ) : (
              <RadioGroup
                aria-labelledby="cert-sign-who"
                value={form.identity ?? ""}
                onChange={(identity) => update({ identity })}
                className={styles.choices}
              >
                {identities.map((identity) => (
                  <IdentityChoice key={identity.id} identity={identity} locale={locale} />
                ))}
              </RadioGroup>
            )}
            {picked ? (
              <UnlockFile name={picked.name} error={unlockError} locale={locale} />
            ) : (
              <div className={styles.row}>
                <Button icon={<KeyRound />} onPress={() => void runCommand("certSign.importFile")}>
                  {t("certSign.importFile")}
                </Button>
              </div>
            )}
            <p className={styles.hint}>{t("certSign.keychainHint")}</p>
          </section>

          <section className={styles.section} aria-labelledby="cert-sign-details">
            <h3 id="cert-sign-details" className={styles.heading}>
              {t("certSign.details")}
            </h3>
            <TextField
              id="cert-sign-reason"
              label={t("certSign.reason")}
              value={form.reason}
              locale={locale}
              onChange={(reason) => update({ reason })}
            />
            <TextField
              id="cert-sign-location"
              label={t("certSign.location")}
              value={form.location}
              locale={locale}
              onChange={(location) => update({ location })}
            />
            <TextField
              id="cert-sign-contact"
              label={t("certSign.contact")}
              value={form.contact}
              locale={locale}
              onChange={(contact) => update({ contact })}
            />
          </section>

          <section className={styles.section} aria-labelledby="cert-sign-look">
            <h3 id="cert-sign-look" className={styles.heading}>
              {t("certSign.appearance")}
            </h3>
            <RadioGroup
              aria-labelledby="cert-sign-look"
              value={form.visible ? "visible" : "invisible"}
              onChange={(value) => update({ visible: value === "visible" })}
              className={styles.choices}
            >
              <Radio value="invisible" className={styles.check}>
                <span className={styles.dot} aria-hidden="true" />
                {t("certSign.invisible")}
              </Radio>
              <Radio value="visible" className={styles.check}>
                <span className={styles.dot} aria-hidden="true" />
                {t("certSign.visible")}
              </Radio>
            </RadioGroup>
            {form.visible ? (
              <>
                <RadioGroup
                  aria-label={t("certSign.graphic")}
                  value={form.graphic === null ? "none" : String(form.graphic)}
                  onChange={(value) => update({ graphic: value === "none" ? null : Number(value) })}
                  className={styles.graphics}
                  orientation="horizontal"
                >
                  <Radio value="none" className={styles.graphic}>
                    <span className={styles.graphicText}>{t("certSign.captionOnly")}</span>
                  </Radio>
                  {kept.map((signature) => (
                    <Radio
                      key={signature.id}
                      value={String(signature.id)}
                      className={styles.graphic}
                      aria-label={t("sign.place", { kind: t(`sign.kind.${signature.kind}`) })}
                    >
                      <SignaturePreview signature={signature} />
                    </Radio>
                  ))}
                </RadioGroup>
                <p className={styles.hint}>{t("certSign.placeHint")}</p>
              </>
            ) : null}
          </section>

          <section className={styles.section} aria-labelledby="cert-sign-time">
            <h3 id="cert-sign-time" className={styles.heading}>
              {t("certSign.time")}
            </h3>
            <Checkbox
              className={styles.check}
              isSelected={form.timestamp}
              onChange={(timestamp) => update({ timestamp })}
            >
              <span className={styles.box} aria-hidden="true" />
              {t("certSign.timestamp")}
            </Checkbox>
            {form.timestamp ? (
              <div className={styles.field}>
                <label className={styles.label} htmlFor="cert-sign-tsa">
                  {t("certSign.tsaUrl")}
                </label>
                <input
                  id="cert-sign-tsa"
                  className={styles.input}
                  type="url"
                  inputMode="url"
                  dir="ltr"
                  spellCheck={false}
                  placeholder="https://"
                  value={form.tsaUrl}
                  onChange={(event) => update({ tsaUrl: event.target.value })}
                />
                <p className={styles.hint}>{t("certSign.tsaHint")}</p>
              </div>
            ) : null}
          </section>

          <p className={styles.hint}>
            {t(encrypted ? "certSign.saveNoteEncrypted" : "certSign.saveNote")}
          </p>
          {error ? (
            <p className={styles.problem} role="alert">
              {t(`errors.${error}`)}
            </p>
          ) : tsaMissing ? (
            <p className={styles.problem}>{t("certSign.tsaMissing")}</p>
          ) : null}
          {busy ? (
            <p className={styles.hint} role="status">
              {t("certSign.signing")}
            </p>
          ) : null}
          <div className={styles.actions}>
            <Button variant="primary" isDisabled={!canSign} onPress={() => sign(false)}>
              {t(form.visible ? "certSign.placeAndSign" : "certSign.sign")}
            </Button>
            <Button variant="secondary" isDisabled={!canSign} onPress={() => sign(true)}>
              {t(form.visible ? "certSign.placeAndSignAs" : "certSign.signAs")}
            </Button>
            <Button
              variant="ghost"
              isDisabled={busy}
              onPress={() => void runCommand("certSign.cancel")}
            >
              {t("certSign.cancel")}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function IdentityChoice({ identity, locale }: { identity: SigningIdentity; locale: string }) {
  const { t } = useTranslation();
  const { certificate } = identity;
  const until = new Intl.DateTimeFormat(formatTag(locale as "en" | "ar"), {
    dateStyle: "medium",
  }).format(new Date(certificate.notAfter * 1000));
  return (
    <div className={styles.identity}>
      <Radio value={identity.id} className={styles.check}>
        <span className={styles.dot} aria-hidden="true" />
        <span className={styles.identityText}>
          {/* Isolated inline, so an Arabic name keeps the list's alignment. */}
          <span className={styles.identityName}>
            <bdi>{certificate.name}</bdi>
          </span>
          <span className={styles.identityMeta}>
            {t(certificate.selfSigned ? "certSign.selfSigned" : "certSign.issuedBy", {
              issuer: certificate.issuer,
            })}
            {" · "}
            {t(`certSign.source.${identity.source}`)}
            {" · "}
            {t("certSign.validUntil", { date: until })}
          </span>
        </span>
      </Radio>
      {identity.source === "file" ? (
        <IconButton
          label={t("certSign.forget")}
          onPress={() => void runCommand("certSign.forget", { identity: identity.id })}
        >
          <Trash2 />
        </IconButton>
      ) : null}
    </div>
  );
}

/** The password of a picked PKCS#12 file; kept here only until it is used. */
function UnlockFile({
  name,
  error,
  locale,
}: {
  name: string;
  error: string | null;
  locale: string;
}) {
  const { t } = useTranslation();
  const [password, setPassword] = useState("");
  const submit = async () => {
    const typed = password;
    setPassword("");
    await runCommand("certSign.unlockFile", { password: typed });
  };
  return (
    <form
      className={styles.unlock}
      onSubmit={(event) => {
        event.preventDefault();
        void submit();
      }}
    >
      <label className={styles.label} htmlFor="cert-sign-password">
        {t("certSign.passwordFor")} <bdi>{name}</bdi>
      </label>
      <input
        id="cert-sign-password"
        className={styles.input}
        type="password"
        autoComplete="off"
        // biome-ignore lint/a11y/noAutofocus: the field the user just asked for
        autoFocus
        lang={locale}
        dir="auto"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
      />
      {error ? (
        <p className={styles.problem} role="alert">
          {t(`errors.${error}`)}
        </p>
      ) : null}
      <div className={styles.row}>
        <Button type="submit" variant="secondary">
          {t("certSign.open")}
        </Button>
        <Button variant="ghost" onPress={() => void runCommand("certSign.cancelFile")}>
          {t("certSign.cancel")}
        </Button>
      </div>
    </form>
  );
}

function TextField(props: {
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
        type="text"
        maxLength={500}
        value={props.value}
        lang={props.locale}
        dir="auto"
        onChange={(event) => props.onChange(event.target.value)}
      />
    </div>
  );
}

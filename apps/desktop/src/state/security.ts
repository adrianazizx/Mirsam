import type { Cipher, Permissions, SecurityChange } from "../backend";

/** What the protection dialog holds while it is open (never kept in the store: it has passwords). */
export interface SecurityForm {
  mode: "set" | "remove";
  cipher: Cipher;
  userPassword: string;
  confirmPassword: string;
  ownerPassword: string;
  permissions: Permissions;
}

export const ALL_PERMISSIONS: Permissions = {
  print: true,
  printHighQuality: true,
  modify: true,
  copy: true,
  annotate: true,
  fillForms: true,
  accessibility: true,
  assemble: true,
};

export const initialForm = (current?: Permissions): SecurityForm => ({
  mode: "set",
  cipher: "aes256",
  userPassword: "",
  confirmPassword: "",
  ownerPassword: "",
  permissions: current ?? ALL_PERMISSIONS,
});

export type FormProblem = "mismatch" | "nothing" | "ascii" | "control";

/** The permissions a person can restrict (accessibility stays allowed: ISO 32000-2). */
export const RESTRICTABLE: (keyof Permissions)[] = [
  "print",
  "printHighQuality",
  "copy",
  "modify",
  "annotate",
  "fillForms",
  "assemble",
];

// biome-ignore lint/suspicious/noControlCharactersInRegex: that is the check
const CONTROL = /[\u0000-\u001f\u007f-\u009f]/;

/** The change the form asks for, or why it cannot be made. */
export function changeOf(
  form: SecurityForm,
): { change: SecurityChange } | { problem: FormProblem } {
  if (form.mode === "remove") return { change: { kind: "remove" } };
  if (form.userPassword !== form.confirmPassword) return { problem: "mismatch" };
  if (CONTROL.test(form.userPassword) || CONTROL.test(form.ownerPassword)) {
    return { problem: "control" };
  }
  if (
    form.cipher === "aes128" &&
    // biome-ignore lint/suspicious/noControlCharactersInRegex: ASCII range check
    !/^[\u0000-\u007f]*$/.test(form.userPassword + form.ownerPassword)
  ) {
    return { problem: "ascii" };
  }
  const restricted = RESTRICTABLE.some((flag) => !form.permissions[flag]);
  if (!form.userPassword && !restricted) return { problem: "nothing" };
  return {
    change: {
      kind: "set",
      cipher: form.cipher,
      userPassword: form.userPassword,
      ownerPassword: form.ownerPassword,
      // High-quality printing means nothing without printing.
      permissions: {
        ...form.permissions,
        printHighQuality: form.permissions.print && form.permissions.printHighQuality,
      },
    },
  };
}

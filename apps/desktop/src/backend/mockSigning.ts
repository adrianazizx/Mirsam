import type {
  Backend,
  CertificateSummary,
  SignatureReport,
  SigningIdentity,
  SignOutcome,
} from "./types";

/**
 * Certificate signatures for the in-browser mock (ADR 0048): one keychain
 * identity with an Arabic name, a PKCS#12 file whose password is "1234",
 * and signatures kept per document so the panel has something to check.
 */

const YEAR = 365 * 86_400;
const NOW = 1_795_400_000;

const certificate = (name: string, issuer: string, selfSigned: boolean): CertificateSummary => ({
  name,
  organization: selfSigned ? null : "مرسام",
  email: null,
  issuer,
  notBefore: NOW - YEAR,
  notAfter: NOW + 2 * YEAR,
  fingerprint: "AB".repeat(32),
  selfSigned,
  key: "p256",
});

const KEYCHAIN: SigningIdentity = {
  id: "keychain:AB",
  source: "keychain",
  certificate: certificate("سارة الحارثي", "Mirsam Demo CA", false),
};

/** The password of the mock's PKCS#12 file. */
export const MOCK_P12_PASSWORD = "1234";

let files: SigningIdentity[] = [];
const signed = new Map<number, SignatureReport[]>();
let nextToken = 0;

function report(identity: SigningIdentity, request: Parameters<Backend["signDocument"]>[1]) {
  const trusted = identity.source === "keychain";
  const visible = request.visible;
  const result: SignatureReport = {
    field: null,
    page: visible?.page ?? 0,
    rect: null,
    view: visible
      ? { x: visible.x, y: visible.y, width: visible.width, height: visible.height }
      : null,
    documentTimestamp: false,
    subFilter: "ETSI.CAdES.detached",
    status: trusted ? "valid" : "unknown",
    integrity: "intact",
    trust: trusted ? "trusted" : "untrusted",
    signer: identity.certificate,
    name: identity.certificate.name,
    reason: request.reason,
    location: request.location,
    contact: request.contact,
    claimedTime: Math.floor(Date.now() / 1000),
    timestamp: request.timestamp
      ? {
          time: Math.floor(Date.now() / 1000),
          authority: "Demo Time-Stamping",
          intact: true,
          trusted: true,
        }
      : null,
    revision: 1,
    revisions: 1,
    coversWholeFile: true,
    changes: null,
    problems: trusted ? [] : ["signer_untrusted"],
  };
  return result;
}

export const mockSigningBackend: Pick<
  Backend,
  | "signingIdentities"
  | "pickCertificateFile"
  | "unlockCertificate"
  | "cancelCertificate"
  | "forgetIdentity"
  | "signDocument"
  | "documentSignatures"
> = {
  signingIdentities: async () => [...files, KEYCHAIN],
  pickCertificateFile: async () => {
    nextToken += 1;
    return { token: nextToken, name: "توقيعي.p12" };
  },
  unlockCertificate: async (token, password) => {
    if (password !== MOCK_P12_PASSWORD) throw { code: "certificate_password", detail: "mac" };
    const identity: SigningIdentity = {
      id: `file:${token}`,
      source: "file",
      certificate: certificate("Test Signer", "Test Signer", true),
    };
    files = [...files.filter((file) => file.id !== identity.id), identity];
    return identity;
  },
  cancelCertificate: async () => {},
  forgetIdentity: async (id) => {
    files = files.filter((file) => file.id !== id);
  },
  signDocument: async (documentId, request, _saveAs): Promise<SignOutcome | null> => {
    const identity = [...files, KEYCHAIN].find((candidate) => candidate.id === request.identity);
    if (!identity) throw { code: "certificate_unreadable", detail: "unknown identity" };
    if (request.timestamp) {
      let url = "";
      try {
        url =
          (
            JSON.parse(localStorage.getItem("mirsam.mock.settings") ?? "{}") as Record<
              string,
              string
            >
          )["signing.tsaUrl"] ?? "";
      } catch {
        url = "";
      }
      if (!url.trim()) throw { code: "timestamp_url", detail: "not set" };
    }
    const earlier = (signed.get(documentId) ?? []).map((one) => ({
      ...one,
      coversWholeFile: false,
      changes: {
        revisions: (one.changes?.revisions ?? 0) + 1,
        pages: [],
        pageOrder: false,
        annotations: 0,
        formFields: 0,
        signatures: (one.changes?.signatures ?? 0) + 1,
        other: 0,
      },
    }));
    const list = [...earlier, report(identity, request)].map((one, index, all) => ({
      ...one,
      revision: index + 2,
      revisions: all.length + 1,
    }));
    signed.set(documentId, list);
    return { name: "signed.pdf", pageCount: 240, modified: false, signatures: list };
  },
  documentSignatures: async (documentId) => signed.get(documentId) ?? [],
};

/** Forgets what the mock signed (tests). */
export function resetMockSigning() {
  files = [];
  signed.clear();
  nextToken = 0;
}

/** Contract between the UI and the Rust shell. Mirrors apps/desktop/src-tauri/src/{commands,viewer}.rs. */

export interface AppInfo {
  version: string;
  devtools: boolean;
}

/** A change to a document's pages. Page numbers are zero-based, before the edit. */
export type PageEdit =
  | { kind: "rotate"; pages: number[]; quarterTurns: number }
  | { kind: "delete"; pages: number[] }
  /** The pages keep their order and the first lands at index `to` of the result. */
  | { kind: "move"; pages: number[]; to: number }
  /** An empty page of `width` × `height` points before index `at`. */
  | { kind: "insertBlank"; at: number; width: number; height: number }
  /** Each copy goes right after its page. */
  | { kind: "duplicate"; pages: number[] };

/** The document after an edit, an undo, or a redo. */
export interface EditOutcome {
  pageCount: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Differs from the file as opened or last saved. */
  modified: boolean;
  /**
   * The pages changed (count, order, sizes, content). Otherwise only their
   * annotations did: the same pages just need painting again.
   */
  pagesChanged: boolean;
  /**
   * What the pages draw as text changed (their pages, or text written into
   * their content, ADR 0021): page text and search results are stale.
   */
  textChanged?: boolean;
}

/** How new page text is set (ADR 0021). */
export interface TextStyle {
  /** The bundled families: IBM Plex Sans (with Plex Sans Arabic), or Amiri. */
  family: "sans" | "naskh";
  /** Points. */
  size: number;
  /** RGB, each 0 to 1. */
  color: [number, number, number];
  /** Start and end follow the text's direction. */
  align: "start" | "center" | "end" | "justify";
  /** Auto: the text's first strong character decides. */
  direction: "auto" | "ltr" | "rtl";
}

/**
 * A paragraph of a page's text, rebuilt from its glyphs (ADR 0023). Its box is
 * on the displayed page: points from the top-left corner, y down.
 */
export interface ParagraphBlock {
  index: number;
  /** Changes when the page's text does: an edit names the block it read. */
  fingerprint: number;
  x: number;
  y: number;
  width: number;
  height: number;
  text: string;
  rtl: boolean;
  align: "left" | "right" | "center" | "justify";
  size: number;
  color: [number, number, number];
  lines: number;
  /** The font it is set in now. */
  font: string;
  /** The paragraph's own font can set new Latin text. */
  fontReusable: boolean;
  /**
   * It holds more than one style (font, size, colour): editing keeps them,
   * and typed text takes the style of the text before it (ADR 0024).
   */
  styled: boolean;
  /** Its styled runs, covering `text` in logical order. */
  runs: StyledRun[];
  editable: boolean;
  /** Why it is not editable: what a press on it says. */
  reason: BlockRefusal | null;
}

/**
 * Why text cannot be edited in place (ADR 0023): shared with other text,
 * unreadable (needs OCR), a font that cannot be read (Type 3), rotated, in an
 * encrypted file, or drawn in a way the editor does not follow yet. Text that
 * is not a paragraph is listed after the paragraphs, as refused areas.
 */
export type BlockRefusal =
  | "shared"
  | "unreadable"
  | "font"
  | "rotated"
  | "encrypted"
  | "unsupported";

/** Bold and italic set on a stretch of a paragraph's new text: UTF-16 offsets. */
export interface BlockMark {
  start: number;
  end: number;
  bold: boolean;
  italic: boolean;
}

/** A stretch of a paragraph's text in one style. */
export interface StyledRun {
  /** UTF-16 offsets into the paragraph's text. */
  start: number;
  end: number;
  /** The PDF's font name. */
  font: string;
  bold: boolean;
  italic: boolean;
  size: number;
  color: [number, number, number];
}

/** A paragraph edited in place: the usual outcome, and what became of its font. */
export interface BlockEditOutcome extends EditOutcome {
  /**
   * Set in a substitute: the original font's name (empty when it has none),
   * the family, and the bundled face chosen (a PostScript name).
   */
  substituted: { from: string; to: "sans" | "naskh"; font: string } | null;
  lines: number;
  /**
   * What the new text is set in: the page's own fonts by their codes, their
   * embedded programs laid out again (ADR 0026), or bundled faces.
   */
  source: "page" | "embedded" | "bundled" | "none";
}

/** What a page object is (ADR 0051). */
export type ObjectKind = "image" | "path" | "text" | "form" | "shading";

/** One object of a page, in drawing order (the last is on top), traced to its content (ADR 0051). */
export interface PageObject {
  index: number;
  /** Changes when the object's bytes do: an edit names the object it read. */
  fingerprint: number;
  kind: ObjectKind;
  /** Its bounds on the displayed page, within any clip. */
  rect: ViewRect;
  /** An image's own size in pixels. */
  image: { width: number; height: number } | null;
  /** Cropped by this app: it can be uncropped. */
  cropped: boolean;
  /** Why it can be seen but not changed. */
  refusal: "textState" | null;
}

/** Where a reorder takes an object. */
export type Arrange = "front" | "forward" | "backward" | "back";

/** A change to one object. `matrix` is an affine map of the displayed page, `[a b c d e f]`. */
export type ObjectEdit =
  | { kind: "transform"; matrix: [number, number, number, number, number, number] }
  | { kind: "delete" }
  | { kind: "arrange"; to: Arrange }
  | { kind: "crop"; rect: ViewRect | null };

/** The document after an object edit, and the object's index afterwards (null once removed). */
export interface ObjectEditOutcome extends EditOutcome {
  index: number | null;
}

/** Where a link goes (ADR 0051). Pages count from 0. */
export type LinkTarget =
  | { kind: "url"; url: string }
  | { kind: "page"; page: number }
  | { kind: "named"; name: string; page?: number | null }
  /** Shown, never followed or written: javascript, launch, remote, named, … */
  | { kind: "other"; action: string };

/** One link of a page: its position in `/Annots`, the areas it covers, and where it goes. */
export interface LinkInfo {
  index: number;
  rects: ViewRect[];
  target: LinkTarget;
}

/** A named destination of the document. */
export interface NamedDestination {
  name: string;
  page: number | null;
}

/** A change to a page's links. */
export type LinkEdit =
  | { kind: "add"; page: number; rects: ViewRect[]; target: LinkTarget }
  | { kind: "setTarget"; page: number; index: number; target: LinkTarget }
  | { kind: "setRect"; page: number; index: number; rect: ViewRect }
  | { kind: "remove"; page: number; index: number };

/** The four text-markup annotations (ADR 0017). */
export type MarkupKind = "highlight" | "underline" | "strikeOut" | "squiggly";

/** Marked text on one page: the displayed-page rectangles of its runs. */
export interface MarkedPage {
  page: number;
  rects: ViewRect[];
}

/** A ready-made pattern for finding what to redact (ADR 0042). */
export type RedactPreset = "email" | "phone" | "iban" | "idNumber" | "cardNumber";

/** What to look for: text (a regular expression with `options.regex`), or a preset. */
export type RedactPattern =
  | { kind: "text"; query: string; options: SearchOptions }
  | { kind: "preset"; preset: RedactPreset };

/** One match, with the boxes to mark (displayed-page points). */
export interface RedactHit {
  page: number;
  /** The matched text as the page has it. */
  text: string;
  before: string;
  after: string;
  rects: ViewRect[];
}

export interface RedactHits {
  hits: RedactHit[];
  truncated: boolean;
}

/** An area to redact: boxes on the displayed page, and the text under them when known. */
export interface RedactMarkArg {
  page: number;
  rects: ViewRect[];
  text: string | null;
}

/** What a redaction removed (ADR 0042). */
export interface RedactReport {
  pages: number;
  counts: {
    glyphs: number;
    imagesBlanked: number;
    imagesDropped: number;
    paths: number;
    hidden: number;
  };
  annotations: number;
  strings: number;
}

export interface RedactOutcome {
  edit: EditOutcome;
  report: RedactReport;
}

/** What sanitizing may take out (ADR 0042). */
export interface SanitizeOptions {
  metadata: boolean;
  scripts: boolean;
  attachments: boolean;
  comments: boolean;
  hiddenContent: boolean;
}

export interface SanitizeReport {
  metadata: number;
  scripts: number;
  attachments: number;
  comments: number;
  hidden: number;
}

export interface SanitizeOutcome {
  edit: EditOutcome;
  report: SanitizeReport;
}

/**
 * The annotations the app writes and can change or remove (ADRs 0017, 0018);
 * `fillText`, `mark`, and `signature` are Fill & Sign items (ADR 0045), not comments.
 */
export type AnnotationKind =
  | MarkupKind
  | "ink"
  | "note"
  | "textBox"
  | "fillText"
  | "mark"
  | "signature";

/** The Fill & Sign items (ADR 0045). */
export const FILL_KINDS: readonly AnnotationKind[] = ["fillText", "mark", "signature"];

/** What a Fill & Sign mark draws. */
export type MarkKind = "check" | "cross" | "dot";

/** A form field's kind (ADR 0045). */
export type FieldKind =
  | "text"
  | "checkbox"
  | "radio"
  | "combo"
  | "list"
  | "pushButton"
  | "signature";

/** A choice of a drop-down or list; a state of a check box or radio button. */
export interface FieldOption {
  /** What is stored. */
  value: string;
  /** What is shown. */
  label: string;
}

/** One place a field is shown. */
export interface FieldWidget {
  page: number | null;
  /** Its box on the displayed page, when it is on one. */
  view: ViewRect | null;
  onState: string | null;
  /** Its position in its page's annotations: the page's tab order (ADR 0047). */
  annotIndex?: number | null;
}

/** Where a field's text sits: the start and end of the text's own direction (ADR 0047). */
export type FieldAlign = "start" | "center" | "end";

/** A bundled family to set a field in. */
export type FieldFont = "sans" | "naskh";

/** A form field to fill in (ADR 0045). */
export interface FormField {
  /** What `setFormField` takes. */
  id: number;
  /** Fully qualified, e.g. `applicant.name`. */
  name: string;
  /** Its own part of the name, what renaming changes (ADR 0047). */
  partialName?: string;
  kind: FieldKind;
  /** What a reader is told about it. */
  tooltip?: string;
  /** What resetting the form returns it to, as `values`. */
  defaultValues?: string[];
  align?: FieldAlign;
  /** The bundled family it is set in; null when it names a font of its own. */
  font?: FieldFont | null;
  /** 0 fits the text to the box. */
  fontSize?: number;
  bordered?: boolean;
  /** A push button's label. */
  caption?: string;
  /**
   * Its value: one text for a text field or drop-down; the chosen values of
   * a list; the "on" state of a checked box or chosen radio button (empty: off).
   */
  values: string[];
  options: FieldOption[];
  readOnly: boolean;
  required: boolean;
  multiline: boolean;
  password: boolean;
  comb: boolean;
  maxLen: number | null;
  multiSelect: boolean;
  editable: boolean;
  widgets: FieldWidget[];
}

/** A document's form. */
export interface FormInfo {
  fields: FormField[];
  /** It also has an XFA form, which is not supported (spec section 5.5). */
  xfa: boolean;
  /** It asked viewers to draw its fields; the first fill draws them all. */
  needAppearances: boolean;
}

/** A field as the form designer sets it (ADR 0047). */
export interface FieldSpec {
  kind: FieldKind;
  /** Its own part of the name: no full stops. */
  name: string;
  tooltip: string;
  required: boolean;
  readOnly: boolean;
  multiline: boolean;
  maxLen: number | null;
  /** As `FormField.values`: a radio group's by its buttons' states. */
  defaultValues: string[];
  align: FieldAlign;
  /** Null keeps a field's own font. */
  font: FieldFont | null;
  fontSize: number;
  /** Choices; a radio group's buttons in order (label: its export value). */
  options: FieldOption[];
  multiSelect: boolean;
  editable: boolean;
  bordered: boolean;
  caption: string;
}

/** One change to a form (ADR 0047); boxes on the displayed page. */
export type DesignEdit =
  | { op: "add"; page: number; rect: ViewRect; spec: FieldSpec }
  | { op: "addButton"; field: number; page: number; rect: ViewRect; value: string }
  | { op: "update"; field: number; spec: FieldSpec }
  | { op: "setRect"; field: number; widget: number; page: number; rect: ViewRect }
  | { op: "removeButton"; field: number; widget: number }
  | { op: "remove"; field: number }
  | { op: "tabOrder"; page: number; fields: number[] };

/** What a form change did, and the field to select after it. */
export interface DesignOutcome {
  edit: EditOutcome;
  field: number | null;
}

/** A field a flat form seems to ask for (ADR 0047). */
export interface DetectedField {
  kind: FieldKind;
  /** Its box on the displayed page. */
  view: ViewRect | null;
  /** From its label; empty when none was found. */
  name: string;
  multiline: boolean;
}

/** A kept signature or initials (ADR 0045). */
export interface SignatureInfo {
  id: number;
  initials: boolean;
  kind: "drawn" | "typed" | "image";
  /** Width over height. */
  aspect: number;
  /** A drawn one's strokes, `[x, y]` with y down, and its pen width. */
  strokes: [number, number][][];
  pen: number;
  /** A typed one's text and family. */
  text: string | null;
  family: "sans" | "naskh" | null;
}

/** An annotation on a page, for hit-testing, showing, and changing it. */
export interface PageAnnotation {
  /** Its position in the page's annotations; what the edit commands take. */
  index: number;
  kind: AnnotationKind;
  /** Displayed-page rectangles: one per run of marked text, else its box. */
  rects: ViewRect[];
  /** A note's text. */
  contents: string | null;
  /** Ink strokes as displayed-page `[x, y]` points. */
  strokes: [number, number][][];
}

/** What inserting pages from files led to: done, or a file's password is needed (ADR 0016). */
export type InsertOutcome =
  | { kind: "inserted"; edit: EditOutcome }
  | { kind: "needsPassword"; token: number; name: string; retry: boolean };

/** Digits a stamp writes numbers in (spec §6): 0–9 or ٠–٩. */
export type StampDigits = "western" | "arabicIndic";

/**
 * A header or footer (page numbers are one), ADR 0050. `{page}` and `{pages}`
 * in the text become the page's number and the last number. `start` sits on
 * the right when `rtl`.
 */
export interface Band {
  edge: "top" | "bottom";
  start: string;
  center: string;
  end: string;
  rtl: boolean;
  family: "sans" | "naskh";
  /** Points. */
  size: number;
  /** RGB, each 0 to 1. */
  color: [number, number, number];
  /** From the page's edges to the text, in points. */
  margin: number;
  digits: StampDigits;
  firstNumber: number;
}

/** What a watermark shows: text, or the image last picked (`width`: of the page's width, 0.05–1). */
export type WatermarkMark =
  | {
      kind: "text";
      text: string;
      family: "sans" | "naskh";
      size: number;
      color: [number, number, number];
    }
  | { kind: "image"; width: number };

export interface Watermark {
  mark: WatermarkMark;
  /** 0 to 1. */
  opacity: number;
  /** Degrees counter-clockwise. */
  rotation: number;
  /** Behind the page's content instead of over it. */
  behind: boolean;
}

export type StampArg = { kind: "band"; band: Band } | { kind: "watermark"; watermark: Watermark };

export type ImageFormat = { kind: "png" } | { kind: "jpeg"; quality: number };

export type TextFormat = "plain" | "markdown";

/** How to compress (ADR 0050). `imageDpi` null keeps images; `jpegQuality` null keeps them lossless. */
export interface CompressOptions {
  imageDpi: number | null;
  jpegQuality: number | null;
  objectStreams: boolean;
}

/** Sizes in bytes before and after, and what compressing did. */
export interface CompressReport {
  before: number;
  after: number;
  imagesReduced: number;
  imagesKept: number;
  streamsCompressed: number;
  objectsRemoved: number;
}

/** How to split a document (ADR 0016): by pages per file, at top-level bookmarks, or by bytes per file. */
export type SplitBy =
  | { kind: "pages"; size: number }
  | { kind: "bookmarks" }
  | { kind: "bytes"; limit: number };

/** One file of a split at the top-level bookmarks: its title and pages (zero-based, end exclusive). */
export interface BookmarkPart {
  title: string;
  start: number;
  end: number;
}

/** A reply in a comment's thread (ADR 0041). */
export interface CommentReply {
  index: number;
  author: string | null;
  /** Unix seconds. */
  modified: number | null;
  contents: string;
}

/** A comment as the comments panel lists it (ADR 0041). */
export interface DocumentComment {
  page: number;
  /** Its position in the page's annotations; what the comment commands take. */
  index: number;
  kind: AnnotationKind;
  author: string | null;
  /** Unix seconds. */
  modified: number | null;
  /** A note's or text box's text, or a comment on markup. */
  contents: string | null;
  /** For markup, the page's text under it. */
  markedText: string | null;
  /** Where it is on the displayed page. */
  rect: ViewRect;
  resolved: boolean;
  /** Oldest first. */
  replies: CommentReply[];
}

/** A point of a pen stroke on the displayed page, with the pen's pressure (0–1). */
export interface StrokePoint {
  x: number;
  y: number;
  pressure: number;
}

/** The document after a save. */
export interface SaveOutcome {
  /** The file's display name (new after Save As). */
  name: string;
  modified: boolean;
  /** Edits were appended to keep the file's signatures valid. */
  incremental: boolean;
}

export interface DocumentInfo {
  id: number;
  /** File name only; the UI never sees filesystem paths. */
  name: string;
  pageCount: number;
  /** Where the file was left the last time it was open, if ever. */
  viewState?: SavedViewState | null;
  /**
   * The document's own reading preferences (its catalog's /ViewerPreferences
   * /Direction and /PageLayout), the defaults for page order and layout.
   */
  readingDirection?: "ltr" | "rtl" | null;
  pageLayout?: "single" | "continuous" | "spread" | "book" | null;
  /** Restored from a recovery file with unsaved changes (ADR 0040). */
  modified?: boolean;
  canUndo?: boolean;
  /** How the file is protected and what the password it was opened with allows (ADR 0043). */
  security?: SecurityInfo;
  /** It carries digital signatures (ADR 0048). */
  signed?: boolean;
}

/** What a reader may do with a file opened with the user password. */
export interface Permissions {
  print: boolean;
  printHighQuality: boolean;
  modify: boolean;
  copy: boolean;
  annotate: boolean;
  fillForms: boolean;
  accessibility: boolean;
  assemble: boolean;
}

export type Cipher = "aes256" | "aes128";

/** What a saved file is protected with. Empty passwords: none to open / a random owner password. */
export interface Protection {
  cipher: Cipher;
  userPassword: string;
  ownerPassword: string;
  permissions: Permissions;
}

/** What a save does to the file's protection (ADR 0043). */
export type SecurityChange = { kind: "keep" } | { kind: "remove" } | ({ kind: "set" } & Protection);

/** A document's protection, as opened. */
export interface SecurityInfo {
  encrypted: boolean;
  cipher: "rc4" | "aes128" | "aes256" | null;
  revision: number;
  permissions: Permissions;
  /** Opened with the owner password (or not encrypted): nothing is restricted. */
  owner: boolean;
}

/** The document after it was saved with another protection. */
export interface SecureOutcome {
  name: string;
  modified: boolean;
  security: SecurityInfo;
}

/** A certificate as the UI shows it (ADR 0048). Times are Unix seconds. */
export interface CertificateSummary {
  /** Its common name (else organisation): may be Arabic; show it in `<bdi>`. */
  name: string;
  organization: string | null;
  email: string | null;
  issuer: string;
  notBefore: number;
  notAfter: number;
  /** SHA-256, upper-case hex. */
  fingerprint: string;
  selfSigned: boolean;
  key: "rsa" | "p256" | "p384" | null;
}

/** A certificate with a key to sign with: read from a PKCS#12 file, or in the macOS keychain. */
export interface SigningIdentity {
  /** `file:N` or `keychain:<fingerprint>`. */
  id: string;
  source: "file" | "keychain";
  certificate: CertificateSummary;
}

/** A PKCS#12 file picked through the native dialog, waiting for its password. */
export interface PickedCertificate {
  token: number;
  name: string;
}

/** What a certificate signature says and shows (ADR 0048). */
export interface CertSignRequest {
  identity: string;
  reason: string | null;
  location: string | null;
  contact: string | null;
  /** Time-stamp from the configured authority (PAdES B-T). */
  timestamp: boolean;
  /** A box on the displayed page (points), a kept signature, and caption lines; null: invisible. */
  visible: {
    page: number;
    x: number;
    y: number;
    width: number;
    height: number;
    signature: number | null;
    caption: string;
  } | null;
}

/** What later revisions changed after a signature. */
export interface SignatureChanges {
  revisions: number;
  /** Pages whose content changed (zero-based). */
  pages: number[];
  pageOrder: boolean;
  annotations: number;
  formFields: number;
  signatures: number;
  other: number;
}

/** One signature, checked (ADR 0048). */
export interface SignatureReport {
  field: string | null;
  page: number | null;
  rect: [number, number, number, number] | null;
  /** Its box on the displayed page, when it is visible. */
  view: ViewRect | null;
  documentTimestamp: boolean;
  subFilter: string | null;
  /** valid: intact and trusted; unknown: intact, signer not vouched for; invalid. */
  status: "valid" | "unknown" | "invalid";
  integrity: "intact" | "altered" | "unchecked";
  trust: "trusted" | "expired" | "untrusted" | "unknown";
  signer: CertificateSummary | null;
  name: string | null;
  reason: string | null;
  location: string | null;
  contact: string | null;
  /** The signer's own clock (Unix seconds): a claim. */
  claimedTime: number | null;
  timestamp: { time: number; authority: string | null; intact: boolean; trusted: boolean } | null;
  revision: number;
  revisions: number;
  coversWholeFile: boolean;
  changes: SignatureChanges | null;
  /** Codes under `signaturesPanel.problem.`. */
  problems: string[];
}

/** The document after it was signed and saved. */
export interface SignOutcome {
  name: string;
  pageCount: number;
  modified: boolean;
  signatures: SignatureReport[];
}

/** Unsaved changes an earlier session left, kept in a recovery file (ADR 0040). */
export interface RecoveryInfo {
  /** Names it in `restoreRecovery` and `discardRecovery`; the UI never sees paths. */
  token: number;
  name: string;
  /** When its latest change was kept (Unix seconds). */
  savedAt: number;
  /** The original changed or went away since: the recovered version is saved as a new file. */
  originalChanged: boolean;
}

/** What restoring a recovery file led to, and the tab it replaced (the original, reopened). */
export interface RestoreOutcome {
  outcome: OpenOutcome;
  replaced: number | null;
}

/** A document's remembered position (safha_core::store::ViewState). */
export interface SavedViewState {
  page: number;
  /** "fit-width" or "custom"; anything else is treated as fit-width. */
  zoomMode: string;
  zoom: number;
  /** How far down the page the view's top edge was, 0 to 1. */
  pageOffset: number;
}

/** What opening a file led to. */
export type OpenOutcome =
  | { kind: "opened"; document: DocumentInfo }
  /** Encrypted: continue with `unlockDocument(token, password)`. `retry` means the last password was wrong. */
  | { kind: "needsPassword"; token: number; name: string; retry: boolean };

/**
 * Documents that arrived while the app runs: a later copy of the app handed
 * them over (ADR 0046), or the platform asked to open them (Finder).
 */
export interface DocumentsArrived {
  documents: OpenOutcome[];
  /** The error code of the first one that could not be opened. */
  error: string | null;
}

export interface Startup {
  /** Restored tabs and the launch document, in tab order. */
  documents: OpenOutcome[];
  /** The document to show first. */
  active: number | null;
  /** Unsaved changes an earlier session left, offered once at the app's start. */
  recoveries?: RecoveryInfo[];
}

export interface RecentFile {
  /** Names the file in `openRecent`/`forgetRecent`; valid until the next `recentFiles` call. */
  token: number;
  name: string;
  /** Unix seconds. */
  lastOpened: number;
  /** False when the file has been moved or deleted since. */
  exists: boolean;
}

export interface OutlineItem {
  /** Logical-order text; may be in either script. */
  title: string;
  /** Zero-based target page, when the item points into this document. */
  page: number | null;
  /** Target height on the page in PDF points from the bottom (y up), when given. */
  top: number | null;
  children: OutlineItem[];
}

export type Direction = "ltr" | "rtl";

/** A rectangle on the displayed page: points from its top-left corner, y down. */
export interface ViewRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface TextSpan {
  /** UTF-16 indices into the line's text. */
  start: number;
  end: number;
  /** Absent for inferred word spaces, which have no glyph. */
  rect: ViewRect | null;
}

export interface TextLine {
  /** Logical (reading) order. */
  text: string;
  /** Logical order. */
  spans: TextSpan[];
  bounds: ViewRect;
}

export interface TextBlock {
  direction: Direction;
  /** Top to bottom. */
  lines: TextLine[];
}

/** A page's text in reading order with character boxes (safha_core::text). */
export interface TextLayer {
  direction: Direction;
  /** "suspect": the page's character mapping looks broken (copy will be wrong; OCR would help). */
  quality: "good" | "suspect" | "noText";
  /** Reading order. */
  blocks: TextBlock[];
}

export interface SearchOptions {
  caseSensitive: boolean;
  wholeWord: boolean;
  regex: boolean;
  /** Ignore tashkeel and tatweel; unify alef forms, ى/ي, digit styles (spec 6.2). */
  arabicNormalize: boolean;
  /** Also treat ة and ه as equal. */
  taMarbutaAsHa: boolean;
}

export interface SearchHit {
  page: number;
  block: number;
  line: number;
  /** UTF-16 indices into the line's text. */
  start: number;
  end: number;
  direction: Direction;
  rects: ViewRect[];
  context: { before: string; matched: string; after: string };
}

export interface SearchResults {
  hits: SearchHit[];
  /** More matches exist than were returned. */
  truncated: boolean;
}

/**
 * Render-queue lane. Each lane has its own viewport generation, so the
 * thumbnail panel and the page view never cancel each other's requests.
 */
export type Lane = "main" | "thumbnails";

/** Page dimensions in PDF points (1/72 inch). Exact f32 values from the engine. */
export interface PageSize {
  width: number;
  height: number;
}

/**
 * A rectangle of a page at a scale. `scale` is device pixels per PDF point and
 * must be an f32 value (`Math.fround`); the whole page is
 * `ceil(width * scale) x ceil(height * scale)` pixels in f32 arithmetic, and
 * the rectangle must lie inside it (see `viewer/geometry.ts`).
 */
export interface TileRequest {
  scale: number;
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Render-queue classes, most urgent first (safha_core::render::Urgency). */
export type Urgency = "placeholder" | "visible" | "thumbnail" | "prefetch";

export interface Priority {
  urgency: Urgency;
  /** Distance from the viewport centre in CSS pixels; nearer renders first. */
  distance: number;
}

/** A queued request that is still wanted after the viewport moved. */
export interface KeptRequest {
  page: number;
  tile: TileRequest;
  priority: Priority;
}

/**
 * A rendered rectangle, decoded and ready to draw (see `backend/rendered.ts`).
 * Whoever receives one owns it and must `releaseRendered` it once drawn or
 * dropped: an `ImageBitmap` holds its pixels until closed.
 */
export interface RenderedPage {
  width: number;
  height: number;
  /**
   * Time the engine spent rasterizing, in microseconds, and whether the tile
   * came from its cache — null where the transport does not say (the shell's
   * image transport; its render queue counts both, see `renderStats`).
   */
  renderMicros: number | null;
  fromCache: boolean | null;
  /**
   * From the shell: an image element the webview loaded and decoded from a
   * BMP, so the pixels never became a JavaScript buffer (ADR 0011). From the
   * in-browser mock: `ImageData`.
   */
  image: HTMLImageElement | ImageBitmap | ImageData;
}

/**
 * A tile the image transport did not deliver: the engine cancelled it (the
 * viewport moved on) or failed it. An image element cannot read the response,
 * so the two look alike; the tile client tells them apart by generation.
 */
export function isTileUnavailable(value: unknown): boolean {
  return isBackendError(value) && value.code === "tile_unavailable";
}

/** Render-queue counters (devtools builds only). */
export interface RenderStats {
  submitted: number;
  cacheHits: number;
  coalesced: number;
  rendered: number;
  failed: number;
  cancelled: number;
  renderMicros: number;
  pageLoads: number;
  cacheBytes: number;
  cacheCapacityBytes: number;
  cacheEntries: number;
  cacheEvictions: number;
  pending: number;
}

export interface SelftestConfig {
  locale: string;
  theme: string;
  /** Name of a spike or benchmark to run instead of the plain self-test. */
  spike: string | null;
}

/** Error payload of a rejected command. `code` maps to a localized message. */
/** What failed (ADR 0061). */
export type CrashKind = "panic" | "native" | "interface";

/** A crash report kept on this computer until it is sent or deleted (ADR 0061). */
export interface CrashReportInfo {
  /** Names it in the crash report calls. */
  id: string;
  kind: CrashKind;
  /** Unix seconds. */
  created: number;
  /** The start of its message, for a list. */
  summary: string;
  /** Exactly what is copied, saved or sent: no documents, file names or paths. */
  text: string;
}

export interface CrashReports {
  /** Newest first. */
  reports: CrashReportInfo[];
  /** The GitHub repository reports can be filed on (`owner/name`), if any. */
  github: string | null;
}

export interface BackendError {
  code: string;
  detail: string;
}

export function isBackendError(value: unknown): value is BackendError {
  return (
    typeof value === "object" && value !== null && typeof (value as BackendError).code === "string"
  );
}

/** A render that was dropped because the viewport moved on. Expected; never shown to the user. */
export function isCancelled(value: unknown): boolean {
  return isBackendError(value) && value.code === "cancelled";
}

/** What an OCR package does in the pipeline (ADR 0030, ADR 0034). */
export type OcrRole = "primary" | "ensemble" | "draft" | "runtime";

export type OcrPackageState =
  | { kind: "notInstalled" }
  /** Part of it is downloaded; downloading again resumes. */
  | { kind: "partial"; received: number }
  | { kind: "installed" }
  /** Another version is installed; downloading replaces it. */
  | { kind: "outdated"; version: string };

/** A model (or the runtime) as the settings panel lists it. Sizes in bytes. */
export interface OcrPackage {
  id: string;
  role: OcrRole;
  /** Its own name (not translated). */
  name: string;
  version: string;
  /** SPDX expression. */
  licence: string;
  homepage: string;
  downloadSize: number;
  /** An upper bound of the disk space it takes once installed. */
  installedSize: number;
  state: OcrPackageState;
  /** OCR cannot run without it. */
  required: boolean;
  /** A download of it is running (also after a window reload). */
  downloading: boolean;
}

export type OcrAccelerator = "metal" | "cuda" | "vulkan";

export interface OcrHardware {
  os: string;
  arch: string;
  accelerators: OcrAccelerator[];
  /** The GPU backend the installed runtime would use; null: the CPU (slow). */
  runtimeGpu: OcrAccelerator | null;
  /** Installed memory in bytes, when known. */
  memory: number | null;
}

export interface OcrModels {
  packages: OcrPackage[];
  hardware: OcrHardware;
  /** False when there is nowhere to install (no app data folder). */
  available: boolean;
}

export type OcrPhase = "checking" | "downloading" | "unpacking" | "installing";

export type OcrModelUpdate =
  | { kind: "progress"; phase: OcrPhase; received: number; total: number }
  | { kind: "installed" }
  /** Stopped by the user; what arrived is kept to resume. */
  | { kind: "cancelled" }
  /** `needed` and `available` (bytes) come with `model_disk_full`. */
  | { kind: "failed"; code: string; needed: number | null; available: number | null };

/** A download moved on (sent to every window). */
export type OcrModelEvent = { id: string } & OcrModelUpdate;

/** Where a background OCR job is (ADR 0035). */
export type OcrJobState = "queued" | "running" | "paused" | "done" | "cancelled" | "failed";
/** The quick draft (platform OCR) or the quality pipeline. */
export type OcrPass = "draft" | "quality";

/** A background OCR job: a document's pages, drafted first, then read in quality mode. */
export interface OcrJob {
  id: number;
  /** The document's file name (not translated). */
  name: string;
  /** How many pages it reads. */
  pages: number;
  state: OcrJobState;
  /** An error code (`errors.*`) when it failed. */
  error: string | null;
  /** Pages that have a draft or the final text. */
  drafted: number;
  /** Pages that have the quality result. */
  finished: number;
  /** The page being read now (zero-based) and by which pass. */
  current: [number, OcrPass] | null;
  /** The open documents showing the job's file. */
  documentIds: number[];
}

export type OcrJobAction = "pause" | "resume" | "cancel";

/** What reconciling did to a word (ADR 0033). */
export type OcrWordStatus =
  | { kind: "unchecked" }
  | { kind: "confirmed" }
  /** Replaced: other readings agreed on something else; `read` is what the page read said. */
  | { kind: "corrected"; read: string }
  /** The readers disagree; the page read was kept. */
  | { kind: "uncertain"; alternatives: string[] }
  /** A number–unit compound put in logical order ("bit-64" to "64-bit"). */
  | { kind: "reordered"; read: string };

/** A word on the pages-needing-review list. */
export interface OcrReviewWord {
  /** Its place among the page's words. */
  index: number;
  text: string;
  /** The engine's probability for it, 0–1. */
  probability: number;
  status: OcrWordStatus;
  /** Its box on the displayed page in points (left, top, right, bottom). */
  bounds: [number, number, number, number] | null;
}

/** A page on the pages-needing-review list (spec §5.8). */
export interface OcrReviewPage {
  page: number;
  uncertain: number;
  corrected: number;
  reordered: number;
  /** The engine's answer was cut off: part of the page may be missing. */
  truncated: boolean;
  /** Mean and lowest word probability, 0–1. */
  confidence: number;
  lowest: number;
  words: OcrReviewWord[];
}

/** OCR text was written into a document (an edit, undoable like any other). */
export interface OcrApplied {
  documentId: number;
  outcome: EditOutcome;
  /** The pages written, each now with the draft or the final text. */
  pages: [number, "draft" | "final"][];
}

export interface Backend {
  appInfo(): Promise<AppInfo>;
  /** Shows the native picker. Resolves to null when the user cancels. */
  openDocumentDialog(): Promise<OpenOutcome | null>;
  /**
   * What to show at launch: the previous session's tabs and the launch
   * document. After a UI reload, the documents still open in the shell.
   */
  startup(): Promise<Startup>;
  unlockDocument(token: number, password: string): Promise<OpenOutcome>;
  cancelUnlock(token: number): Promise<void>;
  recentFiles(limit: number): Promise<RecentFile[]>;
  /** Rejects with code `file_missing` when the file is gone. */
  openRecent(token: number): Promise<OpenOutcome>;
  forgetRecent(token: number): Promise<void>;
  clearRecents(): Promise<void>;
  saveViewState(documentId: number, view: SavedViewState): Promise<void>;
  /** The open tabs in order, restored at the next launch. */
  saveSession(documentIds: number[], active: number | null): Promise<void>;
  /** Stored UI settings (`locale`, `uiDigits`, `theme`, `readingTheme`, `leftPanel`, `search.options`). */
  settings(): Promise<Record<string, string>>;
  setSetting(key: string, value: string): Promise<void>;
  /** Every page's size, read without loading pages. */
  pageSizes(documentId: number): Promise<PageSize[]>;
  documentOutline(documentId: number): Promise<OutlineItem[]>;
  /** Every page's label, or null when the document defines none. */
  pageLabels(documentId: number): Promise<(string | null)[] | null>;
  pageText(documentId: number, page: number): Promise<TextLayer>;
  /**
   * Searches the whole document. The first search builds the index. Rejects
   * with code `invalid_query` (empty, bad regex) or `cancelled` (closed).
   */
  searchDocument(documentId: number, query: string, options: SearchOptions): Promise<SearchResults>;
  /**
   * Renders one rectangle of a page through the engine's render queue.
   * Rejects with code `cancelled` (see `isCancelled`) when a later
   * `updateViewport` dropped it before it started.
   */
  renderTile(
    documentId: number,
    generation: number,
    page: number,
    tile: TileRequest,
    priority: Priority,
    lane?: Lane,
  ): Promise<RenderedPage>;
  /** Advances the lane's viewport generation; queued renders not in `keep` are cancelled. */
  updateViewport(
    documentId: number,
    generation: number,
    keep: KeptRequest[],
    lane?: Lane,
  ): Promise<void>;
  closeDocument(documentId: number): Promise<void>;
  /** Opens an empty window. */
  newWindow(): Promise<void>;
  /**
   * Prints a document through the platform's own PDF viewer and print panel
   * (ADR 0012; unverified on screen so far).
   */
  printDocument(documentId: number): Promise<void>;
  /** Applies a page edit; the document is then at a new revision (ADR 0014). */
  editPages(documentId: number, edit: PageEdit): Promise<EditOutcome>;
  undoEdit(documentId: number): Promise<EditOutcome>;
  redoEdit(documentId: number): Promise<EditOutcome>;
  /**
   * Finds what matches a pattern, with boxes to mark. Rejects with
   * `invalid_query` for an empty query or a bad regular expression.
   */
  findRedactions(documentId: number, pattern: RedactPattern): Promise<RedactHits>;
  /**
   * Removes what is under the marks from the file and draws boxes over them,
   * as one undo step. Rejects with `redaction_failed` if checking the result
   * finds readable text under a mark; nothing changes then.
   */
  applyRedactions(documentId: number, marks: RedactMarkArg[]): Promise<RedactOutcome>;
  /** Strips metadata, scripts, attachments, comments, and hidden content (one undo step). */
  sanitizeDocument(documentId: number, options: SanitizeOptions): Promise<SanitizeOutcome>;
  /** Marks text: one annotation per page, all in one undo step (ADR 0017). */
  addMarkup(documentId: number, kind: MarkupKind, pages: MarkedPage[]): Promise<EditOutcome>;
  /** Adds freehand ink: strokes in displayed-page points (ADR 0018). */
  addInk(documentId: number, page: number, strokes: StrokePoint[][]): Promise<EditOutcome>;
  /** Adds a sticky note whose icon's top-left corner is displayed at (`x`, `y`). */
  addNote(
    documentId: number,
    page: number,
    x: number,
    y: number,
    contents: string,
  ): Promise<EditOutcome>;
  /**
   * Adds a text box (ADR 0019) whose top-left corner is displayed at (`x`,
   * `y`), `width` points wide; its height fits the text.
   */
  addTextBox(
    documentId: number,
    page: number,
    x: number,
    y: number,
    width: number,
    contents: string,
  ): Promise<EditOutcome>;
  /** Replaces a note's or a text box's text. */
  setAnnotationContents(
    documentId: number,
    page: number,
    index: number,
    contents: string,
  ): Promise<EditOutcome>;
  /** Opens a recovery file as a document with unsaved changes; an encrypted one asks first. */
  restoreRecovery(token: number): Promise<RestoreOutcome>;
  /** Tries the password of an encrypted recovery file (the token its restore gave). */
  unlockRecovery(token: number, password: string): Promise<RestoreOutcome>;
  /** Deletes a recovery file the user does not want. */
  discardRecovery(token: number): Promise<void>;
  /** Whether a document's changes are kept in a recovery file; devtools builds only (the self-test). */
  hasRecoveryFile(documentId: number): Promise<boolean | null>;
  /** Every comment in the document, page by page (ADR 0041). */
  documentComments(documentId: number): Promise<DocumentComment[]>;
  /**
   * Writes text into a page's content (ADR 0021), its box's top-left corner
   * at displayed-page (`x`, `y`), wrapped to `width`.
   */
  addText(
    documentId: number,
    page: number,
    x: number,
    y: number,
    width: number,
    text: string,
    style: TextStyle,
  ): Promise<EditOutcome>;
  /** The paragraphs of a page's text, in reading order (ADR 0023). */
  pageTextBlocks(documentId: number, page: number): Promise<ParagraphBlock[]>;
  /**
   * Replaces a paragraph's text in place, reflowed inside its box (ADR 0023).
   * `family` forces a bundled substitute; absent, the paragraph's own font is
   * kept when it can set the text. An empty text removes the paragraph.
   */
  editTextBlock(
    documentId: number,
    edit: {
      page: number;
      index: number;
      fingerprint: number;
      text: string;
      family?: "sans" | "naskh";
      /** Bold and italic as the editor set them (ADR 0025); absent keeps each character's own. */
      marks?: BlockMark[];
    },
  ): Promise<BlockEditOutcome>;
  /** Answers a comment; signed with the `commentAuthor` setting. */
  replyToComment(
    documentId: number,
    page: number,
    index: number,
    contents: string,
  ): Promise<EditOutcome>;
  /** Marks a comment resolved, or open again. */
  setCommentResolved(
    documentId: number,
    page: number,
    index: number,
    resolved: boolean,
  ): Promise<EditOutcome>;
  /** The document's form fields (ADR 0045). */
  formFields(documentId: number): Promise<FormInfo>;
  /** Fills in a field (values as `FormField.values`): one undo step. */
  setFormField(documentId: number, field: number, values: string[]): Promise<EditOutcome>;
  /** Changes the form (ADR 0047): `edits` in order, one undo step. */
  designForm(documentId: number, edits: DesignEdit[]): Promise<DesignOutcome>;
  /** Fields a flat form seems to ask for on `page`, for review (ADR 0047). */
  detectFormFields(documentId: number, page: number): Promise<DetectedField[]>;
  /** Places Fill & Sign text, its top-left corner displayed at (`x`, `y`); black unless `color`. */
  addFillText(
    documentId: number,
    page: number,
    x: number,
    y: number,
    contents: string,
    size?: number,
    color?: [number, number, number],
  ): Promise<EditOutcome>;
  /** Places a tick, cross, or dot, its top-left corner displayed at (`x`, `y`); black unless `color`. */
  addFillMark(
    documentId: number,
    page: number,
    x: number,
    y: number,
    kind: MarkKind,
    size?: number,
    color?: [number, number, number],
  ): Promise<EditOutcome>;
  /**
   * Moves a Fill & Sign item to another page, to a box on that displayed page
   * (ADR 0045): one undo step. It goes last in that page's annotations.
   */
  moveAnnotationToPage(
    documentId: number,
    page: number,
    index: number,
    toPage: number,
    rect: ViewRect,
  ): Promise<EditOutcome>;
  /** Recolours a Fill & Sign text or mark (ADR 0045): one undo step. */
  setAnnotationColor(
    documentId: number,
    page: number,
    index: number,
    color: [number, number, number],
  ): Promise<EditOutcome>;
  /** Places a kept signature `width` points wide, its top-left corner displayed at (`x`, `y`). */
  placeSignature(
    documentId: number,
    page: number,
    x: number,
    y: number,
    width: number,
    signature: number,
  ): Promise<EditOutcome>;
  /** Moves or resizes a Fill & Sign item to a box on the displayed page. */
  setAnnotationRect(
    documentId: number,
    page: number,
    index: number,
    rect: ViewRect,
  ): Promise<EditOutcome>;
  /** The kept signatures and initials, newest first. */
  signatures(): Promise<SignatureInfo[]>;
  /** Keeps a drawn signature (points in the pad's pixels, y down). */
  saveDrawnSignature(
    initials: boolean,
    strokes: StrokePoint[][],
    width: number,
  ): Promise<SignatureInfo>;
  /** Keeps a typed signature. */
  saveTypedSignature(
    initials: boolean,
    text: string,
    family: "sans" | "naskh",
  ): Promise<SignatureInfo>;
  /** Asks for a PNG or JPEG and keeps it; null if cancelled. */
  importSignatureImage(initials: boolean): Promise<SignatureInfo | null>;
  /** Forgets a kept signature. */
  deleteSignature(id: number): Promise<void>;
  /** Where the webview loads a kept image signature from. */
  signatureImageUrl(id: number): string;
  /** Removes annotations of one page (with their pop-ups and replies) in one undo step. */
  removeAnnotations(documentId: number, page: number, indices: number[]): Promise<EditOutcome>;
  /** The page's annotations of the kinds the app writes. */
  pageAnnotations(documentId: number, page: number): Promise<PageAnnotation[]>;
  /** A page's objects in drawing order (ADR 0051). */
  pageObjects(documentId: number, page: number): Promise<PageObject[]>;
  /** Moves, resizes, rotates, crops, removes or reorders an object: one undo step. */
  editObject(
    documentId: number,
    page: number,
    object: { index: number; fingerprint: number },
    edit: ObjectEdit,
  ): Promise<ObjectEditOutcome>;
  /** Asks for a PNG or JPEG and puts it in an image's place; null if cancelled. */
  replaceImage(
    documentId: number,
    page: number,
    object: { index: number; fingerprint: number },
  ): Promise<ObjectEditOutcome | null>;
  /** Asks for a PNG or JPEG and places it on a page, centred on `at` (or the page); null if cancelled. */
  insertImage(
    documentId: number,
    page: number,
    at: { x: number; y: number } | null,
  ): Promise<ObjectEditOutcome | null>;
  /** A page's links and where they go. */
  pageLinks(documentId: number, page: number): Promise<LinkInfo[]>;
  /** The document's named destinations. */
  namedDestinations(documentId: number): Promise<NamedDestination[]>;
  /** Adds, points elsewhere, moves or removes a link: one undo step. */
  editLink(documentId: number, edit: LinkEdit): Promise<EditOutcome>;
  /** Opens a web or mail address in the system's own app, after the user has seen it. */
  openLink(url: string): Promise<void>;
  /**
   * Asks for PDFs and inserts their pages before page `at`, in order (ADR 0016);
   * null if cancelled.
   */
  insertPagesFromFiles(documentId: number, at: number): Promise<InsertOutcome | null>;
  /** Tries the password of the file an insert waits for, and goes on. */
  unlockInsert(token: number, password: string): Promise<InsertOutcome>;
  /** Draws a header, footer, page numbers, or watermark on `pages`: one undo step (ADR 0050). */
  stampPages(documentId: number, pages: number[], stamp: StampArg): Promise<EditOutcome>;
  /** Asks for a PNG or JPEG for an image watermark; its name, or null if cancelled. */
  pickWatermarkImage(): Promise<string | null>;
  /** Asks where, and writes `pages` as images at `dpi`; how many files, or null if cancelled. */
  exportPageImages(
    documentId: number,
    pages: number[],
    dpi: number,
    format: ImageFormat,
  ): Promise<number | null>;
  /** Asks where, and writes the text of `pages`; the file's name, or null if cancelled. */
  exportText(documentId: number, pages: number[], format: TextFormat): Promise<string | null>;
  /** Compresses in memory and reports the sizes; `saveCompressed` writes the result. */
  compressDocument(documentId: number, options: CompressOptions): Promise<CompressReport>;
  /** Asks where, and writes the compressed copy; its name, or null if cancelled. */
  saveCompressed(documentId: number): Promise<string | null>;
  /** Asks where, and writes `pages` as a new PDF; its name, or null if cancelled. */
  extractPages(documentId: number, pages: number[]): Promise<string | null>;
  /** Asks for a folder and writes the document in parts; how many, or null (ADR 0016). */
  splitDocument(documentId: number, by: SplitBy): Promise<number | null>;
  /** Where a split at the top-level bookmarks would fall (fewer than two: none). */
  bookmarkParts(documentId: number): Promise<BookmarkPart[]>;
  /** Saves over the document's file (ADR 0015). */
  saveDocument(documentId: number): Promise<SaveOutcome>;
  /** Asks where to save, saves there, and the document becomes that file; null if cancelled. */
  saveDocumentAs(documentId: number): Promise<SaveOutcome | null>;
  /**
   * Saves with passwords and permissions set, changed or removed (ADR 0043),
   * over the document's file or (`saveAs`) to a file picked; null if cancelled.
   * Earlier edits are written in and undo starts over.
   */
  secureDocument(
    documentId: number,
    change: SecurityChange,
    saveAs: boolean,
  ): Promise<SecureOutcome | null>;
  /** Identities to sign with: files read this run, and the macOS keychain's (ADR 0048). */
  signingIdentities(): Promise<SigningIdentity[]>;
  /** Shows the native picker for a `.p12`/`.pfx` file; null if cancelled. */
  pickCertificateFile(): Promise<PickedCertificate | null>;
  /** Opens a picked file with its password; rejects with `certificate_password` to try again. */
  unlockCertificate(token: number, password: string): Promise<SigningIdentity>;
  cancelCertificate(token: number): Promise<void>;
  forgetIdentity(id: string): Promise<void>;
  /**
   * Signs and saves over the document's file or (`saveAs`) to one picked;
   * null if cancelled. The signed file is shown from then on; undo starts over.
   */
  signDocument(
    documentId: number,
    request: CertSignRequest,
    saveAs: boolean,
  ): Promise<SignOutcome | null>;
  /** The document's signatures, checked against the system's trusted roots. */
  documentSignatures(documentId: number): Promise<SignatureReport[]>;
  /** Gives the document's tab to a new window; the caller drops its own tab. */
  moveToNewWindow(documentId: number): Promise<void>;
  /**
   * Calls `listener` whenever any window stores a setting, so every window
   * follows a change made in one. Returns a function that stops listening.
   */
  onSettingChanged(listener: (key: string, value: string) => void): Promise<() => void>;
  /** Documents arrived for this window while the app runs (ADR 0046). */
  onDocumentsArrived(listener: (arrived: DocumentsArrived) => void): Promise<() => void>;
  /** The shell asks this window to confirm closing it: it holds unsaved changes (ADR 0015). */
  onCloseRequested(listener: () => void): Promise<() => void>;
  /** The shell asks this window to confirm quitting: documents hold unsaved changes. */
  onQuitRequested(listener: () => void): Promise<() => void>;
  /** Closes this window without saving its documents (the user confirmed). */
  closeWindowDiscarding(): Promise<void>;
  /** Quits without saving (the user confirmed). */
  quitDiscarding(): Promise<void>;
  selftestConfig(): Promise<SelftestConfig | null>;
  selftestReport(results: unknown, canvasPng: string | null): Promise<void>;
  /** Devtools builds only; null elsewhere. */
  renderStats(): Promise<RenderStats | null>;
  /** Labels of the open windows; devtools builds only. */
  windowLabels(): Promise<string[] | null>;
  /** Resident memory of the shell process in kilobytes; devtools builds only. */
  shellRssKb(): Promise<number | null>;
  /**
   * Resident memory of the webview's own processes (WebKit content and GPU
   * processes on macOS), found heuristically; devtools builds only.
   */
  webviewRssKb(): Promise<number | null>;
  /** Writes a progress line to the shell's log during a self-test; devtools builds only. */
  selftestLog(message: string): Promise<void>;
  /** A moment of the start-up, for the cold-start budget; devtools builds only. */
  selftestMark(name: string): Promise<void>;
  /** Fails on purpose, for the crash report self-test; devtools builds only. */
  selftestCrash(kind: "panic" | "thread-panic" | "abort"): Promise<void>;
  /** The crash reporter's state, for the crash report self-test; devtools builds only. */
  selftestCrashState(): Promise<unknown>;
  /** Kept crash reports and where they can be filed (ADR 0061). */
  crashReports(): Promise<CrashReports>;
  /**
   * Reports the user has not been asked about yet. Each is offered once, so
   * they are marked as offered now. Empty when crash reports are off.
   */
  newCrashReports(): Promise<CrashReportInfo[]>;
  /** Reports were added while the app runs (a system crash log found late). */
  onCrashReportsAdded(listener: () => void): Promise<() => void>;
  deleteCrashReport(id: string): Promise<void>;
  /** Saves a report as a text file where the user chooses; false when they cancel. */
  saveCrashReport(id: string): Promise<boolean>;
  /**
   * Opens a new GitHub issue with the report filled in, in the browser. True
   * when the stack was shortened to fit the link.
   */
  openCrashIssue(id: string): Promise<boolean>;
  /**
   * Keeps an error the UI did not handle as a report (redacted by the shell).
   * Its id, or null when none was kept (reports off, or enough this run).
   */
  recordUiError(message: string, stack: string, commands: string[]): Promise<string | null>;
  /** The OCR models offered here, what is installed, and the hardware (ADR 0034). */
  ocrModels(): Promise<OcrModels>;
  /**
   * Starts downloading and installing a package and returns at once; progress
   * and the outcome arrive through `onOcrModelProgress`. Rejects with
   * `model_busy` while it is already downloading.
   */
  downloadOcrModel(id: string): Promise<void>;
  /** Stops a download; what arrived is kept to resume. */
  cancelOcrModelDownload(id: string): Promise<void>;
  /** Removes a package and any partial download of it; `model_busy` while it downloads. */
  deleteOcrModel(id: string): Promise<void>;
  onOcrModelProgress(listener: (event: OcrModelEvent) => void): Promise<() => void>;
  /**
   * Queues OCR of a document's pages (zero-based; all when absent) as a
   * background job (ADR 0035). Rejects with `ocr_model_missing`,
   * `ocr_signed_document` or `ocr_unavailable`.
   */
  startOcr(documentId: number, pages?: number[]): Promise<OcrJob>;
  /** The OCR jobs of a document, oldest first. */
  ocrJobs(documentId: number): Promise<OcrJob[]>;
  controlOcrJob(jobId: number, action: OcrJobAction): Promise<OcrJob>;
  /** Forgets a job that is not running, and its part of the review list. */
  removeOcrJob(jobId: number): Promise<void>;
  /** The pages-needing-review list of a document: decisions first. */
  ocrReview(documentId: number): Promise<OcrReviewPage[]>;
  /** Jobs moving on (every window). */
  onOcrJob(listener: (job: OcrJob) => void): Promise<() => void>;
  /** OCR text written into a document shown in this window. */
  onOcrApplied(listener: (applied: OcrApplied) => void): Promise<() => void>;
}

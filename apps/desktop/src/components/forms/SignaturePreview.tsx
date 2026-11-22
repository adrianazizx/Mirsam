import { backend, type SignatureInfo } from "../../backend";
import styles from "./SignaturePreview.module.css";

/** An SVG path through each stroke's points. */
export function strokePaths(strokes: [number, number][][]): string[] {
  return strokes
    .filter((stroke) => stroke.length > 0)
    .map((stroke) => {
      const [first, ...rest] = stroke as [[number, number], ...[number, number][]];
      // A single point is a dot: a zero-length line with round caps.
      const tail = rest.length > 0 ? rest : [first];
      return `M${first[0]} ${first[1]}${tail.map(([x, y]) => `L${x} ${y}`).join("")}`;
    });
}

/** The box around the strokes, with room for the pen. */
export function strokeBox(strokes: [number, number][][], pen: number): string {
  const points = strokes.flat();
  if (points.length === 0) return "0 0 1 1";
  const xs = points.map(([x]) => x);
  const ys = points.map(([, y]) => y);
  const margin = pen;
  const x0 = Math.min(...xs) - margin;
  const y0 = Math.min(...ys) - margin;
  return `${x0} ${y0} ${Math.max(...xs) - x0 + margin} ${Math.max(...ys) - y0 + margin}`;
}

/**
 * A kept signature as it will look: drawn strokes as paths, typed text in
 * its family, an image as the webview decodes it (ADR 0011). Shown as ink
 * on paper in either theme. Decorative: the control around it names it.
 */
export function SignaturePreview({ signature }: { signature: SignatureInfo }) {
  if (signature.kind === "drawn") {
    return (
      <svg
        className={styles.preview}
        viewBox={strokeBox(signature.strokes, signature.pen)}
        preserveAspectRatio="xMidYMid meet"
        aria-hidden="true"
      >
        {strokePaths(signature.strokes).map((d) => (
          <path key={d} d={d} className={styles.stroke} strokeWidth={signature.pen} />
        ))}
      </svg>
    );
  }
  if (signature.kind === "typed") {
    return (
      <span
        className={`${styles.preview} ${styles.typed} ${signature.family === "naskh" ? styles.naskh : ""}`}
        dir="auto"
        aria-hidden="true"
      >
        {signature.text}
      </span>
    );
  }
  return <img className={styles.preview} src={backend.signatureImageUrl(signature.id)} alt="" />;
}

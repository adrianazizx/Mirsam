import { existsSync } from "node:fs";
import path from "node:path";
import type { Lang } from "./i18n";

/** Every screenshot is the app's default 1280×820 window, encoded at this size (scripts/encode-screens.sh). */
export const SHOT_WIDTH = 1600;
export const SHOT_HEIGHT = 1025;

const PUBLIC = path.resolve(process.cwd(), "public");

/**
 * The screenshot for this language (the app in Arabic on the Arabic page),
 * falling back to the English one, or null when neither has been captured.
 */
export function shot(id: string, lang: Lang): string | null {
  for (const candidate of [`${id}-${lang}`, `${id}-en`]) {
    if (existsSync(path.join(PUBLIC, "screens", `${candidate}.webp`))) return `/screens/${candidate}.webp`;
  }
  return null;
}

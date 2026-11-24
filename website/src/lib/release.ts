import { readFileSync } from "node:fs";
import path from "node:path";

export const REPO = "adrianazizx/Mirsam";
export const REPO_URL = `https://github.com/${REPO}`;
export const RELEASES_URL = `${REPO_URL}/releases`;
export const DOCS_URL = `${REPO_URL}/blob/main/docs`;

/** Asset names carry no version, so this link always serves the newest release. */
export const downloadUrl = (file: string): string => `${RELEASES_URL}/latest/download/${file}`;

/**
 * The version on main, from the app's own package.json (kept in step by
 * scripts/set-version.mjs). The build runs in website/, beside apps/.
 */
function appVersion(): string {
  const file = path.resolve(process.cwd(), "../apps/desktop/package.json");
  const { version } = JSON.parse(readFileSync(file, "utf8")) as { version?: string };
  if (!version) throw new Error(`no version in ${file}`);
  return version;
}

interface GithubAsset {
  name: string;
  size: number;
}

/**
 * Installer sizes from the latest GitHub release, when that release is the
 * version on main. Shown only when known: a failed or rate-limited lookup
 * leaves the sizes out rather than guessing them.
 */
async function assetSizes(version: string): Promise<Record<string, number>> {
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json", "User-Agent": "mirsam-website-build" },
      signal: AbortSignal.timeout(5000),
    });
    if (!response.ok) return {};
    const release = (await response.json()) as { tag_name?: string; assets?: GithubAsset[] };
    if (release.tag_name?.replace(/^v/, "") !== version) return {};
    return Object.fromEntries((release.assets ?? []).map((asset) => [asset.name, asset.size]));
  } catch {
    return {};
  }
}

const version = appVersion();
const sizes = await assetSizes(version);

export const release = { version, sizes };

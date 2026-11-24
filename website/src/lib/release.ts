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

interface PublishedRelease {
  version: string;
  sizes: Record<string, number>;
}

/**
 * The latest published GitHub release: what the download links serve. Main can
 * be ahead of it (the version is bumped before the tag is pushed and the draft
 * published), so the page labels what visitors actually get.
 */
async function latestRelease(): Promise<PublishedRelease | null> {
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: "application/vnd.github+json", "User-Agent": "mirsam-website-build" },
      signal: AbortSignal.timeout(5000),
    });
    if (!response.ok) return null;
    const release = (await response.json()) as { tag_name?: string; assets?: GithubAsset[] };
    if (!release.tag_name) return null;
    return {
      version: release.tag_name.replace(/^v/, ""),
      sizes: Object.fromEntries((release.assets ?? []).map((asset) => [asset.name, asset.size])),
    };
  } catch {
    return null;
  }
}

// If GitHub cannot be reached (offline, rate-limited), fall back to the version
// on main and leave the sizes out rather than guess them.
const published = await latestRelease();

export const release: PublishedRelease = published ?? { version: appVersion(), sizes: {} };

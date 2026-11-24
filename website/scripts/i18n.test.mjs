// Every string on the page exists in English and in Arabic, with the same shape
// (same keys, same list lengths) and the same {placeholders}.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const load = (lang) => JSON.parse(readFileSync(new URL(`../src/i18n/${lang}.json`, import.meta.url), "utf8"));
const en = load("en");
const ar = load("ar");

function walk(a, b, at) {
  if (Array.isArray(a)) {
    assert.ok(Array.isArray(b), `${at}: an array in en, not in ar`);
    // The marquee mixes both languages and may differ in length.
    if (at !== "marquee") assert.equal(b.length, a.length, `${at}: ${a.length} items in en, ${b.length} in ar`);
    a.forEach((item, i) => b[i] !== undefined && walk(item, b[i], `${at}[${i}]`));
  } else if (a && typeof a === "object") {
    assert.deepEqual(Object.keys(b).sort(), Object.keys(a).sort(), `${at}: keys differ`);
    for (const key of Object.keys(a)) walk(a[key], b[key], at ? `${at}.${key}` : key);
  } else {
    assert.equal(typeof b, typeof a, `${at}: type differs`);
    if (typeof a === "string") {
      assert.ok(b.trim().length > 0, `${at}: empty in ar`);
      const holes = (s) => [...s.matchAll(/\{(\w+)\}/g)].map((m) => m[1]).sort();
      assert.deepEqual(holes(b), holes(a), `${at}: placeholders differ`);
    }
  }
}

test("en.json and ar.json have the same strings", () => walk(en, ar, ""));

test("identifiers that the page relies on are not translated", () => {
  const ids = (o) => o.features.chapters.map((c) => `${c.id}/${c.shot}`);
  assert.deepEqual(ids(ar), ids(en));
  const files = (o) => o.download.platforms.map((p) => `${p.id}:${p.files.map((f) => f.file).join(",")}`);
  assert.deepEqual(files(ar), files(en));
});

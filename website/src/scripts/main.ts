// Page behaviour: smooth scrolling, reveal-on-scroll, scroll-linked progress
// (the hero window, the OCR scan, the pipeline, the privacy lines), the feature
// story, the Arabic demos, the compare slider and the download button that
// knows the visitor's computer. With reduced motion everything is shown in its
// finished state and nothing is scroll-linked.

import Lenis from "lenis";

const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const $ = <T extends HTMLElement = HTMLElement>(selector: string, root: ParentNode = document): T | null =>
  root.querySelector<T>(selector);
const $$ = <T extends HTMLElement = HTMLElement>(selector: string, root: ParentNode = document): T[] => [
  ...root.querySelectorAll<T>(selector),
];
const clamp = (v: number, min = 0, max = 1) => Math.min(max, Math.max(min, v));
const capitalize = (s: string) => `${s[0]?.toUpperCase() ?? ""}${s.slice(1)}`;

/* Smooth scrolling */
const lenis = reduced ? null : new Lenis({ autoRaf: true, anchors: { offset: -80 }, lerp: 0.11 });

/* Header turns solid once the page scrolls */
const header = $("[data-header]");
const onHeader = () => header?.classList.toggle("is-scrolled", window.scrollY > 12);

/* Reveal elements as they enter the viewport */
const revealer = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      entry.target.classList.add("is-in");
      revealer.unobserve(entry.target);
    }
  },
  { rootMargin: "0px 0px -8% 0px", threshold: 0.12 },
);
for (const el of $$("[data-reveal]")) revealer.observe(el);

/*
 * Scroll-linked progress: --p goes from 0 to 1 while the target's top moves
 * from `start` to `end` (fractions of the viewport height).
 */
interface Linked {
  el: HTMLElement;
  target: HTMLElement;
  start: number;
  end: number;
  last: number;
}
const linked: Linked[] = [];
function link(el: HTMLElement | null, start: number, end: number, target?: HTMLElement | null) {
  if (!el) return;
  if (reduced) {
    el.style.setProperty("--p", "1");
    return;
  }
  linked.push({ el, target: target ?? el, start, end, last: -1 });
}

link($("[data-hero-shot]"), 0.95, 0.3, $("[data-hero-stage]"));
link($("[data-scan]"), 0.75, 0.05);
link($("[data-pipeline]"), 0.85, 0.45);
link($("[data-privacy]"), 0.75, -0.1);

function updateLinked() {
  const vh = window.innerHeight;
  for (const item of linked) {
    const top = item.target.getBoundingClientRect().top / vh;
    const p = clamp((item.start - top) / (item.start - item.end));
    if (Math.abs(p - item.last) < 0.001) continue;
    item.last = p;
    item.el.style.setProperty("--p", p.toFixed(4));
  }
}

/* Navigation: the section being read, and the language switch that keeps it */
const navLinks = $$<HTMLAnchorElement>("[data-nav-link]");
const langSwitch = $<HTMLAnchorElement>("[data-lang-switch]");
const langBase = langSwitch?.getAttribute("href") ?? "/";
let currentHash = "";

function updateNav() {
  const probe = window.innerHeight * 0.4;
  let hash = "";
  for (const a of navLinks) {
    const section = document.getElementById(a.hash.slice(1));
    if (section && section.getBoundingClientRect().top <= probe) hash = a.hash;
  }
  const download = document.getElementById("download");
  if (download && download.getBoundingClientRect().top <= probe) hash = "#download";
  if (hash === currentHash) return;
  currentHash = hash;
  for (const a of navLinks) {
    const current = a.hash === hash;
    a.classList.toggle("is-current", current);
    if (current) a.setAttribute("aria-current", "true");
    else a.removeAttribute("aria-current");
  }
  if (langSwitch) langSwitch.href = `${langBase}${hash}`;
}

let ticking = false;
function onScroll() {
  if (ticking) return;
  ticking = true;
  requestAnimationFrame(() => {
    ticking = false;
    onHeader();
    updateLinked();
    updateNav();
  });
}
window.addEventListener("scroll", onScroll, { passive: true });
window.addEventListener("resize", onScroll, { passive: true });
onScroll();

/* Hero light follows the pointer */
const hero = $("[data-hero]");
if (hero && !reduced && window.matchMedia("(pointer: fine)").matches) {
  hero.addEventListener("pointermove", (event) => {
    const rect = hero.getBoundingClientRect();
    hero.style.setProperty("--mx", `${((event.clientX - rect.left) / rect.width) * 100}%`);
    hero.style.setProperty("--my", `${((event.clientY - rect.top) / rect.height) * 100}%`);
  });
}

/* Cards: a glow that follows the pointer */
for (const card of $$("[data-glow]")) {
  card.addEventListener("pointermove", (event) => {
    const rect = card.getBoundingClientRect();
    card.style.setProperty("--gx", `${event.clientX - rect.left}px`);
    card.style.setProperty("--gy", `${event.clientY - rect.top}px`);
  });
}

/* Feature story: the chapter in the middle of the viewport picks the screenshot */
const slides = $$("[data-slide]");
const dots = $$("[data-dot]");
const story = new IntersectionObserver(
  (entries) => {
    for (const entry of entries) {
      if (!entry.isIntersecting) continue;
      const id = (entry.target as HTMLElement).dataset.chapter;
      for (const slide of slides) slide.classList.toggle("is-active", slide.dataset.slide === id);
      for (const dot of dots) dot.classList.toggle("is-active", dot.dataset.dot === id);
    }
  },
  { rootMargin: "-45% 0px -45% 0px" },
);
for (const chapter of $$("[data-chapter]")) story.observe(chapter);

/* Search demo: type the query, then show the results */
const searchDemo = $("[data-demo='search']");
const typed = searchDemo ? $("[data-type]", searchDemo) : null;
if (searchDemo && typed) {
  const query = typed.dataset.type ?? "";
  if (reduced) {
    searchDemo.classList.add("is-found");
  } else {
    typed.textContent = "";
    const observer = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        observer.disconnect();
        // Letter by letter, so the Arabic joins up as it would in a real field.
        const letters = [...query];
        let i = 0;
        const step = () => {
          i += 1;
          typed.textContent = letters.slice(0, i).join("");
          if (i < letters.length) window.setTimeout(step, 140 + Math.random() * 90);
          else window.setTimeout(() => searchDemo.classList.add("is-found"), 350);
        };
        window.setTimeout(step, 700);
      },
      { threshold: 0.5 },
    );
    observer.observe(searchDemo);
  }
}

/* Compare the English and Arabic interfaces */
const compare = $("[data-compare]");
const range = compare ? $<HTMLInputElement>("[data-compare-range]", compare) : null;
if (compare && range) {
  const set = () => compare.style.setProperty("--pos", `${range.value}%`);
  range.addEventListener("input", set);
  set();
  // A gentle hint that it moves, once, when it first appears.
  if (!reduced) {
    const hint = new IntersectionObserver(
      (entries) => {
        if (!entries.some((e) => e.isIntersecting)) return;
        hint.disconnect();
        const start = performance.now();
        const animate = (now: number) => {
          const t = clamp((now - start) / 1800);
          range.value = String(50 + Math.sin(t * Math.PI * 2) * 18 * (1 - t));
          set();
          if (t < 1) requestAnimationFrame(animate);
        };
        requestAnimationFrame(animate);
      },
      { threshold: 0.6 },
    );
    hint.observe(compare);
  }
}

/* The visitor's computer: the hero button downloads for it, its card is marked */
type Platform = "mac" | "windows" | "linux";
function detectPlatform(): Platform | null {
  const nav = navigator as Navigator & { userAgentData?: { platform?: string; mobile?: boolean } };
  const ua = navigator.userAgent;
  if (nav.userAgentData?.mobile || /Android|iPhone|iPad|iPod/i.test(ua)) return null;
  const platform = (nav.userAgentData?.platform || ua).toLowerCase();
  if (platform.includes("mac")) return "mac";
  if (platform.includes("win")) return "windows";
  if (platform.includes("linux") || platform.includes("x11")) return "linux";
  return null;
}

const platform = detectPlatform();
const cta = $<HTMLAnchorElement>("[data-os-cta]");
if (platform && cta) {
  const name = cta.dataset[`os${capitalize(platform)}`];
  const label = $("[data-os-cta-label]", cta);
  if (name && label && cta.dataset.template) label.textContent = cta.dataset.template.replace("{os}", name);
  const href = cta.dataset[`href${capitalize(platform)}`];
  if (href) {
    cta.href = href;
    cta.setAttribute("download", "");
  }
  $(`[data-platform="${platform}"]`)?.classList.add("is-recommended");
}

// Lenis scrolls to in-page anchors; move focus there too, for keyboard users.
document.addEventListener("click", (event) => {
  if (!lenis) return;
  const anchor = (event.target as Element).closest<HTMLAnchorElement>("a[href^='#']");
  const target = anchor ? document.getElementById(anchor.hash.slice(1)) : null;
  if (!target) return;
  window.setTimeout(() => {
    if (!target.hasAttribute("tabindex")) target.setAttribute("tabindex", "-1");
    target.focus({ preventScroll: true });
  }, 900);
});

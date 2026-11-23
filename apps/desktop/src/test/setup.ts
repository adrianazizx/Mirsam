import { cleanup } from "@testing-library/react";
import { afterEach } from "vitest";
import { setOnboardingAutomatic } from "../state/onboarding";

afterEach(() => {
  cleanup();
});

// jsdom has no layout engine and no matchMedia.
if (!window.matchMedia) {
  window.matchMedia = (query: string) =>
    ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: () => {},
      removeEventListener: () => {},
      addListener: () => {},
      removeListener: () => {},
      dispatchEvent: () => false,
    }) as MediaQueryList;
}

// jsdom has no ResizeObserver; components only need it not to throw.
if (!window.ResizeObserver) {
  window.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

// The first-run guide and the tools' one-time hints stay out of the way of
// screens under test; the onboarding tests turn them on.
setOnboardingAutomatic(false);

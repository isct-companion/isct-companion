/**
 * The gaps between jsdom and the APIs the desktop webview really provides.
 *
 * Every DOM suite used to hand-roll these; `harness/setup.ts` installs them once
 * per jsdom worker instead. Keep additions here, not in individual suites.
 */

type Observer = {
  observe(): void;
  unobserve(): void;
  disconnect(): void;
  takeRecords(): unknown[];
};

const inertObserver = class implements Observer {
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords(): unknown[] {
    return [];
  }
};

export function installDomGlobals(target: typeof globalThis): void {
  const scope = target as unknown as Record<string, unknown>;
  // React's act() environment flag; 24 suites used to set this by hand.
  scope.IS_REACT_ACT_ENVIRONMENT = true;
  if (scope.ResizeObserver === undefined) scope.ResizeObserver = inertObserver;
  if (scope.IntersectionObserver === undefined)
    scope.IntersectionObserver = inertObserver;
}

export function installWindowGlobals(window: Window & typeof globalThis): void {
  const scope = window as unknown as Record<string, unknown>;
  scope.matchMedia ??= (query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener() {},
    removeEventListener() {},
    addListener() {},
    removeListener() {},
    dispatchEvent: () => false,
  });
  // jsdom leaves scrollIntoView undefined; several views call it on reveal.
  if (window.HTMLElement) {
    window.HTMLElement.prototype.scrollIntoView ??=
      function scrollIntoView() {};
  }
  if (window.navigator && !("connection" in window.navigator)) {
    Object.defineProperty(window.navigator, "connection", {
      configurable: true,
      get: () => ({ effectiveType: "4g", saveData: false, type: "wifi" }),
    });
  }
}

/** Restores the application module graph state the harness relies on. */
export function resetDocument(): void {
  if (typeof document === "undefined") return;
  document.body.replaceChildren();
  document.documentElement.removeAttribute("style");
  document.documentElement.removeAttribute("data-mantine-color-scheme");
}

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
    // jsdom has no pointer-capture implementation; Radix uses it on selects.
    window.HTMLElement.prototype.hasPointerCapture ??= () => false;
    window.HTMLElement.prototype.setPointerCapture ??= () => {};
    window.HTMLElement.prototype.releasePointerCapture ??= () => {};
    // jsdom has no layout engine. Supply viewport/row geometry while running
    // the real virtualizer; neither search results nor range selection are mocked.
    Object.defineProperty(window.HTMLElement.prototype, "offsetHeight", {
      configurable: true,
      get() {
        return this.classList.contains("finderResults")
          ? 600
          : this.classList.contains("finderRow")
            ? 96
            : this.classList.contains("diagnosticLogs")
              ? 420
              : this.classList.contains("diagnosticLogRow")
                ? 48
                : 0;
      },
    });
    Object.defineProperty(window.HTMLElement.prototype, "offsetWidth", {
      configurable: true,
      get() {
        return this.classList.contains("finderResults")
          ? 400
          : this.classList.contains("diagnosticLogs")
            ? 800
            : 0;
      },
    });
    window.HTMLElement.prototype.scrollTo ??= function scrollTo(
      options?: ScrollToOptions | number,
      y?: number,
    ) {
      if (typeof options === "number") this.scrollTop = y ?? 0;
      else if (typeof options?.top === "number") this.scrollTop = options.top;
      this.dispatchEvent(new window.Event("scroll"));
    };
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
}

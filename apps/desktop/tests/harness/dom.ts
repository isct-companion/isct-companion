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
    // the real virtualizer; neither search results nor range selection are
    // mocked. The virtualizer reads its viewport's client box (and once read
    // its offset box), so both report the same size, as they do without
    // borders.
    const geometry: Record<string, { width: number; height: number }> = {
      finderResults: { width: 400, height: 600 },
      finderRow: { width: 0, height: 96 },
      diagnosticLogs: { width: 800, height: 420 },
      diagnosticLogRow: { width: 0, height: 48 },
    };
    const size = (element: HTMLElement, axis: "width" | "height") => {
      for (const [name, box] of Object.entries(geometry))
        if (element.classList.contains(name)) return box[axis];
      return 0;
    };
    for (const [property, axis] of [
      ["offsetHeight", "height"],
      ["clientHeight", "height"],
      ["offsetWidth", "width"],
      ["clientWidth", "width"],
    ] as const)
      Object.defineProperty(window.HTMLElement.prototype, property, {
        configurable: true,
        get(this: HTMLElement) {
          return size(this, axis);
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

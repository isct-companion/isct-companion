/**
 * Development-only interaction trace: one timeline of what the person did
 * (clicks, named keys, route changes), what the shell asked the backend
 * (every Tauri invoke with its duration and outcome), what the backend pushed
 * (events) and every error, so a crash found by hand can be read back step by
 * step. The dev server writes it to tmp/interaction-trace/ (see
 * scripts/desktop/interaction-trace.mjs). Typed text is never recorded and
 * argument values under credential-like keys are redacted.
 */
import { listen } from "@tauri-apps/api/event";
import type { createHashRouter } from "react-router";

type TraceEvent = { kind: string } & Record<string, unknown>;

const BACKEND_EVENTS = [
  "workspace-changed",
  "sync-snapshot",
  "school-ingest",
  "school-download",
  "school-login-status",
  "cloud-sync-state",
];
const SECRET_KEY = /pass|secret|token|cookie|credential|otp|auth/i;
const INTERACTIVE =
  "button,a,[role],input,select,textarea,label,summary,[data-testid]";

const session = new Date().toISOString().replace(/[:.]/g, "-");
const origin = performance.now();
let queue: TraceEvent[] = [];
let timer: ReturnType<typeof setTimeout> | undefined;
let installed = false;
let invokeSeq = 0;

const flush = () => {
  clearTimeout(timer);
  timer = undefined;
  if (!queue.length) return;
  const body = JSON.stringify({ session, events: queue });
  queue = [];
  void fetch("/__interaction-trace", {
    method: "POST",
    body,
    keepalive: true,
  }).catch(() => undefined);
};

/** Records one event; a no-op unless the dev recorder is installed. */
export function traceEvent(kind: string, data: Record<string, unknown> = {}) {
  if (!installed) return;
  queue.push({
    t: Math.round(performance.now() - origin),
    at: new Date().toISOString(),
    kind,
    route: location.hash || "#/",
    ...data,
  });
  if (kind === "error") flush();
  else timer ??= setTimeout(flush, 400);
}

const text = (value: string, max = 120) =>
  value.length > max ? `${value.slice(0, max)}…(${value.length})` : value;

/** A small, secret-free picture of an invoke payload or result. */
function summarize(value: unknown, depth = 0): unknown {
  if (value === null || value === undefined) return value;
  if (typeof value === "string") return text(value);
  if (typeof value !== "object") return value;
  if (Array.isArray(value))
    return value.length > 4 || depth > 2
      ? `[${value.length} items]`
      : value.map((item) => summarize(item, depth + 1));
  const name = value.constructor?.name;
  if (name && name !== "Object") return `[${name}]`;
  if (depth > 2) return "{…}";
  return Object.fromEntries(
    Object.entries(value).map(([key, item]) => [
      key,
      SECRET_KEY.test(key) ? "[redacted]" : summarize(item, depth + 1),
    ]),
  );
}

const errorFields = (error: unknown) =>
  error instanceof Error
    ? {
        message: text(error.message, 500),
        stack: error.stack?.split("\n").slice(0, 8).join("\n"),
      }
    : { message: text(String(error), 500) };

function describe(node: EventTarget | null) {
  if (!(node instanceof Element)) return undefined;
  const hit = node.closest(INTERACTIVE);
  const el = hit ?? node;
  const field = el.matches("input,textarea,select");
  // Text of a bare container (body, a panel) is noise, so only named controls get a label.
  const label =
    el.getAttribute("aria-label") ??
    (field
      ? (el.getAttribute("placeholder") ?? el.getAttribute("name"))
      : hit?.textContent);
  const path: string[] = [];
  for (
    let at: Element | null = el;
    at && path.length < 5;
    at = at.parentElement
  )
    path.unshift(
      at.tagName.toLowerCase() +
        (at.classList.length
          ? `.${[...at.classList].slice(0, 2).join(".")}`
          : ""),
    );
  return {
    tag: el.tagName.toLowerCase(),
    role: el.getAttribute("role") ?? undefined,
    label: label ? text(label.replace(/\s+/g, " ").trim(), 60) : undefined,
    href: el.getAttribute("href") ?? undefined,
    path: path.join(" > "),
  };
}

/** Runs one Tauri invoke and records its arguments, duration and outcome. */
export async function tracedInvoke<T>(
  cmd: string,
  args: unknown,
  run: () => Promise<T>,
): Promise<T> {
  if (!installed) return run();
  const id = ++invokeSeq;
  const started = performance.now();
  traceEvent("invoke", { id, cmd, args: summarize(args) });
  try {
    const result = await run();
    traceEvent("invoked", {
      id,
      cmd,
      ms: Math.round(performance.now() - started),
      result: summarize(result, 2),
    });
    return result;
  } catch (error) {
    traceEvent("rejected", {
      id,
      cmd,
      ms: Math.round(performance.now() - started),
      ...errorFields(error),
    });
    throw error;
  }
}

/** Records route changes and route-level errors from the app router. */
export function traceRouter(router: ReturnType<typeof createHashRouter>) {
  if (!installed) return;
  let last = "";
  router.subscribe((state) => {
    const path = state.location.pathname + state.location.search;
    if (path !== last)
      traceEvent("route", { to: path, action: state.historyAction });
    last = path;
    if (state.errors)
      for (const [routeId, error] of Object.entries(state.errors))
        traceEvent("error", {
          source: `route ${routeId}`,
          ...errorFields(error),
        });
  });
}

export function installInteractionTrace() {
  if (installed) return;
  installed = true;
  traceEvent("start", { userAgent: navigator.userAgent });
  for (const type of ["click", "auxclick", "contextmenu"] as const)
    document.addEventListener(
      type,
      (event) =>
        traceEvent(type, {
          target: describe(event.target),
          x: event.clientX,
          y: event.clientY,
        }),
      { capture: true },
    );
  document.addEventListener(
    "keydown",
    (event) => {
      // Named keys and shortcuts only; plain characters would record typing.
      const shortcut = event.metaKey || event.ctrlKey || event.altKey;
      if (event.key.length === 1 && !shortcut) return;
      traceEvent("key", {
        key: event.key,
        meta: event.metaKey || undefined,
        ctrl: event.ctrlKey || undefined,
        alt: event.altKey || undefined,
        shift: event.shiftKey || undefined,
        target: describe(event.target),
      });
    },
    { capture: true },
  );
  // Native <select> menus and pickers change values without a DOM click.
  // Choices are recorded; typed text only as "changed".
  document.addEventListener(
    "change",
    (event) => {
      const field = event.target;
      traceEvent("change", {
        target: describe(field),
        ...(field instanceof HTMLSelectElement
          ? { value: text(field.value, 80) }
          : field instanceof HTMLInputElement &&
              (field.type === "checkbox" || field.type === "radio")
            ? { checked: field.checked }
            : {}),
      });
    },
    { capture: true },
  );
  window.addEventListener("error", (event) =>
    traceEvent("error", {
      source: "window",
      ...errorFields(event.error ?? event.message),
      location: `${event.filename}:${event.lineno}:${event.colno}`,
    }),
  );
  window.addEventListener("unhandledrejection", (event) =>
    traceEvent("error", {
      source: "unhandledrejection",
      ...errorFields(event.reason),
    }),
  );
  for (const level of ["error", "warn"] as const) {
    const original = console[level].bind(console);
    console[level] = (...args: unknown[]) => {
      traceEvent(`console.${level}`, {
        message: text(
          args
            .map((arg) => (arg instanceof Error ? arg.message : String(arg)))
            .join(" "),
          800,
        ),
      });
      original(...args);
    };
  }
  document.addEventListener("visibilitychange", () =>
    traceEvent("visibility", { state: document.visibilityState }),
  );
  window.addEventListener("pagehide", flush);
  for (const name of BACKEND_EVENTS)
    void listen(name, (event) =>
      traceEvent("backend", { event: name, payload: summarize(event.payload) }),
    ).catch(() => undefined);
}

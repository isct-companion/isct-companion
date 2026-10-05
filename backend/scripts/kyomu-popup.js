(() => {
  // 教務Web opens its pickers with showDialog() → window.open(), and they
  // answer through a live opener: picking a course runs
  // `window.opener.__doPostBack(...)`, then `window.close()`. Embedded views
  // have no popup windows, so on 教務Web a same-origin window.open shows the
  // page in an overlay frame inside the opener page, and that frame gets an
  // opener and a close() that work. Runs at document start in every frame.
  const HOST = "kyomu0.gakumu.titech.ac.jp";
  if (location.protocol !== "https:" || location.hostname !== HOST) return;

  const isNative = (value) => {
    try {
      return /\{\s*\[native code\]\s*\}\s*$/.test(
        Function.prototype.toString.call(value),
      );
    } catch {
      return false;
    }
  };
  const forward = (target) => (value) =>
    typeof value === "function" && isNative(value) ? value.bind(target) : value;

  // Reloading the opener would re-POST its last postback (a second 申告, or
  // "不正な画面遷移"); a fresh GET of the same page shows the same result.
  const safeLocation = (win) => {
    const bind = forward(win.location);
    return new Proxy(
      {},
      {
        get: (_, key) =>
          key === "reload"
            ? () => win.location.replace(win.location.href)
            : bind(win.location[key]),
        set: (_, key, value) => {
          win.location[key] = value;
          return true;
        },
      },
    );
  };
  // A stand-in for the opener window. Plain-object target: a Proxy over the
  // window itself could not answer its non-configurable properties.
  const openerFor = (win) => {
    const bind = forward(win);
    const location = safeLocation(win);
    return new Proxy(
      {},
      {
        get: (_, key) => (key === "location" ? location : bind(win[key])),
        set: (_, key, value) => {
          win[key] = value;
          return true;
        },
        has: (_, key) => key in win,
      },
    );
  };

  if (window !== top) {
    let frame = null;
    try {
      frame = window.frameElement;
    } catch {
      return;
    }
    const popups =
      frame && frame.hasAttribute("data-stws-popup") && top.__stwsPopups;
    if (!popups) return;
    try {
      window.opener = openerFor(frame.stwsOpener || parent);
    } catch {}
    window.close = () => popups.close(frame);
    // A picker opening another picker: its opener is this frame.
    window.open = (url, name, features) =>
      popups.open(url, name, features, window);
    return;
  }

  const nativeOpen = window.open;
  const overlays = new Map();
  let unnamed = 0;
  const size = (features, key, fallback, viewport) => {
    const match = new RegExp(`${key}\\s*=\\s*(\\d+)`, "i").exec(features || "");
    const pixels = Math.max(match ? Number(match[1]) : fallback, 320);
    return `min(${pixels}px, ${viewport})`;
  };
  const create = (name, features) => {
    const host = document.createElement("div");
    host.setAttribute("data-stws-popup-host", "");
    host.style.cssText =
      "position:fixed;inset:0;z-index:2147483647;display:flex;align-items:center;justify-content:center;background:rgba(15,23,42,.45)";
    const panel = document.createElement("div");
    panel.style.cssText = `display:flex;flex-direction:column;width:${size(features, "width", 960, "96vw")};height:${size(features, "height", 720, "92vh")};background:#fff;border-radius:8px;overflow:hidden;box-shadow:0 12px 40px rgba(0,0,0,.35)`;
    const bar = document.createElement("div");
    bar.style.cssText =
      "display:flex;align-items:center;gap:8px;padding:4px 6px 4px 12px;background:#1b2a8c;color:#fff;font:600 13px/1.6 system-ui,sans-serif";
    const title = document.createElement("span");
    title.style.cssText =
      "flex:1;overflow:hidden;white-space:nowrap;text-overflow:ellipsis";
    const close = document.createElement("button");
    close.type = "button";
    close.textContent = "\u00d7";
    close.title = "閉じる";
    close.setAttribute("aria-label", "閉じる");
    close.style.cssText =
      "border:0;background:transparent;color:inherit;font:inherit;font-size:20px;line-height:1;padding:2px 8px;cursor:pointer";
    const frame = document.createElement("iframe");
    // Named like the window, so a form posting to that name lands here.
    frame.name = name;
    frame.setAttribute("data-stws-popup", "");
    frame.style.cssText = "flex:1;width:100%;border:0;background:#fff";
    frame.addEventListener("load", () => {
      try {
        title.textContent = frame.contentDocument.title || "";
      } catch {}
    });
    close.addEventListener("click", () => popups.close(frame));
    bar.append(title, close);
    panel.append(bar, frame);
    host.append(panel);
    (document.body || document.documentElement).append(host);
    const entry = { host, frame };
    overlays.set(name, entry);
    return entry;
  };

  const popups = {
    open(url, name, features, opener) {
      const pass = () => nativeOpen.call(window, url, name, features);
      const target = name == null ? "" : String(name);
      if (/^_(self|parent|top)$/i.test(target)) return pass();
      const raw = url == null ? "" : String(url);
      let href = "";
      if (raw && raw !== "about:blank") {
        let parsed;
        try {
          parsed = new URL(raw, opener.location.href);
        } catch {
          return pass();
        }
        // Other services (syllabus, PDFs) keep the app's own routing.
        if (parsed.origin !== location.origin) return pass();
        href = parsed.href;
      }
      const key =
        target && target.toLowerCase() !== "_blank"
          ? target
          : `stws-popup-${++unnamed}`;
      const entry = overlays.get(key) || create(key, features);
      // Read by the frame's own copy of this script at document start.
      entry.frame.stwsOpener = opener;
      if (href) entry.frame.src = href;
      return entry.frame.contentWindow;
    },
    close(frame) {
      for (const [key, entry] of overlays) {
        if (entry.frame !== frame) continue;
        overlays.delete(key);
        // Deferred: the frame's own script is usually still running.
        setTimeout(() => entry.host.remove());
      }
    },
  };
  Object.defineProperty(window, "__stwsPopups", { value: popups });
  window.open = function open(url, name, features) {
    return popups.open(url, name, features, window);
  };
})();

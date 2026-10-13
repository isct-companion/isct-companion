((config) => {
  // SAML hand-off pages auto-submit a form on load; never touch them.
  if (
    document.querySelector(
      'input[name="SAMLResponse"],input[name="SAMLRequest"]',
    )
  )
    return;

  // One instance runs per page load. Everything happens synchronously from DOM
  // changes: sign-in usually runs in a hidden webview, where WebKit throttles
  // timers to seconds or more. Each step acts at most once per page, and a
  // per-run budget in sessionStorage stops retry loops across reloads (a wrong
  // password must never be resubmitted until the account locks).
  const STORE_KEY = `isct-companion-login:${config.run}`;
  const LIMITS = {
    agree: 2,
    identifier: 3,
    password: 1,
    method: 2,
    otp: 2,
    matrix: 1,
  };
  const FAILURE = {
    agree: ["needs-user", "The portal start page changed."],
    identifier: ["error", "The school did not accept this ID."],
    password: ["invalid-credentials", "The ID or password was not accepted."],
    method: ["needs-user", "Choose a verification method."],
    otp: ["invalid-otp", "The one-time password was not accepted."],
    matrix: ["invalid-matrix", "The Matrix answer was not accepted."],
  };
  const IDLE_LIMIT_MS = 15_000;

  let stopped = false;
  let acted = false;
  let leaving = false;
  let sequence = 0;
  const done = new Set();
  const announced = new Set();
  let captureTimer;
  let lastCapture = "";
  addEventListener("beforeunload", () => (leaving = true));

  const storage = (() => {
    try {
      return window.sessionStorage;
    } catch {
      return null;
    }
  })();
  const attempts = () => {
    try {
      return JSON.parse(storage?.getItem(STORE_KEY) || "{}");
    } catch {
      return {};
    }
  };
  const bump = (step) => {
    const next = attempts();
    next[step] = (next[step] || 0) + 1;
    try {
      storage?.setItem(STORE_KEY, JSON.stringify(next));
    } catch {
      // Without storage the per-page guard still prevents loops on this page.
    }
    return next[step];
  };

  const shown = (element) =>
    element instanceof HTMLElement && element.getClientRects().length > 0;
  const visible = (element) => shown(element) && !element.disabled;
  const text = (element) =>
    (element?.innerText || element?.textContent || "")
      .replace(/\s+/g, " ")
      .trim();

  const setValue = (input, value) => {
    if (!(input instanceof HTMLInputElement) || !value) return false;
    input.focus();
    const setter = Object.getOwnPropertyDescriptor(
      HTMLInputElement.prototype,
      "value",
    )?.set;
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    return true;
  };

  const encode = (value) => {
    const bytes = new TextEncoder().encode(JSON.stringify(value));
    let binary = "";
    for (let index = 0; index < bytes.length; index += 0x8000)
      binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    return btoa(binary)
      .replace(/\+/g, "-")
      .replace(/\//g, "_")
      .replace(/=+$/, "");
  };

  // Reports go through the title, not a navigation, so they can never cancel
  // the page's own form POST or redirect.
  const relay = (state, message) => {
    document.title =
      config.authRelay +
      encode({
        profile: config.profile,
        run: config.run,
        seq: ++sequence,
        state,
        message,
      });
  };

  const progress = (step) => {
    if (announced.has(step)) return;
    announced.add(step);
    relay("progress", step);
  };

  const report = (state, message) => {
    if (stopped) return true;
    stopped = true;
    relay(state, message);
    return true;
  };

  // Runs a step once per page and within its per-run budget.
  const act = (step, perform) => {
    if (done.has(step)) return true;
    done.add(step);
    acted = true;
    if (bump(step) > LIMITS[step]) return report(...FAILURE[step]);
    progress(step);
    perform();
    return true;
  };

  const otpNow = () => {
    const windows = Array.isArray(config.otp) ? config.otp : [];
    const period = windows.length > 1 ? windows[1].from - windows[0].from : 30;
    const now = Date.now() / 1000;
    return (
      windows.find((window) => window.from <= now && now < window.from + period)
        ?.code || ""
    );
  };

  const captureHtml = () => {
    if (!config.debugRelay || leaving) return;
    const root = document.documentElement.cloneNode(true);
    root
      .querySelectorAll("script,style,noscript")
      .forEach((node) => node.remove());
    root.querySelectorAll("input,textarea,select").forEach((element) => {
      element.removeAttribute("value");
      element.removeAttribute("checked");
      if (element instanceof HTMLTextAreaElement) element.textContent = "";
      if (element instanceof HTMLSelectElement)
        element
          .querySelectorAll("option")
          .forEach((option) => option.removeAttribute("selected"));
    });
    root.querySelectorAll("*").forEach((element) => {
      [...element.attributes].forEach((attribute) => {
        if (/token|secret|nonce|session|credential/i.test(attribute.name))
          element.removeAttribute(attribute.name);
        else if (/^(href|src|action|formaction)$/i.test(attribute.name)) {
          try {
            const url = new URL(attribute.value, location.href);
            element.setAttribute(attribute.name, url.origin + url.pathname);
          } catch {
            element.setAttribute(
              attribute.name,
              attribute.value.split(/[?#]/, 1)[0],
            );
          }
        }
      });
    });
    let html = "<!doctype html>\n" + root.outerHTML;
    if (config.username)
      html = html.split(config.username).join("[REDACTED_USERNAME]");
    html = html.slice(0, 2 * 1024 * 1024);
    const signature = `${location.origin}${location.pathname}:${html.length}:${html.slice(-120)}`;
    if (signature === lastCapture) return;
    lastCapture = signature;
    const link = document.createElement("a");
    link.href =
      config.debugRelay +
      encode({ url: location.origin + location.pathname, html });
    link.click();
  };

  const queueCapture = () => {
    if (!config.debugRelay) return;
    clearTimeout(captureTimer);
    captureTimer = setTimeout(captureHtml, 80);
  };

  // Extic (isct.ex-tic.com): /auth/session is identifier-first with one Next
  // button for both steps; /auth/session/second_factor optionally shows an
  // App/Email picker before the TOTP form, which submits over AJAX.
  const science = () => {
    const login = document.querySelector("form#login");
    if (login) {
      const warning = login.querySelector(".message.warning");
      if (shown(warning))
        return report(
          attempts().password ? "invalid-credentials" : "error",
          text(warning),
        );
      if (!shown(login)) {
        // The passkey tab is active; switch to the password form.
        const tab = document.querySelector("#password-form-selector");
        if (tab && !done.has("tab")) {
          done.add("tab");
          acted = true;
          tab.click();
        }
        return;
      }
      const submit = login.querySelector('button[type="submit"]');
      if (!visible(submit)) return; // Waits for the page to re-enable it.
      const passwordField = login.querySelector("#password-field-wrapper");
      if (passwordField && !passwordField.classList.contains("move-off-screen"))
        return act("password", () => {
          setValue(login.querySelector("#password"), config.password);
          submit.click();
        });
      if (shown(login.querySelector("#identifier-field-wrapper")))
        return act("identifier", () => {
          setValue(login.querySelector("#identifier"), config.username);
          submit.click();
        });
      return;
    }

    const totpForm = document.querySelector("form#totp-form");
    if (!totpForm) return;
    const warning = totpForm.querySelector(".message.warning");
    if (shown(warning)) return report("invalid-otp", text(warning));
    const code = otpNow();
    if (shown(totpForm)) {
      if (!code) return report("needs-user", "Enter the one-time password.");
      const submit = totpForm.querySelector('button[type="submit"]');
      if (!visible(submit)) return;
      return act("otp", () => {
        setValue(totpForm.querySelector("#totp"), code);
        submit.click();
      });
    }
    const appOtp = document.querySelector("#totp-form-selector");
    if (shown(appOtp)) {
      if (!code) return report("needs-user", "Choose a verification method.");
      return act("method", () => appOtp.click());
    }
    if (
      shown(document.querySelector("#emailotp-form-selector")) ||
      shown(document.querySelector("#emailotp-form-wrapper"))
    )
      return report("needs-user", "Enter the code sent by email.");
  };

  const COORDINATE = /\[\s*([A-J])\s*,\s*([1-7])\s*\]/gi;
  const matrixCells = () => {
    const inputs = [...document.querySelectorAll("input")].filter(
      (input) =>
        visible(input) &&
        /^(password|text)$/i.test(input.type) &&
        !/^(usr_name|usr_password|q)$/i.test(input.name),
    );
    if (!inputs.length || inputs.length > 5) return [];
    const global = [
      ...(document.body?.innerText || document.body?.textContent || "")
        .toUpperCase()
        .matchAll(COORDINATE),
    ];
    if (global.length < inputs.length) return [];
    return inputs.map((input, index) => {
      const row = input.closest("tr");
      const local = row
        ? [...(row.textContent || "").toUpperCase().matchAll(COORDINATE)]
        : [];
      const match =
        local.length === 1
          ? local[0]
          : global[global.length - inputs.length + index];
      return {
        input,
        column: match[1].toUpperCase().charCodeAt(0) - 65,
        row: Number(match[2]) - 1,
      };
    });
  };

  const submitOf = (element) =>
    element
      ?.closest("form")
      ?.querySelector('input[type="submit"],button[type="submit"]');

  // GetAccess (portal.nap.gsic.titech.ac.jp): agreement page, account and
  // password, an optional OTP page, then the Matrix challenge ([E,6] cells).
  const tokyoTech = () => {
    const params = new URLSearchParams(location.search);
    if (params.get("Message") === "fail" && attempts().password)
      return report(...(attempts().matrix ? FAILURE.matrix : FAILURE.password));

    const form = document.querySelector('form[name="login"]');
    const user = form?.querySelector('input[name="usr_name"]');
    const password = form?.querySelector('input[name="usr_password"]');
    if (user && password) {
      const submit = submitOf(password);
      if (!visible(submit)) return;
      return act("password", () => {
        // The portal rejects any lowercase letter in the account name.
        setValue(user, config.username.toUpperCase());
        setValue(password, config.password);
        submit.click();
      });
    }

    const cells = matrixCells();
    if (cells.length) {
      if (config.matrix.length !== 70)
        return report("needs-user", "Enter the Matrix code.");
      const submit = submitOf(cells[0].input);
      if (!visible(submit)) return;
      return act("matrix", () => {
        cells.forEach(({ input, row, column }) =>
          setValue(input, config.matrix[row * 10 + column]),
        );
        submit.click();
      });
    }

    // Accounts with OTP and Matrix land on the OTP page first; its "Move to
    // Other Authentication" select offers GridAuthOption (Matrix). Accounts
    // with one method go straight to that method's page.
    const toMatrix = document.querySelector(
      'form[name="login"] select option[value="GridAuthOption"]',
    );
    if (toMatrix) {
      const select = toMatrix.closest("select");
      const submit = submitOf(select);
      if (!visible(submit)) return;
      return act("method", () => {
        select.value = toMatrix.value;
        select.dispatchEvent(new Event("change", { bubbles: true }));
        submit.click();
      });
    }
    if (document.querySelector('form[name="login"] table#authentication'))
      return report("needs-user", "Enter the one-time password.");

    // Start page: two "agree" buttons; take the password/Matrix one.
    const agree = [
      ...document.querySelectorAll('input[type="button"],button'),
    ].find((element) =>
      /Template=userpass_key/.test(element.getAttribute("onclick") || ""),
    );
    if (agree) return act("agree", () => agree.click());
  };

  const run = () => {
    if (stopped || leaving) return;
    if (document.querySelector('iframe[src*="recaptcha" i],.g-recaptcha'))
      return report("needs-user", "Additional verification is required.");
    if (config.profile === "science") science();
    else tokyoTech();
  };

  const start = () => {
    // Observe before the first run: its actions change the DOM synchronously.
    const observer = new MutationObserver(() => {
      queueCapture();
      run();
    });
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "style", "disabled"],
    });
    queueCapture();
    run();
    const startedAt = Date.now();
    // Fallback only; hidden pages may run this rarely.
    const fallback = setInterval(() => {
      run();
      // A sign-in page with nothing recognizable to do needs the person.
      if (!acted && !stopped && Date.now() - startedAt > IDLE_LIMIT_MS)
        report("needs-user", "This sign-in step needs you.");
      if (stopped || leaving || Date.now() - startedAt > 120_000) {
        observer.disconnect();
        clearInterval(fallback);
      }
    }, 1000);
  };

  if (document.readyState === "loading")
    addEventListener("DOMContentLoaded", start, { once: true });
  else start();
})(__ISCT_COMPANION_LOGIN_CONFIG__);

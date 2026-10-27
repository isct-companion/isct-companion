// @vitest-environment jsdom
// @ts-expect-error jsdom ships without type declarations here.
import { JSDOM, VirtualConsole } from "jsdom";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";

import type {
  SchoolSessionEvent,
  SchoolSessionSnapshot,
} from "@app/platform/desktop-bridge";
import { bootstrapReminders } from "@app/platform/reminders";
import { schoolSessions } from "@app/platform/school-session";
import { schoolWebBounds } from "@app/platform/school-web-bounds";
import { networkHint } from "@app/platform/sync";
import { readRepoFile, schoolProfiles } from "@tests/harness";

/**
 * Every school-web contract reachable from the webview: child-webview geometry,
 * the scripts Core runs in school pages, the session projection the shell
 * renders, reference-polling network policy and the reminder-log migration.
 *
 * The scripts are shipped by Rust; this suite runs the shipped files rather
 * than a transcription of them. Which pages a view may load, open or save
 * from is Core's rule table, tested where it runs
 * (`crates/app-core/src/tests/school_browser*.rs`).
 */

const readScript = (name: string) =>
  readRepoFile(`crates/adpt-isct/assets/${name}`);

/* ------------------------------------------------------------------ *
 * School webview bounds
 * ------------------------------------------------------------------ */

const viewport = (
  dimensions: Partial<{
    innerWidth: number;
    innerHeight: number;
    outerWidth: number;
    outerHeight: number;
  }> = {},
) =>
  ({
    innerWidth: 1180,
    innerHeight: 740,
    outerWidth: 1196,
    outerHeight: 779,
    ...dimensions,
  }) as Window;

describe("school webview bounds", () => {
  it("uses the DOM client-area rectangle unchanged on Windows", () => {
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport(),
        "windows",
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
  });

  it("keeps the WKWebView title-bar compensation on macOS", () => {
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport({ outerWidth: 1180, outerHeight: 740 }),
        "macos",
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
  });

  it("subtracts half the horizontal chrome inset from the macOS origin", () => {
    // 1196x779 outer against a 1180x740 client area: 16px of side chrome and a
    // 39px measured title-bar inset, so x shifts by 8 and y by 39 - 8 = 31.
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport(),
        "macos",
      ),
    ).toEqual({ x: 188, y: 63, width: 988, height: 675 });
  });

  it("reads the injected native OS when the caller does not name one", () => {
    const scope = window as unknown as Record<string, unknown>;
    scope.__ISCT_COMPANION_NATIVE_OS__ = "windows";
    expect(
      schoolWebBounds(
        { left: 180.4, top: 32.4, width: 987.6, height: 674.6 },
        viewport(),
      ),
    ).toEqual({ x: 180, y: 32, width: 988, height: 675 });
    scope.__ISCT_COMPANION_NATIVE_OS__ = "macos";
    expect(
      schoolWebBounds(
        { left: 180, top: 32, width: 988, height: 675 },
        viewport({ outerWidth: 1180, outerHeight: 740 }),
      ),
    ).toEqual({ x: 180, y: 60, width: 988, height: 675 });
    delete scope.__ISCT_COMPANION_NATIVE_OS__;
  });
});

/* ------------------------------------------------------------------ *
 * Injected sign-in script (school-login.js)
 * ------------------------------------------------------------------ */

const LOGIN_SOURCE = readScript("school-login.js");

type Relay = {
  profile: string;
  run: string;
  seq: number;
  state: string;
  message: string;
  submitted: string[];
};

type LoginPage = {
  window: Window & typeof globalThis;
  document: Document;
  $: (selector: string) => HTMLElement;
  relays: Relay[];
  submitted: Record<string, string>[];
  start: () => void;
};

const loginPages: { window: { close(): void } }[] = [];
afterEach(() => loginPages.splice(0).forEach((page) => page.window.close()));

const nowSeconds = () => Math.floor(Date.now() / 1000);
const liveOtp = (now = nowSeconds()) => [
  { from: now - 5, code: "123456" },
  { from: now + 25, code: "654321" },
];

function loadLoginPage(
  html: string,
  options: {
    url?: string;
    profile?: "science" | "tokyo-tech";
    otp?: { from: number; code: string }[];
    matrix?: string;
    storage?: Record<string, string>;
    /** Pins the page's clock (seconds), so a run that crosses a second or
     * an OTP window boundary still sees the windows it was given. */
    now?: number;
  } = {},
): LoginPage {
  const dom = new JSDOM(html, {
    url: options.url ?? "https://isct.ex-tic.com/auth/session",
    runScripts: "outside-only",
    virtualConsole: new VirtualConsole(),
  });
  loginPages.push(dom as unknown as { window: { close(): void } });
  const window = dom.window as Window & typeof globalThis;
  const pinned = options.now;
  if (pinned !== undefined) window.Date.now = () => pinned * 1000;
  const { document } = window;
  for (const [key, value] of Object.entries(options.storage ?? {}))
    window.sessionStorage.setItem(key, value);
  // jsdom has no layout: treat display:none, [hidden] and hidden inputs as
  // invisible, everything else (including off-screen) as rendered.
  window.HTMLElement.prototype.getClientRects = function (this: HTMLElement) {
    for (let node: HTMLElement | null = this; node; node = node.parentElement) {
      if (
        node.hidden ||
        window.getComputedStyle(node).display === "none" ||
        (node instanceof window.HTMLInputElement && node.type === "hidden")
      )
        return [] as unknown as DOMRectList;
    }
    return [{}] as unknown as DOMRectList;
  };
  // Reports arrive through document.title, as the native side reads them.
  const relays: Relay[] = [];
  Object.defineProperty(document, "title", {
    configurable: true,
    get: () => "",
    set: (value: string) => {
      const encoded = value.replace(/^relay:/, "");
      const binary = atob(encoded.replace(/-/g, "+").replace(/_/g, "/"));
      const bytes = Uint8Array.from(binary, (char) => char.charCodeAt(0));
      relays.push(JSON.parse(new TextDecoder().decode(bytes)) as Relay);
    },
  });
  const submitted: Record<string, string>[] = [];
  document.addEventListener("submit", (event: Event) => {
    event.preventDefault();
    const form = event.target as HTMLFormElement;
    submitted.push(
      Object.fromEntries(
        [...form.querySelectorAll("input")]
          .filter((input) => input.name)
          .map((input) => [input.name, input.value]),
      ),
    );
  });
  const config = {
    profile: options.profile ?? "science",
    run: "run-1",
    username: "ab12cd34",
    password: "pw",
    otp: options.otp ?? liveOtp(),
    matrix: options.matrix ?? "",
    authRelay: "relay:",
    debugRelay: null,
  };
  const start = () => {
    window.eval(
      LOGIN_SOURCE.replace(
        "__ISCT_COMPANION_LOGIN_CONFIG__",
        JSON.stringify(config),
      ),
    );
  };
  const $ = (selector: string) =>
    document.querySelector(selector) as HTMLElement;
  return { window, document, $, relays, submitted, start };
}

const settle = (ms = 60) => new Promise((resolve) => setTimeout(resolve, ms));
const progress = (relays: Relay[]) =>
  relays
    .filter((relay) => relay.state === "progress")
    .map((relay) => relay.message);

const identifierPage = `
  <a class="form-selector nav-link" id="fido2-form-selector">Passwordless</a>
  <a class="form-selector nav-link active" id="password-form-selector">Password</a>
  <form id="login" action="/auth/session" method="post">
    <input type="hidden" name="authenticity_token" value="x">
    <div class="message warning" style="display: none">Login failed.</div>
    <div id="identifier-field-wrapper"><div class="input-group mb-3">
      <label class="input-group-text" for="identifier"></label>
      <input type="text" name="identifier" id="identifier" placeholder="User Name" class="form-control" autocomplete="username">
    </div></div>
    <div class="move-off-screen" id="password-message-wrapper"></div>
    <div class="login-username" id="identifier-wrapper" style="display: none"></div>
    <div class="login-username-suffix" id="suffix-wrapper" style="display: none"></div>
    <div class=" move-off-screen" id="password-field-wrapper"><div class="input-group mb-3">
      <label class="input-group-text" for="password"></label>
      <input type="password" name="password" id="password" placeholder="Password" class="form-control" autocomplete="current-password">
    </div></div>
    <button class="btn btn-info" style="margin-bottom: 0.5em" type="submit">Next</button>
  </form>
  <div id="fido2-form-wrapper" style="display: none;">
    <div class="message warning" id="fido2-unavailable">FIDO2 is unavailable</div>
  </div>`;

const secondFactorPage = (picker: boolean) => `
  <div class="lower">
    <form id="authenticator-selector" ${picker ? "" : 'style="display: none;"'}>
      <p>Please select an authentication method.</p>
      <button class="btn btn-primary select mb-3" data-target="totp-form-wrapper" id="totp-form-selector">OTP (App) Authentication</button>
      <button class="btn btn-primary select mb-3" data-target="emailotp-form-wrapper" id="emailotp-form-selector">OTP (Email) Authentication</button>
    </form>
    <div id="totp-form-wrapper" style="${picker ? "display: none" : ""}">
      <form id="totp-form" action="/auth/session/second_factor" data-remote="true" method="post">
        <input type="hidden" name="authenticity_token" value="x">
        <div class="message warning" style="display: none">Login failed.</div>
        <div class="input-group mb-3"><label class="input-group-text" for="totp"></label>
          <input type="text" name="totp" id="totp" placeholder="One-Time Password" class="form-control"></div>
        <button class="btn btn-info mb-3" type="submit">Next</button>
      </form>
    </div>
    <div id="emailotp-form-wrapper" style="display: none">
      <form id="send-otp-form" action="/auth/session/emailotp" method="post">
        <div class="message warning" style="display: none">Login failed.</div>
        <button class="btn btn-success sm mb-3" type="submit">Send One-Time Password</button>
      </form>
      <form id="emailotp-form" action="/auth/session/second_factor" data-remote="true" method="post">
        <input type="text" name="emailotp" id="emailotp" placeholder="One-Time Password" class="form-control">
        <button class="btn btn-info mb-3" type="submit">Next</button>
      </form>
    </div>
    <form action="/auth/session" method="post"><input type="hidden" name="_method" value="delete">
      <button class="btn btn-default sm cancel" type="submit">Cancel</button></form>
  </div>`;
const SECOND_FACTOR = "https://isct.ex-tic.com/auth/session/second_factor";

describe("Science Tokyo sign-in", () => {
  it("drives both identifier-first steps through the shared Next button", async () => {
    const page = loadLoginPage(identifierPage);
    // The page's first step: AJAX lookup, then reveal the password field.
    page.$("form#login").addEventListener(
      "submit",
      () => {
        page.$("#identifier-field-wrapper").style.display = "none";
        page.$("#identifier-wrapper").style.display = "";
        page.$("#password-field-wrapper").classList.remove("move-off-screen");
      },
      { once: true },
    );
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["identifier", "password"]);
    expect(page.submitted).toHaveLength(2);
    expect(page.submitted[0].identifier).toBe("ab12cd34");
    expect(page.submitted[1].password).toBe("pw");
  });

  it("waits while the page keeps Next disabled", async () => {
    const page = loadLoginPage(identifierPage);
    page.$("form#login button").setAttribute("disabled", "");
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    page.$("form#login button").removeAttribute("disabled");
    await settle();
    expect(page.submitted).toHaveLength(1);
  });

  it("stops acting once Core abandons the run", async () => {
    const page = loadLoginPage(identifierPage);
    page.$("form#login button").setAttribute("disabled", "");
    page.start();
    await settle();
    page.window.eval(readScript("school-stop.js"));
    page.$("form#login button").removeAttribute("disabled");
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays).toHaveLength(0);
  });

  it("never resubmits a rejected password", async () => {
    const page = loadLoginPage(
      identifierPage.replace('class=" move-off-screen" ', ""),
      {
        storage: {
          "isct-companion-login:run-1": JSON.stringify({ password: 1 }),
        },
      },
    );
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("invalid-credentials");
  });

  it("reports the login warning as a rejected password", async () => {
    const page = loadLoginPage(
      identifierPage.replace(
        '<div class="message warning" style="display: none">',
        '<div class="message warning">',
      ),
      {
        storage: {
          "isct-companion-login:run-1": JSON.stringify({ password: 1 }),
        },
      },
    );
    page.start();
    await settle();
    expect(page.relays.at(-1)).toMatchObject({
      state: "invalid-credentials",
      message: "Login failed.",
      submitted: ["password"],
    });

    // The same warning before this run submitted anything is the page's own
    // notice, not a verdict on the saved password: Core retries it later.
    const notice = loadLoginPage(
      identifierPage.replace(
        '<div class="message warning" style="display: none">',
        '<div class="message warning">',
      ),
    );
    notice.start();
    await settle();
    expect(notice.submitted).toHaveLength(0);
    expect(notice.relays.at(-1)).toMatchObject({
      state: "error",
      submitted: [],
    });
  });

  it("picks App OTP from the method picker, then fills the TOTP", async () => {
    const page = loadLoginPage(secondFactorPage(true), { url: SECOND_FACTOR });
    // The page's picker handler: hide the picker, show the chosen form.
    page.$("#totp-form-selector").addEventListener("click", (event: Event) => {
      event.preventDefault();
      page.$("#authenticator-selector").style.display = "none";
      page.$("#totp-form-wrapper").style.display = "";
    });
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["method", "otp"]);
    expect(page.submitted).toEqual([
      expect.objectContaining({ totp: "123456" }),
    ]);
  });

  it("fills the TOTP directly when there is no picker", async () => {
    const page = loadLoginPage(secondFactorPage(false), {
      url: SECOND_FACTOR,
    });
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["otp"]);
    expect(page.submitted[0].totp).toBe("123456");
  });

  it("uses the code for the window that is live when the field appears", async () => {
    const page = loadLoginPage(secondFactorPage(false), {
      url: SECOND_FACTOR,
      otp: [
        { from: nowSeconds() - 65, code: "111111" },
        { from: nowSeconds() - 35, code: "222222" },
        { from: nowSeconds() - 5, code: "333333" },
      ],
    });
    page.start();
    await settle();
    expect(page.submitted[0].totp).toBe("333333");
  });

  it("holds a code that is about to expire and sends the next window's", async () => {
    // Extic answers a code that expires on its side (its clock runs a little
    // ahead) by showing the method picker again, with no message; on Windows
    // a code sent with 3.3 s left came back that way twice.
    const now = Date.now() / 1000;
    const page = loadLoginPage(secondFactorPage(false), {
      url: SECOND_FACTOR,
      otp: [
        { from: now - 28.8, code: "111111" },
        { from: now + 1.2, code: "222222" },
      ],
    });
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.filter((relay) => relay.state !== "progress")).toEqual(
      [],
    );
    // The page's 1 s fallback sends the fresh code once its window is live.
    await settle(2600);
    expect(page.submitted.map((form) => form.totp)).toEqual(["222222"]);
  }, 10_000);

  it("does not resend a turned-down code and reports a second refusal", async () => {
    const pickHandler = (page: LoginPage) =>
      page.$("#totp-form-selector").addEventListener("click", (event) => {
        event.preventDefault();
        page.$("#authenticator-selector").style.display = "none";
        page.$("#totp-form-wrapper").style.display = "";
      });
    // One moment for all three loads: the retries must see the same window.
    const now = nowSeconds();
    const otp = liveOtp(now);
    const first = loadLoginPage(secondFactorPage(true), {
      url: SECOND_FACTOR,
      otp,
      now,
    });
    pickHandler(first);
    first.start();
    await settle();
    expect(first.submitted.map((form) => form.totp)).toEqual(["123456"]);
    const key = "isct-companion-login:run-1";
    const storage = { [key]: first.window.sessionStorage.getItem(key)! };

    // The picker again with the same window live: the code was turned down.
    const second = loadLoginPage(secondFactorPage(true), {
      url: SECOND_FACTOR,
      storage,
      otp,
      now,
    });
    pickHandler(second);
    second.start();
    await settle();
    expect(second.submitted).toHaveLength(0);
    expect(second.relays.filter((relay) => relay.state !== "progress")).toEqual(
      [],
    );

    // Two codes turned down: say so, rather than wait for the person.
    const third = loadLoginPage(secondFactorPage(true), {
      url: SECOND_FACTOR,
      storage: { [key]: JSON.stringify({ method: 2, otp: 2 }) },
      otp,
      now,
    });
    third.start();
    await settle();
    expect(third.submitted).toHaveLength(0);
    expect(third.relays.at(-1)).toMatchObject({ state: "invalid-otp" });
  });

  it("reports a rejected TOTP without retrying", async () => {
    const page = loadLoginPage(secondFactorPage(false), {
      url: SECOND_FACTOR,
    });
    page.start();
    await settle();
    page.$("#totp-form .message.warning").style.display = "";
    await settle();
    expect(page.submitted).toHaveLength(1);
    expect(page.relays.at(-1)).toMatchObject({
      state: "invalid-otp",
      submitted: ["otp"],
    });

    // A warning already on the form before any code was sent says nothing
    // about the saved secret.
    const stale = loadLoginPage(
      secondFactorPage(false).replace(
        '<div class="message warning" style="display: none">',
        '<div class="message warning">',
      ),
      { url: SECOND_FACTOR },
    );
    stale.start();
    await settle();
    expect(stale.submitted).toHaveLength(0);
    expect(stale.relays.at(-1)?.state).toBe("error");
  });

  it("hands an email-only second factor to the person", async () => {
    const page = loadLoginPage(
      secondFactorPage(false)
        .replace(/<button[^>]*id="totp-form-selector"[^>]*>.*?<\/button>/, "")
        .replace(
          'id="totp-form-wrapper" style=""',
          'id="totp-form-wrapper" style="display: none"',
        )
        .replace(
          'id="emailotp-form-wrapper" style="display: none"',
          'id="emailotp-form-wrapper"',
        ),
      { url: SECOND_FACTOR },
    );
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("needs-user");
  });

  it("leaves SAML hand-off pages alone", async () => {
    const page = loadLoginPage(
      `<form method="post" action="https://isct.ex-tic.com/idm/user/saml2/acs">
        <input type="hidden" name="SAMLResponse" value="x"><button type="submit">Continue</button></form>`,
    );
    page.start();
    await settle();
    expect(page.relays).toEqual([]);
    expect(page.submitted).toEqual([]);
  });
});

describe("Tokyo Tech sign-in", () => {
  const tokyo = {
    profile: "tokyo-tech" as const,
    otp: [] as { from: number; code: string }[],
    matrix: "ABCDEFGHIJ".repeat(7),
  };
  const GETACCESS = "https://portal.nap.gsic.titech.ac.jp/GetAccess/Login";
  const getAccessPage = (title: string, rows: string) => `
    <form name="login" method="POST" autocomplete="off" action="${GETACCESS}">
      <table border="3"><tbody><tr><td>
        <table border="0" id="authentication"><tbody>
          <tr><td colspan="3" align="center">${title}</td></tr>
          <tr><th></th><th></th><td><input type="hidden" name="message0" value="1"></td></tr>
          ${rows}
          <tr><td align="center" colspan="3">
            <input type="submit" name="OK" value="    OK    ">
            <input type="hidden" name="AUTHMETHOD" value="IG">
            <input type="hidden" name="Template" value="idg_key">
          </td></tr>
        </tbody></table>
      </td></tr></tbody></table>
    </form>`;
  const moveTo = (name: string, option: string, label: string) => `
    <tr><th align="left">Move to <br>Other Authentication</th><th></th><td>
      <select name="${name}" size="1"><option value="NoOtherIGAuthOption"></option>
        <option value="${option}">${label}</option></select></td></tr>`;
  const otpPage = (withMatrix: boolean) =>
    getAccessPage(
      "One-Time Password Authentication",
      `<tr><th align="left">Select Label for OTP</th><th></th><td>
        <select name="message2" size="1"><option value=" "> </option>
          <option value="science tokyo mail">science tokyo mail</option></select></td></tr>
      ${withMatrix ? moveTo("message3", "GridAuthOption", "Matrix") : ""}`,
    );
  const matrixPage = getAccessPage(
    "Matrix Authentication",
    `<tr><th></th><th></th><td>152526</td></tr>
    <tr><th align="left">[E,6]</th><th></th><td><input type="password" name="message3" value="" size="32" maxlength="200"></td></tr>
    <tr><th align="left">[G,5]</th><th></th><td><input type="password" name="message4" value="" size="32" maxlength="200"></td></tr>
    <tr><th align="left">[I,1]</th><th></th><td><input type="password" name="message5" value="" size="32" maxlength="200"></td></tr>
    ${moveTo("message6", "OTPAuthOption", "OTP (One-Time Password)")}`,
  );
  const afterPassword = {
    url: GETACCESS,
    ...tokyo,
    storage: {
      "isct-companion-login:run-1": JSON.stringify({ password: 1 }),
    },
  };

  it("takes the password agreement button and ignores the site search box", async () => {
    const page = loadLoginPage(
      `<form><input type="button" value="同意（証明書認証）"
          onclick="location.href='https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=cert_key'"></form>
        <form><input id="agree" type="button" value="同意（マトリクス/OTP/ソフトトークン認証）"
          onclick="location.href='https://portal.nap.gsic.titech.ac.jp/GetAccess/Login?Template=userpass_key&AUTHMETHOD=UserPassword'"></form>
        <form id="cse-search-box" action="https://google.com/cse">
          <input type="text" name="q"><input type="submit" value="検索"></form>`,
      {
        url: "https://portal.nap.gsic.titech.ac.jp/portal.pl?GASF=x",
        ...tokyo,
      },
    );
    let agreed = false;
    page.$("#agree").addEventListener("click", () => (agreed = true));
    page.start();
    await settle();
    expect(agreed).toBe(true);
    expect(page.submitted).toHaveLength(0);
    expect((page.$('[name="q"]') as HTMLInputElement).value).toBe("");
  });

  it("submits an upper-case account name over the prefilled one", async () => {
    const page = loadLoginPage(
      `<form name="login" method="POST" autocomplete="off" action="${GETACCESS}">
        <input type="text" name="usr_name" value="PREFILLED" size="32" maxlength="255">
        <input type="password" name="usr_password" value="" size="32" maxlength="32">
        <input type="submit" name="OK" value="    OK    ">
        <input type="hidden" name="Template" value="userpass_key"></form>`,
      { url: `${GETACCESS}?Template=userpass_key`, ...tokyo },
    );
    page.start();
    await settle();
    expect(page.submitted[0].usr_name).toBe("AB12CD34");
    expect(page.submitted[0].usr_password).toBe("pw");
  });

  it("moves from the OTP page to Matrix when both are set up", async () => {
    const page = loadLoginPage(otpPage(true), afterPassword);
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["method"]);
    expect(page.submitted).toHaveLength(1);
    expect((page.$('select[name="message3"]') as HTMLSelectElement).value).toBe(
      "GridAuthOption",
    );
  });

  it("hands an OTP-only account to the person", async () => {
    const page = loadLoginPage(otpPage(false), afterPassword);
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("needs-user");
  });

  it("answers the Matrix challenge by coordinate and stays on Matrix", async () => {
    const page = loadLoginPage(matrixPage, afterPassword);
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["matrix"]);
    // Every matrix row is "ABCDEFGHIJ", so each answer is its column letter.
    expect(page.submitted[0]).toMatchObject({
      message3: "E",
      message4: "G",
      message5: "I",
    });
    expect((page.$('select[name="message6"]') as HTMLSelectElement).value).toBe(
      "NoOtherIGAuthOption",
    );
  });

  it("reports a Matrix challenge that comes back", async () => {
    const page = loadLoginPage(matrixPage, {
      ...afterPassword,
      storage: {
        "isct-companion-login:run-1": JSON.stringify({
          password: 1,
          matrix: 1,
        }),
      },
    });
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("invalid-matrix");
  });

  it("hands the post-sign-in password change form to the person", async () => {
    const page = loadLoginPage(
      `<form name="pwchange" method="POST" action="${GETACCESS}">
        <p>パスワード変更</p>
        <input type="password" name="old_password">
        <input type="password" name="new_password">
        <input type="password" name="confirm_password">
        <input type="submit" value="変更"></form>`,
      afterPassword,
    );
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)).toMatchObject({
      state: "needs-user",
      message: expect.stringContaining("change your password"),
    });
    expect((page.$('[name="new_password"]') as HTMLInputElement).value).toBe(
      "",
    );
  });

  it("reports a rejected password from the fail redirect", async () => {
    const page = loadLoginPage(`<p>portal</p>`, {
      url: "https://portal.nap.gsic.titech.ac.jp/?Message=fail&GAURI=x",
      ...tokyo,
      storage: {
        "isct-companion-login:run-1": JSON.stringify({ password: 1 }),
      },
    });
    page.start();
    await settle();
    expect(page.relays.at(-1)?.state).toBe("invalid-credentials");
  });
});

/* ------------------------------------------------------------------ *
 * School session projection
 * ------------------------------------------------------------------ */

const INITIAL_SESSION: SchoolSessionSnapshot = {
  revision: 1,
  profiles: schoolProfiles("pending", "not-configured"),
};

function sessionBridge(snapshot: SchoolSessionSnapshot = INITIAL_SESSION) {
  let emit: (event: SchoolSessionEvent) => void = () => undefined;
  const order: string[] = [];
  const dispose = vi.fn();
  return {
    order,
    dispose,
    emit: (event: SchoolSessionEvent) => emit(event),
    onSchoolSession: vi.fn(
      async (callback: (event: SchoolSessionEvent) => void) => {
        order.push("listen");
        emit = callback;
        return dispose;
      },
    ),
    schoolSessionSnapshot: vi.fn(async () => {
      order.push("snapshot");
      return snapshot;
    }),
    schoolSessionLifecycle: vi.fn(async () => undefined),
  };
}

describe("school session projection", () => {
  beforeEach(() => {
    schoolSessions.reset();
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "visible",
    });
  });

  afterEach(() => schoolSessions.reset());

  it("subscribes before reading the initial snapshot and maps pending to no badge", async () => {
    const native = sessionBridge();
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.order).toEqual(["listen", "snapshot"]),
    );
    expect(schoolSessions.get("science")).toBeUndefined();
    expect(schoolSessions.get("tokyo-tech")).toBe("not-configured");
    expect(native.schoolSessionLifecycle).toHaveBeenCalledWith({
      kind: "launch",
      visible: true,
    });
    detach();
  });

  it("does not let an older snapshot overwrite an event received during the read", async () => {
    let resolveSnapshot!: (snapshot: SchoolSessionSnapshot) => void;
    const native = sessionBridge();
    native.schoolSessionSnapshot.mockImplementation(
      () =>
        new Promise<SchoolSessionSnapshot>((resolve) => {
          resolveSnapshot = resolve;
        }),
    );
    schoolSessions.attach(native);
    await vi.waitFor(() => expect(native.onSchoolSession).toHaveBeenCalled());
    native.emit({ profile: "science", state: "active", revision: 3 });
    resolveSnapshot({
      revision: 2,
      profiles: schoolProfiles("failed", "active"),
    });
    await vi.waitFor(() =>
      expect(schoolSessions.get("tokyo-tech")).toBe("active"),
    );
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("ignores stale and duplicate revisions for each profile", async () => {
    const native = sessionBridge({
      revision: 4,
      profiles: schoolProfiles("active", "failed"),
    });
    schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("active"),
    );
    native.emit({ profile: "science", state: "failed", revision: 3 });
    native.emit({ profile: "science", state: "failed", revision: 4 });
    expect(schoolSessions.get("science")).toBe("active");
    native.emit({ profile: "science", state: "failed", revision: 5 });
    expect(schoolSessions.get("science")).toBe("failed");
  });

  it("reference-counts consumers and cleans up before reconnecting", async () => {
    const first = sessionBridge();
    const detachFirst = schoolSessions.attach(first);
    const detachSecond = schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(first.onSchoolSession).toHaveBeenCalledOnce(),
    );
    detachFirst();
    expect(first.dispose).not.toHaveBeenCalled();
    detachSecond();
    expect(first.dispose).toHaveBeenCalledOnce();

    const second = sessionBridge();
    schoolSessions.attach(second);
    await vi.waitFor(() =>
      expect(second.onSchoolSession).toHaveBeenCalledOnce(),
    );
  });

  it("rejects late events from an old bridge and accepts a new revision epoch", async () => {
    const first = sessionBridge({
      revision: 8,
      profiles: schoolProfiles("active", "failed"),
    });
    schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("active"),
    );

    const second = sessionBridge({
      revision: 0,
      profiles: schoolProfiles("pending", "not-configured"),
    });
    schoolSessions.attach(second);
    await vi.waitFor(() =>
      expect(schoolSessions.get("tokyo-tech")).toBe("not-configured"),
    );
    expect(schoolSessions.get("science")).toBeUndefined();

    first.emit({ profile: "science", state: "failed", revision: 99 });
    expect(schoolSessions.get("science")).toBeUndefined();
    second.emit({ profile: "science", state: "active", revision: 1 });
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("preserves revisions across a brief reconnect to the same bridge", async () => {
    const native = sessionBridge({
      revision: 5,
      profiles: schoolProfiles("failed", "pending"),
    });
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("failed"),
    );
    native.emit({ profile: "science", state: "active", revision: 6 });
    detach();
    native.schoolSessionSnapshot.mockResolvedValue({
      revision: 0,
      profiles: schoolProfiles("failed", "failed"),
    });

    schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.schoolSessionSnapshot).toHaveBeenCalledTimes(2),
    );
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("forwards focus and visibility facts without browser-side throttling", async () => {
    const native = sessionBridge();
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(native.schoolSessionLifecycle).toHaveBeenCalledWith({
        kind: "launch",
        visible: true,
      }),
    );
    window.dispatchEvent(new Event("focus"));
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      value: "hidden",
    });
    document.dispatchEvent(new Event("visibilitychange"));
    expect(native.schoolSessionLifecycle.mock.calls.slice(1)).toEqual([
      [{ kind: "focus", visible: true }],
      [{ kind: "focus", visible: false }],
    ]);
    detach();
  });
});

/* ------------------------------------------------------------------ *
 * Reference polling network policy
 * ------------------------------------------------------------------ */

const onlineDescriptor = Object.getOwnPropertyDescriptor(navigator, "onLine");
const connectionDescriptor = Object.getOwnPropertyDescriptor(
  navigator,
  "connection",
);

function setNetwork(
  online: boolean,
  connection?: { type?: string; saveData?: boolean; metered?: boolean },
) {
  Object.defineProperty(navigator, "onLine", {
    configurable: true,
    value: online,
  });
  Object.defineProperty(navigator, "connection", {
    configurable: true,
    value: connection,
  });
}

function restoreProperty(
  name: "onLine" | "connection",
  descriptor?: PropertyDescriptor,
) {
  if (descriptor) Object.defineProperty(navigator, name, descriptor);
  else Reflect.deleteProperty(navigator, name);
}

describe("network hint for the sync planner", () => {
  afterEach(() => {
    restoreProperty("onLine", onlineDescriptor);
    restoreProperty("connection", connectionDescriptor);
  });

  it("pauses offline and metered connections", () => {
    setNetwork(false);
    expect(networkHint()).toBe("offline");

    for (const connection of [
      { type: "cellular" },
      { type: "wifi", saveData: true },
      { type: "wifi", metered: true },
    ]) {
      setNetwork(true, connection);
      expect(networkHint()).toBe("cellular");
    }
  });

  it("names Wi-Fi and leaves an unreported link unknown for Core", () => {
    setNetwork(true, { type: "wifi" });
    expect(networkHint()).toBe("wifi");
    setNetwork(true);
    expect(networkHint()).toBe("unknown");
  });
});

/* ------------------------------------------------------------------ *
 * Reminder log migration
 * ------------------------------------------------------------------ */

describe("reminder log migration", () => {
  it("moves the legacy log only after Core has persisted it", async () => {
    const order: string[] = [];
    const fired = { "class:abc": 123 };
    const storage = {
      getItem: () => JSON.stringify(fired),
      removeItem: vi.fn(() => {
        order.push("remove");
      }),
    };
    const bridge = {
      bootstrapReminders: vi.fn(async (value: unknown) => {
        expect(value).toEqual(fired);
        order.push("persist");
        return true;
      }),
    };
    await bootstrapReminders(bridge, storage);
    expect(order).toEqual(["persist", "remove"]);
    expect(storage.removeItem).toHaveBeenCalledWith("isct-reminders-fired");
  });

  it("keeps the legacy record when native persistence fails", async () => {
    const storage = { getItem: () => '{"class:abc":123}', removeItem: vi.fn() };
    await expect(
      bootstrapReminders(
        {
          bootstrapReminders: async () => {
            throw new Error("disk full");
          },
        },
        storage,
      ),
    ).rejects.toThrow("disk full");
    expect(storage.removeItem).not.toHaveBeenCalled();
  });

  it("transfers an empty log when the legacy browser storage is invalid", async () => {
    const bootstrap = vi.fn(async () => true);
    await bootstrapReminders(
      { bootstrapReminders: bootstrap },
      { getItem: () => "invalid", removeItem: () => undefined },
    );
    expect(bootstrap).toHaveBeenCalledWith({});
  });
});

/* Assignment draft upload: the external Moodle uploader owns the network work. */
describe("assignment upload injection", () => {
  function uploadPage(
    url = "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission",
    loaded = true,
  ) {
    const page = new JSDOM(
      `<form><div class="fitem"><input name="files_filemanager"><div class="filemanager ${loaded ? "fm-loaded" : "fm-loading"}"><div class="filemanager-container"></div></div></div><button type="submit" id="final-submit">Submit</button></form>`,
      { url, runScripts: "outside-only" },
    );
    loginPages.push(page);
    const w = page.window;
    // jsdom has no drag-transfer APIs; retain real File, DOM events and the injected script.
    w.DataTransfer = class {
      files: File[] = [];
      types = ["Files"];
      items = { add: (file: File) => this.files.push(file) };
    };
    w.DragEvent = class extends w.Event {
      dataTransfer: unknown;
      constructor(type: string, options: { dataTransfer: unknown }) {
        super(type, options);
        this.dataTransfer = options.dataTransfer;
      }
    };
    const drops: File[][] = [];
    w.document
      .querySelector(".filemanager-container")!
      .addEventListener("drop", (event: Event) => {
        event.preventDefault();
        drops.push([
          ...(event as unknown as { dataTransfer: { files: File[] } })
            .dataTransfer.files,
        ]);
      });
    const submitted = vi.fn((event: Event) => event.preventDefault());
    w.document.querySelector("form")!.addEventListener("submit", submitted);
    const start = () =>
      w.eval(
        `${readScript("assignment-upload.js")}(${JSON.stringify({ request: "test-upload", url: "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&action=editsubmission", files: [{ name: "report.txt", data: Buffer.from("my report").toString("base64") }], messages: { waiting: "waiting", queued: "Review and submit yourself.", failed: "Choose files in LMS." } })})`,
      );
    return { w, drops, submitted, start };
  }

  it("hands files to Moodle once and leaves the final submission to the user", () => {
    const page = uploadPage();
    page.start();
    page.start();
    expect(page.drops).toHaveLength(1);
    expect(page.drops[0][0].name).toBe("report.txt");
    expect(page.drops[0][0].size).toBe(9);
    expect(page.w.document.querySelector('[role="status"]')?.textContent).toBe(
      "Review and submit yourself.",
    );
    expect(page.submitted).not.toHaveBeenCalled();
    page.w.document.querySelector("#final-submit")!.click();
    expect(page.submitted).toHaveBeenCalledTimes(1);
  });

  it.each([
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=200&action=editsubmission",
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100",
    "https://lms.s.isct.ac.jp/2025/mod/assign/view.php?id=100&id=200&action=editsubmission",
    "https://example.com/2025/mod/assign/view.php?id=100&action=editsubmission",
  ])("never transfers files on an unrequested page: %s", (url) => {
    const page = uploadPage(url);
    page.start();
    expect(page.drops).toHaveLength(0);
    expect(page.submitted).not.toHaveBeenCalled();
  });

  it("waits for Moodle initialization and cancels on navigation", async () => {
    const page = uploadPage(undefined, false);
    page.start();
    expect(page.drops).toHaveLength(0);
    page.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(page.drops).toHaveLength(1);
    const leaving = uploadPage(undefined, false);
    leaving.start();
    leaving.w.dispatchEvent(new leaving.w.Event("pagehide"));
    leaving.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(leaving.drops).toHaveLength(0);
    const changed = uploadPage(undefined, false);
    changed.start();
    changed.w.history.replaceState(null, "", "?id=200&action=editsubmission");
    changed.w.document.querySelector(".filemanager")!.className =
      "filemanager fm-loaded";
    await Promise.resolve();
    expect(changed.drops).toHaveLength(0);
  });
});

describe("visible school OTP enrollment forms", () => {
  it("reads the candidate and submits only the same account, secret, CSRF form and live code", () => {
    const dom = new JSDOM(
      `<a class="nav-link dropdown-toggle">student / Name</a>
      <form method="post" action="./add"><input name="_csrf" value="csrf">
      <input name="secret" value="JBSWY3DPEHPK3PXP"><input id="token" name="token">
      <button type="button" onclick="doSubmit(forms[0]); return false;">Setup</button></form>`,
      {
        url: "https://isct.ex-tic.com/idm/user/totp_setting/add",
        runScripts: "outside-only",
      },
    );
    const window = dom.window as Window & typeof globalThis;
    window.TextEncoder = TextEncoder;
    window.setTimeout = (() => 0) as unknown as typeof window.setTimeout;
    const button = window.document.querySelector("button")!;
    const clicked = vi.fn();
    button.addEventListener("click", clicked);
    const config = {
      operation: 1,
      sequence: 2,
      token: "nonce",
      relay: "relay:",
      action: "inspect",
      owner: "student",
      secret: "JBSWY3DPEHPK3PXP",
      code: "123456",
      validUntil: Date.now() / 1000 + 30,
    };
    const run = (extra = {}) =>
      window.eval(
        readScript("otp-enrollment.js").replace(
          "__OTP_ENROLLMENT_CONFIG__",
          JSON.stringify({ ...config, ...extra }),
        ),
      );
    const relay = () =>
      JSON.parse(
        Buffer.from(window.document.title.slice(6), "base64url").toString(),
      );
    try {
      run();
      expect(relay()).toMatchObject({
        kind: "secret",
        owner: "student",
        secret: config.secret,
        token: "nonce",
      });
      for (const invalid of [
        { owner: "other" },
        { secret: "other" },
        { validUntil: 0 },
      ]) {
        run({ action: "submit", ...invalid });
        expect(relay().kind).toBe("error");
        expect(clicked).not.toHaveBeenCalled();
      }
      run({ action: "submit" });
      expect(clicked).toHaveBeenCalledTimes(1);
      expect(
        (window.document.querySelector("#token") as HTMLInputElement).value,
      ).toBe("123456");
      window.document.querySelector('input[name="_csrf"]')!.remove();
      run({ action: "submit" });
      expect(relay().kind).toBe("error");
      expect(clicked).toHaveBeenCalledTimes(1);
    } finally {
      window.close();
    }
  });
});

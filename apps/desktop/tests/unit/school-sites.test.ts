// @vitest-environment jsdom
import { runInNewContext } from "node:vm";

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
import { subscriptionNetwork } from "@app/platform/subscription-polling";
import { readRepoFile } from "@tests/harness";

/**
 * Every school-web contract reachable from the webview: child-webview geometry,
 * the Rust host/URL policy that gates navigation, the scripts the native side
 * injects, the session projection the shell renders, reference-polling network
 * policy and the reminder-log migration.
 *
 * The injected scripts are shipped by Rust; this suite runs the shipped files
 * rather than a transcription of them. The navigation policy itself lives in
 * `crates/isct-core` and cannot be executed in this runtime, so the tests over
 * it assert the documented policy encoded in that source (host list, sanitising
 * rules, `SchoolSite` mapping) instead of pretending to call it.
 */

const SCHOOL_BROWSER = "crates/isct-core/src/school_browser";
const readScript = (name: string) =>
  readRepoFile(`${SCHOOL_BROWSER}/assets/${name}`);

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
 * Official school host policy (crates/isct-core)
 * ------------------------------------------------------------------ */

const policySource = () => readRepoFile(`${SCHOOL_BROWSER}/policy.rs`);
const lifecycleSource = () => readRepoFile("crates/isct-core/src/lifecycle.rs");

/** The `SCHOOL_HOSTS` array literal, comments stripped. */
function schoolHostBlock(): string {
  const source = policySource();
  const declaration = source.indexOf("pub const SCHOOL_HOSTS");
  expect(declaration, "SCHOOL_HOSTS is declared").toBeGreaterThan(-1);
  const equals = source.indexOf("=", declaration);
  const open = source.indexOf("[", equals);
  const end = source.indexOf("];", open);
  expect(open, "SCHOOL_HOSTS has an array literal").toBeGreaterThan(-1);
  expect(end, "SCHOOL_HOSTS is a finished array").toBeGreaterThan(open);
  return source
    .slice(open, end)
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n");
}

function schoolHosts(): string[] {
  return [...schoolHostBlock().matchAll(/"([^"]+)"/g)].map((match) => match[1]);
}

/** The body of `sanitize_school_url`. */
function sanitizeBody(): string {
  const source = policySource();
  const start = source.indexOf("pub fn sanitize_school_url");
  expect(start, "sanitize_school_url is defined").toBeGreaterThan(-1);
  const end = source.indexOf("\n}", start);
  expect(end, "sanitize_school_url is a finished function").toBeGreaterThan(
    start,
  );
  return source.slice(start, end);
}

/** host -> SchoolSite variant, from `SchoolSite::for_url`. */
function siteMapping(): Record<string, string> {
  const source = lifecycleSource();
  const mapping: Record<string, string> = {};
  // Handles both `=> Some(Self::X)` and a braced `=> {\n Some(Self::X)\n }`.
  const arm =
    /("(?:[^"\\]|\\.)*"(?:\s*\|\s*"(?:[^"\\]|\\.)*")*)\s*=>\s*\{?\s*Some\(Self::(\w+)\)/g;
  for (const match of source.matchAll(arm)) {
    for (const host of [...match[1].matchAll(/"([^"]+)"/g)].map(
      (value) => value[1],
    ))
      mapping[host] = match[2];
  }
  return mapping;
}

describe("official school hosts", () => {
  it("allowlists exactly the documented official school hosts", () => {
    expect(schoolHosts()).toEqual([
      "isct.ex-tic.com",
      "portal.isct.ac.jp",
      "portal.titech.ac.jp",
      "lms.s.isct.ac.jp",
      "t2schola.titech.ac.jp",
      "syllabus.s.isct.ac.jp",
      "portal.nap.gsic.titech.ac.jp",
      "rp.nap.gsic.titech.ac.jp",
      "mailv4.m.titech.ac.jp",
      "kyomu0.gakumu.titech.ac.jp",
      "request.libra.titech.ac.jp",
      "portal.t4.gsic.titech.ac.jp",
      "www.titech.ac.jp",
      "yushima2.tmd.ac.jp",
      "lib02.tmd.ac.jp",
      // Services linked from the Tokyo Tech Portal resource menu.
      "portfolio2.gakumu.titech.ac.jp",
      "healdb.gakumu.titech.ac.jp",
      "acc1.dx.titech.ac.jp",
      "vlmgr-42.edu2024.gsic.titech.ac.jp",
      "lsci.rpd.titech.ac.jp",
      "apm.nap.gsic.titech.ac.jp",
      "search-inside.star.titech.ac.jp",
      "signin.noc.titech.ac.jp",
      "idp-gakunin.nap.gsic.titech.ac.jp",
      "idp-saas.nap.gsic.titech.ac.jp",
    ]);
  });
});

describe("school URL sanitisation", () => {
  // `sanitize_school_url` is Rust and cannot run in this runtime, so these
  // assertions pin the policy it encodes: the guard clauses and the exact set
  // of query parameters that survive.
  it("documents rejection of non-https, credential-bearing and foreign-host URLs", () => {
    const body = sanitizeBody();
    expect(body).toMatch(/url\.scheme\(\)\s*!=\s*"https"/);
    expect(body).toMatch(/url\.username\(\)\.is_empty\(\)/);
    expect(body).toMatch(/url\.password\(\)\.is_some\(\)/);
    expect(body).toMatch(/SCHOOL_HOSTS\.contains\(&host\)/);
  });

  it("documents the bounded query parameters that survive", () => {
    const body = sanitizeBody();
    // Identifier keys accepted with a bounded alphanumeric value...
    const identifierKeys = [
      ...(
        body.match(/matches!\(key\.as_ref\(\),\s*([^)]*)\)/)?.[1] ?? ""
      ).matchAll(/"([a-z]+)"/g),
    ].map((match) => match[1]);
    // ...plus the exact keys with their own value grammar.
    const exactKeys = [...body.matchAll(/key\s*==\s*"([a-z]+)"/g)].map(
      (match) => match[1],
    );
    expect(identifierKeys).toEqual(["id", "course", "page", "section"]);
    expect(exactKeys).toEqual(["action", "jwc", "lang", "plang"]);
    expect([...identifierKeys, ...exactKeys]).not.toContain("x");
    // Bounds the values must satisfy, and the fragment/query rewrite.
    expect(body).toMatch(/value\.len\(\)\s*<=\s*128/);
    expect(body).toMatch(/value\.len\(\)\s*==\s*9/);
    expect(body).toMatch(/is_ascii_digit/);
    expect(body).toMatch(/set_fragment\(None\)/);
    expect(body).toMatch(/set_query\(None\)/);
  });
});

describe("school site mapping", () => {
  it("maps every verified allowlisted host to a school site", () => {
    const mapping = siteMapping();
    expect(mapping).toEqual({
      "lms.s.isct.ac.jp": "ScienceLms",
      "t2schola.titech.ac.jp": "ScienceLms",
      "syllabus.s.isct.ac.jp": "Syllabus",
      "isct.ex-tic.com": "SciencePortal",
      "portal.isct.ac.jp": "SciencePortal",
      "portal.titech.ac.jp": "TokyoPortal",
      "portal.nap.gsic.titech.ac.jp": "TokyoPortal",
      "rp.nap.gsic.titech.ac.jp": "TokyoPortal",
      "mailv4.m.titech.ac.jp": "TokyoMail",
      "kyomu0.gakumu.titech.ac.jp": "Kyomu",
      "request.libra.titech.ac.jp": "Library",
      "portal.t4.gsic.titech.ac.jp": "Tsubame",
      "www.titech.ac.jp": "StudyGuide",
    });
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
const liveOtp = () => [
  { from: nowSeconds() - 5, code: "123456" },
  { from: nowSeconds() + 25, code: "654321" },
];

function loadLoginPage(
  html: string,
  options: {
    url?: string;
    profile?: "science" | "tokyo-tech";
    otp?: { from: number; code: string }[];
    matrix?: string;
    storage?: Record<string, string>;
  } = {},
): LoginPage {
  const dom = new JSDOM(html, {
    url: options.url ?? "https://isct.ex-tic.com/auth/session",
    runScripts: "outside-only",
    virtualConsole: new VirtualConsole(),
  });
  loginPages.push(dom as unknown as { window: { close(): void } });
  const window = dom.window as Window & typeof globalThis;
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

  it("reports a rejected TOTP without retrying", async () => {
    const page = loadLoginPage(secondFactorPage(false), {
      url: SECOND_FACTOR,
    });
    page.start();
    await settle();
    page.$("#totp-form .message.warning").style.display = "";
    await settle();
    expect(page.submitted).toHaveLength(1);
    expect(page.relays.at(-1)?.state).toBe("invalid-otp");
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
  states: { science: "pending", tokyoTech: "not-configured" },
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
      states: { science: "failed", tokyoTech: "active" },
    });
    await vi.waitFor(() =>
      expect(schoolSessions.get("tokyo-tech")).toBe("active"),
    );
    expect(schoolSessions.get("science")).toBe("active");
  });

  it("ignores stale and duplicate revisions for each profile", async () => {
    const native = sessionBridge({
      revision: 4,
      states: { science: "active", tokyoTech: "failed" },
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
      states: { science: "active", tokyoTech: "failed" },
    });
    schoolSessions.attach(first);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("active"),
    );

    const second = sessionBridge({
      revision: 0,
      states: { science: "pending", tokyoTech: "not-configured" },
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
      states: { science: "failed", tokyoTech: "pending" },
    });
    const detach = schoolSessions.attach(native);
    await vi.waitFor(() =>
      expect(schoolSessions.get("science")).toBe("failed"),
    );
    native.emit({ profile: "science", state: "active", revision: 6 });
    detach();
    native.schoolSessionSnapshot.mockResolvedValue({
      revision: 0,
      states: { science: "failed", tokyoTech: "failed" },
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
 * Injected link routing (school-links.js)
 * ------------------------------------------------------------------ */

const run = (source: string) => new Function(source)();

describe("school link routing", () => {
  const open = vi.fn();
  let prevented: boolean | undefined;

  beforeAll(() => {
    window.open = open;
    run(readScript("school-links.js"));
    // Registered last: records the outcome, then stops jsdom navigating.
    window.addEventListener("click", (event) => {
      prevented = event.defaultPrevented;
      event.preventDefault();
    });
  });

  afterEach(() => {
    open.mockClear();
    prevented = undefined;
    document.body.replaceChildren();
  });

  const click = (attributes: Record<string, string>, init?: MouseEventInit) => {
    const link = document.createElement("a");
    for (const [key, value] of Object.entries(attributes))
      link.setAttribute(key, value);
    link.textContent = "course";
    document.body.append(link);
    link.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true, ...init }),
    );
  };

  it("sends cross-origin new-window links through window.open", () => {
    // 教務Web's course names; the app routes them to the Syllabus service.
    click({
      href: "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      target: "_blank",
    });
    expect(open).toHaveBeenCalledWith(
      "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      "_blank",
      "noopener",
    );
    expect(prevented).toBe(true);
    click({ href: "https://www.isct.ac.jp/ja/a.pdf", target: "_BLANK" });
    expect(open).toHaveBeenLastCalledWith(
      "https://www.isct.ac.jp/ja/a.pdf",
      "_blank",
      "noopener",
    );
  });

  it("leaves every other click to the page and WebKit", () => {
    click({ href: "https://syllabus.s.isct.ac.jp/" });
    click({ href: "/same-origin", target: "_blank" });
    click({ href: "mailto:office@example.com", target: "_blank" });
    click(
      { href: "https://syllabus.s.isct.ac.jp/", target: "_blank" },
      { metaKey: true },
    );
    const link = document.createElement("a");
    link.href = "https://syllabus.s.isct.ac.jp/";
    link.target = "_blank";
    link.addEventListener("click", (event) => event.preventDefault());
    document.body.append(link);
    link.dispatchEvent(
      new MouseEvent("click", { bubbles: true, cancelable: true }),
    );
    expect(open).not.toHaveBeenCalled();
  });
});

/* ------------------------------------------------------------------ *
 * 教務Web popup overlay (kyomu-popup.js)
 * ------------------------------------------------------------------ */

describe("教務Web popup overlay", () => {
  const nativeOpen = vi.fn(() => null);

  beforeAll(() => {
    window.open = nativeOpen;
    // The script only runs on https://kyomu0…; jsdom serves http://localhost.
    run(
      readScript("kyomu-popup.js")
        .replace(
          'location.protocol !== "https:"',
          'location.protocol !== "http:"',
        )
        .replace('"kyomu0.gakumu.titech.ac.jp"', `"${location.hostname}"`),
    );
  });

  afterEach(() => {
    nativeOpen.mockClear();
    document.body.replaceChildren();
  });

  const overlays = () => [
    ...document.querySelectorAll<HTMLIFrameElement>(
      "iframe[data-isct-companion-popup]",
    ),
  ];

  it("shows same-origin popups in a named frame that knows its opener", async () => {
    const popup = window.open(
      "/Titech/PID1_3_1.aspx?cps=1",
      "PID13_1aspx",
      "width=600, height=400",
    );
    const [frame] = overlays();
    expect(frame.name).toBe("PID13_1aspx");
    expect(frame.src).toBe(
      new URL("/Titech/PID1_3_1.aspx?cps=1", location.href).href,
    );
    expect(
      (frame as HTMLIFrameElement & { isctCompanionOpener?: Window })
        .isctCompanionOpener,
    ).toBe(window);
    expect(popup).toBe(frame.contentWindow);
    // The same window name reuses the overlay, like a named popup window.
    window.open("/Titech/PID1_3_1.aspx?cps=2", "PID13_1aspx");
    expect(overlays()).toHaveLength(1);
    expect(nativeOpen).not.toHaveBeenCalled();

    (
      window as unknown as Window & {
        __isctCompanionPopups: { close(frame: Element): void };
      }
    ).__isctCompanionPopups.close(frame);
    await new Promise((resolve) => setTimeout(resolve));
    expect(overlays()).toHaveLength(0);
  });

  it("keeps the app's routing for other origins and named targets", () => {
    window.open(
      "https://syllabus.s.isct.ac.jp/courses?jwc=202512345",
      "_blank",
      "noopener",
    );
    window.open("/Titech/Default.aspx", "_top");
    expect(nativeOpen).toHaveBeenCalledTimes(2);
    expect(overlays()).toHaveLength(0);
  });
});

/* ------------------------------------------------------------------ *
 * Android support injection (android-school-support.js)
 * ------------------------------------------------------------------ */

type SupportFrame = {
  win: {
    eval: (source: string) => void;
    document: {
      addEventListener: (
        type: string,
        callback: (event: unknown) => void,
      ) => void;
      querySelectorAll: (selector: string) => { contentWindow: unknown }[];
    };
    __isctCompanionAndroidSupportInstalled?: boolean;
  };
  scripts: string[];
  listeners: ((event: unknown) => void)[];
  children: { contentWindow: unknown }[];
};

const SUPPORT_SCRIPTS = ["popup support", "link support"];

function supportFrame(): SupportFrame {
  const scripts: string[] = [];
  const listeners: ((event: unknown) => void)[] = [];
  const children: { contentWindow: unknown }[] = [];
  const win = {
    eval: (source: string) => scripts.push(source),
    document: {
      addEventListener: (_: string, callback: (event: unknown) => void) =>
        listeners.push(callback),
      querySelectorAll: () => children,
    },
  } as SupportFrame["win"];
  return { win, scripts, listeners, children };
}

const ANDROID_SUPPORT = readScript("android-school-support.js").replace(
  "__ISCT_SUPPORT_SCRIPTS__",
  JSON.stringify(SUPPORT_SCRIPTS),
);

const installAndroidSupport = (frame: SupportFrame) =>
  runInNewContext(ANDROID_SUPPORT, { window: frame.win });

const fireFrameLoad = (frame: SupportFrame, child: SupportFrame) => {
  for (const listener of frame.listeners)
    listener({ target: { tagName: "IFRAME", contentWindow: child.win } });
};

describe("Android school support injection", () => {
  it("installs the support scripts once per document", () => {
    const main = supportFrame();
    installAndroidSupport(main);
    installAndroidSupport(main);
    expect(main.scripts).toEqual(SUPPORT_SCRIPTS);
  });

  it("teaches frames that already exist on the page", () => {
    const main = supportFrame();
    const existing = supportFrame();
    main.children.push({ contentWindow: existing.win });
    installAndroidSupport(main);
    expect(existing.scripts).toEqual(SUPPORT_SCRIPTS);
  });

  it("teaches a later frame exactly once, when its load event fires", () => {
    const main = supportFrame();
    installAndroidSupport(main);
    const popup = supportFrame();
    fireFrameLoad(main, popup);
    fireFrameLoad(main, popup);
    expect(popup.scripts).toEqual(SUPPORT_SCRIPTS);
  });

  it("propagates through nested frames", () => {
    const main = supportFrame();
    installAndroidSupport(main);
    const popup = supportFrame();
    fireFrameLoad(main, popup);
    const nested = supportFrame();
    fireFrameLoad(popup, nested);
    expect(nested.scripts).toEqual(SUPPORT_SCRIPTS);
  });

  it("ignores a cross-origin frame instead of failing the injection", () => {
    const main = supportFrame();
    installAndroidSupport(main);
    const foreign = {
      get document(): never {
        throw new Error("Cross origin");
      },
    };
    expect(() =>
      fireFrameLoad(main, { win: foreign } as unknown as SupportFrame),
    ).not.toThrow();
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

describe("reference subscription shell policy", () => {
  afterEach(() => {
    restoreProperty("onLine", onlineDescriptor);
    restoreProperty("connection", connectionDescriptor);
  });

  it("pauses offline and metered connections", () => {
    setNetwork(false);
    expect(subscriptionNetwork()).toBe("offline");

    for (const connection of [
      { type: "cellular" },
      { type: "wifi", saveData: true },
      { type: "wifi", metered: true },
    ]) {
      setNetwork(true, connection);
      expect(subscriptionNetwork()).toBe("cellular");
    }
  });

  it("treats Wi-Fi and unknown connections as unmetered", () => {
    setNetwork(true, { type: "wifi" });
    expect(subscriptionNetwork()).toBe("wifi");
    setNetwork(true);
    expect(subscriptionNetwork()).toBe("wifi");
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

describe("complete native extraction relay", () => {
  const render = (extractor: string) =>
    readScript("extraction-relay.js")
      .replace(
        "__ISCT_RELAY__",
        JSON.stringify("isct-companion-extract://test/"),
      )
      .replace("__ISCT_RUN__", JSON.stringify("run-test"))
      .replace("__ISCT_EXTRACTOR__", extractor);
  const execute = async (extractor: string) => {
    const location = { href: "https://example.invalid/" };
    runInNewContext(render(extractor), {
      location,
      btoa: (text: string) => Buffer.from(text, "binary").toString("base64"),
      unescape,
      encodeURIComponent,
    });
    await vi.waitFor(() =>
      expect(location.href).toMatch(/^isct-companion-extract:/),
    );
    return JSON.parse(
      Buffer.from(location.href.split("test/")[1], "base64url").toString(
        "utf8",
      ),
    );
  };
  it("executes real semicolon-terminated scripts and relays async failures instead of timing out", async () => {
    for (const name of ["kyomu-extract.js", "kyomu-search.js"]) {
      const result = await execute(readScript(name));
      expect(result._isctExtractionRun).toBe("run-test");
      expect(result.status).toBeDefined();
    }
    expect(
      await execute(
        "// leading comment\n(async () => ({ status: 'ok', title: '日本語' }))();",
      ),
    ).toMatchObject({
      status: "ok",
      title: "日本語",
      _isctExtractionRun: "run-test",
    });
    expect(
      await execute(
        "(async () => { throw new Error('private page contents'); })();",
      ),
    ).toMatchObject({
      status: "error",
      error: "School page extraction failed.",
      _isctExtractionRun: "run-test",
    });
  });
});

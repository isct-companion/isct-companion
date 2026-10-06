// @ts-expect-error jsdom ships without type declarations here.
import { JSDOM, VirtualConsole } from "jsdom";
import { afterEach, describe, expect, it } from "vitest";

import source from "../../backend/scripts/school-login.js?raw";

// Fixtures copy the live Extic / GetAccess markup; values, tokens and the
// pages' own scripts are omitted (their effects are simulated where needed).

type Relay = { profile: string; run: string; state: string; message: string };

type Page = { window: Window & typeof globalThis };
const pages: Page[] = [];
afterEach(() => pages.splice(0).forEach((dom) => dom.window.close()));

const now = () => Math.floor(Date.now() / 1000);
const liveOtp = () => [
  { from: now() - 5, code: "123456" },
  { from: now() + 25, code: "654321" },
];

function load(
  html: string,
  {
    url = "https://isct.ex-tic.com/auth/session",
    profile = "science",
    otp = liveOtp() as { from: number; code: string }[],
    matrix = "",
    storage = {} as Record<string, string>,
  } = {},
) {
  const dom = new JSDOM(html, {
    url,
    runScripts: "outside-only",
    virtualConsole: new VirtualConsole(),
  });
  pages.push(dom);
  const window = (dom as Page).window;
  const { document } = window;
  Object.entries(storage).forEach(([key, value]) =>
    window.sessionStorage.setItem(key, value),
  );
  // jsdom has no layout: treat display:none, [hidden] and hidden inputs as
  // invisible, everything else (including off-screen) as rendered.
  window.HTMLElement.prototype.getClientRects = function () {
    for (let node: HTMLElement | null = this; node; node = node.parentElement) {
      if (
        node.hidden ||
        window.getComputedStyle(node).display === "none" ||
        (node instanceof window.HTMLInputElement &&
          (node as HTMLInputElement).type === "hidden")
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
      relays.push(JSON.parse(new TextDecoder().decode(bytes)));
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
    profile,
    run: "run-1",
    username: "ab12cd34",
    password: "pw",
    otp,
    matrix,
    authRelay: "relay:",
    debugRelay: null,
  };
  const start = () =>
    window.eval(
      source.replace("__ISCT_COMPANION_LOGIN_CONFIG__", JSON.stringify(config)),
    );
  const $ = (selector: string) =>
    document.querySelector(selector) as HTMLElement;
  return { window, document, $, relays, submitted, start };
}

const settle = (ms = 60) => new Promise((resolve) => setTimeout(resolve, ms));
const progress = (relays: Relay[]) =>
  relays.filter((relay) => relay.state === "progress").map((r) => r.message);

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
    const page = load(identifierPage);
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
    const page = load(identifierPage);
    page.$("form#login button").setAttribute("disabled", "");
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    page.$("form#login button").removeAttribute("disabled");
    await settle();
    expect(page.submitted).toHaveLength(1);
  });

  it("never resubmits a rejected password", async () => {
    const page = load(identifierPage.replace('class=" move-off-screen" ', ""), {
      storage: { "isct-companion-login:run-1": JSON.stringify({ password: 1 }) },
    });
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("invalid-credentials");
  });

  it("reports the login warning as a rejected password", async () => {
    const page = load(
      identifierPage.replace(
        '<div class="message warning" style="display: none">',
        '<div class="message warning">',
      ),
      { storage: { "isct-companion-login:run-1": JSON.stringify({ password: 1 }) } },
    );
    page.start();
    await settle();
    expect(page.relays.at(-1)).toMatchObject({
      state: "invalid-credentials",
      message: "Login failed.",
    });
  });

  it("picks App OTP from the method picker, then fills the TOTP", async () => {
    const page = load(secondFactorPage(true), { url: SECOND_FACTOR });
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
    const page = load(secondFactorPage(false), { url: SECOND_FACTOR });
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["otp"]);
    expect(page.submitted[0].totp).toBe("123456");
  });

  it("uses the code for the window that is live when the field appears", async () => {
    const page = load(secondFactorPage(false), {
      url: SECOND_FACTOR,
      otp: [
        { from: now() - 65, code: "111111" },
        { from: now() - 35, code: "222222" },
        { from: now() - 5, code: "333333" },
      ],
    });
    page.start();
    await settle();
    expect(page.submitted[0].totp).toBe("333333");
  });

  it("reports a rejected TOTP without retrying", async () => {
    const page = load(secondFactorPage(false), { url: SECOND_FACTOR });
    page.start();
    await settle();
    page.$("#totp-form .message.warning").style.display = "";
    await settle();
    expect(page.submitted).toHaveLength(1);
    expect(page.relays.at(-1)?.state).toBe("invalid-otp");
  });

  it("hands an email-only second factor to the person", async () => {
    const page = load(
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
    const page = load(
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
    profile: "tokyo-tech",
    otp: [],
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
    storage: { "isct-companion-login:run-1": JSON.stringify({ password: 1 }) },
  };

  it("takes the password agreement button and ignores the site search box", async () => {
    const page = load(
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
    const page = load(
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
    const page = load(otpPage(true), afterPassword);
    page.start();
    await settle();
    expect(progress(page.relays)).toEqual(["method"]);
    expect(page.submitted).toHaveLength(1);
    expect((page.$('select[name="message3"]') as HTMLSelectElement).value).toBe(
      "GridAuthOption",
    );
  });

  it("hands an OTP-only account to the person", async () => {
    const page = load(otpPage(false), afterPassword);
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("needs-user");
  });

  it("answers the Matrix challenge by coordinate and stays on Matrix", async () => {
    const page = load(matrixPage, afterPassword);
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
    const page = load(matrixPage, {
      ...afterPassword,
      storage: {
        "isct-companion-login:run-1": JSON.stringify({ password: 1, matrix: 1 }),
      },
    });
    page.start();
    await settle();
    expect(page.submitted).toHaveLength(0);
    expect(page.relays.at(-1)?.state).toBe("invalid-matrix");
  });

  it("reports a rejected password from the fail redirect", async () => {
    const page = load(`<p>portal</p>`, {
      url: "https://portal.nap.gsic.titech.ac.jp/?Message=fail&GAURI=x",
      ...tokyo,
      storage: { "isct-companion-login:run-1": JSON.stringify({ password: 1 }) },
    });
    page.start();
    await settle();
    expect(page.relays.at(-1)?.state).toBe("invalid-credentials");
  });
});

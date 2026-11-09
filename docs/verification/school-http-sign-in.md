# School sign-in over HTTP: verification record

Date: 2026-11-01. A record of what was measured against the real schools, not
a design or a plan; decisions live in slotledger
(`approach/school-sign-in-core-identity` and the keys around it).

Cookie values, CSRF tokens, SAML/RelayState and ViewState were never written
down; comparisons used the first 8 hex digits of a SHA-256. URLs keep host and
path only.

## Summary

| Question | Finding |
|---|---|
| Can Core sign in to Extic (Science) over plain HTTP and reach the LMS? | Yes, with no browser or script. |
| Can Core sign in to GetAccess (Tokyo Tech, Matrix) over plain HTTP and reach 教務Web? | Yes, with no browser or script. |
| Does a web view accept cookies Core writes into it? | Yes, on macOS only through WebKit's own Set-Cookie parser. Tauri's `set_cookie` turns a domain cookie into a host-only one. |
| Are cookie attributes kept? | HttpOnly, Secure, SameSite, Path and Domain all survive. |
| Is a session bound to Core's User-Agent? | No. The web view, with Safari's UA, is signed in. |
| Is the identity cookie alone enough for a clean browser? | Yes, for both schools, in WebKit and Chromium. |
| Do Core's and the web view's site sessions push each other out? | No, 10/10 rounds on each school. |
| Does a later full sign-in end earlier sessions? | No. Older identity cookies still pass SSO. |
| Does a site's SSO rotate the identity cookie? | No. Each new sign-in has new values; old ones keep working. |
| Wrong password | Extic: HTTP 401 (`text/javascript`). GetAccess: 302 to `portal.titech.ac.jp` or `Message=fail`. One submission, no retry. |
| Wrong Matrix | HTTP 200 with the OTP fallback page, not an error page or a redirect. |
| Forced password change, email OTP | Not recognisable or not reachable with the test accounts; Core hands both to the person. |

Two accounts (Science: ID, password, app OTP; Tokyo Tech: account, password,
Matrix) were used. Two deliberately wrong passwords and one wrong Matrix did
not lock either account.

## 1. Science (Extic → LMS)

### 1.1 Request chain

The LMS home page is the entry; SAML brings up the sign-in page.

| # | Request | Response |
|---|---|---|
| 1 | `GET lms.s.isct.ac.jp/2025/my/` | 303 → `isct.ex-tic.com/auth/saml2/ib-<id>/assertions` |
| 2 | `GET …/assertions` | 302 → `…/assertions/<hash>` |
| 3 | `GET …/assertions/<hash>` | 302 → `isct.ex-tic.com/auth/session` |
| 4 | `GET /auth/session` | 200, sign-in page |
| 5 | `POST /auth/session` | 200 (password accepted) |
| 6 | `GET /auth/session/second_factor` | 200, TOTP page |
| 7 | `POST /auth/session/second_factor` | 200 (code accepted) |
| 8 | `GET lms.s.isct.ac.jp/2025/my/` | 303 → assertions → 302 → `…/assertions/<hash>` |
| 9 | `GET …/assertions/<hash>` | 200: a self-submitting SAML form (HTTP-POST binding), not a redirect |
| 10 | `POST lms.s.isct.ac.jp/2025/auth/saml2/sp/saml2-acs.php/lms.s.isct.ac.jp` | 303 → `/2025/my/` |
| 11 | `GET /2025/my/` | 200, `ダッシュボード \| Science Tokyo LMS` |

### 1.2 Pages

- Sign-in: `form#login`, `action=/auth/session`, `data-remote=true`, fields
  `utf8`, `authenticity_token`, `identifier`, `password`. The page shows the
  password box only after the ID, but one POST with both fields is accepted.
  The request needs `X-Requested-With: XMLHttpRequest` and a
  `text/javascript` Accept.
- Second factor: `form#totp-form`, fields `authenticity_token` and `totp`. The
  method picker only toggles two wrappers; the TOTP form is always in the DOM,
  so it is read before the email one.
- Extic's clock runs slightly fast: a code with less than 8 s left is taken as
  expired and the page silently returns to the picker. Core waits for the next
  window instead (`OTP_MARGIN`).

### 1.3 Cookies

| Host | Name | Domain | Path | Secure | HttpOnly | SameSite | Lifetime | Kind |
|---|---|---|---|---|---|---|---|---|
| isct.ex-tic.com | `_exgen_idp_session` | host-only | / | yes | yes | None | session | identity |
| lms.s.isct.ac.jp | `MoodleSession` | host-only | / | yes | yes | — | session | site |
| lms.s.isct.ac.jp | `MDL_SSP_SessID` | host-only | / | yes | yes | — | session | site |
| lms.s.isct.ac.jp | `MDL_SSP_AuthToken` | host-only | / | yes | yes | — | session | site |
| lms.s.isct.ac.jp | `MOODLEID1_` | host-only | / | yes | yes | — | Max-Age 60 days | site |

The first GET of the LMS already sets a guest `MoodleSession`, so a
`MoodleSession` does not mean signed in. Only the target page decides (it
links `login/logout.php` and is titled as the dashboard).

## 2. Tokyo Tech (GetAccess + Matrix → 教務Web)

### 2.1 Request chain

| # | Request | Response |
|---|---|---|
| 1 | `GET portal.nap.gsic.titech.ac.jp/GetAccess/ResourceList` | 302 → `portal.pl` |
| 2 | `GET portal.pl` | 200, agreement page (`東工大ポータル`) |
| 3 | `GET GetAccess/Login?Template=userpass_key&AUTHMETHOD=UserPassword…` | 200, account and password |
| 4 | `POST GetAccess/Login` | 302 → `rp.nap.gsic.titech.ac.jp/Router/` |
| 5 | `GET Router/` | 302 → `portal.pl` |
| 6 | `GET portal.pl` | 302 → `GetAccess/Login?Template=idg_key&AUTHMETHOD=IG…` |
| 7 | `GET GetAccess/Login?Template=idg_key…` | 200, OTP/Matrix choice (`GridAuthOption`) |
| 8 | `POST` (Matrix chosen) | 200, Matrix challenge (3 cells) |
| 9 | `POST` (3 cells) | 302 → `rp.nap…/Router/` |
| 10 | Router | 302 → `GetAccess/ResourceList` |
| 11 | `GET ResourceList` | 200, `TokyoTech Portal リソース メニュー` |
| 12 | `GET kyomu0.gakumu.titech.ac.jp/Titech/Default.aspx` | 302 → `/Titech/Student/` |
| 13 | `GET /Titech/Student/` | 200, `学生トップ - 教務Webシステム` |
| 14 | `GET /Titech/Student/学生情報/PID4_2.aspx` | 200, parsed by `adpt_isct::kyomu::extract_page` as `profile` |

### 2.2 Points the flow depends on

- `rp.nap.gsic.titech.ac.jp` (the Router) is on the path after the password;
  it is `Host::GetAccessRouter`.
- 教務Web signs in only through `Default.aspx`. A deep link stops at the
  resource menu.
- The agreement page's buttons carry every parameter (`GAURI`,
  `GARESOURCEID`, `APPID`, `Reason`, `URI`) in `onclick`; Core keeps the
  `Template=userpass_key` one as is.
- The credentials form posts `AUTHTYPE`, `HiddenURI`, `GARESOURCEID`,
  `Template`, `AUTHMETHOD`, `pageGenTime`, `LOCALE` and `CSRFFormToken`.
- The account name must be upper case.

### 2.3 Cookies

| Host | Name | Domain | Path | Secure | HttpOnly | SameSite | Lifetime | Kind |
|---|---|---|---|---|---|---|---|---|
| portal.nap.gsic.titech.ac.jp | `JSESSIONID` | host-only | /GetAccess | yes | yes | — | session | identity |
| portal.nap.gsic.titech.ac.jp | `AUTH_SESSION_ID` | `.titech.ac.jp` | / | no | yes | — | session | identity |
| rp.nap.gsic.titech.ac.jp | `Tomcat-D_lb` | host-only | / | no | no | — | session | identity |
| kyomu0.gakumu.titech.ac.jp | `ASP.NET_SessionId` | host-only | / | no | yes | Lax | session | site |
| kyomu0.gakumu.titech.ac.jp | `TitechForm` | host-only | / | no | no | — | session | site |
| kyomu0.gakumu.titech.ac.jp | `Language` | host-only | / | no | yes | — | ~100 days | site |
| kyomu0.gakumu.titech.ac.jp | `titech.kyomuweb.*` (5) | host-only | / | no | yes | — | ~6 hours | site |
| kyomu0.gakumu.titech.ac.jp | `KYOMU-http` | host-only | / | no | no | — | session | site |

`AUTH_SESSION_ID` is the GetAccess SSO session and must reach every
`*.titech.ac.jp` host, 教務Web included.

## 3. Writing cookies into the web views

Core hands the shell the identity snapshot as raw `Set-Cookie` lines with the
URL of the response that set each one (`WebCookieSeed`). The shell must parse
them the way its browser parses a response:

- macOS (`apps/desktop/src-tauri/src/school_cookies.rs`): each line goes
  through `NSHTTPCookie.cookiesWithResponseHeaderFields:forURL:` and into
  `WKWebsiteDataStore.httpCookieStore`.
- Tauri's `set_cookie` cannot do this. `tauri::webview::Cookie` is the
  `cookie` crate's type, whose `domain()` strips the leading dot; wry fills
  `NSHTTPCookieDomain` from it, and WebKit takes a dot-less domain as
  host-only. `AUTH_SESSION_ID` then sits in the store but never reaches
  `kyomu0…`. A cookie-only control (`probe_domain=1; Domain=.isct.ac.jp`)
  was invisible on `www.isct.ac.jp` through Tauri's path and visible through
  the native one.
- Reading the store back through Tauri still shows `titech.ac.jp` without the
  dot, while the stored cookie is `.titech.ac.jp`. Do not judge scope from the
  read-back string.
- Windows (WebView2 `CreateCookie`/`AddOrUpdateCookie`, leading dot restored)
  follows the same rule but was not measured; see the manual check below.

Read back after the native write:

| Cookie | Domain | Path | Secure | HttpOnly | SameSite |
|---|---|---|---|---|---|
| `MoodleSession` | lms.s.isct.ac.jp | / | true | true | — |
| `AUTH_SESSION_ID` | .titech.ac.jp | / | false | true | — |
| `ASP.NET_SessionId` | kyomu0.gakumu.titech.ac.jp | / | false | true | Lax |
| `JSESSIONID` | portal.nap.gsic.titech.ac.jp | /GetAccess | true | true | — |

Sessions are not bound to the User-Agent: Core signed in as
`IsctCompanion/0.1`, and the web view with Safari's UA was signed in on both
schools.

## 4. Failure paths

| Case | Measured | Core's answer | Retried automatically |
|---|---|---|---|
| Extic wrong password | 401, `text/javascript`, empty body | `invalid-credentials` | No, one submission |
| Extic wrong code | Not measured (same endpoint, 401 family) | `invalid-otp` | No |
| GetAccess wrong Matrix | 200 with the OTP fallback page | `invalid-matrix` | No, one submission |
| GetAccess wrong password | 302 to `portal.titech.ac.jp` or `Message=fail` (unknown account) | `invalid-credentials` | No |
| Forced password change | The reachable page is an Angular app (`<app-root>`, no form) | `password-change` / `unknown-page`: the person signs in on the page | — |
| Email OTP | Test account has none | `email-otp`: the person signs in on the page | — |

The Matrix challenge asks 3 of 70 cells. A Matrix with one wrong cell can
still pass; the wrong-Matrix test answers every cell wrong.

## 5. Session experiment: one identity, many site sessions

Run with Python `requests` as Core and Playwright (WebKit, Chromium) as the
web view, to decide whether Core may keep one identity snapshot and build
independent site sessions from it.

| Question | Science (Extic → LMS) | Tokyo Tech (GetAccess → 教務Web) |
|---|---|---|
| Q1 Does a clean browser get in with the identity cookies alone? | Yes (WebKit, Chromium) | Yes (WebKit, Chromium) |
| Q2 Do two site sessions push each other out? | No, 10/10 rounds each | No, 10/10 rounds; an English session 5/5, languages never mixed |
| Q3 After a new full sign-in, do earlier sessions still work? | Yes; the earlier identity still passes SSO | Yes; the earlier identity still passes SSO |
| Q4 After signing out on one side? | The other site session and the old identity still work | The other site sessions and the old identity still work |
| Q5 Does SSO rotate the identity cookie? | No | No |
| Q7 Overnight | Not run | Not run |

Details:

- Q1 controls (no cookies) landed on the sign-in pages. With only the identity
  cookies, both browsers reached `ダッシュボード | Science Tokyo LMS` and
  `学生トップ - 教務Webシステム`. On Tokyo Tech, 教務Web completed SSO
  from `AUTH_SESSION_ID` without visiting `portal.nap`.
- Q2: rounds 20–30 s apart, alternating Core's session (A) and the web view's
  (B). 教務Web's English session (`A-en`) kept English titles throughout, so
  the English site needs its own instance and never shares one with Japanese.
- Q3/Q5: a new sign-in (by Core or by a person in the web view) produced new
  identity values; sessions built on the old values stayed signed in, and the
  old identity cookies still passed SSO. The identity cookie did not change
  across a site's SSO.
- Q4: Science's LMS sign-out goes through Extic's SLO endpoints and ends on
  the IdP account page, yet the other LMS session and the old identity still
  worked. Tokyo Tech's sign-out ends on the GetAccess resource menu; the other
  sessions stayed signed in.
- Submissions: Science 6 passwords and 6 TOTPs; Tokyo Tech 6 passwords and 6
  Matrix answers, over two days' limits.

What the design takes from it: Core keeps the identity snapshot as received
and never writes back into it; each site (LMS, 教務Web, 教務Web English) and
the web view build their own session from a copy; a site response writes only
to the instance it set out with; a sign-in by the person in the web view does
not invalidate Core's sessions, and the reverse.

Measurement limits: the Science Q1 cookie-sent check was not yet recorded (the
successful entry itself shows the cookie was used); request-level host
sequences on sign-out were recorded only for Science.

## 5a. Desktop dev app on the real schools (macOS)

| Date (JST) | What | Result |
| --- | --- | --- |
| 11-01 23:31 | Launch with saved credentials | Both accounts signed in by Core over HTTP on the first attempt. |
| 11-01 23:38–23:46 | Three restarts | Saved sessions restored; no `signing in` line. |
| 11-01 23:35 | School-cache reset with the old reset, which reopened Science while Tokyo was still being cleared | First sign-in of each account refused (Extic 401; GetAccess 302 to `portal.titech.ac.jp`); a Retry with the same credentials signed in. See below. |
| 11-01 23:47, 11-02 00:10 | School-cache reset with both accounts closed until both are clean | Both accounts signed in on the first attempt. |
| 11-02 00:10 | English course catalog after the reset | `KyomuEnglish session ready` after switching on the course search page; every `catalog.official/2026/*/en` key complete. The previous switch, on the student top, left the session Japanese. |
| 11-02 | Open 教務Web, then the LMS, drag a file into an assignment draft | The file reached the LMS; no `denied site=Kyomu reason=Scheme url=file://`. |

The 23:35 refusals were not caused by the credentials Core sent. In a debug
build the credentials item lives in the login keychain, which is not
shared, so Retry's re-read changes nothing; the device-local candidates are
read once per process and only a refusal writes to them. Both attempts of
each pair therefore sent the same credentials, and a refused account signs
in again only on a forced attempt (the person's Retry). The one condition
that has not recurred is the old reset order, which let one account sign
in while the other was being closed and the web view's school cookies were
being cleared. The reset now closes both accounts first and reopens them
together; two resets since then signed in at once. The school-side reason
for the two refusals was not determined.

## 6. Not verified

1. The app's own school views on Windows: the native cookie writer compiles
   but was not run (`DESKTOP-SCHOOL-COOKIES-WINDOWS`).
2. iOS and Android `WebCookieSeed` implementations: iOS compiled only,
   Android code only.
3. Session expiry over hours and Core's own recovery afterwards.
4. Real forced password change and email OTP pages; a real wrong TOTP.
5. Neither school's public sign-in page had a captcha, bot check or
   script-computed field when measured; that can change.

## 7. Rerunning

The real-account tests read credentials only from the environment and never
print or store them. Keep to a few full sign-ins per account per day, at
least 60 s apart.

```
ISCT_SCIENCE_ID, ISCT_SCIENCE_PASSWORD, ISCT_SCIENCE_OTP   # otpauth:// link or base32 secret
ISCT_TOKYO_ID, ISCT_TOKYO_PASSWORD, ISCT_TOKYO_MATRIX      # 70 letters; spaces and newlines allowed
```

```bash
cargo test -p app-core --lib school_session::login -- --ignored --nocapture
```

The flows live in `crates/app-core/src/school_session/` (`login.rs` for the
two sign-ins, `walk.rs` for redirects and forms, `site.rs` for each site's
SSO) and the page readers in `crates/adpt-isct/src/login.rs`.

((config) => {
  // Native evaluation returns this directly to Core. Never put it in a
  // URL, title, console, or app bridge available to the page.
  const send = (kind, extra = {}) => JSON.stringify({ kind, ...extra });
  if (location.origin !== "https://isct.ex-tic.com") return;
  const path = location.pathname.replace(/\/$/, "");
  if (path !== config.path || window.top !== window) return send("error");
  if (config.action === "inspect") {
    Object.defineProperty(document, "__isctOtpDocument", { value: config.document, configurable: true });
  } else if (document.__isctOtpDocument !== config.document) {
    return send("error");
  }
  const owner = (document.querySelector(".nav-link.dropdown-toggle")?.textContent || "")
    .replace(/^[^A-Za-z0-9]+/, "").split("/")[0].trim();
  if (!owner || owner.length > 128) return send("error");
  const validForm = (form, expected) => form && form.method.toLowerCase() === "post"
    && new URL(form.action, location.href).href === `https://isct.ex-tic.com${expected}`
    && form.querySelector('input[name="_csrf"]')?.value;
  if (config.action === "remove") {
    if (path !== "/idm/user/mfa_setting") return send("error");
    const form = document.querySelector("form#del_totp");
    const row = document.querySelector('label[for="appAuth"]')?.closest(".form-group");
    const button = row?.querySelector('button[onclick*="del_totp"]');
    if (!validForm(form, "/idm/user/totp_setting/del") || !button || owner !== config.owner) return send("error");
    button.click();
    return send("sent");
  }
  if (config.action === "submit") {
    if (path !== "/idm/user/totp_setting/add") return send("error");
    const input = document.querySelector('input#token[name="token"]');
    const form = input?.form;
    const button = form?.querySelector('button[onclick*="doSubmit"]');
    if (!validForm(form, "/idm/user/totp_setting/add") || !button
      || form.querySelector('input[name="secret"]')?.value !== config.secret
      || owner !== config.owner || Date.now() / 1000 < config.validFrom
      || Date.now() / 1000 >= config.validUntil) return send("error");
    input.value = config.code;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    button.click();
    return send("sent");
  }
  if (path === "/idm/user/mfa_setting") {
    const row = document.querySelector('label[for="appAuth"]')?.closest(".form-group");
    const unset = !!row?.querySelector('button[onclick*="totp_setting/add"]');
    const registered = !!row?.querySelector("button.btn-danger");
    if (unset === registered) return send("error");
    return send("settings", { owner, registered });
  }
  if (path === "/idm/user/totp_setting/add") {
    const input = document.querySelector('input#token[name="token"]');
    const secret = input?.form?.querySelector('input[name="secret"]')?.value;
    if (!validForm(input?.form, "/idm/user/totp_setting/add") || !secret || secret.length > 1024) return send("error");
    return send("secret", { owner, secret });
  }
})(__OTP_ENROLLMENT_CONFIG__);

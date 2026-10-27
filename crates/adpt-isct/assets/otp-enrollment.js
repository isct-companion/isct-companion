((config) => {
  // This private, correlated native relay is never a URL or a log entry.
  const send = (kind, extra = {}) => {
    const text = JSON.stringify({ operation: config.operation, sequence: config.sequence, token: config.token, kind, ...extra });
    const bytes = new TextEncoder().encode(text);
    let raw = "";
    for (const byte of bytes) raw += String.fromCharCode(byte);
    const title = document.title;
    const relay = config.relay + btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
    document.title = relay;
    setTimeout(() => { if (document.title === relay) document.title = title; }, 250);
  };
  if (location.origin !== "https://isct.ex-tic.com") return;
  const path = location.pathname.replace(/\/$/, "");
  const owner = (document.querySelector(".nav-link.dropdown-toggle")?.textContent || "")
    .replace(/^[^A-Za-z0-9]+/, "").split("/")[0].trim();
  if (!owner || owner.length > 128) return send("error");
  const validForm = (form, expected) => form && form.method.toLowerCase() === "post"
    && new URL(form.action, location.href).href === `https://isct.ex-tic.com${expected}`
    && form.querySelector('input[name="_csrf"]')?.value;
  if (config.action === "remove") {
    if (path !== "/idm/user/mfa_setting") return send("error");
    const form = document.querySelector("form#del_totp");
    const button = document.querySelector('button[onclick*="del_totp"]');
    if (!validForm(form, "/idm/user/totp_setting/del") || !button || owner !== config.owner) return send("error");
    button.click();
    return;
  }
  if (config.action === "submit") {
    if (path !== "/idm/user/totp_setting/add") return send("error");
    const input = document.querySelector('input#token[name="token"]');
    const form = input?.form;
    const button = form?.querySelector('button[onclick*="doSubmit"]');
    if (!validForm(form, "/idm/user/totp_setting/add") || !button
      || form.querySelector('input[name="secret"]')?.value !== config.secret
      || owner !== config.owner || Date.now() / 1000 >= config.validUntil) return send("error");
    input.value = config.code;
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
    button.click();
    return;
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

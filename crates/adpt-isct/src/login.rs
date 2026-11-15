//! The two school sign-in pages, read the way a browser submits them.
//!
//! Extic (Science Tokyo ID) is one page for the ID and the password, then one
//! for the App one-time password. GetAccess (Tokyo Tech) walks agreement →
//! account and password → one-time password or the Matrix challenge, each
//! page its own form.
//!
//! This module only reads a page: it names the step and hands back the form a
//! browser would send (action, every control in document order, the option a
//! browser would have selected). Which step to take, how often and when to
//! stop belong to Core, which also owns the requests and the cookies.

use scraper::{ElementRef, Html, Selector};

/// A form as a browser would submit it: the action resolved against the page
/// and the `name=value` pairs of its controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Form {
    pub action: String,
    fields: Vec<(String, String)>,
}

impl Form {
    pub fn fields(&self) -> &[(String, String)] {
        &self.fields
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Replaces the value of `name`, or adds it when the page has no such
    /// control.
    pub fn set(&mut self, name: &str, value: &str) {
        match self.fields.iter_mut().find(|(key, _)| key == name) {
            Some((_, slot)) => *slot = value.to_owned(),
            None => self.fields.push((name.to_owned(), value.to_owned())),
        }
    }
}

/// What an Extic page asks for next.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SciencePage {
    /// The ID and password form (`form#login`).
    Credentials(Form),
    /// The App one-time password form (`form#totp-form`). Choosing App OTP on
    /// the method picker only reveals this form, so it is the same step.
    Totp(Form),
    /// The account can only receive its code by email: a person must act.
    EmailOtp,
    /// A SAML hand-off the page itself would auto-submit.
    Saml(Form),
    /// A page after sign-in, or one this reader does not know.
    Other,
}

/// Reads an Extic page fetched from `base`.
pub fn science(html: &str, base: &str) -> SciencePage {
    let document = Html::parse_document(html);
    // SAML hand-off pages auto-submit a form on load; a client sends it too.
    if let Some(form) = select(&document, "form").into_iter().find(|form| {
        input(*form, "SAMLResponse").is_some() || input(*form, "SAMLRequest").is_some()
    }) {
        return SciencePage::Saml(form_at(form, base));
    }
    if let Some(form) = select_one(&document, "form#login") {
        if input(form, "identifier").is_some() && input(form, "password").is_some() {
            return SciencePage::Credentials(form_at(form, base));
        }
    }
    if let Some(form) = select_one(&document, "form#totp-form") {
        return SciencePage::Totp(form_at(form, base));
    }
    if !select(&document, "#emailotp-form, #emailotp-form-wrapper").is_empty() {
        return SciencePage::EmailOtp;
    }
    SciencePage::Other
}

/// One GetAccess step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TokyoPage {
    /// The agreement page, with the address of the password/Matrix login the
    /// page's own button opens.
    Agreement { password_url: String },
    /// The account and password form.
    Credentials(Form),
    /// The Matrix challenge: which cell each input holds.
    Matrix(MatrixChallenge),
    /// A page offering "Move to Other Authentication": the form already
    /// carries [`GRID`] in the select that chooses Matrix.
    MethodPicker(Form),
    /// The portal asks the person to choose a new password.
    PasswordChange,
    /// The account has a one-time password but no Matrix option here.
    NeedsOtp,
    /// A page after sign-in, or one this reader does not know.
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixChallenge {
    /// The form as the page sent it, before the cells are filled.
    pub form: Form,
    pub cells: Vec<MatrixCell>,
}

impl MatrixChallenge {
    /// Fills every requested cell from the 70-letter matrix, row-major
    /// (A1…J1, A2…J2, …, A7…J7 as on the editor).
    pub fn answer(&mut self, matrix: &str) -> bool {
        let cells: Vec<char> = matrix.chars().collect();
        if cells.len() != 70 {
            return false;
        }
        for cell in &self.cells {
            let Some(letter) = cells.get(cell.row * 10 + cell.column) else {
                return false;
            };
            self.form.set(&cell.name, &letter.to_string());
        }
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MatrixCell {
    pub name: String,
    /// 0-based row (1–7 on the page) and column (A–J).
    pub row: usize,
    pub column: usize,
}

/// Reads a GetAccess page fetched from `base`.
pub fn tokyo(html: &str, base: &str) -> TokyoPage {
    let document = Html::parse_document(html);
    let login = select_one(&document, "form[name=\"login\"]");
    if let Some(form) = login {
        if input(form, "usr_name").is_some() && input(form, "usr_password").is_some() {
            return TokyoPage::Credentials(form_at(form, base));
        }
        if let Some(challenge) = matrix(form, base) {
            return TokyoPage::Matrix(challenge);
        }
        if let Some(select) = grid_option(form) {
            let mut form = form_at(form, base);
            form.set(&select, GRID);
            return TokyoPage::MethodPicker(form);
        }
    }
    // Two password fields are the portal's "change your password" form; a
    // matrix challenge was recognised above.
    if select(&document, "input[type=\"password\"]").len() >= 2 {
        return TokyoPage::PasswordChange;
    }
    if login.is_some_and(|form| !select_in(form, "table#authentication").is_empty()) {
        return TokyoPage::NeedsOtp;
    }
    match password_login_url(&document) {
        Some(password_url) => TokyoPage::Agreement { password_url },
        None => TokyoPage::Other,
    }
}

/// One step on the way into the campus VPN (the F5 APM). The APM hands the
/// browser to GetAccess's proxy, which asks for the portal password again;
/// the account comes from the portal session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VpnPage {
    /// The proxy's form with a single password field and no account field.
    Password(Form),
    /// A page whose only form posts hidden fields onward: a hand-off a
    /// browser's script would submit on load. The APM's sets the form's
    /// address from its script (`document.<form>.action = unescape("…")`);
    /// that address is the action here.
    Relay(Form),
    /// Anything else: the APM itself, a GetAccess sign-in, a refusal.
    Other,
}

/// Reads a page met on the way into the campus VPN, fetched from `base`.
pub fn vpn(html: &str, base: &str) -> VpnPage {
    let document = Html::parse_document(html);
    for form in select(&document, "form") {
        let passwords = select_in(form, "input[type=\"password\"]");
        if passwords.len() == 1
            && passwords[0]
                .attr("name")
                .is_some_and(|name| name.eq_ignore_ascii_case("password"))
            && input(form, "usr_name").is_none()
        {
            return VpnPage::Password(form_at(form, base));
        }
    }
    let forms = select(&document, "form");
    if let [form] = forms.as_slice() {
        let method = form.attr("method").unwrap_or("get");
        let inputs = select_in(*form, "input");
        let hidden_only = !inputs.is_empty()
            && inputs.iter().all(|input| {
                input
                    .attr("type")
                    .is_some_and(|kind| kind.eq_ignore_ascii_case("hidden"))
            });
        if method.eq_ignore_ascii_case("post") && hidden_only {
            let mut relay = form_at(*form, base);
            if let Some(action) = scripted_action(&document, form.attr("name")) {
                relay.action = resolve(base, &action);
            }
            return VpnPage::Relay(relay);
        }
    }
    VpnPage::Other
}

/// The address a page's script gives form `name` before submitting it:
/// `document.<name>.action = unescape("…")` or `= "…"`.
fn scripted_action(document: &Html, name: Option<&str>) -> Option<String> {
    let name = name?;
    let needle = format!("{name}.action");
    select(document, "script").into_iter().find_map(|script| {
        let text: String = script.text().collect();
        let after = &text[text.find(&needle)? + needle.len()..];
        let value = after.trim_start().strip_prefix('=')?.trim_start();
        let value = value
            .strip_prefix("unescape(")
            .unwrap_or(value)
            .trim_start();
        let quote = value.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let rest = &value[1..];
        let address = &rest[..rest.find(quote)?];
        Some(percent_decode(address))
    })
}

/// `unescape`'s `%XX` decoding, enough for an address.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let escaped = (bytes[at] == b'%')
            .then(|| bytes.get(at + 1..at + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                at += 3;
            }
            None => {
                out.push(bytes[at]);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The value GetAccess gives the Matrix choice in "Move to Other
/// Authentication".
pub const GRID: &str = "GridAuthOption";

fn grid_option(form: ElementRef<'_>) -> Option<String> {
    select_in(form, "select").into_iter().find_map(|select| {
        let offers_grid = select_in(select, "option")
            .into_iter()
            .any(|option| option.attr("value") == Some(GRID));
        let name = select.attr("name").filter(|name| !name.is_empty())?;
        offers_grid.then(|| name.to_owned())
    })
}

fn matrix(login: ElementRef<'_>, base: &str) -> Option<MatrixChallenge> {
    let requested: Vec<(String, ElementRef<'_>)> = select_in(login, "input")
        .into_iter()
        .filter_map(|input| {
            let kind = input.attr("type").unwrap_or("text").to_ascii_lowercase();
            let name = input.attr("name").filter(|name| !name.is_empty())?;
            (matches!(kind.as_str(), "text" | "password")
                && input.attr("disabled").is_none()
                && !matches!(name, "usr_name" | "usr_password" | "q"))
            .then(|| (name.to_owned(), input))
        })
        .collect();
    if requested.is_empty() || requested.len() > 5 {
        return None;
    }
    let global = coordinates(&login.text().collect::<String>());
    let mut cells = Vec::with_capacity(requested.len());
    for (index, (name, input)) in requested.iter().enumerate() {
        let row = input
            .ancestors()
            .filter_map(ElementRef::wrap)
            .find(|element| element.value().name() == "tr")
            .map(|row| coordinates(&row.text().collect::<String>()))
            .unwrap_or_default();
        // A row naming exactly one cell is the reliable case; otherwise the
        // page lists the requested cells in order, as the login script reads
        // them.
        let cell = match row.as_slice() {
            [(column, row)] => Some((*column, *row)),
            _ => global
                .len()
                .checked_sub(requested.len())
                .and_then(|first| global.get(first + index))
                .copied(),
        };
        let (column, row) = cell?;
        cells.push(MatrixCell {
            name: name.clone(),
            row,
            column,
        });
    }
    Some(MatrixChallenge {
        form: form_at(login, base),
        cells,
    })
}

/// `[E,6]`-style cell references, as (column 0–9, row 0–6).
fn coordinates(text: &str) -> Vec<(usize, usize)> {
    static PATTERN: std::sync::LazyLock<Option<regex::Regex>> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\[\s*([A-Ja-j])\s*,\s*([1-7])\s*\]").ok());
    let Some(pattern) = PATTERN.as_ref() else {
        return Vec::new();
    };
    pattern
        .captures_iter(text)
        .filter_map(|capture| {
            let column = capture.get(1)?.as_str().to_ascii_uppercase();
            let row = capture.get(2)?.as_str().parse::<usize>().ok()?;
            Some((
                usize::from(column.as_bytes().first()? - b'A'),
                row.checked_sub(1)?,
            ))
        })
        .collect()
}

fn input<'a>(form: ElementRef<'a>, name: &str) -> Option<ElementRef<'a>> {
    select_in(form, "input").into_iter().find(|input| {
        input
            .attr("name")
            .is_some_and(|value| value.eq_ignore_ascii_case(name))
    })
}

fn form_at(form: ElementRef<'_>, base: &str) -> Form {
    let action = form.attr("action").unwrap_or("").trim();
    Form {
        action: resolve(base, action),
        fields: fields_of(form),
    }
}

fn resolve(base: &str, action: &str) -> String {
    match url::Url::parse(base) {
        Ok(mut base) => {
            base.set_fragment(None);
            base.join(action).unwrap_or(base).to_string()
        }
        Err(_) => action.to_owned(),
    }
}

/// The pairs a browser sends for this form: visible and hidden inputs,
/// checked checkboxes, each select's selected option (or its first), and
/// textareas. A submit control is included, as a click would include it.
fn fields_of(form: ElementRef<'_>) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    for element in form.descendent_elements() {
        if element.attr("disabled").is_some() {
            continue;
        }
        match element.value().name() {
            "input" => {
                let Some(name) = element.attr("name").filter(|name| !name.is_empty()) else {
                    continue;
                };
                let kind = element.attr("type").unwrap_or("text").to_ascii_lowercase();
                let value = element.attr("value").unwrap_or("");
                match kind.as_str() {
                    "button" | "reset" | "file" => {}
                    "checkbox" | "radio" => {
                        if element.attr("checked").is_some() {
                            fields.push((name.to_owned(), value.to_owned()));
                        }
                    }
                    _ => fields.push((name.to_owned(), value.to_owned())),
                }
            }
            "select" => {
                let Some(name) = element.attr("name").filter(|name| !name.is_empty()) else {
                    continue;
                };
                let options = select_in(element, "option");
                let chosen = options
                    .iter()
                    .rfind(|option| option.attr("selected").is_some())
                    .or_else(|| options.first());
                if let Some(option) = chosen {
                    let value = option
                        .attr("value")
                        .map(str::to_owned)
                        .unwrap_or_else(|| option.text().collect::<String>().trim().to_owned());
                    fields.push((name.to_owned(), value));
                }
            }
            "textarea" => {
                let Some(name) = element.attr("name").filter(|name| !name.is_empty()) else {
                    continue;
                };
                fields.push((name.to_owned(), element.text().collect()));
            }
            _ => {}
        }
    }
    fields
}

/// The first address in the page that opens the password/Matrix login. The
/// agreement page puts it in the button's `onclick`, and its parameters are
/// what the portal needs back.
fn password_login_url(document: &Html) -> Option<String> {
    for element in document.root_element().descendent_elements() {
        for attribute in ["onclick", "href"] {
            let Some(value) = element.attr(attribute) else {
                continue;
            };
            if let Some(url) = url_mentioning(value, "template=userpass_key") {
                return Some(unescape_ampersands(url));
            }
        }
    }
    None
}

fn url_mentioning<'a>(attribute: &'a str, needle: &str) -> Option<&'a str> {
    attribute
        .split(['"', '\''])
        .find(|part| part.to_ascii_lowercase().contains(needle))
}

fn unescape_ampersands(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#38;", "&")
        .replace("&#x26;", "&")
}

fn select<'a>(root: &'a Html, raw: &str) -> Vec<ElementRef<'a>> {
    Selector::parse(raw)
        .map(|selector| root.select(&selector).collect())
        .unwrap_or_default()
}

fn select_one<'a>(root: &'a Html, raw: &str) -> Option<ElementRef<'a>> {
    Selector::parse(raw)
        .ok()
        .and_then(|selector| root.select(&selector).next())
}

fn select_in<'a>(root: ElementRef<'a>, raw: &str) -> Vec<ElementRef<'a>> {
    Selector::parse(raw)
        .map(|selector| root.select(&selector).collect())
        .unwrap_or_default()
}

#[cfg(test)]
#[path = "tests/login.rs"]
mod tests;

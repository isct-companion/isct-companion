use super::*;
use crate::ports::vault_key;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

const ACCOUNT: &str = "school-session-tokyo-tech";

/// A vault that refuses what its items cannot hold, as Credential Manager
/// does past 2560 bytes of UTF-16, and records the accounts it was asked for.
#[derive(Clone)]
pub(crate) struct LimitedVault {
    pub(crate) items: Arc<Mutex<HashMap<String, String>>>,
    limit: Option<usize>,
    pub(crate) touched: Arc<Mutex<Vec<String>>>,
}

impl LimitedVault {
    pub(crate) fn with_limit(limit: Option<usize>) -> Self {
        Self {
            items: Arc::default(),
            limit,
            touched: Arc::default(),
        }
    }

    pub(crate) fn parts(&self) -> usize {
        let items = self.items.lock().unwrap();
        items.keys().filter(|key| key.contains(".part-")).count()
    }

    fn touch(&self, account: &str) {
        self.touched.lock().unwrap().push(account.into());
    }
}

impl SecretVault for LimitedVault {
    fn store(&self, account: &str, secret: &str) -> Result<(), SecretError> {
        self.touch(account);
        // Every account it holds has a place in the platform keychain.
        vault_key("app", account)?;
        if self
            .limit
            .is_some_and(|limit| secret.encode_utf16().count() > limit)
        {
            return Err(SecretError::Access("too long for one item".into()));
        }
        self.items
            .lock()
            .unwrap()
            .insert(account.into(), secret.into());
        Ok(())
    }

    fn load(&self, account: &str) -> Result<Option<String>, SecretError> {
        self.touch(account);
        Ok(self.items.lock().unwrap().get(account).cloned())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        self.touch(account);
        self.items.lock().unwrap().remove(account);
        Ok(())
    }

    fn item_limit(&self) -> Option<usize> {
        self.limit
    }
}

// A platform without a limit never meets this module: however long the secret,
// it is one item, and nothing asks the vault about parts. On the macOS login
// keychain each extra read could ask the person for their password.
#[test]
fn a_vault_without_a_limit_keeps_one_item_and_is_never_asked_about_parts() {
    let vault = LimitedVault::with_limit(None);
    let secret = "s".repeat(100_000);
    store(&vault, ACCOUNT, &secret).unwrap();
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some(&*secret));
    delete(&vault, ACCOUNT).unwrap();
    assert_eq!(*vault.touched.lock().unwrap(), [ACCOUNT, ACCOUNT, ACCOUNT]);
    assert!(vault.items.lock().unwrap().is_empty());
}

// A cut never falls inside a surrogate pair, whose two units count against
// the limit, and the longest secret one item holds stays one item.
#[test]
fn a_cut_respects_utf16_units_and_a_secret_that_fits_is_not_split() {
    let vault = LimitedVault::with_limit(Some(1280));
    let exact = "s".repeat(1280);
    store(&vault, ACCOUNT, &exact).unwrap();
    assert_eq!(vault.parts(), 0);
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some(&*exact));

    let wide = format!("s{}", "𝒳".repeat(2000));
    store(&vault, ACCOUNT, &wide).unwrap();
    assert_eq!(vault.parts(), 4);
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some(&*wide));
}

// An item written before parts existed has no manifest: it reads as itself,
// and the next write replaces it.
#[test]
fn an_item_an_earlier_build_wrote_stays_readable() {
    let vault = LimitedVault::with_limit(Some(1280));
    let earlier = r#"{"version":2,"identity":[],"sites":{}}"#;
    vault.store(ACCOUNT, earlier).unwrap();
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some(earlier));

    let secret = "s".repeat(3000);
    store(&vault, ACCOUNT, &secret).unwrap();
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some(&*secret));
}

// A write cut short, or parts of two writes, read as no session rather than
// as a mix: the school would refuse it, or worse, take it for someone's.
#[test]
fn a_missing_or_foreign_part_reads_as_nothing() {
    let vault = LimitedVault::with_limit(Some(1280));
    store(&vault, ACCOUNT, &"a".repeat(3000)).unwrap();

    let second = part_account(ACCOUNT, 1);
    let kept = vault.load(&second).unwrap().unwrap();
    vault.store(&second, &"b".repeat(kept.len())).unwrap();
    assert_eq!(load(&vault, ACCOUNT).unwrap(), None);

    vault.delete(&second).unwrap();
    assert_eq!(load(&vault, ACCOUNT).unwrap(), None);
}

// A write that needs fewer items leaves none of the longer secret behind, and
// a sign-out also takes a part no count names any more.
#[test]
fn shrinking_and_deleting_leave_no_part_behind() {
    let vault = LimitedVault::with_limit(Some(1280));
    store(&vault, ACCOUNT, &"a".repeat(5000)).unwrap();
    assert_eq!(vault.parts(), 4);
    store(&vault, ACCOUNT, &"b".repeat(2000)).unwrap();
    assert_eq!(vault.parts(), 2);
    assert_eq!(
        load(&vault, ACCOUNT).unwrap().as_deref(),
        Some(&*"b".repeat(2000))
    );
    store(&vault, ACCOUNT, "short").unwrap();
    assert_eq!(vault.parts(), 0);
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some("short"));

    store(&vault, ACCOUNT, &"c".repeat(3000)).unwrap();
    vault
        .store(&part_account(ACCOUNT, MAX_PARTS - 1), "left behind")
        .unwrap();
    delete(&vault, ACCOUNT).unwrap();
    assert!(vault.items.lock().unwrap().is_empty());
}

// Past the bound a write is refused before it changes anything, so the
// session saved before it stays as it was.
#[test]
fn a_secret_past_the_bound_is_refused_and_changes_nothing() {
    let vault = LimitedVault::with_limit(Some(1280));
    store(&vault, ACCOUNT, "kept").unwrap();
    assert_eq!(
        store(&vault, ACCOUNT, &"s".repeat(1280 * MAX_PARTS + 1)),
        Err(SecretError::InvalidPayload)
    );
    assert_eq!(load(&vault, ACCOUNT).unwrap().as_deref(), Some("kept"));
    assert_eq!(vault.parts(), 0);
}

// The keychain's allowlist opens for the parts of a saved session and nothing
// else: not another account's, and not an index a sweep would miss.
#[test]
fn only_a_saved_sessions_parts_have_a_place_in_the_keychain() {
    for allowed in [
        "school-session-science.part-0",
        "school-session-tokyo-tech.part-63",
    ] {
        assert_eq!(
            vault_key("app", allowed).unwrap(),
            format!("app.passwords.{allowed}")
        );
    }
    for refused in [
        "school-accounts.part-0",
        "school-session-science.part-64",
        "school-session-science.part-01",
        "school-session-science.part-x",
        "school-session-science.part-",
    ] {
        assert_eq!(
            vault_key("app", refused),
            Err(SecretError::InvalidAccount),
            "{refused}"
        );
    }
}

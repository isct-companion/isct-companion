//! One secret across several vault items, for a vault whose items are smaller
//! than it (`SecretVault::item_limit`). Every part stays in the OS vault,
//! protected as any other item; nothing of it reaches a file.
//!
//! A secret that fits stays one item under its own account, as on a vault
//! without a limit. A longer one goes to `<account>.part-<n>` items, and the
//! account's own item then holds their count and digest, written last. So an
//! item an earlier build wrote reads as itself, and a write cut short reads
//! as nothing, never as pieces of two secrets: only the saved session is kept
//! this way, and the next sign-in replaces it.

use crate::ports::{SecretError, SecretVault};
use sha2::{Digest, Sha256};

const MANIFEST: &str = "vault-parts/1";
/// Bounds what a sign-out has to sweep. At Credential Manager's limit this is
/// about 80 KB, past the 64 KiB a desktop vault accepts in one item.
const MAX_PARTS: usize = 64;

fn part_account(account: &str, index: usize) -> String {
    format!("{account}.part-{index}")
}

/// Whether `account` is a part of an item kept this way.
pub(crate) fn is_part(account: &str) -> bool {
    account.split_once(".part-").is_some_and(|(item, index)| {
        matches!(item, "school-session-science" | "school-session-tokyo-tech")
            && index
                .parse::<usize>()
                .is_ok_and(|parsed| parsed < MAX_PARTS && parsed.to_string() == index)
    })
}

fn digest(secret: &str) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(secret.as_bytes()))
}

/// The part count and digest a stored item names; `None` for an item that
/// holds its secret itself.
fn manifest(stored: &str) -> Option<(usize, &str)> {
    let mut fields = stored.strip_prefix(MANIFEST)?.split_ascii_whitespace();
    let count = fields.next()?.parse().ok()?;
    let digest = fields.next()?;
    (count <= MAX_PARTS && fields.next().is_none()).then_some((count, digest))
}

/// `secret` cut at characters into pieces of at most `limit` UTF-16 units.
fn split(secret: &str, limit: usize) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut start, mut units) = (0, 0);
    for (at, character) in secret.char_indices() {
        if units + character.len_utf16() > limit {
            parts.push(&secret[start..at]);
            (start, units) = (at, 0);
        }
        units += character.len_utf16();
    }
    parts.push(&secret[start..]);
    parts
}

pub(crate) fn store(
    vault: &dyn SecretVault,
    account: &str,
    secret: &str,
) -> Result<(), SecretError> {
    let Some(limit) = vault.item_limit() else {
        return vault.store(account, secret);
    };
    let parts = split(secret, limit);
    if parts.len() > MAX_PARTS {
        return Err(SecretError::InvalidPayload);
    }
    let previous = vault
        .load(account)?
        .as_deref()
        .and_then(manifest)
        .map_or(0, |(count, _)| count);
    let kept = if parts.len() == 1 {
        vault.store(account, secret)?;
        0
    } else {
        for (index, part) in parts.iter().enumerate() {
            vault.store(&part_account(account, index), part)?;
        }
        let record = format!("{MANIFEST} {} {}", parts.len(), digest(secret));
        vault.store(account, &record)?;
        parts.len()
    };
    for index in kept..previous {
        // Nothing reads a part past the count; a sign-out sweeps what stays.
        let _ = vault.delete(&part_account(account, index));
    }
    Ok(())
}

pub(crate) fn load(vault: &dyn SecretVault, account: &str) -> Result<Option<String>, SecretError> {
    let Some(stored) = vault.load(account)? else {
        return Ok(None);
    };
    if vault.item_limit().is_none() {
        return Ok(Some(stored));
    }
    let Some((count, expected)) = manifest(&stored) else {
        return Ok(Some(stored));
    };
    let mut secret = String::new();
    for index in 0..count {
        let Some(part) = vault.load(&part_account(account, index))? else {
            return Ok(None);
        };
        secret.push_str(&part);
    }
    Ok((digest(&secret) == expected).then_some(secret))
}

/// Removes the account's own item first, so nothing reads a partial set, then
/// every part any earlier write may have left, whatever the count said.
pub(crate) fn delete(vault: &dyn SecretVault, account: &str) -> Result<(), SecretError> {
    vault.delete(account)?;
    if vault.item_limit().is_none() {
        return Ok(());
    }
    for index in 0..MAX_PARTS {
        vault.delete(&part_account(account, index))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/secret_parts.rs"]
pub(crate) mod tests;

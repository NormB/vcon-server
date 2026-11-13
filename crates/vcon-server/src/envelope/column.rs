//! The codec of the PostgreSQL catalog's sensitive columns. A stored
//! value is sealed under a key of its own, bound to the tenant, table,
//! column and row, so a value copied to another row, column or tenant does
//! not open; a value SQL must find or compare is kept as a MAC under
//! another key derived from the same catalog key. Both keys are derived
//! from the catalog key, so the same rotation that makes a local catalog
//! unreadable on old copies does so for sealed columns and MACs.
//!
//! The local backend keeps SQLCipher and does not use this.

use std::fmt;

use aws_lc_rs::hmac;

use super::locked::LockedPage;
use super::{
    FORMAT, KEY_LEN, Key, KeyError, LABEL, NONCE_LEN, TAG_LEN, gcm_open, gcm_seal, hkdf_sha256,
};
use crate::ids::TenantId;

/// The HKDF info that makes the key sealed columns are derived from.
const COLUMN_LABEL: &[u8] = b"vcon-server/v1 columns";

/// The HKDF info that makes the key of every MAC.
const INDEX_LABEL: &[u8] = b"vcon-server/v1 index";

/// The kind byte in a sealed column's associated data: 1 and 2 are
/// [`super::Context`]'s, 3 is a column value.
const COLUMN_KIND: u8 = 3;

/// The tenant field of a registry column: the registry belongs to no
/// tenant, and no tenant id is all zeros (ids are random, 128 bits).
pub const REGISTRY: TenantId = TenantId([0; 16]);

/// How many bytes a sealed column value holds beyond its plaintext: the
/// format byte, the nonce and the tag.
pub const SEALED_COLUMN_OVERHEAD: usize = 1 + NONCE_LEN + TAG_LEN;

/// The two keys derived from a catalog key at open, held in locked pages
/// and never stored.
pub struct ColumnKeys {
    column: Key,
    index: Key,
}

impl fmt::Debug for ColumnKeys {
    /// Shows nothing of the keys, not even their ids.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColumnKeys").finish_non_exhaustive()
    }
}

/// Where a sealed value belongs. The tenant is [`REGISTRY`] for a registry
/// column, the table and column are the SQL names, and the row key is what
/// identifies the row within its tenant (such as a version id, or a uuid's
/// MAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnContext<'a> {
    /// The tenant the row belongs to.
    pub tenant: TenantId,
    /// The table.
    pub table: &'a str,
    /// The column.
    pub column: &'a str,
    /// The row's key.
    pub row: &'a [u8],
}

impl ColumnKeys {
    /// The column and index keys of the catalog under `catalog`.
    ///
    /// # Errors
    ///
    /// [`KeyError::Memory`] if a key's page cannot be locked.
    pub fn derive(catalog: &Key) -> Result<ColumnKeys, KeyError> {
        Ok(ColumnKeys {
            column: derive_key(catalog.bytes(), COLUMN_LABEL)?,
            index: derive_key(catalog.bytes(), INDEX_LABEL)?,
        })
    }
}

/// Appends `bytes` after its length, four bytes big-endian, so that two
/// different sequences of fields cannot give the same bytes.
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).expect("a table, column or row key is under 4 GiB");
    out.extend(len.to_be_bytes());
    out.extend(bytes);
}

/// A key of 32 bytes made by HKDF-SHA256 of `parent` with no salt and the
/// info `info`, in a locked page of its own.
fn derive_key(parent: &[u8], info: &[u8]) -> Result<Key, KeyError> {
    let mut page = LockedPage::new().map_err(KeyError::Memory)?;
    hkdf_sha256(&[], parent, info, &mut page.bytes_mut()[..KEY_LEN]);
    Ok(Key { page })
}

/// The HKDF info of a row's subkey: the table, the column and the row key,
/// each after its length.
fn subkey_info(ctx: &ColumnContext<'_>) -> Vec<u8> {
    let mut info = Vec::new();
    field(&mut info, ctx.table.as_bytes());
    field(&mut info, ctx.column.as_bytes());
    field(&mut info, ctx.row);
    info
}

/// The associated data of a sealed value: the label, the blob's format
/// byte, the kind byte, the tenant, then the table, the column and the row
/// key, each after its length.
fn associated_data(ctx: &ColumnContext<'_>, format: u8) -> Vec<u8> {
    let mut ad = LABEL.to_vec();
    ad.push(format);
    ad.push(COLUMN_KIND);
    ad.extend(ctx.tenant.0);
    ad.extend(subkey_info(ctx));
    ad
}

/// The subkey of one row's column, made in a locked page of its own that is
/// zeroed when dropped.
fn subkey(keys: &ColumnKeys, ctx: &ColumnContext<'_>) -> Result<Key, KeyError> {
    derive_key(keys.column.bytes(), &subkey_info(ctx))
}

/// Seals `plaintext` for the place `ctx` names, with a random nonce. The
/// blob is the format byte, the nonce, the ciphertext and the tag.
///
/// # Errors
///
/// [`KeyError::Memory`] if the subkey's page cannot be locked.
///
/// # Panics
///
/// Never: AWS-LC aborts the process rather than fail to give random bytes.
pub fn seal_column(
    keys: &ColumnKeys,
    ctx: &ColumnContext<'_>,
    plaintext: &[u8],
) -> Result<Vec<u8>, KeyError> {
    // Drawn whole from the system generator, as `seal` draws its nonce.
    let nonce: [u8; NONCE_LEN] = aws_lc_rs::rand::generate(&aws_lc_rs::rand::SystemRandom::new())
        .expect("AWS-LC aborts instead of failing")
        .expose();
    seal_column_with_nonce(keys, ctx, nonce, plaintext)
}

fn seal_column_with_nonce(
    keys: &ColumnKeys,
    ctx: &ColumnContext<'_>,
    nonce: [u8; NONCE_LEN],
    plaintext: &[u8],
) -> Result<Vec<u8>, KeyError> {
    let key = subkey(keys, ctx)?;
    let mut blob = Vec::with_capacity(SEALED_COLUMN_OVERHEAD + plaintext.len());
    blob.push(FORMAT);
    blob.extend(nonce);
    blob.extend(plaintext);
    let ad = associated_data(ctx, FORMAT);
    let tag = gcm_seal(key.bytes(), nonce, &ad, &mut blob[1 + NONCE_LEN..]);
    blob.extend(tag);
    Ok(blob)
}

/// Opens a value [`seal_column`] made for the same keys and context; any
/// change to the blob, the keys or the context is an error, never partial
/// plaintext.
///
/// # Errors
///
/// [`KeyError::Damaged`] for a blob that does not open, and
/// [`KeyError::Memory`] if the subkey's page cannot be locked.
pub fn open_column(
    keys: &ColumnKeys,
    ctx: &ColumnContext<'_>,
    blob: &[u8],
) -> Result<Vec<u8>, KeyError> {
    let (&format, rest) = blob.split_first().ok_or(KeyError::Damaged)?;
    let (nonce, sealed) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(KeyError::Damaged)?;
    if sealed.len() < TAG_LEN {
        return Err(KeyError::Damaged);
    }
    let key = subkey(keys, ctx)?;
    let mut text = sealed.to_vec();
    let len = gcm_open(
        key.bytes(),
        *nonce,
        &associated_data(ctx, format),
        &mut text,
    )
    .map_err(|_| KeyError::Damaged)?;
    text.truncate(len);
    Ok(text)
}

/// Seals `key` as the column value of the place `ctx` names: a blob like
/// [`seal_column`]'s, whose plaintext is the key.
///
/// # Errors
///
/// [`KeyError::Memory`] if the subkey's page cannot be locked.
pub fn wrap_key(
    keys: &ColumnKeys,
    ctx: &ColumnContext<'_>,
    key: &Key,
) -> Result<Vec<u8>, KeyError> {
    seal_column(keys, ctx, key.bytes())
}

/// Opens a key [`wrap_key`] made for the same keys and context. It is
/// opened straight into the new key's own locked page, so the plain key
/// is never anywhere else; on any error that page is dropped, and so
/// zeroed.
///
/// # Errors
///
/// [`KeyError::Damaged`] for a blob that does not open under these keys
/// and this context to exactly 32 bytes, and [`KeyError::Memory`] if a
/// page cannot be locked.
pub fn unwrap_key(
    keys: &ColumnKeys,
    ctx: &ColumnContext<'_>,
    wrapped: &[u8],
) -> Result<Key, KeyError> {
    let (&format, rest) = wrapped.split_first().ok_or(KeyError::Damaged)?;
    let (nonce, sealed) = rest
        .split_first_chunk::<NONCE_LEN>()
        .ok_or(KeyError::Damaged)?;
    // Only a key's ciphertext and its tag fit: any other length is not a
    // key, and is refused before anything is opened.
    if sealed.len() != KEY_LEN + TAG_LEN {
        return Err(KeyError::Damaged);
    }
    let subkey = subkey(keys, ctx)?;
    let mut page = LockedPage::new().map_err(KeyError::Memory)?;
    let text = &mut page.bytes_mut()[..KEY_LEN + TAG_LEN];
    text.copy_from_slice(sealed);
    match gcm_open(subkey.bytes(), *nonce, &associated_data(ctx, format), text) {
        Ok(KEY_LEN) => Ok(Key { page }),
        _ => Err(KeyError::Damaged),
    }
}

/// HMAC-SHA256 of `value`'s fixed bytes under the index key: equal values
/// give equal MACs, which SQL needs to find them, and which is all a MAC
/// shows.
#[must_use]
pub fn mac(keys: &ColumnKeys, value: &[u8]) -> [u8; 32] {
    hmac_sha256(keys.index.bytes(), value)
}

fn hmac_sha256(key: &[u8], value: &[u8]) -> [u8; 32] {
    let key = hmac::Key::new(hmac::HMAC_SHA256, key);
    hmac::sign(&key, value)
        .as_ref()
        .try_into()
        .expect("HMAC-SHA256 is 32 bytes")
}

#[cfg(test)]
mod key_tests;
#[cfg(test)]
mod tests;

//! A tenant key's rotation, as the Store sees it: every version's data
//! key, wrapped under the old tenant key, is rewrapped under the new one.
//! Bodies are never re-encrypted.

use super::{Store, StoreError, commit};
use crate::envelope::{self, Context, Key, WrapLimit};

impl Store {
    /// Rewraps up to `limit` of the data keys still wrapped under this
    /// Store's tenant key so they are wrapped under `new`, in one
    /// transaction, and returns how many it rewrapped; 0 means none is left.
    /// Each rewrap is counted against `new`, and refused at its wrap limit.
    pub fn rewrap(&self, new: &Key, limit: usize) -> Result<usize, StoreError> {
        let old = self.tenant_key.id();
        let mut tx = self.catalog.immediate()?;
        let rows = tx.wrapped_under(old, limit)?;
        let after = tx.wraps(new.id())?.saturating_add(rows.len() as u64);
        if !rows.is_empty() && envelope::wrap_limit(after - 1) == WrapLimit::Refuse {
            return Err(StoreError::LimitReached);
        }
        for row in &rows {
            let ctx = Context::Version {
                tenant: self.tenant,
                uuid: row.uuid,
                version: row.version,
            };
            let rewrapped = envelope::rewrap(&self.tenant_key, new, &ctx, &row.wrapped)
                .map_err(|_| StoreError::Damaged)?;
            tx.set_wrapped(row.version, new.id(), &rewrapped)?;
            tx.add_wrap(new.id())?;
        }
        commit(tx)?;
        Ok(rows.len())
    }

    /// How many version rows name a key other than this Store's tenant key:
    /// a rotation stopped part way leaves some.
    pub fn rows_under_other_keys(&self) -> Result<u64, StoreError> {
        self.catalog
            .snapshot()?
            .count_not_under(self.tenant_key.id())
    }
}

#[cfg(test)]
mod rekey_tests {
    use super::super::testing::vcon;
    use super::super::tests::{Fixture, fixture};
    use super::*;
    use crate::envelope::{self, Context};
    use crate::ids::VersionId;
    use crate::store::Created;

    /// Every version row: its id, uuid, wrapping key id and wrapped key.
    fn rows(f: &Fixture) -> Vec<(VersionId, uuid::Uuid, Vec<u8>, Vec<u8>)> {
        f.store
            .conn()
            .prepare(
                "SELECT version, uuid, tenant_key_id, wrapped_key FROM version ORDER BY version",
            )
            .expect("prepare")
            .query_map([], |r| {
                Ok((
                    VersionId(r.get(0)?),
                    uuid::Uuid::from_bytes(r.get(1)?),
                    r.get(2)?,
                    r.get(3)?,
                ))
            })
            .expect("query")
            .collect::<Result<_, _>>()
            .expect("rows")
    }

    fn store_five(f: &Fixture) {
        for n in 0..5 {
            let uuid = format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}");
            let (found, body) = vcon(&uuid, "kept");
            assert!(matches!(
                f.store.create(&f.caller, &found, &body),
                Ok(Created::New(_))
            ));
        }
    }

    fn wraps_of(f: &Fixture, key: &Key) -> i64 {
        f.store
            .conn()
            .query_row(
                "SELECT coalesce(sum(wraps), 0) FROM key_use WHERE tenant_key_id = ?1",
                [key.id().bytes()],
                |r| r.get(0),
            )
            .expect("wraps")
    }

    #[test]
    fn rewrap_moves_every_data_key_to_the_new_key_in_batches() {
        let f = fixture();
        store_five(&f);
        let (old, new) = (f.tenant_key(), Key::generate().expect("key"));
        assert_eq!(f.store.rewrap(&new, 2), Ok(2));
        assert_eq!(f.store.rewrap(&new, 2), Ok(2));
        assert_eq!(f.store.rewrap(&new, 2), Ok(1));
        assert_eq!(f.store.rewrap(&new, 2), Ok(0), "none left");
        let rows = rows(&f);
        assert_eq!(rows.len(), 5);
        for (version, uuid, key_id, wrapped) in rows {
            let ctx = Context::Version {
                tenant: f.tenant(),
                uuid,
                version,
            };
            assert_eq!(
                key_id,
                new.id().bytes().to_vec(),
                "the row names the new key"
            );
            assert!(
                envelope::unwrap(&new, &ctx, &wrapped).is_ok(),
                "opens under the new key"
            );
            assert!(
                envelope::unwrap(&old, &ctx, &wrapped).is_err(),
                "not under the old one"
            );
        }
        assert_eq!(
            wraps_of(&f, &new),
            5,
            "each rewrap is counted against the new key"
        );
    }

    #[test]
    fn a_rewrap_cut_short_leaves_every_row_opening_under_one_key_or_the_other() {
        let f = fixture();
        store_five(&f);
        let (old, new) = (f.tenant_key(), Key::generate().expect("key"));
        assert_eq!(f.store.rewrap(&new, 3), Ok(3));
        let (mut under_old, mut under_new) = (0, 0);
        for (version, uuid, key_id, wrapped) in rows(&f) {
            let ctx = Context::Version {
                tenant: f.tenant(),
                uuid,
                version,
            };
            let (key, count) = if key_id == new.id().bytes().to_vec() {
                (&new, &mut under_new)
            } else {
                (&old, &mut under_old)
            };
            assert!(envelope::unwrap(key, &ctx, &wrapped).is_ok(), "{version}");
            *count += 1;
        }
        assert_eq!((under_old, under_new), (2, 3));
    }

    #[test]
    fn a_new_key_at_its_wrap_limit_is_refused_and_nothing_changes() {
        let f = fixture();
        store_five(&f);
        let new = Key::generate().expect("key");
        f.store
            .conn()
            .execute(
                "INSERT INTO key_use (tenant_key_id, wraps) VALUES (?1, ?2)",
                rusqlite::params![new.id().bytes(), 1_i64 << 32],
            )
            .expect("at the limit");
        let before = rows(&f);
        assert_eq!(f.store.rewrap(&new, 10), Err(StoreError::LimitReached));
        assert_eq!(rows(&f), before);
    }
}

//! What `export parquet` reads of one tenant's Store: every kept version in
//! one snapshot of the catalog, then each version's object, outside any
//! transaction so the export never holds back the server's checkpoints.
//! Reading changes nothing.

use uuid::Uuid;

use super::packed::Unread;
use super::{Store, StoreError, key_failed, read_failed};
use crate::backend::ObjectError;
use crate::envelope::{self, Context};
use crate::ids::{TenantId, VersionId};

/// A kept version as the export lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeptVersion {
    pub version: VersionId,
    pub uuid: Uuid,
    /// The form's name as stored: unsigned, signed or encrypted.
    pub form: String,
    pub gzip: bool,
    /// The body's size in bytes, as received.
    pub size: i64,
    /// Nanoseconds since the epoch.
    pub stored_at: i64,
    /// Whether it is its uuid's current version and the uuid is not
    /// deleted: what a read returns.
    pub live: bool,
    /// The data key, wrapped under the tenant key.
    pub wrapped: Vec<u8>,
    /// Where its sealed blob is when it is packed; None: in its object.
    pub(crate) place: Option<crate::backend::Packed>,
}

impl Store {
    /// The tenant this Store holds.
    pub fn tenant(&self) -> TenantId {
        self.tenant
    }

    /// Every kept version, live or hidden, stored at or after `since` and
    /// before `until` (nanoseconds since the epoch), by store time and then
    /// version id, all from one snapshot of the catalog.
    pub fn kept_versions(
        &self,
        since: Option<i64>,
        until: Option<i64>,
    ) -> Result<Vec<KeptVersion>, StoreError> {
        let rows = self.catalog.snapshot()?.exported(since, until)?;
        Ok(rows
            .into_iter()
            .map(|row| KeptVersion {
                version: row.version,
                uuid: row.uuid,
                form: row.form,
                gzip: row.gzip,
                size: row.size,
                stored_at: row.stored_at,
                live: row.live,
                wrapped: row.wrapped,
                place: row.place,
            })
            .collect())
    }

    /// The sealed blob of a version `kept_versions` listed, from its object
    /// or its pack file, byte for byte. None when the version has been
    /// purged since: its row is gone and its blob with it. A blob missing
    /// while its row is still there is damage.
    pub fn sealed_of(&self, kept: &KeptVersion) -> Result<Option<Vec<u8>>, StoreError> {
        match self.sealed_blob(kept.version, kept.place.as_ref()) {
            Ok(sealed) => Ok(Some(sealed)),
            Err(Unread::Damaged) => Err(StoreError::Damaged),
            Err(Unread::Object(ObjectError::Missing)) => {
                // A purge removes the row first, then the object.
                let mut tx = self.catalog.snapshot()?;
                if tx.has_version(kept.version, kept.uuid)? {
                    Err(StoreError::Damaged)
                } else {
                    Ok(None)
                }
            }
            Err(Unread::Object(e @ ObjectError::Io(_))) => Err(read_failed(e)),
        }
    }

    /// The body `sealed`, the object of `kept`, holds: what `get` returns.
    pub fn open_sealed(&self, kept: &KeptVersion, sealed: &[u8]) -> Result<Vec<u8>, StoreError> {
        let ctx = Context::Version {
            tenant: self.tenant,
            uuid: kept.uuid,
            version: kept.version,
        };
        let data_key =
            envelope::unwrap(&self.tenant_key, &ctx, &kept.wrapped).map_err(key_failed)?;
        envelope::open(&data_key, &ctx, sealed).map_err(|_| StoreError::Damaged)
    }
}

#[cfg(test)]
mod tests {
    use time::Duration;

    use super::*;
    use crate::audit::Actor;
    use crate::clock::{Clock, nanos};
    use crate::fs::Fs;
    use crate::retention::Retention;
    use crate::store::Created;
    use crate::store::testing::vcon;
    use crate::store::tests::{Capture, Fixture, ReadsFail, fixture};

    const A: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
    const B: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc47";
    const C: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc48";
    const D: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc49";

    fn later(f: &Fixture, by: Duration) {
        f.clock.set(f.clock.now() + by);
    }

    fn create(f: &Fixture, uuid: &str, subject: &str) -> VersionId {
        let (found, body) = vcon(uuid, subject);
        match f.store.create(&f.caller, &found, &body).expect("create") {
            Created::New(v) => v,
            other => panic!("{other:?}"),
        }
    }

    /// A's first version replaced by its second, B stored, C deleted, D
    /// purged; a minute between each store. Returns A's two versions, B's
    /// and C's, in that order.
    fn stored(f: &Fixture) -> [VersionId; 4] {
        let a1 = create(f, A, "first");
        later(f, Duration::minutes(1));
        let (found, body) = vcon(A, "second");
        let a2 = f
            .store
            .replace(&f.caller, a1, &found, &body)
            .expect("replace");
        later(f, Duration::minutes(1));
        let b1 = create(f, B, "other");
        later(f, Duration::minutes(1));
        let c1 = create(f, C, "deleted");
        f.store
            .delete(&f.caller, C.parse().expect("uuid"), Retention::Years(7))
            .expect("delete");
        later(f, Duration::minutes(1));
        create(f, D, "purged");
        // A delete under a period already passed purges at once.
        f.store
            .delete(&f.caller, D.parse().expect("uuid"), Retention::Days(0))
            .expect("delete");
        assert_eq!(
            f.store.legal_get(&f.caller, D.parse().expect("uuid"), None),
            Err(StoreError::Gone),
            "D is purged"
        );
        later(f, Duration::minutes(1));
        [a1, a2, b1, c1]
    }

    #[test]
    fn every_kept_version_is_listed_once_and_no_purged_one() {
        let f = fixture();
        let [a1, a2, b1, c1] = stored(&f);
        let kept = f.store.kept_versions(None, None).expect("lists");
        let listed: Vec<(VersionId, bool)> = kept.iter().map(|k| (k.version, k.live)).collect();
        assert_eq!(
            listed,
            [(a1, false), (a2, true), (b1, true), (c1, false)],
            "in store order; only a current version of a live uuid is live"
        );
        let a = &kept[0];
        assert_eq!(a.uuid, A.parse::<Uuid>().expect("uuid"), "its uuid");
        assert_eq!(a.form, "unsigned", "its form");
        assert!(!a.gzip, "not gzipped");
        assert_eq!(a.size, vcon(A, "first").1.len() as i64, "the body's size");
        assert!(!a.wrapped.is_empty(), "its wrapped key");
        assert!(
            kept.windows(2).all(|w| w[0].stored_at < w[1].stored_at),
            "store times rise"
        );
    }

    /// `since` is inclusive and `until` exclusive, as for the list.
    #[test]
    fn the_window_takes_since_and_leaves_until() {
        let f = fixture();
        let [_, a2, b1, _] = stored(&f);
        let all = f.store.kept_versions(None, None).expect("lists");
        let at = |v: VersionId| all.iter().find(|k| k.version == v).expect("kept").stored_at;
        let window = f
            .store
            .kept_versions(Some(at(a2)), Some(at(b1) + 1))
            .expect("lists");
        let listed: Vec<VersionId> = window.iter().map(|k| k.version).collect();
        assert_eq!(listed, [a2, b1], "from A2's time to just after B1's");
        let none = f
            .store
            .kept_versions(Some(at(a2)), Some(at(a2)))
            .expect("lists");
        assert!(none.is_empty(), "until is left out: {none:?}");
        assert!(
            f.store
                .kept_versions(Some(nanos(f.clock.now())), None)
                .expect("lists")
                .is_empty(),
            "nothing stored since the last store"
        );
    }

    #[test]
    fn a_sealed_object_opens_to_the_body_get_returns() {
        let f = fixture();
        stored(&f);
        for kept in f.store.kept_versions(None, None).expect("lists") {
            let sealed = f.store.sealed_of(&kept).expect("reads").expect("kept");
            let body = f.store.open_sealed(&kept, &sealed).expect("opens");
            let read = f
                .store
                .legal_get(&f.caller, kept.uuid, Some(kept.version))
                .expect("legal read");
            assert_eq!(
                body, read.stored.body,
                "{}: the body as stored",
                kept.version
            );
            let mut other = kept.clone();
            other.version = VersionId([0xee; 16]);
            assert_eq!(
                f.store.open_sealed(&other, &sealed),
                Err(StoreError::Damaged),
                "bound to its version"
            );
        }
    }

    /// An object the storage fails to give is Io, logged by its kind as
    /// every read is: the export stops, and calls no version damaged.
    #[test]
    fn an_object_the_storage_fails_to_give_is_io_and_logged() {
        let mut f = fixture();
        stored(&f);
        let kept = f.store.kept_versions(None, None).expect("lists");
        f.store.objects = std::sync::Arc::new(ReadsFail {
            inner: f.store.objects.clone(),
            fail: || ObjectError::Io(std::io::Error::from_raw_os_error(libc::EIO)),
        });
        let capture = Capture::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish();
        let read = tracing::subscriber::with_default(subscriber, || f.store.sealed_of(&kept[0]));
        assert_eq!(read, Err(StoreError::Io), "a storage failure is Io");
        let log = String::from_utf8(capture.0.lock().expect("lock").clone()).expect("UTF-8");
        assert!(
            log.contains("a stored version's object could not be read")
                && log.contains(&format!("os_error={}", libc::EIO)),
            "the failure is logged by its kind: {log}"
        );
    }

    /// A version purged after the listing is left out; an object lost
    /// while its row stays is damage, never left out.
    #[test]
    fn a_purge_after_the_listing_is_skipped_and_a_lost_object_is_damage() {
        let f = fixture();
        let [_, _, b1, c1] = stored(&f);
        let kept = f.store.kept_versions(None, None).expect("lists");
        let of = |v: VersionId| kept.iter().find(|k| k.version == v).expect("kept");
        f.store
            .purge_hidden(
                Actor::Cli,
                Retention::Days(0),
                Some(C.parse().expect("uuid")),
            )
            .expect("purges C");
        assert_eq!(
            f.store.sealed_of(of(c1)),
            Ok(None),
            "C1 was purged since the listing"
        );
        f.fs.remove(&f.store.objects_dir.join(b1.to_string()))
            .expect("removes B1's object");
        assert_eq!(
            f.store.sealed_of(of(b1)),
            Err(StoreError::Damaged),
            "B1's row is still there"
        );
    }
}

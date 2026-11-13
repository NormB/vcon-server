//! Where each tenant's catalog and objects are, with the catalogs in
//! PostgreSQL: a tenant's catalog is its rows in the shared tables, and
//! its objects are in the bucket (a PostgreSQL catalog is configured only
//! with `storage.objects = "s3"`).
//!
//! - A tenant is made by its lock row and the id of its catalog key
//!   ([`PgCatalog::create`]), and is present while it has a lock row or
//!   objects in the bucket.
//! - A removal deletes the bucket's objects first and then the tenant's
//!   rows, a batch at a time, the lock row last: one that is cut off
//!   leaves a tenant that is still present, and the next start removes it
//!   again.
//! - A tenant's audit records are never deleted. The removal keeps the
//!   rows of `vcon.audit`, which the database refuses to delete from every
//!   role (schema version 8).
//! - A catalog is moved to a new key in one transaction
//!   ([`PgCatalog::reseal`]): every sealed column and every MAC of the
//!   tenant's rows is made again under the new key, and the key of the
//!   tenant's audit log is sealed again. No audit record is changed: its
//!   target is sealed under the audit key, which stays the same.

#![cfg_attr(
    not(all(test, feature = "backend-tests")),
    expect(
        dead_code,
        reason = "serve opens this space from a later change; the tests that need a server use it"
    )
)]

use std::sync::Arc;

use super::catalog::{CatalogError, Database, PgCatalog};
use crate::backend::s3::S3;
use crate::backend::{TenantSpace, Under};
use crate::clock::Clock;
use crate::envelope::{self, Key};
use crate::fs::Fs;
use crate::ids::TenantId;
use crate::store::{OpenError, Store, StoreError};
use crate::tenants::TenantError;

/// The rows of one table a removal deletes in one transaction.
const REMOVE_BATCH: i64 = 10_000;

/// The rows of one table a re-seal reads and writes in one statement.
const RESEAL_BATCH: i64 = 1_000;

/// Each tenant's rows in the database and its objects in the bucket.
pub(crate) struct PgTenantSpace {
    database: Arc<Database>,
    s3: Arc<S3>,
    fs: Arc<dyn Fs>,
}

impl PgTenantSpace {
    /// The space of the tenants whose catalogs are in `database` and whose
    /// objects are in the bucket of `s3`.
    pub(crate) fn new(database: Arc<Database>, s3: Arc<S3>, fs: Arc<dyn Fs>) -> PgTenantSpace {
        PgTenantSpace { database, s3, fs }
    }
}

/// What a Store's open says of a catalog that did not open.
fn open_error(e: CatalogError) -> OpenError {
    match e {
        CatalogError::WrongKey => OpenError::WrongKey,
        _ => OpenError::Store(StoreError::Io),
    }
}

fn tenant_io<E>(_: E) -> TenantError {
    TenantError::Io
}

impl TenantSpace for PgTenantSpace {
    /// The lock row and the key's id are made in one transaction. A
    /// tenant that has them is opened under the key instead, as a create
    /// that is made again finds its directory: under another key that is
    /// `WrongKey`, and nothing is changed.
    fn make(&self, id: TenantId, key: Key, clock: Arc<dyn Clock>) -> Result<(), OpenError> {
        let catalog_key = envelope::catalog_key(&key).map_err(|_| StoreError::Io)?;
        match PgCatalog::create(self.database.clone(), id, &catalog_key) {
            Ok(_) | Err(CatalogError::Exists) => {}
            Err(e) => return Err(open_error(e)),
        }
        drop(self.open(id, key, true, clock)?);
        Ok(())
    }

    fn open(
        &self,
        id: TenantId,
        key: Key,
        recover: bool,
        clock: Arc<dyn Clock>,
    ) -> Result<Store, OpenError> {
        let catalog_key = envelope::catalog_key(&key).map_err(|_| StoreError::Io)?;
        let catalog =
            PgCatalog::open(self.database.clone(), id, &catalog_key).map_err(open_error)?;
        Store::open_on(
            id,
            key,
            Box::new(catalog),
            Arc::new(self.s3.tenant(id)),
            Some(Arc::new(self.s3.packs(id))),
            self.fs.clone(),
            clock,
            recover,
        )
    }

    /// A tenant with a lock row, and one with objects in the bucket and no
    /// row: a create or a removal that was cut off leaves either.
    fn present(&self) -> Result<Vec<TenantId>, TenantError> {
        let mut ids = PgCatalog::present(&self.database).map_err(tenant_io)?;
        ids.extend(self.s3.tenants().map_err(tenant_io)?);
        ids.sort_by_key(|id| id.0);
        ids.dedup();
        Ok(ids)
    }

    /// The bucket's objects go first, then the rows, the lock row last.
    /// The tenant's audit records are kept. A tenant that has neither
    /// objects nor rows is removed already. The registry's id has no
    /// objects, and the statements refuse it.
    fn remove(&self, id: TenantId) -> Result<(), TenantError> {
        self.s3.remove_tenant(id).map_err(tenant_io)?;
        PgCatalog::remove_in(&self.database, id, REMOVE_BATCH).map_err(tenant_io)
    }

    fn catalog_under(&self, id: TenantId, old: &Key, new: &Key) -> Result<Under, TenantError> {
        PgCatalog::under(&self.database, id, old, new).map_err(tenant_io)
    }

    /// One transaction: see the module's last point.
    fn reseal(&self, id: TenantId, old: &Key, new: &Key) -> Result<(), TenantError> {
        PgCatalog::reseal(&self.database, id, old, new, RESEAL_BATCH).map_err(tenant_io)
    }
}

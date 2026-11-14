//! The claim of the bucket prefix for catalogs in PostgreSQL: the one row
//! of `vcon.storage_claim` (schema version 13), where a data directory
//! has the file `storage.claim`. Every server on the database reads the
//! same claim, and a server on another database is refused the prefix
//! (`backend/s3/claim.rs` holds the rule).

use std::sync::Arc;

use tokio_postgres::types::Type;

use super::catalog::{Begin, Database, PgTx};
use crate::backend::s3::claim::{Home, parse_id};
use crate::config::StorageError;
use crate::envelope::REGISTRY;

/// The claim the table holds.
const READ: &str = "SELECT claim_id FROM vcon.storage_claim";

/// Makes the claim where there is none. A server that claims at the same
/// moment waits for the registry's lock, which a write of the registry
/// holds, and then finds the row made.
const KEEP: &str = "INSERT INTO vcon.storage_claim (claim_id) VALUES ($1) ON CONFLICT DO NOTHING";

/// A database as the home of a claim.
#[derive(Debug, Clone)]
pub(crate) struct PgClaim(pub(crate) Arc<Database>);

/// The error of a claim that cannot be `done` ("read" or "written"). The
/// database's own words are not repeated: the log of the PostgreSQL
/// server has them.
fn cannot_be(done: &str) -> StorageError {
    StorageError(format!(
        "catalog.postgres: the claim of the bucket prefix cannot be {done} (the row of the \
         table vcon.storage_claim)"
    ))
}

/// The claim a transaction finds in the table, if there is a row.
fn claim_in(tx: &PgTx<'_>, done: &str) -> Result<Option<String>, StorageError> {
    let rows = tx.query(READ, &[]).map_err(|_| cannot_be(done))?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let text: String = row.try_get(0).map_err(|_| cannot_be(done))?;
    match parse_id(&text) {
        Some(id) if id == text => Ok(Some(text)),
        _ => Err(StorageError(
            "catalog.postgres: the row of the table vcon.storage_claim does not hold a claim \
             id (32 hexadecimal digits)"
                .to_owned(),
        )),
    }
}

impl Home for PgClaim {
    fn noun(&self) -> &'static str {
        "database"
    }

    fn read(&self) -> Result<Option<String>, StorageError> {
        let lease = self.0.lease().map_err(|_| cannot_be("read"))?;
        let tx = PgTx::begin(&self.0, lease, REGISTRY, None, Begin::Read);
        claim_in(&tx, "read")
    }

    fn keep(&self, id: &str) -> Result<String, StorageError> {
        if parse_id(id) != Some(id) {
            return Err(StorageError(
                "catalog.postgres: the claim to keep is not a claim id (32 hexadecimal digits)"
                    .to_owned(),
            ));
        }
        let lease = self.0.lease().map_err(|_| cannot_be("written"))?;
        let tx = PgTx::begin(&self.0, lease, REGISTRY, None, Begin::Write);
        tx.execute(KEEP, &[(&id, Type::TEXT)])
            .map_err(|_| cannot_be("written"))?;
        let kept = claim_in(&tx, "written")?.ok_or_else(|| cannot_be("written"))?;
        tx.finish().map_err(|_| cannot_be("written"))?;
        Ok(kept)
    }
}

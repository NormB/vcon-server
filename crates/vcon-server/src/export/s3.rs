//! An export target in an S3-compatible bucket: each file one PutObject
//! with `If-None-Match: *`, which writes it whole or not at all and never
//! replaces an object, so of two exports racing for one manifest name one
//! is refused.

use std::sync::Arc;

use super::target::{Target, TargetError};
use crate::backend::s3::{S3, client_for};
use crate::config::{S3Settings, StorageError};

/// A bucket and prefix, through the S3 client.
#[derive(Debug)]
pub struct S3Target {
    s3: Arc<S3>,
}

impl S3Target {
    /// A target for the bucket and prefix `settings` names; `table` is the
    /// configuration table the service and keys came from, for errors.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the secret file, the CA file or the client.
    pub fn connect(settings: &S3Settings, table: &str) -> Result<S3Target, StorageError> {
        Ok(S3Target {
            s3: client_for(settings, table)?,
        })
    }

    /// The client, for tests that read the bucket.
    #[cfg(all(test, feature = "backend-tests"))]
    pub(crate) fn client(&self) -> &Arc<S3> {
        &self.s3
    }
}

impl Target for S3Target {
    fn put_new(&self, path: &str, bytes: &[u8]) -> Result<(), TargetError> {
        match self.s3.put_new(path, bytes.to_vec()) {
            Ok(true) => Ok(()),
            Ok(false) => Err(TargetError::Exists),
            Err(e) => Err(TargetError::Io(e.to_string())),
        }
    }

    fn list(&self, dir: &str) -> Result<Vec<String>, TargetError> {
        let names = self
            .s3
            .names_under(dir)
            .map_err(|e| TargetError::Io(e.to_string()))?;
        Ok(names
            .into_iter()
            .map(|n| n.trim_end_matches('/').to_owned())
            .collect())
    }

    fn remove(&self, path: &str) -> Result<(), TargetError> {
        self.s3
            .remove_named(path)
            .map_err(|e| TargetError::Io(e.to_string()))
    }

    fn describe(&self) -> String {
        self.s3.url()
    }
}

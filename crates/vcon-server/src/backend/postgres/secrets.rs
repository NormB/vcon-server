//! The files `[catalog.postgres]` names, read once at start: the password,
//! the client certificate and its key, and the certificate authorities.
//! Nothing here is read again; the pool reconnects from what is held.

use std::path::Path;

use rustls::RootCertStore;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use zeroize::{Zeroize, Zeroizing};

use crate::config::{PostgresSettings, StorageError};

/// What the connector logs in with, read from the files once.
pub(crate) struct Secrets {
    /// The role's password, from `password_file`.
    pub(super) password: Option<Zeroizing<String>>,
    /// The authorities trusted for the servers' certificates.
    pub(super) roots: RootCertStore,
    /// The client certificate chain and its key.
    pub(super) client: Option<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)>,
}

impl std::fmt::Debug for Secrets {
    /// Says which secrets are held, never their bytes.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Secrets")
            .field("password", &self.password.as_ref().map(|_| "set"))
            .field("roots", &self.roots.len())
            .field(
                "client",
                &self.client.as_ref().map(|(chain, _)| chain.len()),
            )
            .finish()
    }
}

impl Secrets {
    /// Reads every file `settings` names. An error names the key and the
    /// file, never what the file holds.
    pub(crate) fn read(settings: &PostgresSettings) -> Result<Secrets, StorageError> {
        let password = settings
            .password_file
            .as_deref()
            .map(read_password)
            .transpose()?;
        let roots = match &settings.ca_file {
            Some(path) => {
                let pem = crate::backend::s3::read_ca(path, "catalog.postgres")?;
                let mut roots = RootCertStore::empty();
                let (_, _) = roots.add_parsable_certificates(
                    CertificateDer::pem_slice_iter(&pem).filter_map(Result::ok),
                );
                roots
            }
            None => system_roots()?,
        };
        let client = match (&settings.client_cert_file, &settings.client_key_file) {
            (Some(cert), Some(key)) => Some(read_client(cert, key)?),
            _ => None,
        };
        Ok(Secrets {
            password,
            roots,
            client,
        })
    }
}

/// The system's certificate authorities.
fn system_roots() -> Result<RootCertStore, StorageError> {
    let mut roots = RootCertStore::empty();
    let (_, _) = roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if roots.is_empty() {
        return Err(StorageError(
            "catalog.postgres: no certificate authority of the system's could be read; set \
             catalog.postgres.ca_file"
                .to_owned(),
        ));
    }
    Ok(roots)
}

/// The password in `path`: one line of UTF-8, its line ending left out.
fn read_password(path: &Path) -> Result<Zeroizing<String>, StorageError> {
    let refused = |why: &str| {
        StorageError(format!(
            "catalog.postgres.password_file {}: {why}",
            path.display()
        ))
    };
    let mut bytes = read_private(path, "catalog.postgres.password_file")?;
    let text = String::from_utf8(std::mem::take(&mut *bytes)).map_err(|e| {
        e.into_bytes().zeroize();
        refused("the file is not UTF-8")
    })?;
    let text = Zeroizing::new(text);
    crate::backend::s3::secret_line(&text).map_err(refused)
}

/// The client certificate chain in `cert` and its private key in `key`.
fn read_client(
    cert: &Path,
    key: &Path,
) -> Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>), StorageError> {
    let pem = std::fs::read(cert).map_err(|e| {
        StorageError(format!(
            "catalog.postgres.client_cert_file {}: {e}",
            cert.display()
        ))
    })?;
    let chain = CertificateDer::pem_slice_iter(&pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| {
            StorageError(format!(
                "catalog.postgres.client_cert_file {}: not PEM: {e}",
                cert.display()
            ))
        })?;
    if chain.is_empty() {
        return Err(StorageError(format!(
            "catalog.postgres.client_cert_file {}: the file holds no certificate",
            cert.display()
        )));
    }
    let bytes = read_private(key, "catalog.postgres.client_key_file")?;
    let key = PrivateKeyDer::from_pem_slice(&bytes).map_err(|_| {
        StorageError(format!(
            "catalog.postgres.client_key_file {}: the file holds no PEM private key",
            key.display()
        ))
    })?;
    Ok((chain, key))
}

/// The bytes of the secret file `path`, named `key` in an error, held to
/// the one secret-file rule every secret the configuration names is held to
/// (`crate::secret_file`).
pub(crate) fn read_private(path: &Path, key: &str) -> Result<Zeroizing<Vec<u8>>, StorageError> {
    crate::secret_file::read(path, key).map_err(StorageError)
}

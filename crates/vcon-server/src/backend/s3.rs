//! Sealed objects in an S3-compatible bucket (RustFS, MinIO, AWS S3): the
//! blob of each version, byte for byte as the local backend writes it, at
//! `<prefix>tenants/<tenant id>/objects/<version id>`. A PUT that returns
//! success is the durable write, and no partial object is ever visible,
//! so there are no temporary names.

use std::future::Future;
use std::io::Read;
use std::sync::{Arc, mpsc};

use aws_sdk_s3::Client;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{BucketVersioningStatus, Delete, ObjectIdentifier};
use aws_smithy_http_client::tls;
use zeroize::Zeroizing;

mod claim;

use super::{ObjectError, Objects};
use crate::config::{S3Settings, StorageError};
use crate::ids::{TenantId, VersionId};

/// A bucket's versioning state, as GetBucketVersioning reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Versioning {
    /// Versioning was never turned on.
    Never,
    Enabled,
    Suspended,
}

impl Versioning {
    /// Whether the server may keep its objects in a bucket in this state:
    /// a versioned bucket keeps a removed object's earlier versions, so it
    /// is used only when the configuration says so.
    pub(crate) fn allowed(self, allow_versioning: bool) -> bool {
        self == Versioning::Never || allow_versioning
    }
}

/// The bucket `settings` names, reached and checked, as `objects_for`
/// describes.
///
/// # Errors
///
/// A `StorageError` naming the secret file, the bucket, or the versioning
/// setting.
pub(crate) fn place(
    settings: &S3Settings,
    data_dir: &std::path::Path,
) -> Result<super::ObjectPlace, StorageError> {
    let secret = settings
        .secret_access_key_file
        .as_deref()
        .map(read_secret)
        .transpose()?;
    let ca = settings.ca_file.as_deref().map(read_ca).transpose()?;
    let s3 = S3::connect(settings, secret.as_ref().map(|s| s.as_str()), ca.as_deref())
        .map_err(|e| StorageError(format!("the S3 client cannot start: {e}")))?;
    let versioning = s3.versioning().map_err(|e| {
        StorageError(format!(
            "bucket {:?} cannot be reached or read: {e}",
            settings.bucket
        ))
    })?;
    if !versioning.allowed(settings.allow_versioning) {
        return Err(StorageError(format!(
            "bucket {:?} has versioning {versioning:?}, which keeps a removed object's \
             earlier versions; turn it off or set storage.s3.allow_versioning = true",
            settings.bucket
        )));
    }
    s3.claim_for(data_dir)?;
    Ok(super::ObjectPlace::S3(s3))
}

/// The secret access key in `path`: one line, its line ending left out.
/// It is read into a buffer of the file's size, so no shorter copy is left
/// behind by a reallocation, and every copy here is zeroed when dropped.
/// An error names the key and the file, never the text.
fn read_secret(path: &std::path::Path) -> Result<Zeroizing<String>, StorageError> {
    let refused = |why: &str| {
        StorageError(format!(
            "storage.s3.secret_access_key_file {}: {why}",
            path.display()
        ))
    };
    let mut file = std::fs::File::open(path).map_err(|e| refused(&e.to_string()))?;
    let size = file.metadata().map_err(|e| refused(&e.to_string()))?.len();
    let mut text = Zeroizing::new(String::with_capacity(
        usize::try_from(size).unwrap_or(0).saturating_add(1),
    ));
    file.read_to_string(&mut text)
        .map_err(|e| refused(&e.to_string()))?;
    let line = text.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err(refused("the file is empty"));
    }
    if line.contains(['\r', '\n']) {
        return Err(refused("the file holds more than one line"));
    }
    Ok(Zeroizing::new(line.to_owned()))
}

/// The PEM file in `path`, once each certificate in it has been parsed
/// and taken as a trust anchor; at least one must be there. The client's
/// TLS layer panics on a file that fails either step, so it is refused
/// here, by name.
fn read_ca(path: &std::path::Path) -> Result<Vec<u8>, StorageError> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;

    let refused = |why: &str| StorageError(format!("storage.s3.ca_file {}: {why}", path.display()));
    let pem = std::fs::read(path).map_err(|e| refused(&e.to_string()))?;
    let mut roots = rustls::RootCertStore::empty();
    for cert in CertificateDer::pem_slice_iter(&pem) {
        let cert = cert.map_err(|e| refused(&format!("not PEM: {e}")))?;
        roots
            .add(cert)
            .map_err(|e| refused(&format!("a certificate cannot be trusted: {e}")))?;
    }
    if roots.is_empty() {
        return Err(refused("the file holds no certificate"));
    }
    Ok(pem)
}

/// The runtime every call to the bucket runs on, apart from the caller's.
fn client_runtime() -> Result<tokio::runtime::Runtime, ObjectError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("vcon-s3")
        .enable_all()
        .build()
        .map_err(ObjectError::Io)
}

/// Runs `call` on `runtime` and waits for its answer. Unlike
/// `Runtime::block_on`, it may be called from any thread, inside another
/// runtime or not: `serve` makes the client inside its own.
fn run_on<T: Send + 'static>(
    runtime: &tokio::runtime::Runtime,
    call: impl Future<Output = T> + Send + 'static,
) -> Result<T, ObjectError> {
    let (tx, rx) = mpsc::sync_channel(1);
    runtime.spawn(async move {
        // The receiver waits below, so a send fails only if this call
        // was abandoned, which leaves nothing to report.
        let _ = tx.send(call.await);
    });
    rx.recv()
        .map_err(|_| ObjectError::Io(std::io::Error::other("the S3 call was dropped")))
}

/// The AWS default credential chain (environment, shared files, web
/// identity, container and instance credentials), set up for the
/// configured region. Its credentials are fetched at the first request,
/// not here.
fn shared_config(
    runtime: &tokio::runtime::Runtime,
    settings: &S3Settings,
) -> Result<aws_config::SdkConfig, ObjectError> {
    let loader = aws_config::defaults(BehaviorVersion::latest())
        .region(Region::new(settings.region.clone()));
    run_on(runtime, loader.load())
}

/// How many objects one DeleteObjects call removes, S3's own limit.
const DELETE_BATCH: usize = 1000;

/// The bucket's client and the runtime its calls run on, shared by every
/// tenant.
pub(crate) struct S3 {
    client: Client,
    bucket: String,
    prefix: String,
    runtime: Option<tokio::runtime::Runtime>,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3")
            .field("bucket", &self.bucket)
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl Drop for S3 {
    fn drop(&mut self) {
        // A runtime dropped inside another runtime's task panics; this way
        // it never waits for its threads.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

impl S3 {
    /// A client for the bucket `settings` names, with `secret` the secret
    /// access key read from its file, and `ca` the PEM certificates of the
    /// authorities trusted for the endpoint, if each is configured.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the client's runtime cannot be started.
    pub(crate) fn connect(
        settings: &S3Settings,
        secret: Option<&str>,
        ca: Option<&[u8]>,
    ) -> Result<Arc<S3>, ObjectError> {
        let runtime = client_runtime()?;
        let mut builder = aws_sdk_s3::config::Builder::new()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(settings.region.clone()))
            .force_path_style(settings.path_style)
            // Every blob carries its own AEAD tag, which binds its tenant,
            // uuid and version, so no request depends on a service's
            // support for checksum trailers.
            .request_checksum_calculation(RequestChecksumCalculation::WhenRequired)
            .response_checksum_validation(ResponseChecksumValidation::WhenRequired)
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(settings.timeout)
                    .build(),
            );
        if let Some(endpoint) = &settings.endpoint {
            builder = builder.endpoint_url(endpoint);
        }
        if let Some(pem) = ca {
            // Only these authorities, not the system's as well, as with
            // curl's --cacert.
            let trust = tls::TrustStore::empty().with_pem_certificate(pem.to_vec());
            let context = tls::TlsContext::builder()
                .with_trust_store(trust)
                .build()
                .map_err(failed)?;
            builder = builder.http_client(
                aws_smithy_http_client::Builder::new()
                    .tls_provider(tls::Provider::Rustls(
                        tls::rustls_provider::CryptoMode::AwsLc,
                    ))
                    .tls_context(context)
                    .build_https(),
            );
        }
        match (&settings.access_key_id, secret) {
            (Some(id), Some(secret)) => {
                builder = builder.credentials_provider(Credentials::new(
                    id.clone(),
                    secret.to_owned(),
                    None,
                    None,
                    "vcon-server configuration",
                ));
            }
            _ => {
                let shared = shared_config(&runtime, settings)?;
                if let Some(provider) = shared.credentials_provider() {
                    builder = builder.credentials_provider(provider);
                }
            }
        }
        Ok(Arc::new(S3 {
            client: Client::from_conf(builder.build()),
            bucket: settings.bucket.clone(),
            prefix: settings.prefix.clone(),
            runtime: Some(runtime),
        }))
    }

    /// Runs `call` on the client's runtime and waits for its answer. It
    /// may be called from any thread, inside another runtime or not.
    fn wait<T: Send + 'static>(
        &self,
        call: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, ObjectError> {
        let runtime = self
            .runtime
            .as_ref()
            .expect("the runtime lives until the client is dropped");
        run_on(runtime, call)
    }

    /// The bucket's versioning state.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be reached or read.
    pub(crate) fn versioning(&self) -> Result<Versioning, ObjectError> {
        let call = self
            .client
            .get_bucket_versioning()
            .bucket(&self.bucket)
            .send();
        let answer = self.wait(call)?.map_err(failed)?;
        Ok(match answer.status() {
            Some(BucketVersioningStatus::Enabled) => Versioning::Enabled,
            Some(BucketVersioningStatus::Suspended) => Versioning::Suspended,
            _ => Versioning::Never,
        })
    }

    /// Binds the prefix to `data_dir`, as `claim` describes: uses it,
    /// claims it, or refuses.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the bucket and prefix, or the step that
    /// failed.
    fn claim_for(&self, data_dir: &std::path::Path) -> Result<(), StorageError> {
        let place = format!("bucket {:?}, prefix {:?}", self.bucket, self.prefix);
        let unreadable = |e: ObjectError| StorageError(format!("{place} cannot be read: {e}"));
        let local = claim::read_file(data_dir)?;
        let bucket = self.read_claim().map_err(unreadable)?;
        let holds_tenants = !self.tenants().map_err(unreadable)?.is_empty();
        let write = |id: &str| {
            self.write_claim(id)
                .map_err(|e| StorageError(format!("{place} cannot be claimed: {e}")))
        };
        match claim::decide(local.as_deref(), bucket.as_deref(), holds_tenants, &place)? {
            claim::Step::Use => Ok(()),
            claim::Step::Claim => {
                let id = claim::new_id();
                claim::write_file(data_dir, &id)?;
                write(&id)
            }
            claim::Step::Finish => write(local.as_deref().unwrap_or_default()),
        }
    }

    /// The claim object's name.
    fn claim_key(&self) -> String {
        format!("{}{}", self.prefix, claim::OBJECT)
    }

    /// The claim object's id under the prefix, if there is one.
    fn read_claim(&self) -> Result<Option<String>, ObjectError> {
        let call = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(self.claim_key())
            .send();
        let bytes = self.wait(async move {
            match call.await {
                Ok(out) => out
                    .body
                    .collect()
                    .await
                    .map(|bytes| Some(bytes.to_vec()))
                    .map_err(failed),
                Err(e) => match e.into_service_error() {
                    e if e.is_no_such_key() => Ok(None),
                    e => Err(failed(e)),
                },
            }
        })??;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let text = String::from_utf8_lossy(&bytes);
        claim::parse_id(&text)
            .map(|id| Some(id.to_owned()))
            .ok_or_else(|| {
                ObjectError::Io(std::io::Error::other(format!(
                    "{} does not hold a claim id",
                    self.claim_key()
                )))
            })
    }

    /// Writes `id` as the claim object, only if there is none
    /// (`If-None-Match: *`): of two servers claiming an empty prefix at
    /// once, one is refused.
    fn write_claim(&self, id: &str) -> Result<(), ObjectError> {
        let call = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(self.claim_key())
            .if_none_match("*")
            .body(ByteStream::from(format!("{id}\n").into_bytes()))
            .send();
        self.wait(call)?.map(drop).map_err(failed)
    }

    /// The name before every object of `tenant`.
    pub(crate) fn tenant_dir(&self, tenant: TenantId) -> String {
        tenant_dir(&self.prefix, tenant)
    }

    /// One tenant's objects.
    pub(crate) fn tenant(self: &Arc<S3>, tenant: TenantId) -> S3Objects {
        S3Objects {
            s3: self.clone(),
            dir: self.tenant_dir(tenant),
        }
    }

    /// The tenants that have any object in the bucket.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be listed.
    pub(crate) fn tenants(&self) -> Result<Vec<TenantId>, ObjectError> {
        let root = format!("{}tenants/", self.prefix);
        let names = self.names(&root, Some("/"))?;
        Ok(names
            .iter()
            .filter_map(|name| name.strip_suffix('/').and_then(TenantId::parse))
            .collect())
    }

    /// Every name under `dir`, without `dir`, sorted; with a delimiter, the
    /// common prefixes up to it as well.
    fn names(&self, dir: &str, delimiter: Option<&str>) -> Result<Vec<String>, ObjectError> {
        let client = self.client.clone();
        let bucket = self.bucket.clone();
        let dir = dir.to_owned();
        let delimiter = delimiter.map(str::to_owned);
        let mut names = self.wait(async move {
            let mut names = Vec::new();
            let mut pages = client
                .list_objects_v2()
                .bucket(bucket)
                .prefix(&dir)
                .set_delimiter(delimiter)
                .into_paginator()
                .send();
            while let Some(page) = pages.next().await {
                let page = page.map_err(failed)?;
                let keys = page.contents().iter().filter_map(|o| o.key());
                let common = page.common_prefixes().iter().filter_map(|p| p.prefix());
                for key in keys.chain(common) {
                    if let Some(name) = key.strip_prefix(&dir) {
                        names.push(name.to_owned());
                    }
                }
            }
            Ok::<_, ObjectError>(names)
        })??;
        names.sort();
        Ok(names)
    }

    /// Removes every object of `tenant`, a thousand at a time.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be listed or an object not
    /// removed.
    pub(crate) fn remove_tenant(&self, tenant: TenantId) -> Result<(), ObjectError> {
        let dir = self.tenant_dir(tenant);
        let names = self.names(&dir, None)?;
        for chunk in names.chunks(DELETE_BATCH) {
            let ids = chunk
                .iter()
                .map(|name| {
                    ObjectIdentifier::builder()
                        .key(format!("{dir}{name}"))
                        .build()
                        .map_err(failed)
                })
                .collect::<Result<Vec<_>, _>>()?;
            let delete = Delete::builder()
                .set_objects(Some(ids))
                .build()
                .map_err(failed)?;
            let call = self
                .client
                .delete_objects()
                .bucket(&self.bucket)
                .delete(delete)
                .send();
            let answer = self.wait(call)?.map_err(failed)?;
            if let Some(error) = answer.errors().first() {
                return Err(ObjectError::Io(std::io::Error::other(format!(
                    "an object was not removed: {}",
                    error.message().unwrap_or("no reason given")
                ))));
            }
        }
        Ok(())
    }
}

/// The name before every object of `tenant`, under `prefix`.
fn tenant_dir(prefix: &str, tenant: TenantId) -> String {
    format!("{prefix}tenants/{tenant}/objects/")
}

/// `e` as an object error whose text holds its whole chain of causes: the
/// SDK's own text names only the step ("service error", "dispatch
/// failure"), and the cause (the service's error code, a refused
/// connection, an untrusted certificate) is in its sources. None of them
/// holds a credential.
fn failed<E: std::error::Error + Send + Sync + 'static>(e: E) -> ObjectError {
    let mut text = e.to_string();
    let mut cause = e.source();
    while let Some(next) = cause {
        let more = next.to_string();
        // Some layers repeat their source's text in their own.
        if !text.ends_with(&more) {
            text.push_str(": ");
            text.push_str(&more);
        }
        cause = next.source();
    }
    ObjectError::Io(std::io::Error::other(text))
}

/// One tenant's objects, under its own name in the bucket.
#[derive(Debug)]
pub(crate) struct S3Objects {
    s3: Arc<S3>,
    dir: String,
}

impl S3Objects {
    fn key(&self, version: VersionId) -> String {
        format!("{}{version}", self.dir)
    }
}

impl Objects for S3Objects {
    fn put(&self, version: VersionId, blob: &[u8]) -> Result<(), ObjectError> {
        let call = self
            .s3
            .client
            .put_object()
            .bucket(&self.s3.bucket)
            .key(self.key(version))
            .body(ByteStream::from(blob.to_vec()))
            .send();
        self.s3.wait(call)?.map(drop).map_err(failed)
    }

    fn get(&self, version: VersionId) -> Result<Vec<u8>, ObjectError> {
        let call = self
            .s3
            .client
            .get_object()
            .bucket(&self.s3.bucket)
            .key(self.key(version))
            .send();
        self.s3.wait(async move {
            match call.await {
                Ok(out) => out
                    .body
                    .collect()
                    .await
                    .map(|bytes| bytes.to_vec())
                    .map_err(failed),
                Err(e) => match e.into_service_error() {
                    e if e.is_no_such_key() => Err(ObjectError::Missing),
                    e => Err(failed(e)),
                },
            }
        })?
    }

    /// S3 answers success for a name it does not hold, so an absent object
    /// is not told apart here.
    fn remove(&self, version: VersionId) -> Result<(), ObjectError> {
        self.remove_name(&version.to_string())
    }

    fn list(&self) -> Result<Vec<String>, ObjectError> {
        self.s3.names(&self.dir, None)
    }

    fn remove_name(&self, name: &str) -> Result<(), ObjectError> {
        let call = self
            .s3
            .client
            .delete_object()
            .bucket(&self.s3.bucket)
            .key(format!("{}{name}", self.dir))
            .send();
        self.s3.wait(call)?.map(drop).map_err(failed)
    }

    /// A DeleteObject is durable once it returns.
    fn sync(&self) -> Result<(), ObjectError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Settings for a service that is never called: nothing listens on
    /// port 9 of the loopback address.
    fn unreachable() -> S3Settings {
        S3Settings {
            endpoint: Some("http://127.0.0.1:9".to_owned()),
            region: "eu-west-3".to_owned(),
            bucket: "vcon".to_owned(),
            prefix: String::new(),
            path_style: true,
            access_key_id: None,
            secret_access_key_file: None,
            allow_http: true,
            ca_file: None,
            allow_versioning: false,
            timeout: Duration::from_secs(1),
        }
    }

    /// `serve` makes the client inside its own runtime, where a second
    /// runtime may not block.
    #[tokio::test]
    async fn a_client_without_keys_starts_inside_a_runtime() {
        let s3 = S3::connect(&unreachable(), None, None).expect("a client");
        drop(s3);
    }

    /// A secret file in a temporary directory, removed with it.
    fn secret_in(text: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("s3.secret");
        std::fs::write(&path, text).expect("writes the secret file");
        (dir, path)
    }

    #[test]
    fn a_secret_file_is_read_without_its_line_ending() {
        for text in [
            &b"k3y with spaces"[..],
            b"k3y with spaces\n",
            b"k3y with spaces\r\n",
        ] {
            let (_dir, path) = secret_in(text);
            assert_eq!(
                read_secret(&path).expect("reads").as_str(),
                "k3y with spaces",
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_secret_file_that_is_empty_or_has_two_lines_is_refused_without_its_text() {
        for text in [&b""[..], b"\n", b"id-and\nk3y-in-one-file\n"] {
            let (_dir, path) = secret_in(text);
            let error = read_secret(&path).expect_err("refused");
            assert!(
                error.0.contains("storage.s3.secret_access_key_file"),
                "the error names the key: {error}"
            );
            assert!(
                !error.0.contains("k3y"),
                "the error shows the secret: {error}"
            );
        }
        let error = read_secret(std::path::Path::new("/nonexistent/s3.secret"))
            .expect_err("a missing file");
        assert!(
            error
                .0
                .contains("storage.s3.secret_access_key_file /nonexistent/s3.secret"),
            "the error names the key and the file: {error}"
        );
    }

    /// A certificate authority, and a certificate it signed for
    /// 127.0.0.1 with its key, all PEM.
    struct Pki {
        ca: String,
        cert: String,
        key: String,
    }

    fn pki() -> Pki {
        let ca_key = rcgen::KeyPair::generate().expect("a CA key");
        let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
        ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![
            rcgen::KeyUsagePurpose::KeyCertSign,
            rcgen::KeyUsagePurpose::DigitalSignature,
        ];
        let ca = ca_params.self_signed(&ca_key).expect("the CA");
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let key = rcgen::KeyPair::generate().expect("a key");
        let cert = rcgen::CertificateParams::new(vec!["127.0.0.1".to_owned()])
            .expect("params")
            .signed_by(&key, &issuer)
            .expect("the certificate");
        Pki {
            ca: ca.pem(),
            cert: cert.pem(),
            key: key.serialize_pem(),
        }
    }

    #[test]
    fn a_ca_file_without_a_valid_certificate_is_refused_by_name() {
        let made = pki();
        for (text, why) in [
            (made.key.clone(), "a key, no certificate"),
            ("not PEM at all\n".to_owned(), "no PEM"),
            (
                "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n".to_owned(),
                "a certificate that does not parse",
            ),
            (
                format!(
                    "{}-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n",
                    made.ca
                ),
                "a good certificate, then one that does not parse",
            ),
        ] {
            let (_dir, path) = secret_in(text.as_bytes());
            let error = read_ca(&path).expect_err(why);
            assert!(
                error.0.contains("storage.s3.ca_file"),
                "{why}: the error names the key: {error}"
            );
        }
        let (_dir, path) = secret_in(made.ca.as_bytes());
        assert!(read_ca(&path).is_ok(), "a CA certificate is read");
    }

    /// An HTTPS service on the loopback address with `pki`'s certificate:
    /// to every request it answers as S3 does for a bucket never
    /// versioned. Its address, and how many requests it has answered.
    fn https_service(pki: &Pki) -> (std::net::SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
        use rustls::pki_types::pem::PemObject;
        use rustls::pki_types::{CertificateDer, PrivateKeyDer};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let certs = vec![CertificateDer::from_pem_slice(pki.cert.as_bytes()).expect("cert")];
        let key = PrivateKeyDer::from_pem_slice(pki.key.as_bytes()).expect("key");
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("server config");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        listener.set_nonblocking(true).expect("nonblocking");
        let address = listener.local_addr().expect("address");
        let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = answered.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("runtime");
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).expect("listener");
                loop {
                    let Ok((tcp, _)) = listener.accept().await else {
                        continue;
                    };
                    let acceptor = acceptor.clone();
                    let count = count.clone();
                    tokio::spawn(async move {
                        let Ok(mut tls) = acceptor.accept(tcp).await else {
                            return;
                        };
                        let mut request = Vec::new();
                        let mut buf = [0; 4096];
                        while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                            match tls.read(&mut buf).await {
                                Ok(0) | Err(_) => return,
                                Ok(n) => request.extend_from_slice(&buf[..n]),
                            }
                        }
                        let body = r#"<?xml version="1.0" encoding="UTF-8"?><VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"/>"#;
                        let answer = format!(
                            "HTTP/1.1 200 OK\r\ncontent-type: application/xml\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = tls.write_all(answer.as_bytes()).await;
                        let _ = tls.shutdown().await;
                    });
                }
            });
        });
        (address, answered)
    }

    #[test]
    fn a_private_ca_is_trusted_only_when_named() {
        let made = pki();
        let (address, answered) = https_service(&made);
        let settings = S3Settings {
            endpoint: Some(format!("https://{address}")),
            allow_http: false,
            access_key_id: Some("vcon-test".to_owned()),
            // Long enough for the SDK's retries of a failed handshake.
            timeout: Duration::from_secs(20),
            ..unreachable()
        };
        let trusting = S3::connect(&settings, Some("test-secret"), Some(made.ca.as_bytes()))
            .expect("a client");
        assert_eq!(
            trusting.versioning().expect("the bucket is read over TLS"),
            Versioning::Never,
            "with the CA named, the service's certificate is trusted"
        );
        assert_eq!(
            answered.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "the one request was answered"
        );
        let plain = S3::connect(&settings, Some("test-secret"), None).expect("a client");
        let refused = plain
            .versioning()
            .expect_err("the system does not trust the CA");
        assert!(
            refused.to_string().contains("UnknownIssuer"),
            "the error says the handshake failed on the issuer: {refused}"
        );
        assert_eq!(
            answered.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "no request is sent over an untrusted connection"
        );
    }

    #[test]
    fn an_unreachable_service_is_named_in_the_error() {
        let settings = S3Settings {
            access_key_id: Some("vcon-test".to_owned()),
            // Long enough for the SDK's retries.
            timeout: Duration::from_secs(20),
            ..unreachable()
        };
        let s3 = S3::connect(&settings, Some("test-secret"), None).expect("a client");
        let refused = s3.versioning().expect_err("nothing listens");
        assert!(
            refused.to_string().contains("Connection refused"),
            "the error gives the cause, not only the step: {refused}"
        );
    }

    #[test]
    fn the_credential_chain_signs_for_the_configured_region() {
        let runtime = client_runtime().expect("a runtime");
        let config = shared_config(&runtime, &unreachable()).expect("the chain loads");
        assert_eq!(
            config.region().map(Region::as_ref),
            Some("eu-west-3"),
            "the region comes from the configuration, not the environment"
        );
    }

    #[test]
    fn an_object_is_named_by_prefix_tenant_and_version() {
        let tenant = TenantId([0x11; 16]);
        assert_eq!(
            tenant_dir("server-a/", tenant),
            format!("server-a/tenants/{tenant}/objects/"),
            "the layout mirrors the data directory's, under the prefix"
        );
        assert_eq!(
            tenant_dir("", tenant),
            format!("tenants/{tenant}/objects/"),
            "an empty prefix starts at tenants/"
        );
    }

    #[test]
    fn only_a_bucket_never_versioned_is_used_unless_allowed() {
        assert!(
            Versioning::Never.allowed(false),
            "a bucket never versioned keeps no removed object"
        );
        for state in [Versioning::Enabled, Versioning::Suspended] {
            assert!(
                !state.allowed(false),
                "{state:?} keeps a removed object's earlier versions"
            );
            assert!(state.allowed(true), "{state:?} is used when allowed");
        }
    }
}

/// Against a real S3-compatible service, whose address and keys come from
/// `VCON_TEST_S3_ENDPOINT`, `VCON_TEST_S3_ACCESS_KEY` and
/// `VCON_TEST_S3_SECRET_KEY`. Each test makes a bucket of its own. Built
/// only with the `backend-tests` feature; without the addresses every test
/// fails, so a run that cannot reach the service never passes by skipping.
#[cfg(all(test, feature = "backend-tests"))]
mod remote {
    use std::time::Duration;

    use super::*;
    use crate::backend::conformance::{self, Fresh, Harness, TestClock};
    use crate::backend::local::LocalCatalog;
    use crate::envelope::{self, Key};
    use crate::fs::OsFs;
    use crate::store::{Caller, Store};

    fn var(name: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| {
            panic!("{name} is not set: these tests need a real S3-compatible service")
        })
    }

    /// The settings for a new bucket on the test service.
    fn settings(bucket: &str) -> S3Settings {
        S3Settings {
            endpoint: Some(var("VCON_TEST_S3_ENDPOINT")),
            region: "us-east-1".to_owned(),
            bucket: bucket.to_owned(),
            prefix: "test/".to_owned(),
            path_style: true,
            access_key_id: Some(var("VCON_TEST_S3_ACCESS_KEY")),
            secret_access_key_file: None,
            allow_http: true,
            ca_file: None,
            allow_versioning: false,
            timeout: Duration::from_secs(30),
        }
    }

    /// A new bucket, versioned or not, and the client for it.
    fn bucket(versioned: bool) -> (Arc<S3>, S3Settings) {
        let name = format!("vcon-test-{}", TenantId::generate())
            .chars()
            .take(40)
            .collect::<String>();
        let settings = settings(&name);
        let s3 =
            S3::connect(&settings, Some(&var("VCON_TEST_S3_SECRET_KEY")), None).expect("a client");
        let call = s3.client.create_bucket().bucket(&name).send();
        s3.wait(call).expect("waits").expect("the bucket is made");
        if versioned {
            let config = aws_sdk_s3::types::VersioningConfiguration::builder()
                .status(BucketVersioningStatus::Enabled)
                .build();
            let call = s3
                .client
                .put_bucket_versioning()
                .bucket(&name)
                .versioning_configuration(config)
                .send();
            s3.wait(call)
                .expect("waits")
                .expect("versioning is turned on");
        }
        (s3, settings)
    }

    /// Every object in the bucket, by name, with its bytes.
    fn everything(s3: &S3) -> Vec<(String, Vec<u8>)> {
        let names = s3.names("", None).expect("lists the bucket");
        names
            .into_iter()
            .map(|name| {
                let call = s3.client.get_object().bucket(&s3.bucket).key(&name).send();
                let bytes = s3
                    .wait(async move {
                        let out = call.await.expect("reads an object");
                        out.body.collect().await.expect("its body").to_vec()
                    })
                    .expect("waits");
                (name, bytes)
            })
            .collect()
    }

    /// A local catalog with its objects in the bucket.
    struct LocalAndS3 {
        dir: tempfile::TempDir,
        s3: Arc<S3>,
    }

    impl Harness for LocalAndS3 {
        fn fresh(&self, clock: Arc<TestClock>) -> Fresh {
            let tenant = TenantId::generate();
            let dir = self.dir.path().join(tenant.to_string());
            std::fs::create_dir(&dir).expect("tenant directory");
            let master = Key::generate().expect("master");
            let ctx = envelope::Context::Tenant { tenant };
            let wrapped = envelope::wrap(&master, &ctx, &Key::generate().expect("key"));
            let s3 = self.s3.clone();
            let objects = Arc::new(s3.tenant(tenant));
            let planted = self.s3.clone();
            let open = move || {
                let key = envelope::unwrap(&master, &ctx, &wrapped).expect("unwraps");
                let catalog_key = envelope::catalog_key(&key).expect("catalog key");
                let catalog =
                    LocalCatalog::open(&dir.join("catalog.sqlite"), catalog_key).expect("catalog");
                Store::open_on(
                    tenant,
                    key,
                    Box::new(catalog),
                    Arc::new(s3.tenant(tenant)),
                    Arc::new(OsFs),
                    clock.clone(),
                    true,
                )
                .expect("opens")
            };
            Fresh {
                store: open(),
                reopen: Box::new(open),
                objects,
                plant: Box::new(move |name, bytes| {
                    let call = planted
                        .client
                        .put_object()
                        .bucket(&planted.bucket)
                        .key(format!("{}{name}", planted.tenant_dir(tenant)))
                        .body(ByteStream::from(bytes.to_vec()))
                        .send();
                    planted
                        .wait(call)
                        .expect("waits")
                        .expect("plants an object");
                }),
            }
        }
    }

    #[test]
    fn a_local_catalog_with_s3_objects_conforms() {
        let (s3, _) = bucket(false);
        conformance::run(&LocalAndS3 {
            dir: tempfile::tempdir().expect("temp dir"),
            s3,
        });
    }

    #[test]
    fn an_object_reads_back_and_an_absent_one_is_missing() {
        let (s3, _) = bucket(false);
        let objects = s3.tenant(TenantId::generate());
        let version = VersionId::generate();
        objects.put(version, b"sealed bytes").expect("put");
        assert_eq!(
            objects.get(version).expect("get"),
            b"sealed bytes",
            "an object reads back byte for byte"
        );
        assert_eq!(
            objects.list().expect("list"),
            [version.to_string()],
            "the tenant lists its one object by version id"
        );
        objects.remove(version).expect("remove");
        assert!(
            matches!(objects.get(version), Err(ObjectError::Missing)),
            "a removed object is missing, not an I/O failure"
        );
        assert!(objects.list().expect("list").is_empty(), "nothing is left");
    }

    #[test]
    fn recovery_removes_an_object_no_row_names() {
        let (s3, _) = bucket(false);
        let harness = LocalAndS3 {
            dir: tempfile::tempdir().expect("temp dir"),
            s3: s3.clone(),
        };
        let fresh = harness.fresh(TestClock::at(conformance::START));
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let (found, body) = conformance::vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc01", "kept");
        fresh.store.create(&who, &found, &body).expect("stored");
        let leftover = VersionId::generate();
        let tenant_objects = s3.names("test/tenants/", None).expect("lists");
        let dir = tenant_objects[0].rsplit_once('/').expect("a tenant name").0;
        let call = s3
            .client
            .put_object()
            .bucket(&s3.bucket)
            .key(format!("test/tenants/{dir}/{leftover}"))
            .body(ByteStream::from(b"no row".to_vec()))
            .send();
        s3.wait(call)
            .expect("waits")
            .expect("a leftover is written");
        drop(fresh.store);
        let store = (fresh.reopen)();
        let names = s3.names("test/tenants/", None).expect("lists");
        assert_eq!(
            names.len(),
            1,
            "recovery removed the object no row names: {names:?}"
        );
        assert_eq!(
            store.get(&who, found.uuid).expect("reads").body,
            body,
            "the stored vCon is kept"
        );
    }

    #[test]
    fn a_tenant_removed_leaves_no_object() {
        let (s3, _) = bucket(false);
        let (gone, kept) = (TenantId::generate(), TenantId::generate());
        for n in 0..3 {
            s3.tenant(gone)
                .put(VersionId::generate(), format!("blob {n}").as_bytes())
                .expect("put");
        }
        s3.tenant(kept)
            .put(VersionId::generate(), b"blob")
            .expect("put");
        assert_eq!(
            s3.tenants().expect("tenants").len(),
            2,
            "both tenants have objects"
        );
        s3.remove_tenant(gone).expect("removes");
        assert!(
            s3.tenant(gone).list().expect("list").is_empty(),
            "its prefix is empty"
        );
        assert_eq!(
            s3.tenants().expect("tenants"),
            [kept],
            "the other tenant is kept"
        );
    }

    #[test]
    fn a_versioned_bucket_is_refused_unless_allowed() {
        let secret = secret_file();
        let (_, mut versioned) = bucket(true);
        versioned.secret_access_key_file = Some(secret.1.clone());
        let data = tempfile::tempdir().expect("temp dir");
        let refused = place(&versioned, data.path()).expect_err("a versioned bucket is refused");
        assert!(
            refused.0.contains("storage.s3.allow_versioning"),
            "the refusal names the setting: {refused}"
        );
        versioned.allow_versioning = true;
        assert!(
            place(&versioned, data.path()).is_ok(),
            "allowed, it is used"
        );
        let (_, mut plain) = bucket(false);
        plain.secret_access_key_file = Some(secret.1.clone());
        let other = tempfile::tempdir().expect("temp dir");
        assert!(
            place(&plain, other.path()).is_ok(),
            "a bucket never versioned is used"
        );
    }

    /// The test service's secret in a file, as the configuration names it,
    /// in a directory removed when the first value is dropped.
    fn secret_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("s3.secret");
        std::fs::write(&path, format!("{}\n", var("VCON_TEST_S3_SECRET_KEY"))).expect("secret");
        (dir, path)
    }

    /// The registry in `dir`, under the master key in `dir/master.key`
    /// (made at first), with its tenants' objects in `s3`.
    fn tenants_in(dir: &std::path::Path, s3: &Arc<S3>) -> crate::tenants::Tenants {
        let file = dir.join("master.key");
        if !file.exists() {
            let key: [u8; 32] = std::array::from_fn(|_| rand_byte());
            std::fs::write(&file, key).expect("a master key file");
        }
        let master = Key::load_master(&file).expect("the master key");
        let data = dir.join("data");
        std::fs::create_dir_all(&data).expect("a data directory");
        crate::tenants::Tenants::open_in(
            &data,
            master,
            Arc::new(OsFs),
            TestClock::at(conformance::START),
            crate::backend::ObjectPlace::S3(s3.clone()),
        )
        .expect("the registry opens")
    }

    fn rand_byte() -> u8 {
        VersionId::generate().0[0]
    }

    #[test]
    fn a_tenant_keeps_its_objects_in_the_bucket_and_removal_empties_them() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let (found, body) = conformance::vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc02", "kept");
        tenants
            .store(id)
            .expect("its store")
            .create(&who, &found, &body)
            .expect("stored");
        assert_eq!(
            s3.tenant(id).list().expect("list").len(),
            1,
            "the tenant's version is an object under its own name in the bucket"
        );
        assert!(
            !dir.path()
                .join("data/tenants")
                .join(id.to_string())
                .join("objects")
                .exists(),
            "no objects directory is made beside the catalog"
        );
        tenants
            .remove(crate::audit::Actor::Cli, id, crate::tenants::Removal::Force)
            .expect("removed");
        assert!(
            s3.tenant(id).list().expect("list").is_empty(),
            "removing the tenant removes its objects"
        );
        assert!(s3.tenants().expect("tenants").is_empty(), "nothing is left");
    }

    #[test]
    fn a_backup_copies_the_objects_into_files_and_takes_no_claim() {
        let (s3, settings, _secret) = bucket_with_secret();
        let dir = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir_all(dir.path().join("data")).expect("a data directory");
        place(&settings, &dir.path().join("data")).expect("claims the prefix");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let (found, body) = conformance::vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc03", "kept");
        tenants
            .store(id)
            .expect("its store")
            .create(&who, &found, &body)
            .expect("stored");
        let to = dir.path().join("backup");
        tenants
            .backup(crate::audit::Actor::Cli, &to)
            .expect("backed up");
        let version = s3.tenant(id).list().expect("list");
        let copied = to
            .join("tenants")
            .join(id.to_string())
            .join("objects")
            .join(&version[0]);
        assert_eq!(
            std::fs::read(&copied).expect("the object is a file in the backup"),
            s3.tenant(id)
                .get(VersionId::parse(&version[0]).expect("a version id"))
                .expect("get"),
            "the file holds the object's bytes"
        );
        assert!(
            !to.join(claim::FILE).exists(),
            "a server restored from the backup does not take the bucket's claim"
        );
    }

    #[test]
    fn start_up_removes_the_objects_of_a_tenant_with_no_row() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        drop(tenants_in(dir.path(), &s3));
        let stray = TenantId::generate();
        s3.tenant(stray)
            .put(VersionId::generate(), b"a create cut off")
            .expect("put");
        drop(tenants_in(dir.path(), &s3));
        assert!(
            s3.tenants().expect("tenants").is_empty(),
            "start-up recovery removes objects no tenant row names"
        );
    }

    /// The settings for a new bucket, with the secret in a file.
    fn bucket_with_secret() -> (Arc<S3>, S3Settings, (tempfile::TempDir, std::path::PathBuf)) {
        let secret = secret_file();
        let (s3, mut settings) = bucket(false);
        settings.secret_access_key_file = Some(secret.1.clone());
        (s3, settings, secret)
    }

    #[test]
    fn a_second_data_directory_on_one_prefix_is_refused() {
        use std::os::unix::fs::PermissionsExt;

        let (s3, settings, _secret) = bucket_with_secret();
        let (first, second) = (
            tempfile::tempdir().expect("dir"),
            tempfile::tempdir().expect("dir"),
        );
        place(&settings, first.path()).expect("the first data directory claims the prefix");
        let id = claim::read_file(first.path())
            .expect("reads")
            .expect("the data directory holds its claim");
        assert_eq!(
            s3.read_claim().expect("reads"),
            Some(id.clone()),
            "the bucket holds the same claim"
        );
        let mode = std::fs::metadata(first.path().join(claim::FILE))
            .expect("the claim file")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the claim file is the server's alone");
        place(&settings, first.path()).expect("the claiming data directory starts again");
        let refused = place(&settings, second.path()).expect_err("another data directory");
        assert!(
            refused.0.contains("another data directory") && refused.0.contains(&settings.bucket),
            "the refusal says why and names the bucket: {refused}"
        );
        assert_eq!(
            claim::read_file(second.path()).expect("reads"),
            None,
            "a refused data directory claims nothing"
        );
        let mut other = settings.clone();
        other.prefix = "other/".to_owned();
        place(&other, second.path()).expect("a prefix of its own is used");
        assert_eq!(
            s3.read_claim().expect("reads"),
            Some(id),
            "the first prefix keeps its claim"
        );
    }

    #[test]
    fn a_wrong_secret_or_bucket_is_named_and_the_secret_never_shown() {
        let (_, mut settings, _secret) = bucket_with_secret();
        let data = tempfile::tempdir().expect("dir");
        let absent = S3Settings {
            bucket: format!("{}-absent", settings.bucket),
            ..settings.clone()
        };
        let refused = place(&absent, data.path()).expect_err("no such bucket");
        assert!(
            refused.0.contains("NoSuchBucket"),
            "the service's error code is given: {refused}"
        );
        let wrong = remote_file(b"not-the-secret\n");
        settings.secret_access_key_file = Some(wrong.1.clone());
        let refused = place(&settings, data.path()).expect_err("a wrong secret");
        assert!(
            refused.0.contains("SignatureDoesNotMatch"),
            "the service's error code is given: {refused}"
        );
        for secret in ["not-the-secret", &var("VCON_TEST_S3_SECRET_KEY")] {
            assert!(
                !refused.0.contains(secret),
                "the secret is shown: {refused}"
            );
        }
    }

    /// A file holding `text`, in a directory removed with the first value.
    fn remote_file(text: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("s3.secret");
        std::fs::write(&path, text).expect("writes the file");
        (dir, path)
    }

    #[test]
    fn a_prefix_holding_objects_and_no_claim_is_refused_and_kept() {
        let (s3, settings, _secret) = bucket_with_secret();
        let tenant = TenantId::generate();
        s3.tenant(tenant)
            .put(VersionId::generate(), b"another server's object")
            .expect("put");
        let data = tempfile::tempdir().expect("dir");
        let refused = place(&settings, data.path()).expect_err("unclaimed objects");
        assert!(refused.0.contains("no claim"), "{refused}");
        assert_eq!(
            s3.tenant(tenant).list().expect("list").len(),
            1,
            "nothing is removed"
        );
        assert_eq!(s3.read_claim().expect("reads"), None, "nothing is claimed");
    }

    #[test]
    fn a_claim_cut_off_after_its_first_step_is_finished() {
        let (s3, settings, _secret) = bucket_with_secret();
        let data = tempfile::tempdir().expect("dir");
        claim::write_file(data.path(), &claim::new_id()).expect("the first step");
        place(&settings, data.path()).expect("the claim is finished");
        assert_eq!(
            s3.read_claim().expect("reads"),
            claim::read_file(data.path()).expect("reads"),
            "the bucket holds the data directory's claim"
        );
    }

    #[test]
    fn a_claim_object_is_never_written_over() {
        let (s3, _) = bucket(false);
        let first = claim::new_id();
        s3.write_claim(&first).expect("the first claim");
        assert!(
            s3.write_claim(&claim::new_id()).is_err(),
            "a second claim is refused by the service"
        );
        assert_eq!(
            s3.read_claim().expect("reads"),
            Some(first),
            "the first claim is kept"
        );
    }

    #[test]
    fn no_object_holds_the_vcon_or_its_uuid() {
        let (s3, _) = bucket(false);
        let harness = LocalAndS3 {
            dir: tempfile::tempdir().expect("temp dir"),
            s3: s3.clone(),
        };
        let fresh = harness.fresh(TestClock::at(conformance::START));
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let uuid = "01a07da8-c2bb-83e5-b9a2-279e0d16bcee";
        let marker = "plain-subject-marker-5f1c";
        let (found, body) = conformance::vcon(uuid, marker);
        fresh.store.create(&who, &found, &body).expect("stored");
        let all = everything(&s3);
        assert_eq!(all.len(), 1, "one object for the one version");
        let uuid_bytes = *found.uuid.as_bytes();
        for (name, bytes) in &all {
            for needle in [marker.as_bytes(), uuid.as_bytes(), &uuid_bytes[..]] {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle),
                    "{name} holds a marker in plaintext"
                );
                assert!(
                    !name.as_bytes().windows(needle.len()).any(|w| w == needle),
                    "the name {name} shows the vCon"
                );
            }
        }
    }
}

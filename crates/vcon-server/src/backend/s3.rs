//! Sealed objects in an S3-compatible bucket (RustFS, MinIO, AWS S3): the
//! blob of each version, byte for byte as the local backend writes it, at
//! `<prefix>tenants/<tenant id>/objects/<version id>`. A PUT that returns
//! success is the durable write, and no partial object is ever visible,
//! so there are no temporary names. A tenant's pack files are under the
//! same tenant name, at `<prefix>tenants/<tenant id>/` and the name
//! `pack::name` gives, and are read by ranges.

use std::future::Future;
use std::sync::{Arc, mpsc};

use aws_sdk_s3::Client;
use aws_sdk_s3::config::http::HttpResponse;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{
    BehaviorVersion, Credentials, Region, RequestChecksumCalculation, ResponseChecksumValidation,
};
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{BucketVersioningStatus, Delete, ObjectIdentifier};
use aws_smithy_http_client::tls;
use zeroize::Zeroizing;

pub(crate) mod claim;

use super::{ObjectError, Objects, Packs};
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

/// Why Parquet storage may not use `bucket`, if it may not: its pack files
/// are rewritten and deleted, and a purge must remove a deleted file's
/// bytes, which a versioned bucket keeps as an earlier version and Object
/// Lock may refuse to delete. A bucket created with Object Lock also
/// reports versioning on; the lock is named, as the reason.
pub(crate) fn parquet_refusal(
    bucket: &str,
    versioning: Versioning,
    locked: bool,
) -> Option<String> {
    if locked {
        return Some(format!(
            "bucket {bucket:?} has Object Lock on, which can refuse to delete a file a \
             purge rewrote; Parquet storage (storage.parquet) needs a bucket without Object Lock"
        ));
    }
    (versioning != Versioning::Never).then(|| {
        format!(
            "bucket {bucket:?} has versioning {versioning:?}, which keeps a deleted file's \
             bytes as an earlier version; Parquet storage (storage.parquet) needs a bucket \
             that was never versioned, and storage.s3.allow_versioning does not apply"
        )
    })
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
    parquet: bool,
    data_dir: &std::path::Path,
) -> Result<super::ObjectPlace, StorageError> {
    place_for(settings, parquet, &claim::DataDir(data_dir))
}

/// [`place`], for catalogs that keep their claim of the prefix in `home`:
/// a data directory, or a database.
///
/// # Errors
///
/// As [`place`], and a `StorageError` naming what of `home` could not be
/// read or written.
pub(crate) fn place_for(
    settings: &S3Settings,
    parquet: bool,
    home: &dyn claim::Home,
) -> Result<super::ObjectPlace, StorageError> {
    let s3 = client_for(settings, "storage.s3")?;
    let unreadable = |e: ObjectError| {
        StorageError(format!(
            "bucket {:?} cannot be reached or read: {e}",
            settings.bucket
        ))
    };
    let versioning = s3.versioning().map_err(unreadable)?;
    if parquet {
        let locked = s3.object_lock().map_err(unreadable)?;
        if let Some(why) = parquet_refusal(&settings.bucket, versioning, locked) {
            return Err(StorageError(why));
        }
    } else if !versioning.allowed(settings.allow_versioning) {
        return Err(StorageError(format!(
            "bucket {:?} has versioning {versioning:?}, which keeps a removed object's \
             earlier versions; turn it off or set storage.s3.allow_versioning = true",
            settings.bucket
        )));
    }
    s3.claim_for(home)?;
    Ok(super::ObjectPlace::S3(s3))
}

/// A bucket's prefix as the bucket's side of a claim, with `place` naming
/// both in an error.
struct ClaimedPrefix<'a> {
    s3: &'a S3,
    place: &'a str,
}

impl claim::Prefix for ClaimedPrefix<'_> {
    fn found(&self) -> Result<(Option<String>, bool), StorageError> {
        let unreadable =
            |e: ObjectError| StorageError(format!("{} cannot be read: {e}", self.place));
        let claim = self.s3.read_claim().map_err(unreadable)?;
        let holds_tenants = !self.s3.tenants().map_err(unreadable)?.is_empty();
        Ok((claim, holds_tenants))
    }

    fn write(&self, id: &str) -> Result<(), StorageError> {
        self.s3
            .write_claim(id)
            .map_err(|e| StorageError(format!("{} cannot be claimed: {e}", self.place)))
    }
}

/// A client for the bucket `settings` names, with the secret access key
/// and the CA certificates read from the files `settings` names; an error
/// names the key of `table` at fault. Nothing is sent to the service.
///
/// # Errors
///
/// A `StorageError` naming the secret file, the CA file, or the client.
pub(crate) fn client_for(settings: &S3Settings, table: &str) -> Result<Arc<S3>, StorageError> {
    let secret = settings
        .secret_access_key_file
        .as_deref()
        .map(|path| read_secret(path, table))
        .transpose()?;
    let ca = settings
        .ca_file
        .as_deref()
        .map(|path| read_ca(path, table))
        .transpose()?;
    S3::connect(settings, secret.as_ref().map(|s| s.as_str()), ca.as_deref())
        .map_err(|e| StorageError(format!("the S3 client cannot start: {e}")))
}

/// The secret access key in `path`: one line, its line ending left out.
/// The file is held to the one secret-file rule (`crate::secret_file`). It
/// is read into a buffer of the file's size, so no shorter copy is left
/// behind by a reallocation, and every copy here is zeroed when dropped.
/// An error names the key and the file, never the text.
fn read_secret(path: &std::path::Path, table: &str) -> Result<Zeroizing<String>, StorageError> {
    let key = format!("{table}.secret_access_key_file");
    let refused = |why: &str| StorageError(format!("{key} {}: {why}", path.display()));
    let mut bytes = crate::secret_file::read(path, &key).map_err(StorageError)?;
    let text = String::from_utf8(std::mem::take(&mut *bytes)).map_err(|e| {
        zeroize::Zeroize::zeroize(&mut e.into_bytes());
        refused("the file is not UTF-8")
    })?;
    let text = Zeroizing::new(text);
    secret_line(&text).map_err(refused)
}

/// The one line a secret file holds, its line ending left out; the reason
/// when the text is empty or holds more than one line. The PostgreSQL
/// catalog's password file is held to the same rule.
pub(crate) fn secret_line(text: &str) -> Result<Zeroizing<String>, &'static str> {
    let line = text.trim_end_matches(['\r', '\n']);
    if line.is_empty() {
        return Err("the file is empty");
    }
    if line.contains(['\r', '\n']) {
        return Err("the file holds more than one line");
    }
    Ok(Zeroizing::new(line.to_owned()))
}

/// The PEM file in `path`, once each certificate in it has been parsed
/// and taken as a trust anchor; at least one must be there. The client's
/// TLS layer panics on a file that fails either step, so it is refused
/// here, by name. The PostgreSQL catalog's `ca_file` is read with it too.
pub(crate) fn read_ca(path: &std::path::Path, table: &str) -> Result<Vec<u8>, StorageError> {
    use rustls::pki_types::CertificateDer;
    use rustls::pki_types::pem::PemObject;

    let refused = |why: &str| StorageError(format!("{table}.ca_file {}: {why}", path.display()));
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
        let answer = self.wait(call)?.map_err(sdk_failed)?;
        Ok(match answer.status() {
            Some(BucketVersioningStatus::Enabled) => Versioning::Enabled,
            Some(BucketVersioningStatus::Suspended) => Versioning::Suspended,
            _ => Versioning::Never,
        })
    }

    /// Whether the bucket has Object Lock on, as
    /// GetObjectLockConfiguration reports it; a bucket with no lock
    /// configuration answers 404.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be reached or read.
    pub(crate) fn object_lock(&self) -> Result<bool, ObjectError> {
        let call = self
            .client
            .get_object_lock_configuration()
            .bucket(&self.bucket)
            .send();
        match self.wait(call)? {
            Ok(answer) => Ok(answer
                .object_lock_configuration()
                .and_then(|c| c.object_lock_enabled())
                .is_some_and(|e| *e == aws_sdk_s3::types::ObjectLockEnabled::Enabled)),
            Err(SdkError::ServiceError(e)) if e.raw().status().as_u16() == 404 => Ok(false),
            Err(e) => Err(sdk_failed(e)),
        }
    }

    /// Binds the prefix to the catalogs of `home`, as `claim` describes:
    /// uses it, claims it, or refuses.
    ///
    /// # Errors
    ///
    /// A `StorageError` naming the bucket and prefix, or the step that
    /// failed.
    fn claim_for(&self, home: &dyn claim::Home) -> Result<(), StorageError> {
        let place = format!("bucket {:?}, prefix {:?}", self.bucket, self.prefix);
        let prefix = ClaimedPrefix {
            s3: self,
            place: &place,
        };
        claim::settle(home, &prefix, &place).map(drop)
    }

    /// The claim object's name.
    fn claim_key(&self) -> String {
        format!("{}{}", self.prefix, claim::OBJECT)
    }

    /// The claim object's id under the prefix, if there is one.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be read or the object does
    /// not hold a claim id.
    pub(crate) fn read_claim(&self) -> Result<Option<String>, ObjectError> {
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
                Err(SdkError::ServiceError(e)) if e.err().is_no_such_key() => Ok(None),
                Err(e) => Err(sdk_failed(e)),
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
        self.wait(call)?.map(drop).map_err(sdk_failed)
    }

    /// Writes `bytes` as the object `name` under the prefix, only if there
    /// is none (`If-None-Match: *`). A refusal for that reason is
    /// `Ok(false)`, never a write under another name.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be reached or written.
    pub(crate) fn put_new(&self, name: &str, bytes: Vec<u8>) -> Result<bool, ObjectError> {
        let call = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(format!("{}{name}", self.prefix))
            .if_none_match("*")
            .body(ByteStream::from(bytes))
            .send();
        match self.wait(call)? {
            Ok(_) => Ok(true),
            Err(e) if e.raw_response().map(|r| r.status().as_u16()) == Some(412) => Ok(false),
            Err(e) => Err(failed(e)),
        }
    }

    /// The names directly under `dir` below the prefix, `dir` left out,
    /// sorted; a name that has more below it ends in `/`.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be listed.
    pub(crate) fn names_under(&self, dir: &str) -> Result<Vec<String>, ObjectError> {
        self.names(&format!("{}{dir}", self.prefix), Some("/"))
    }

    /// Removes the object `name` under the prefix; S3 answers success for
    /// one already gone.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be reached.
    pub(crate) fn remove_named(&self, name: &str) -> Result<(), ObjectError> {
        let call = self
            .client
            .delete_object()
            .bucket(&self.bucket)
            .key(format!("{}{name}", self.prefix))
            .send();
        self.wait(call)?.map(drop).map_err(failed)
    }

    /// The bucket and prefix, as `s3://bucket/prefix`.
    pub(crate) fn url(&self) -> String {
        format!("s3://{}/{}", self.bucket, self.prefix)
    }

    /// The name before every object of `tenant`.
    pub(crate) fn tenant_dir(&self, tenant: TenantId) -> String {
        tenant_dir(&self.prefix, tenant)
    }

    /// One tenant's pack files.
    pub(crate) fn packs(self: &Arc<S3>, tenant: TenantId) -> S3Packs {
        S3Packs {
            s3: self.clone(),
            dir: tenant_root(&self.prefix, tenant),
        }
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
                let page = page.map_err(sdk_failed)?;
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

    /// Removes every object and pack file of `tenant`, a thousand at a
    /// time.
    ///
    /// # Errors
    ///
    /// `ObjectError::Io` if the bucket cannot be listed or an object not
    /// removed.
    pub(crate) fn remove_tenant(&self, tenant: TenantId) -> Result<(), ObjectError> {
        let dir = tenant_root(&self.prefix, tenant);
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
            let answer = self.wait(call)?.map_err(sdk_failed)?;
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

/// The name before every object and pack file of `tenant`, under
/// `prefix`.
fn tenant_root(prefix: &str, tenant: TenantId) -> String {
    format!("{prefix}tenants/{tenant}/")
}

/// The name before every object of `tenant`, under `prefix`.
fn tenant_dir(prefix: &str, tenant: TenantId) -> String {
    format!("{}objects/", tenant_root(prefix, tenant))
}

/// `e` as an object error whose text holds its whole chain of causes: the
/// SDK's own text names only the step ("service error", "dispatch
/// failure"), and the cause (the service's error code, a refused
/// connection, an untrusted certificate) is in its sources. None of them
/// holds a credential. Its kind is that of the first I/O error in the
/// chain, or `Other`.
fn failed<E: std::error::Error + Send + Sync + 'static>(e: E) -> ObjectError {
    failed_as(cause_kind(&e), &e)
}

/// `failed` for a request the SDK sent, with the kind of failure it was,
/// named as the system names a file's, so that both backends report a
/// failure alike: no answer in time is `TimedOut`, a refused key (401 or
/// 403: `AccessDenied`, `InvalidAccessKeyId`, `SignatureDoesNotMatch`)
/// `PermissionDenied`, an answer that does not parse `InvalidData`, and a
/// connection that failed the system's own kind, such as
/// `ConnectionRefused`.
fn sdk_failed<E: std::error::Error + Send + Sync + 'static>(
    e: SdkError<E, HttpResponse>,
) -> ObjectError {
    use std::io::ErrorKind;
    let kind = match &e {
        SdkError::TimeoutError(_) => ErrorKind::TimedOut,
        SdkError::DispatchFailure(failure) if failure.is_timeout() => ErrorKind::TimedOut,
        SdkError::ServiceError(answer) if matches!(answer.raw().status().as_u16(), 401 | 403) => {
            ErrorKind::PermissionDenied
        }
        SdkError::ResponseError(_) => ErrorKind::InvalidData,
        _ => cause_kind(&e),
    };
    failed_as(kind, &e)
}

/// The kind of the first I/O error in `e`'s chain of causes, or `Other`.
fn cause_kind(e: &(dyn std::error::Error + 'static)) -> std::io::ErrorKind {
    let mut cause = Some(e);
    while let Some(next) = cause {
        if let Some(io) = next.downcast_ref::<std::io::Error>() {
            return io.kind();
        }
        cause = next.source();
    }
    std::io::ErrorKind::Other
}

/// An object error of `kind`, whose text is `e`'s whole chain of causes.
fn failed_as(kind: std::io::ErrorKind, e: &(dyn std::error::Error + 'static)) -> ObjectError {
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
    ObjectError::Io(std::io::Error::new(kind, text))
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
        self.s3.wait(call)?.map(drop).map_err(sdk_failed)
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
                Err(SdkError::ServiceError(e)) if e.err().is_no_such_key() => {
                    Err(ObjectError::Missing)
                }
                Err(e) => Err(sdk_failed(e)),
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
        self.s3.wait(call)?.map(drop).map_err(sdk_failed)
    }

    /// A DeleteObject is durable once it returns.
    fn sync(&self) -> Result<(), ObjectError> {
        Ok(())
    }
}

/// One tenant's pack files, under its own name in the bucket.
#[derive(Debug)]
pub(crate) struct S3Packs {
    s3: Arc<S3>,
    dir: String,
}

impl Packs for S3Packs {
    /// A ranged GetObject. A range that starts at or past the end of the
    /// file (416) reads as no bytes: the file ends sooner.
    fn read(&self, name: &str, start: u64, len: u64) -> Result<Vec<u8>, ObjectError> {
        let Some(last) = len.checked_sub(1).and_then(|n| start.checked_add(n)) else {
            return Ok(Vec::new());
        };
        let call = self
            .s3
            .client
            .get_object()
            .bucket(&self.s3.bucket)
            .key(format!("{}{name}", self.dir))
            .range(format!("bytes={start}-{last}"))
            .send();
        self.s3.wait(async move {
            match call.await {
                Ok(out) => out
                    .body
                    .collect()
                    .await
                    .map(|bytes| bytes.to_vec())
                    .map_err(failed),
                Err(SdkError::ServiceError(e)) if e.err().is_no_such_key() => {
                    Err(ObjectError::Missing)
                }
                Err(SdkError::ServiceError(e)) if e.raw().status().as_u16() == 416 => {
                    Ok(Vec::new())
                }
                Err(e) => Err(sdk_failed(e)),
            }
        })?
    }

    /// A HeadObject.
    fn size(&self, name: &str) -> Result<Option<u64>, ObjectError> {
        let call = self
            .s3
            .client
            .head_object()
            .bucket(&self.s3.bucket)
            .key(format!("{}{name}", self.dir))
            .send();
        match self.s3.wait(call)? {
            Ok(out) => {
                let length = out.content_length().unwrap_or(0);
                u64::try_from(length).map(Some).map_err(|_| {
                    ObjectError::Io(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "a negative length",
                    ))
                })
            }
            Err(SdkError::ServiceError(e)) if e.err().is_not_found() => Ok(None),
            Err(e) => Err(sdk_failed(e)),
        }
    }

    /// A PutObject with `If-None-Match: *`; a 412 is the name taken.
    fn put_new(&self, name: &str, bytes: &[u8]) -> Result<bool, ObjectError> {
        let call = self
            .s3
            .client
            .put_object()
            .bucket(&self.s3.bucket)
            .key(format!("{}{name}", self.dir))
            .if_none_match("*")
            .body(ByteStream::from(bytes.to_vec()))
            .send();
        match self.s3.wait(call)? {
            Ok(_) => Ok(true),
            Err(e) if e.raw_response().map(|r| r.status().as_u16()) == Some(412) => Ok(false),
            Err(e) => Err(sdk_failed(e)),
        }
    }

    /// A DeleteObject, which S3 answers with success for a name it does
    /// not hold.
    fn remove(&self, name: &str) -> Result<(), ObjectError> {
        let call = self
            .s3
            .client
            .delete_object()
            .bucket(&self.s3.bucket)
            .key(format!("{}{name}", self.dir))
            .send();
        self.s3.wait(call)?.map(drop).map_err(sdk_failed)
    }

    fn list(&self) -> Result<Vec<String>, ObjectError> {
        let names = self.s3.names(&format!("{}parquet/", self.dir), None)?;
        Ok(names.into_iter().map(|n| format!("parquet/{n}")).collect())
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

    /// A plain-HTTP service on the loopback address: to every request it
    /// gives `answer`, or with None it never answers and holds the
    /// connection open.
    pub(super) fn http_service(answer: Option<&'static str>) -> std::net::SocketAddr {
        use std::io::{BufRead, BufReader, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("binds");
        let address = listener.local_addr().expect("address");
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                    let mut writer = stream;
                    loop {
                        // A GET has no body: the request ends at its blank line.
                        let mut line = String::new();
                        loop {
                            line.clear();
                            match reader.read_line(&mut line) {
                                Ok(0) | Err(_) => return,
                                Ok(_) if line == "\r\n" => break,
                                Ok(_) => {}
                            }
                        }
                        let Some(answer) = answer else {
                            std::thread::sleep(Duration::from_secs(60));
                            return;
                        };
                        if writer.write_all(answer.as_bytes()).is_err() {
                            return;
                        }
                    }
                });
            }
        });
        address
    }

    /// RustFS 1.0.1's answer to a GetObject by a key that may not get
    /// objects, as captured from it.
    const ACCESS_DENIED: &str = "HTTP/1.1 403 Forbidden\r\ncontent-type: application/xml\r\n\
        content-length: 110\r\n\r\n<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error>\
        <Code>AccessDenied</Code><Message>Access Denied</Message></Error>";

    /// S3's answer for a key the bucket does not hold.
    const NO_SUCH_KEY: &str = "HTTP/1.1 404 Not Found\r\ncontent-type: application/xml\r\n\
        content-length: 127\r\n\r\n<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error>\
        <Code>NoSuchKey</Code><Message>The specified key does not exist.</Message></Error>";

    /// The example key pair of the AWS documentation.
    const KEY_ID: &str = "AKIAIOSFODNN7EXAMPLE";
    const SECRET: &str = "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY";

    /// A tenant's objects at `endpoint`, with the example keys, each call
    /// given `timeout` including its retries.
    fn objects_at(endpoint: String, timeout: Duration, tenant: TenantId) -> S3Objects {
        let settings = S3Settings {
            endpoint: Some(endpoint),
            access_key_id: Some(KEY_ID.to_owned()),
            timeout,
            ..unreachable()
        };
        S3::connect(&settings, Some(SECRET), None)
            .expect("a client")
            .tenant(tenant)
    }

    /// A read the service does not answer, or answers with a refusal, is a
    /// storage failure of the kind the system would give for a file: never
    /// a missing object, which is damage. Its text holds no credential.
    #[test]
    fn a_read_the_service_fails_is_a_failure_by_kind_never_missing() {
        // A refused connection is retried, so it is reported once every
        // try has failed, well within its 30 seconds.
        let cases = [
            (
                "http://127.0.0.1:9".to_owned(),
                Duration::from_secs(30),
                std::io::ErrorKind::ConnectionRefused,
                "nothing listens",
            ),
            (
                format!("http://{}", http_service(None)),
                Duration::from_secs(1),
                std::io::ErrorKind::TimedOut,
                "no answer within the timeout",
            ),
            (
                format!("http://{}", http_service(Some(ACCESS_DENIED))),
                Duration::from_secs(30),
                std::io::ErrorKind::PermissionDenied,
                "the key may not get objects",
            ),
        ];
        for (endpoint, timeout, kind, why) in cases {
            let objects = objects_at(endpoint, timeout, TenantId::generate());
            match objects.get(VersionId::generate()) {
                Err(ObjectError::Io(e)) => {
                    assert_eq!(e.kind(), kind, "{why}: {e}");
                    let text = e.to_string();
                    // Named by what each is, so a failure prints neither.
                    for (what, value) in [("the access key id", KEY_ID), ("the secret key", SECRET)]
                    {
                        assert!(
                            !text.contains(value),
                            "{why}: {what} is in the error's text"
                        );
                    }
                }
                Err(ObjectError::Missing) => panic!("{why}: missing"),
                Ok(_) => panic!("{why}: read"),
            }
        }
    }

    #[test]
    fn a_key_the_bucket_does_not_hold_is_missing() {
        let objects = objects_at(
            format!("http://{}", http_service(Some(NO_SUCH_KEY))),
            Duration::from_secs(30),
            TenantId::generate(),
        );
        assert!(
            matches!(
                objects.get(VersionId::generate()),
                Err(ObjectError::Missing)
            ),
            "NoSuchKey is a missing object"
        );
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
        secret_mode(&path, 0o600);
        (dir, path)
    }

    fn secret_mode(path: &std::path::Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    /// `unreachable()` with its secret in a file of `mode`.
    fn with_secret(mode: u32) -> (tempfile::TempDir, S3Settings) {
        let (dir, path) = secret_in(b"k3y-0123\n");
        secret_mode(&path, mode);
        let mut settings = unreachable();
        settings.access_key_id = Some("id".to_owned());
        settings.secret_access_key_file = Some(path);
        (dir, settings)
    }

    #[test]
    fn a_storage_secret_other_users_may_read_is_refused_with_the_fix() {
        let (_dir, settings) = with_secret(0o644);
        let path = settings.secret_access_key_file.clone().expect("set");
        let error = client_for(&settings, "storage.s3").expect_err("refused");
        assert!(
            error.0.starts_with(&format!(
                "storage.s3.secret_access_key_file {}: other users may read or change the file (mode 0644); run `chmod 600 {}`",
                path.display(),
                path.display()
            )),
            "{error}"
        );
        assert!(!error.0.contains("k3y"), "{error}");
    }

    #[test]
    fn a_storage_secret_its_group_may_write_is_refused() {
        let (_dir, settings) = with_secret(0o660);
        let error = client_for(&settings, "storage.s3").expect_err("refused");
        assert!(error.0.contains("(mode 0660)"), "{error}");
    }

    #[test]
    fn a_storage_secret_of_0604_is_refused() {
        let (_dir, settings) = with_secret(0o604);
        assert!(
            client_for(&settings, "storage.s3").is_err(),
            "others may read a 0604 secret"
        );
    }

    #[test]
    fn a_storage_secret_of_0600_or_0640_or_0400_makes_a_client() {
        for mode in [0o600, 0o640, 0o400] {
            let (_dir, settings) = with_secret(mode);
            assert!(client_for(&settings, "storage.s3").is_ok(), "{mode:o}");
        }
    }

    #[test]
    fn an_export_secret_is_refused_under_the_export_table() {
        let (_dir, settings) = with_secret(0o644);
        let error = client_for(&settings, "export.s3").expect_err("refused");
        assert!(
            error.0.starts_with("export.s3.secret_access_key_file "),
            "{error}"
        );
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
                read_secret(&path, "storage.s3").expect("reads").as_str(),
                "k3y with spaces",
                "{text:?}"
            );
        }
    }

    #[test]
    fn a_secret_file_that_is_empty_or_has_two_lines_is_refused_without_its_text() {
        for text in [&b""[..], b"\n", b"id-and\nk3y-in-one-file\n"] {
            let (_dir, path) = secret_in(text);
            let error = read_secret(&path, "storage.s3").expect_err("refused");
            assert!(
                error.0.contains("storage.s3.secret_access_key_file"),
                "the error names the key: {error}"
            );
            assert!(
                !error.0.contains("k3y"),
                "the error does not show the secret"
            );
        }
        let error = read_secret(std::path::Path::new("/nonexistent/s3.secret"), "storage.s3")
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
            let error = read_ca(&path, "storage.s3").expect_err(why);
            assert!(
                error.0.contains("storage.s3.ca_file"),
                "{why}: the error names the key: {error}"
            );
        }
        let (_dir, path) = secret_in(made.ca.as_bytes());
        assert!(
            read_ca(&path, "storage.s3").is_ok(),
            "a CA certificate is read"
        );
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

    /// Parquet storage takes only a bucket never versioned and without
    /// Object Lock; each refusal says which, and Object Lock is named even
    /// though a locked bucket also reports versioning on.
    #[test]
    fn parquet_storage_takes_only_a_bucket_that_keeps_no_deleted_bytes() {
        assert_eq!(
            parquet_refusal("vcon", Versioning::Never, false),
            None,
            "never versioned, not locked"
        );
        for state in [Versioning::Enabled, Versioning::Suspended] {
            let why = parquet_refusal("vcon", state, false).expect("refused");
            assert!(
                why.contains(&format!("{state:?}")) && why.contains("versioning"),
                "names the versioning state: {why}"
            );
            assert!(
                why.contains("storage.s3.allow_versioning does not apply"),
                "says the override does not apply: {why}"
            );
        }
        for state in [Versioning::Never, Versioning::Enabled] {
            let why = parquet_refusal("vcon", state, true).expect("refused");
            assert!(why.contains("Object Lock"), "names Object Lock: {why}");
            assert!(why.contains("\"vcon\""), "names the bucket: {why}");
        }
    }
}

/// Against a real S3-compatible service, whose address and keys come from
/// `VCON_TEST_S3_ENDPOINT`, `VCON_TEST_S3_ACCESS_KEY` and
/// `VCON_TEST_S3_SECRET_KEY`. Each test makes a bucket of its own. Built
/// only with the `backend-tests` feature; without the addresses every test
/// fails, so a run that cannot reach the service never passes by skipping.
#[cfg(all(test, feature = "backend-tests"))]
pub(crate) mod remote {
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
    pub(crate) fn bucket(versioned: bool) -> (Arc<S3>, S3Settings) {
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

    /// Opens a tenant's catalog under its catalog key; called once for a
    /// new tenant and again each time its Store is opened.
    pub(crate) type CatalogFor = Arc<dyn Fn(TenantId, Key) -> Box<dyn crate::backend::Catalog>>;

    /// A catalog from `catalog` with its objects in the bucket; `packed`:
    /// with Parquet storage, a pack run after every write.
    pub(crate) struct CatalogAndS3 {
        pub(crate) catalog: CatalogFor,
        pub(crate) s3: Arc<S3>,
        pub(crate) packed: bool,
    }

    /// Local catalogs, each in a directory of its tenant under `dir`.
    fn local_catalogs(dir: &tempfile::TempDir) -> CatalogFor {
        let dir = dir.path().to_owned();
        Arc::new(move |tenant, catalog_key| {
            let dir = dir.join(tenant.to_string());
            std::fs::create_dir_all(&dir).expect("tenant directory");
            Box::new(LocalCatalog::open(&dir.join("catalog.sqlite"), catalog_key).expect("catalog"))
        })
    }

    impl Harness for CatalogAndS3 {
        fn fresh(&self, clock: Arc<TestClock>) -> Fresh {
            let packed = self.packed;
            let tenant = TenantId::generate();
            let catalog_for = self.catalog.clone();
            let master = Key::generate().expect("master");
            let ctx = envelope::Context::Tenant { tenant };
            let wrapped = envelope::wrap(&master, &ctx, &Key::generate().expect("key"));
            let s3 = self.s3.clone();
            let objects = Arc::new(s3.tenant(tenant));
            let planted = self.s3.clone();
            // A client of a service that never answers, for `unreadable`.
            let silent = S3::connect(
                &S3Settings {
                    endpoint: Some(format!("http://{}", super::tests::http_service(None))),
                    timeout: Duration::from_secs(1),
                    ..settings("unanswered")
                },
                Some(&var("VCON_TEST_S3_SECRET_KEY")),
                None,
            )
            .expect("a client");
            let open = std::rc::Rc::new(move |s3: &Arc<S3>, recover: bool| {
                let key = envelope::unwrap(&master, &ctx, &wrapped).expect("unwraps");
                let catalog_key = envelope::catalog_key(&key).expect("catalog key");
                Store::open_on(
                    tenant,
                    key,
                    catalog_for(tenant, catalog_key),
                    Arc::new(s3.tenant(tenant)),
                    Some(Arc::new(s3.packs(tenant))),
                    Arc::new(OsFs),
                    clock.clone(),
                    recover,
                )
                .map(|mut store| {
                    if packed {
                        store.pack_after_every_write(crate::store::packer::PackPlan {
                            merge_below: 0,
                            ..crate::store::packer::PackPlan::with_limit(1 << 20)
                        });
                    }
                    store
                })
                .expect("opens")
            });
            let reopen = open.clone();
            let unreadable = open.clone();
            let plant: conformance::SharedPlant = Arc::new(move |name, bytes| {
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
            });
            let packs: Option<Arc<dyn Packs>> =
                packed.then(|| Arc::new(self.s3.packs(tenant)) as Arc<dyn Packs>);
            let (held, lose, alter) = conformance::blobs(objects, packs, plant.clone());
            Fresh {
                store: open(&s3, true),
                reopen: Box::new(move || reopen(&s3, true)),
                unreadable: Box::new(move || unreadable(&silent, false)),
                held,
                lose,
                alter,
                plant: Box::new(move |name, bytes| plant(name, bytes)),
            }
        }
    }

    #[test]
    fn a_local_catalog_with_s3_objects_conforms() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        conformance::run(&CatalogAndS3 {
            catalog: local_catalogs(&dir),
            s3,
            packed: false,
        });
    }

    /// 4.1: the same suite on Parquet storage in the bucket, every version
    /// packed after the write that adds it.
    #[test]
    fn a_local_catalog_with_parquet_storage_in_the_bucket_conforms() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        conformance::run(&CatalogAndS3 {
            catalog: local_catalogs(&dir),
            s3,
            packed: true,
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

    /// A read with a key the service refuses (one it does not know, or the
    /// right one signed with a wrong secret) is a refused read, the kind a
    /// file the process may not read gives, not a missing object; its
    /// text holds neither credential.
    #[test]
    fn a_read_with_a_refused_key_is_a_failure_not_missing() {
        let (s3, settings) = bucket(false);
        let tenant = TenantId::generate();
        let version = VersionId::generate();
        s3.tenant(tenant)
            .put(version, b"sealed bytes")
            .expect("put");
        let known = var("VCON_TEST_S3_ACCESS_KEY");
        for (id, secret, why) in [
            (
                "vcon-test-unknown-key",
                "an-unknown-secret",
                "a key it does not know",
            ),
            (known.as_str(), "not-the-secret", "a wrong secret"),
        ] {
            let refused = S3::connect(
                &S3Settings {
                    access_key_id: Some(id.to_owned()),
                    ..settings.clone()
                },
                Some(secret),
                None,
            )
            .expect("a client");
            match refused.tenant(tenant).get(version) {
                Err(ObjectError::Io(e)) => {
                    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied, "{why}: {e}");
                    let text = e.to_string();
                    // A failure names the credential, never its value.
                    assert!(
                        !text.contains(secret),
                        "{why}: the refused secret is in the text"
                    );
                    assert!(
                        !text.contains(&var("VCON_TEST_S3_SECRET_KEY")),
                        "{why}: the service's secret is in the text"
                    );
                }
                Err(ObjectError::Missing) => panic!("{why}: missing"),
                Ok(_) => panic!("{why}: read"),
            }
        }
    }

    #[test]
    fn recovery_removes_an_object_no_row_names() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let harness = CatalogAndS3 {
            catalog: local_catalogs(&dir),
            s3: s3.clone(),
            packed: false,
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

    /// A pack file is read by byte ranges and its length by HeadObject. A
    /// range past its end reads what there is, one from its end reads no
    /// bytes, and a name the bucket does not hold is Missing for a read and
    /// no length for HeadObject. The file is under its tenant's name.
    #[test]
    fn a_pack_file_is_read_by_ranges_and_its_length_by_head() {
        let (s3, _) = bucket(false);
        let tenant = TenantId::generate();
        let packs = s3.packs(tenant);
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let name = crate::backend::pack::name("2026-11-06", crate::ids::FileId::generate());
        assert!(
            s3.put_new(&format!("tenants/{tenant}/{name}"), bytes.clone())
                .expect("written"),
            "a new name"
        );
        let read = |start, len| packs.read(&name, start, len).expect("reads");
        assert_eq!(read(0, 10), bytes[..10], "the first bytes");
        assert_eq!(read(2048, 300), bytes[2048..2348], "bytes inside");
        assert_eq!(read(4990, 10), bytes[4990..], "the last bytes");
        assert_eq!(read(4995, 10), bytes[4995..], "past the end: what there is");
        assert_eq!(read(5000, 10), Vec::<u8>::new(), "from the end: none");
        assert_eq!(read(7, 0), Vec::<u8>::new(), "no bytes asked");
        assert_eq!(
            packs.size(&name).expect("a length"),
            Some(5000),
            "HeadObject gives the file's length"
        );
        let absent = crate::backend::pack::name("2026-11-06", crate::ids::FileId::generate());
        assert!(
            matches!(packs.read(&absent, 0, 8), Err(ObjectError::Missing)),
            "a name not held is Missing"
        );
        assert_eq!(packs.size(&absent).expect("answers"), None, "no length");
        let names: Vec<String> = everything(&s3).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [format!("test/tenants/{tenant}/{name}")],
            "where it is"
        );
    }

    /// A Store over the bucket reads each packed version from its row in a
    /// pack file in the bucket, byte for byte, and `check` finds nothing
    /// missing; no object is left.
    #[test]
    fn a_packed_version_reads_back_from_the_bucket() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let harness = CatalogAndS3 {
            catalog: local_catalogs(&dir),
            s3: s3.clone(),
            packed: false,
        };
        let fresh = harness.fresh(TestClock::at(conformance::START));
        let caller = Caller {
            key: crate::ids::KeyId::generate(),
        };
        let mut kept = Vec::new();
        for n in 0..5u8 {
            let uuid = format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}");
            let (found, body) = conformance::vcon(&uuid, &"subject ".repeat(usize::from(n) * 9));
            let Ok(crate::store::Created::New(version)) =
                fresh.store.create(&caller, &found, &body)
            else {
                panic!("a new vCon")
            };
            kept.push((found.uuid, version, body));
        }
        let tenant = fresh.store.tenant();
        let put = |name: &str, bytes: &[u8]| {
            assert!(
                s3.put_new(&format!("tenants/{tenant}/{name}"), bytes.to_vec())
                    .expect("written"),
                "a new name"
            );
        };
        let versions: Vec<_> = kept.iter().map(|(_, v, _)| *v).collect();
        let (_, _, places) =
            crate::store::packed::tests::pack_with(&fresh.store, &put, &versions, 600);
        assert!(places.iter().any(|p| p.group > 0), "several row groups");
        for store in [&fresh.store, &(fresh.reopen)()] {
            for (uuid, version, body) in &kept {
                let got = store.get(&caller, *uuid).expect("reads");
                assert_eq!(got.version, *version, "the current version");
                assert_eq!(&got.body, body, "byte for byte");
            }
            assert_eq!(store.check().expect("checked"), [], "nothing missing");
        }
        let names: Vec<String> = everything(&s3).into_iter().map(|(n, _)| n).collect();
        assert!(
            names.iter().all(|n| !n.contains("/objects/")),
            "no object is left: {names:?}"
        );
        assert_eq!(
            names.iter().filter(|n| n.ends_with(".parquet")).count(),
            1,
            "one pack file: {names:?}"
        );
    }

    /// A tenant's Store, opened by the registry as a server opens it, reads
    /// its packed versions from the bucket.
    #[test]
    fn a_tenants_store_reads_its_packed_versions_from_the_bucket() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let (found, body) = conformance::vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc03", "packed");
        let store = tenants.store(id).expect("its store");
        let Ok(crate::store::Created::New(version)) = store.create(&who, &found, &body) else {
            panic!("a new vCon")
        };
        let put = |name: &str, bytes: &[u8]| {
            assert!(
                s3.put_new(&format!("tenants/{id}/{name}"), bytes.to_vec())
                    .expect("written"),
                "a new name"
            );
        };
        crate::store::packed::tests::pack_with(&store, &put, &[version], usize::MAX);
        assert!(s3.tenant(id).list().expect("list").is_empty(), "no object");
        assert_eq!(
            store.get(&who, found.uuid).expect("reads").body,
            body,
            "read from the pack file, byte for byte"
        );
    }

    /// Every key ListObjectVersions gives for the bucket: each version and
    /// each delete marker of every key.
    fn every_version_key(s3: &S3) -> Vec<String> {
        let client = s3.client.clone();
        let bucket = s3.bucket.clone();
        s3.wait(async move {
            let mut keys = Vec::new();
            let (mut key_marker, mut version_marker) = (None::<String>, None::<String>);
            loop {
                let page = client
                    .list_object_versions()
                    .bucket(&bucket)
                    .set_key_marker(key_marker.take())
                    .set_version_id_marker(version_marker.take())
                    .send()
                    .await
                    .expect("lists the versions");
                keys.extend(
                    page.versions()
                        .iter()
                        .filter_map(|v| v.key())
                        .map(str::to_owned),
                );
                keys.extend(
                    page.delete_markers()
                        .iter()
                        .filter_map(|m| m.key())
                        .map(str::to_owned),
                );
                if !page.is_truncated().unwrap_or(false) {
                    break keys;
                }
                key_marker = page.next_key_marker().map(str::to_owned);
                version_marker = page.next_version_id_marker().map(str::to_owned);
            }
        })
        .expect("waits")
    }

    /// 4.6: once `purge_due` returns, the bucket holds no key of the file
    /// that held the purged row, by ListObjectsV2 and by
    /// ListObjectVersions, HeadObject on it answers 404, and no object in
    /// the bucket holds the purged vCon's sealed bytes, its uuid (as text
    /// or its 16 bytes) or its subject. The other versions read back.
    #[test]
    fn a_purge_leaves_no_byte_of_the_purged_vcon_in_the_bucket() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let store = tenants.store(id).expect("its store");
        let mut kept = Vec::new();
        for n in 0..3u8 {
            let subject = format!("PURGE-SUBJECT-{n}-{}", "y".repeat(30));
            let (found, body) =
                conformance::vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc2{n}"), &subject);
            let Ok(crate::store::Created::New(version)) = store.create(&who, &found, &body) else {
                panic!("a new vCon")
            };
            let sealed = s3.tenant(id).get(version).expect("its object");
            kept.push((found.uuid, body, subject, sealed));
        }
        store
            .pack(&crate::store::packer::PackPlan::with_limit(1 << 20))
            .expect("packs");
        let files: Vec<String> = everything(&s3)
            .into_iter()
            .map(|(n, _)| n)
            .filter(|n| n.ends_with(".parquet"))
            .collect();
        assert_eq!(files.len(), 1, "one pack file: {files:?}");
        let old = files[0].clone();
        let (uuid, _, subject, sealed) = kept[0].clone();
        store
            .delete(&who, uuid, crate::retention::Retention::Days(0))
            .expect("deleted, and purged at once");
        let names: Vec<String> = everything(&s3).into_iter().map(|(n, _)| n).collect();
        assert!(
            !names.contains(&old),
            "ListObjectsV2 has no key of the old file"
        );
        assert!(
            !every_version_key(&s3).contains(&old),
            "ListObjectVersions has no version or marker of the old file"
        );
        let call = s3.client.head_object().bucket(&s3.bucket).key(&old).send();
        let head = s3.wait(call).expect("waits");
        assert_eq!(
            head.err()
                .and_then(|e| e.raw_response().map(|r| r.status().as_u16())),
            Some(404),
            "HeadObject on the old file answers 404"
        );
        for (name, bytes) in everything(&s3) {
            let holds = |needle: &[u8]| bytes.windows(needle.len()).any(|w| w == needle);
            assert!(!holds(&sealed), "{name} holds the purged sealed bytes");
            assert!(!holds(uuid.to_string().as_bytes()), "{name} holds the uuid");
            assert!(!holds(uuid.as_bytes()), "{name} holds the uuid's bytes");
            assert!(!holds(subject.as_bytes()), "{name} holds the subject");
        }
        for (n, (uuid, body, _, _)) in kept.iter().enumerate().skip(1) {
            assert_eq!(
                &store.get(&who, *uuid).expect("reads").body,
                body,
                "version {n} reads back"
            );
        }
    }

    /// The server's pack run packs every tenant's objects in the bucket and
    /// counts what it did.
    #[test]
    fn the_servers_pack_run_packs_every_tenant() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let mut ids = Vec::new();
        for (n, name) in ["acme", "globex"].into_iter().enumerate() {
            let id = tenants
                .create(crate::audit::Actor::Cli, name, None)
                .expect("a tenant");
            for m in 0..2 {
                let (found, body) = conformance::vcon(
                    &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc3{}", n * 2 + m),
                    "kept",
                );
                tenants
                    .store(id)
                    .expect("its store")
                    .create(&who, &found, &body)
                    .expect("stored");
            }
            ids.push(id);
        }
        let report = crate::serve::pack_all(
            &tenants,
            &crate::config::ParquetSettings {
                pack_interval: std::time::Duration::from_secs(300),
                pack_limit: 1 << 20,
                reader_grace: crate::config::READER_GRACE,
            },
        );
        assert_eq!(
            report,
            crate::serve::PackRunReport {
                files: 2,
                versions: 4,
                failed: 0
            },
            "one file per tenant"
        );
        for id in ids {
            assert!(
                s3.tenant(id).list().expect("list").is_empty(),
                "no object left"
            );
        }
    }

    /// 4.11: DuckDB, with httpfs on the test service, reading the files the
    /// newest manifest names and keeping the rows it lists as live, sees
    /// exactly the versions the Store serves, after a create, a replace, a
    /// delete, a restore and a purge, each followed by a pack run.
    #[test]
    fn duckdb_through_the_newest_manifest_sees_exactly_the_live_versions() {
        use crate::export::tests::duckdb::query;
        let (s3, settings) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let store = tenants.store(id).expect("its store");
        let plan = crate::store::packer::PackPlan::with_limit(1 << 20);
        let uuids = [
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc51",
            "01a07da8-c2bb-83e5-b9a2-279e0d16bc52",
        ];
        let endpoint = var("VCON_TEST_S3_ENDPOINT");
        let host = endpoint.trim_start_matches("http://");
        let root = format!("s3://{}/{}tenants/{id}/", settings.bucket, settings.prefix);
        let seen = |step: &str| {
            let names: Vec<String> = everything(&s3)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.contains(&format!("tenants/{id}/manifest/")))
                .collect();
            let newest = names.iter().max().expect("a manifest");
            let manifest = format!("s3://{}/{newest}", settings.bucket);
            let sql = format!(
                "INSTALL httpfs; LOAD httpfs; \
                 SET s3_region = 'us-east-1'; SET s3_access_key_id = '{key}'; \
                 SET s3_secret_access_key = '{secret}'; SET s3_endpoint = '{host}'; \
                 SET s3_url_style = 'path'; SET s3_use_ssl = false; \
                 SET VARIABLE files = (SELECT list('{root}' || f.path) FROM \
                   (SELECT unnest(files) AS f FROM read_json('{manifest}'))); \
                 SET VARIABLE live = (SELECT live FROM read_json('{manifest}')); \
                 SELECT replace(version::VARCHAR, '-', '') AS v FROM read_parquet(getvariable('files')) \
                 WHERE list_contains(getvariable('live'), replace(version::VARCHAR, '-', '')) ORDER BY v;",
                key = var("VCON_TEST_S3_ACCESS_KEY"),
                secret = var("VCON_TEST_S3_SECRET_KEY"),
            );
            let rows = query(&sql);
            let mut got: Vec<String> = rows
                .iter()
                .filter_map(|r| r["v"].as_str().map(str::to_owned))
                .collect();
            got.sort();
            let mut want: Vec<String> = uuids
                .iter()
                .filter_map(|u| store.get(&who, u.parse().expect("uuid")).ok())
                .map(|s| s.version.to_string())
                .collect();
            want.sort();
            assert_eq!(got, want, "{step}: DuckDB sees exactly the live versions");
        };
        for (n, u) in uuids.iter().enumerate() {
            let (found, body) = conformance::vcon(u, &format!("first {n}"));
            store.create(&who, &found, &body).expect("stored");
        }
        store.pack(&plan).expect("packs");
        seen("created");
        let current = store
            .get(&who, uuids[0].parse().expect("uuid"))
            .expect("reads");
        let (found, body) = conformance::vcon(uuids[0], "second");
        store
            .replace(&who, current.version, &found, &body)
            .expect("replaced");
        store.pack(&plan).expect("packs");
        seen("replaced");
        store
            .delete(
                &who,
                uuids[1].parse().expect("uuid"),
                crate::retention::Retention::Years(7),
            )
            .expect("deleted");
        store.pack(&plan).expect("packs");
        seen("deleted");
        store
            .restore(&who, uuids[1].parse().expect("uuid"))
            .expect("restored");
        store.pack(&plan).expect("packs");
        seen("restored");
        store
            .delete(
                &who,
                uuids[0].parse().expect("uuid"),
                crate::retention::Retention::Days(0),
            )
            .expect("deleted and purged");
        seen("purged");
    }

    /// 4.13: removing a tenant whose versions are packed leaves nothing in
    /// the bucket under the tenant's name: no object, no pack file.
    #[test]
    fn a_removed_tenant_leaves_no_pack_file_in_the_bucket() {
        let (s3, _) = bucket(false);
        let dir = tempfile::tempdir().expect("temp dir");
        let tenants = tenants_in(dir.path(), &s3);
        let id = tenants
            .create(crate::audit::Actor::Cli, "acme", None)
            .expect("a tenant");
        let who = Caller {
            key: Key::generate().expect("key").id(),
        };
        let store = tenants.store(id).expect("its store");
        for n in 0..2u8 {
            let (found, body) =
                conformance::vcon(&format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc1{n}"), "kept");
            store.create(&who, &found, &body).expect("stored");
        }
        store
            .pack(&crate::store::packer::PackPlan::with_limit(1 << 20))
            .expect("packs");
        let held = |s3: &S3| -> Vec<String> {
            everything(s3)
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.contains(&format!("tenants/{id}/")))
                .collect()
        };
        assert!(
            held(&s3).iter().any(|n| n.ends_with(".parquet")),
            "a pack file is there: {:?}",
            held(&s3)
        );
        drop(store);
        tenants
            .remove(crate::audit::Actor::Cli, id, crate::tenants::Removal::Force)
            .expect("removed");
        assert_eq!(held(&s3), Vec::<String>::new(), "nothing under the tenant");
        assert!(
            s3.tenants().expect("tenants").is_empty(),
            "no tenant is listed"
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
        let refused =
            place(&versioned, false, data.path()).expect_err("a versioned bucket is refused");
        assert!(
            refused.0.contains("storage.s3.allow_versioning"),
            "the refusal names the setting: {refused}"
        );
        versioned.allow_versioning = true;
        assert!(
            place(&versioned, false, data.path()).is_ok(),
            "allowed, it is used"
        );
        let (_, mut plain) = bucket(false);
        plain.secret_access_key_file = Some(secret.1.clone());
        let other = tempfile::tempdir().expect("temp dir");
        assert!(
            place(&plain, false, other.path()).is_ok(),
            "a bucket never versioned is used"
        );
    }

    /// With Parquet storage, a versioned bucket is refused even with
    /// `allow_versioning`, and a bucket with Object Lock is refused, each
    /// with its own message; a plain bucket is used. Without Parquet
    /// storage the versioned bucket is used when allowed, as before.
    #[test]
    fn parquet_storage_refuses_a_versioned_or_locked_bucket() {
        let secret = secret_file();
        let (_, mut versioned) = bucket(true);
        versioned.secret_access_key_file = Some(secret.1.clone());
        versioned.allow_versioning = true;
        let data = tempfile::tempdir().expect("temp dir");
        let refused = place(&versioned, true, data.path()).expect_err("refused");
        assert!(
            refused.0.contains("versioning Enabled")
                && refused.0.contains("allow_versioning does not apply"),
            "the versioning refusal: {refused}"
        );
        assert!(
            place(&versioned, false, data.path()).is_ok(),
            "without Parquet storage, allowed versioning is used"
        );
        let (s3, mut locked) = bucket(false);
        let name = format!("{}-lock", locked.bucket)
            .chars()
            .take(50)
            .collect::<String>();
        let call = s3
            .client
            .create_bucket()
            .bucket(&name)
            .object_lock_enabled_for_bucket(true)
            .send();
        s3.wait(call)
            .expect("waits")
            .expect("a locked bucket is made");
        locked.bucket = name;
        locked.secret_access_key_file = Some(secret.1.clone());
        locked.allow_versioning = true;
        let other = tempfile::tempdir().expect("temp dir");
        let refused = place(&locked, true, other.path()).expect_err("refused");
        assert!(
            refused.0.contains("Object Lock"),
            "the Object Lock refusal: {refused}"
        );
        let (_, mut plain) = bucket(false);
        plain.secret_access_key_file = Some(secret.1.clone());
        let third = tempfile::tempdir().expect("temp dir");
        assert!(
            place(&plain, true, third.path()).is_ok(),
            "a plain bucket is used"
        );
    }

    /// The test service's secret in a file, as the configuration names it,
    /// in a directory removed when the first value is dropped.
    pub(crate) fn secret_file() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("s3.secret");
        std::fs::write(&path, format!("{}\n", var("VCON_TEST_S3_SECRET_KEY"))).expect("secret");
        std::fs::set_permissions(
            &path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
        (dir, path)
    }

    /// The registry in `dir`, under the master key in `dir/master.key`
    /// (made at first), with its tenants' objects in `s3`.
    fn tenants_in(dir: &std::path::Path, s3: &Arc<S3>) -> crate::tenants::Tenants {
        let file = dir.join("master.key");
        if !file.exists() {
            let key: [u8; 32] = std::array::from_fn(|_| rand_byte());
            std::fs::write(&file, key).expect("a master key file");
            std::fs::set_permissions(
                &file,
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
            )
            .expect("chmod 0600");
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
            &crate::backend::Catalogs::Local,
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
        place(&settings, false, &dir.path().join("data")).expect("claims the prefix");
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
        place(&settings, false, first.path()).expect("the first data directory claims the prefix");
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
        place(&settings, false, first.path()).expect("the claiming data directory starts again");
        let refused = place(&settings, false, second.path()).expect_err("another data directory");
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
        place(&other, false, second.path()).expect("a prefix of its own is used");
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
        let refused = place(&absent, false, data.path()).expect_err("no such bucket");
        assert!(
            refused.0.contains("NoSuchBucket"),
            "the service's error code is given: {refused}"
        );
        let wrong = remote_file(b"not-the-secret\n");
        settings.secret_access_key_file = Some(wrong.1.clone());
        let refused = place(&settings, false, data.path()).expect_err("a wrong secret");
        assert!(
            refused.0.contains("SignatureDoesNotMatch"),
            "the service's error code is given: {refused}"
        );
        for secret in ["not-the-secret", &var("VCON_TEST_S3_SECRET_KEY")] {
            assert!(
                !refused.0.contains(secret),
                "the refusal does not show a secret"
            );
        }
    }

    /// A file holding `text`, in a directory removed with the first value.
    fn remote_file(text: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("s3.secret");
        std::fs::write(&path, text).expect("writes the file");
        std::fs::set_permissions(
            &path,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
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
        let refused = place(&settings, false, data.path()).expect_err("unclaimed objects");
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
        place(&settings, false, data.path()).expect("the claim is finished");
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
        let dir = tempfile::tempdir().expect("temp dir");
        let harness = CatalogAndS3 {
            catalog: local_catalogs(&dir),
            s3: s3.clone(),
            packed: false,
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

    /// An export target for the bucket of `settings` under `prefix`, its
    /// secret read from a file as the command line reads it; the file is
    /// removed with the directory returned.
    fn export_target(
        settings: S3Settings,
        prefix: &str,
    ) -> (tempfile::TempDir, crate::export::s3::S3Target) {
        let dir = tempfile::tempdir().expect("temp dir");
        let secret = dir.path().join("export.secret");
        std::fs::write(&secret, var("VCON_TEST_S3_SECRET_KEY")).expect("the secret file");
        std::fs::set_permissions(
            &secret,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o600),
        )
        .expect("chmod 0600");
        let target = crate::export::s3::S3Target::connect(
            &S3Settings {
                prefix: prefix.to_owned(),
                secret_access_key_file: Some(secret),
                ..settings
            },
            "export.s3",
        )
        .expect("a target");
        (dir, target)
    }

    /// A store with three vCons on the local backend, exported to the
    /// bucket under `test/` at `level`, through `target`.
    fn exported_to(
        target: &dyn crate::export::Target,
        level: crate::export::Metadata,
    ) -> (
        crate::store::tests::Fixture,
        Result<crate::export::Exported, crate::export::ExportError>,
    ) {
        use crate::clock::Clock;
        let f = crate::store::tests::fixture();
        for n in 0..3 {
            let (found, body) = crate::store::testing::vcon(
                &format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}"),
                &format!("export-subject-marker-{n}"),
            );
            f.store.create(&f.caller, &found, &body).expect("stored");
        }
        let plan = crate::export::Plan {
            level,
            since: None,
            until: None,
            sizes: crate::export::Sizes {
                row_group: 1,
                file: 1,
            },
            limits: vcon::read::Limits::new(1 << 20),
        };
        let done = crate::export::export(&f.store, &plan, target, f.clock.now());
        (f, done)
    }

    /// 3.8 and the bucket's half of the manifest rule: an export to a
    /// bucket names each file it wrote, byte for byte, and no other; a name
    /// already there is refused with `If-None-Match: *`, never replaced;
    /// at level `none` no object holds a uuid or subject.
    #[test]
    fn an_export_to_a_bucket_is_whole_and_never_replaces_an_object() {
        use crate::export::{Metadata, Target, TargetError};
        let (_, settings) = bucket(false);
        let (_secret, target) = export_target(settings, "exports/");
        let (f, done) = exported_to(&target, Metadata::None);
        let done = done.expect("exports");
        assert_eq!(done.files.len(), 3, "a file a row");
        let all: std::collections::BTreeMap<String, Vec<u8>> =
            everything(target.client()).into_iter().collect();
        let mut expected: Vec<String> = done
            .files
            .iter()
            .map(|e| format!("exports/{}", e.path))
            .collect();
        expected.push(format!("exports/{}", done.manifest));
        expected.sort();
        assert_eq!(
            all.keys().cloned().collect::<Vec<_>>(),
            expected,
            "the files and the manifest, under the prefix"
        );
        for entry in &done.files {
            let bytes = &all[&format!("exports/{}", entry.path)];
            assert_eq!(bytes.len() as u64, entry.bytes, "{}", entry.path);
            assert!(
                bytes.starts_with(b"PAR1") && bytes.ends_with(b"PAR1"),
                "each object is a whole Parquet file"
            );
        }
        let manifest: serde_json::Value =
            serde_json::from_slice(&all[&format!("exports/{}", done.manifest)]).expect("JSON");
        assert_eq!(
            manifest["rows"], 3,
            "the manifest counts the three versions"
        );
        for n in 0..3 {
            let marker = format!("export-subject-marker-{n}");
            let uuid: uuid::Uuid = format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc4{n}")
                .parse()
                .expect("uuid");
            for (name, bytes) in &all {
                for needle in [marker.as_bytes(), &uuid.as_bytes()[..]] {
                    assert!(
                        !bytes.windows(needle.len()).any(|w| w == needle),
                        "{name} holds vCon {n} in clear"
                    );
                }
            }
        }
        assert_eq!(
            target.put_new(&done.manifest, b"another"),
            Err(TargetError::Exists),
            "a taken name is refused"
        );
        assert_eq!(
            everything(target.client())
                .into_iter()
                .find(|(name, _)| name.ends_with(&done.manifest))
                .map(|(_, bytes)| bytes),
            Some(all[&format!("exports/{}", done.manifest)].clone()),
            "and the manifest is unchanged"
        );
        let second = exported_to(&target, Metadata::None).1;
        let _ = f;
        assert!(
            second.is_ok(),
            "a second export of another store: {second:?}"
        );
    }

    /// 3.8: an export cut off between its file PUTs and its manifest PUT
    /// leaves no manifest, so none names a missing file; a run that fails
    /// removes its files, and one whose process died leaves only files no
    /// manifest names.
    #[test]
    fn an_export_cut_off_before_its_manifest_leaves_no_manifest() {
        use crate::export::{Metadata, Target, TargetError};
        /// The target, failing at the manifest, and with `dead` at every
        /// call after it too, as a killed process would.
        struct CutOff<'a> {
            inner: &'a dyn Target,
            dead: bool,
            failed: std::cell::Cell<bool>,
        }
        impl Target for CutOff<'_> {
            fn put_new(&self, path: &str, bytes: &[u8]) -> Result<(), TargetError> {
                if path.contains("/manifest/") {
                    self.failed.set(true);
                    return Err(TargetError::Io("the process has died".to_owned()));
                }
                self.inner.put_new(path, bytes)
            }
            fn list(&self, dir: &str) -> Result<Vec<String>, TargetError> {
                self.inner.list(dir)
            }
            fn remove(&self, path: &str) -> Result<(), TargetError> {
                if self.dead && self.failed.get() {
                    return Err(TargetError::Io("the process has died".to_owned()));
                }
                self.inner.remove(path)
            }
            fn describe(&self) -> String {
                self.inner.describe()
            }
        }
        for dead in [false, true] {
            let (_, settings) = bucket(false);
            let (_secret, target) = export_target(settings, "test/");
            let cut = CutOff {
                inner: &target,
                dead,
                failed: std::cell::Cell::new(false),
            };
            let (_, done) = exported_to(&cut, Metadata::Counts);
            assert!(
                matches!(done, Err(crate::export::ExportError::Target(_))),
                "{done:?}"
            );
            let names: Vec<String> = everything(target.client())
                .into_iter()
                .map(|(name, _)| name)
                .collect();
            assert!(
                !names.iter().any(|n| n.contains("/manifest/")),
                "dead {dead}: no manifest: {names:?}"
            );
            assert_eq!(
                names.len(),
                if dead { 3 } else { 0 },
                "dead {dead}: files left: {names:?}"
            );
        }
    }
}

//! The HTTP API, served over TLS only.

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::body::Bytes;
use axum::extract::{MatchedPath, Path as UriPath, Request, State};
use axum::http::header::{
    AUTHORIZATION, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_TYPE, ETAG, IF_MATCH, LOCATION,
    WWW_AUTHENTICATE,
};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use http_body_util::LengthLimitError;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto;
use hyper_util::service::TowerToHyperService;
use rustls::ServerConfig;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::de::{Deserialize, DeserializeOwned, Deserializer, Error as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::access::{self, AuthError, Caller, Scope, Scopes};
use crate::audit::{Action, Actor, AuditQuery, Entry};
use crate::ids::TenantId;
use crate::ids::{KeyId, VersionId};
use crate::retention::Retention;
use crate::store::{Caller as StoreCaller, Created, ListQuery, Store, StoreError};
use crate::tenants::{KeyInfo, Removal, TenantError, TenantInfo, Tenants};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;
use vcon::{Inspection, Rejection};

/// The limits from configuration that the API applies to every request.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// The most bytes a request body may hold, as received.
    pub body: usize,
    /// The most bytes gzip inflation may produce while a body is inspected.
    pub inflated: u64,
    /// How long a request body may take to arrive.
    pub body_timeout: Duration,
}

/// What the API serves from.
pub struct App {
    pub tenants: Arc<Tenants>,
    pub limits: Limits,
    /// The must_support extensions this server supports.
    pub supported: BTreeSet<String>,
    /// The retention period of a tenant that sets none.
    pub retention_default: Retention,
}

/// The routes, served from `app`.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/vcons", post(create).get(list))
        .route("/vcons/{uuid}", get(read).put(replace).delete(delete))
        .route("/health", get(health))
        .route("/key", get(key_info))
        .route("/audit", get(audit_read))
        .route("/admin/audit", get(admin_audit_read))
        .route("/openapi.json", get(openapi))
        .route("/legal/vcons/{uuid}", get(legal_read))
        .route("/legal/vcons/{uuid}/versions", get(legal_versions))
        .route("/legal/vcons/{uuid}/versions/{version}", get(legal_version))
        .route("/legal/vcons/{uuid}/restore", post(legal_restore))
        .route("/legal/vcons/{uuid}/hold", put(hold_on).delete(hold_off))
        .route("/admin/tenants", post(tenant_create).get(tenant_list))
        .route(
            "/admin/tenants/{id}",
            patch(tenant_retention).delete(tenant_remove),
        )
        .route("/admin/tenants/{id}/keys", post(key_create).get(key_list))
        .route("/admin/keys/{id}", axum::routing::delete(key_revoke))
        .merge(crate::console::router())
        .layer(axum::middleware::from_fn(log_request))
        .with_state(app)
}

/// Logs each request once it is answered: a request id, the method, the
/// route template, never the path, which holds ids, the status and how
/// long it took. Bodies, tokens, uuids and tenant ids are in the audit
/// only.
async fn log_request(matched: Option<MatchedPath>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let route = matched
        .as_ref()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let answer = next.run(request).await;
    // In the unit tests every answer must be one the description lists, so
    // a route or status added without its description fails the test
    // that produced it. A method a path does not take (405) and a path no
    // route matches are the router's, not a route's.
    #[cfg(test)]
    if route != "unmatched" && answer.status() != StatusCode::METHOD_NOT_ALLOWED {
        let status = answer.status().as_u16();
        assert!(
            description::lists(method.as_str(), &route, status),
            "openapi.json does not list {status} for {method} {route}"
        );
    }
    let mut id = [0; 8];
    // AWS-LC's RAND_bytes aborts the process when it cannot get enough
    // randomness, so `fill` never returns its error.
    aws_lc_rs::rand::fill(&mut id).expect("AWS-LC aborts instead of failing");
    tracing::info!(
        request_id = %u64::from_be_bytes(id),
        %method,
        %route,
        status = answer.status().as_u16(),
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        "request"
    );
    answer
}

/// The OpenAPI description of every route, as `GET /openapi.json` serves
/// it and the website publishes it.
pub const OPENAPI: &str = include_str!("../openapi.json");

/// `GET /openapi.json`: the description, to any valid key, recorded as
/// openapi in the key's tenant log or the admin log.
async fn openapi(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let caller = match authenticated(&app, &headers).await {
        Ok(caller) => caller,
        Err(refused) => return refused.into_response(),
    };
    let tenants = app.tenants.clone();
    let recorded = blocking(move || {
        let entry = |status| Entry {
            actor: Actor::Key(caller.key),
            action: Action::Openapi,
            target: None,
            version: None,
            status,
        };
        let logged = |done: bool, answer| {
            if done {
                answer
            } else {
                Err(Problem::AuditFailed)
            }
        };
        match caller.tenant.map(|t| tenants.store(t)) {
            Some(Ok(store)) => logged(store.log(&entry(200)).is_ok(), Ok(())),
            // A tenant being removed, or whose log cannot be opened, answers
            // 503 here too, recorded in the admin log.
            Some(Err(_)) => logged(
                tenants.record_alone(&entry(503)).is_ok(),
                Err(Problem::Unavailable),
            ),
            None => logged(tenants.record_alone(&entry(200)).is_ok(), Ok(())),
        }
    })
    .await;
    match recorded {
        Ok(()) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "application/json")],
            OPENAPI,
        )
            .into_response(),
        Err(refused) => refused.into_response(),
    }
}

/// `POST /vcons`: stores a new vCon, or answers 200 when the same bytes
/// are already current.
async fn create(State(app): State<Arc<App>>, headers: HeaderMap, body: Body) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Write, Action::Create).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let found = match inspect(&app, &admitted, &headers, Action::Create).await {
        Ok(found) => found,
        Err(refused) => return refused.into_response(),
    };
    let (store, body, key) = (
        admitted.store.clone(),
        admitted.body.clone(),
        admitted.caller.key,
    );
    // Store records its own outcomes.
    match blocking(move || store.create(&StoreCaller { key }, &found, &body)).await {
        Ok(Created::New(version)) => (
            StatusCode::CREATED,
            [
                (LOCATION, format!("/vcons/{}", found.uuid)),
                (ETAG, entity_tag(version)),
            ],
        )
            .into_response(),
        Ok(Created::Same(version)) => {
            (StatusCode::OK, [(ETAG, entity_tag(version))]).into_response()
        }
        Err(error) => failed(error).into_response(),
    }
}

/// Reads an admitted body as a vCon: its media type must be one a vCon is
/// sent as (415), and Format must take it (422, or 413 when it inflates
/// too far). A refusal here is recorded here, since Store never sees it.
async fn inspect(
    app: &Arc<App>,
    admitted: &Admitted,
    headers: &HeaderMap,
    action: Action,
) -> Result<Inspection, Problem> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .map(|v| v.to_str().unwrap_or("\u{0}").to_owned());
    let found = if content_type.as_deref().is_none_or(is_vcon_media_type) {
        let (worker, body) = (app.clone(), admitted.body.clone());
        blocking(move || {
            let limits = vcon::Limits {
                inflated: worker.limits.inflated,
            };
            vcon::inspect(&body, content_type.as_deref(), &limits, &worker.supported)
                .map_err(|r| rejected(&r))
        })
        .await
    } else {
        Err(Problem::UnsupportedMediaType)
    };
    match found {
        Ok(found) => Ok(found),
        Err(refused) => {
            let entry = refusal(admitted.caller.key, action, refused);
            Err(app.refuse(Some(admitted.tenant), entry, refused).await)
        }
    }
}

/// `GET /vcons`: a page of the tenant's live vCons, newest stored first,
/// as JSON, with the cursor of the next page.
async fn list(State(app): State<Arc<App>>, uri: Uri, headers: HeaderMap, body: Body) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Read, Action::List).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let Ok(query) = ListQuery::from_query(uri.query()) else {
        let refused = Problem::BadQuery;
        let entry = refusal(admitted.caller.key, Action::List, refused);
        return app
            .refuse(Some(admitted.tenant), entry, refused)
            .await
            .into_response();
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    // Store records its own outcomes.
    match blocking(move || store.list(&StoreCaller { key }, &query)).await {
        Ok(listing) => json_answer(StatusCode::OK, &listing.to_json()),
        Err(error) => failed(error).into_response(),
    }
}

/// The media type of an audit page: JSON lines, one record a line.
const JSON_LINES: &str = "application/jsonl";

/// `GET /audit`: a page of the key's tenant's log, the lines `audit export`
/// writes for the same records.
async fn audit_read(
    State(app): State<Arc<App>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Audit, Action::AuditRead).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let Ok(query) = AuditQuery::from_query(uri.query()) else {
        let refused = Problem::BadQuery;
        let entry = refusal(admitted.caller.key, Action::AuditRead, refused);
        return app
            .refuse(Some(admitted.tenant), entry, refused)
            .await
            .into_response();
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    // Store records its own outcomes.
    match blocking(move || store.audit_page(&StoreCaller { key }, &query)).await {
        Ok(lines) => (StatusCode::OK, [(CONTENT_TYPE, JSON_LINES)], lines).into_response(),
        Err(error) => failed(error).into_response(),
    }
}

/// `GET /admin/audit`: a page of the admin log, the lines
/// `audit export --admin` writes for the same records.
async fn admin_audit_read(
    State(app): State<Arc<App>>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::AuditRead;
    let read = async {
        let (key, _) = admin_admit(&app, &headers, body, action).await?;
        let Ok(query) = AuditQuery::from_query(uri.query()) else {
            let refused = Problem::BadQuery;
            return Err(app
                .refuse(None, refusal(key, action, refused), refused)
                .await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.audit_page(Actor::Key(key), &query))
            .await
            .map_err(Problem::Tenant)
    };
    match read.await {
        Ok(lines) => (StatusCode::OK, [(CONTENT_TYPE, JSON_LINES)], lines).into_response(),
        Err(refused) => refused.into_response(),
    }
}

/// `GET /vcons/{uuid}`: the current version, as stored.
async fn read(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Read, Action::Read).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let Some(uuid) = vcon::parse_uuid(&uuid) else {
        let refused = Problem::BadPathUuid;
        let entry = refusal(admitted.caller.key, Action::Read, refused);
        return app
            .refuse(Some(admitted.tenant), entry, refused)
            .await
            .into_response();
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    match blocking(move || store.get(&StoreCaller { key }, uuid)).await {
        Ok(stored) => {
            let content_type = if stored.gzip {
                "application/vcon+gzip"
            } else {
                "application/vcon"
            };
            (
                StatusCode::OK,
                [
                    (CONTENT_TYPE, content_type.to_owned()),
                    (ETAG, entity_tag(stored.version)),
                ],
                stored.body,
            )
                .into_response()
        }
        // Store records its own outcomes.
        Err(error) => failed(error).into_response(),
    }
}

/// `PUT /vcons/{uuid}`: replaces the current version named by `If-Match`.
async fn replace(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Write, Action::Replace).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let (shared, key, tenant) = (&app, admitted.caller.key, admitted.tenant);
    let refuse = |refused| async move {
        let entry = refusal(key, Action::Replace, refused);
        shared
            .refuse(Some(tenant), entry, refused)
            .await
            .into_response()
    };
    let Some(uuid) = vcon::parse_uuid(&uuid) else {
        return refuse(Problem::BadPathUuid).await;
    };
    let current = match if_match(&headers) {
        None => return refuse(Problem::PreconditionRequired).await,
        Some(None) => return refuse(Problem::Store(StoreError::PreconditionFailed)).await,
        Some(Some(current)) => current,
    };
    let found = match inspect(&app, &admitted, &headers, Action::Replace).await {
        Ok(found) => found,
        Err(refused) => return refused.into_response(),
    };
    if found.uuid != uuid {
        return refuse(Problem::PathUuidMismatch).await;
    }
    let (store, body, key) = (
        admitted.store.clone(),
        admitted.body.clone(),
        admitted.caller.key,
    );
    // Store records its own outcomes.
    match blocking(move || store.replace(&StoreCaller { key }, current, &found, &body)).await {
        Ok(version) => (StatusCode::OK, [(ETAG, entity_tag(version))]).into_response(),
        Err(error) => failed(error).into_response(),
    }
}

/// The version an `If-Match` header names: None without the header, and
/// Some(None) when it is anything but one strong tag holding a version id,
/// such as `*`, a weak tag or a list, since that names no version this
/// server could replace.
fn if_match(headers: &HeaderMap) -> Option<Option<VersionId>> {
    let value = headers.get(IF_MATCH)?;
    Some(
        value
            .to_str()
            .ok()
            .and_then(|tag| tag.trim().strip_prefix('"')?.strip_suffix('"'))
            .and_then(VersionId::parse),
    )
}

/// `DELETE /vcons/{uuid}`: hides the vCon; also 204 when it is already
/// deleted or purged.
async fn delete(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let admitted = match admit(&app, &headers, body, Scope::Delete, Action::Delete).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let Some(uuid) = vcon::parse_uuid(&uuid) else {
        let refused = Problem::BadPathUuid;
        let entry = refusal(admitted.caller.key, Action::Delete, refused);
        return app
            .refuse(Some(admitted.tenant), entry, refused)
            .await
            .into_response();
    };
    // The tenant's own period, else the server's, read now so a change
    // counts from the next delete. Unread, nothing is deleted.
    let (tenants, tenant) = (app.tenants.clone(), admitted.tenant);
    let retention = match blocking(move || tenants.retention(tenant)).await {
        Ok(own) => own.unwrap_or(app.retention_default),
        Err(_) => {
            let refused = Problem::Unavailable;
            let entry = refusal(admitted.caller.key, Action::Delete, refused);
            return app
                .refuse(Some(tenant), entry, refused)
                .await
                .into_response();
        }
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    // Store records its own outcomes.
    match blocking(move || store.delete(&StoreCaller { key }, uuid, retention)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failed(error).into_response(),
    }
}

/// Whether a legal read is of a live or a deleted uuid.
const VCON_STATE: HeaderName = HeaderName::from_static("vcon-state");

/// The legal routes' shared start: a key with the legal scope, then the
/// path's uuid, each refusal recorded as `action`.
async fn legal_admit(
    app: &Arc<App>,
    headers: &HeaderMap,
    body: Body,
    action: Action,
    uuid: &str,
) -> Result<(Admitted, Uuid), Problem> {
    let admitted = admit(app, headers, body, Scope::Legal, action).await?;
    let Some(uuid) = vcon::parse_uuid(uuid) else {
        let refused = Problem::BadPathUuid;
        let entry = refusal(admitted.caller.key, action, refused);
        return Err(app.refuse(Some(admitted.tenant), entry, refused).await);
    };
    Ok((admitted, uuid))
}

/// `GET /legal/vcons/{uuid}`: the current version, or the latest kept one
/// of a deleted uuid, as stored, with `Vcon-State`.
async fn legal_read(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    match legal_admit(&app, &headers, body, Action::LegalRead, &uuid).await {
        Ok((admitted, uuid)) => legal_body(&admitted, uuid, None).await,
        Err(refused) => refused.into_response(),
    }
}

/// `GET /legal/vcons/{uuid}/versions/{version}`: that kept version, as
/// stored.
async fn legal_version(
    State(app): State<Arc<App>>,
    UriPath((uuid, version)): UriPath<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (admitted, uuid) = match legal_admit(&app, &headers, body, Action::LegalRead, &uuid).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let Some(version) = VersionId::parse(&version) else {
        let refused = Problem::BadPathVersion;
        let entry = refusal(admitted.caller.key, Action::LegalRead, refused);
        return app
            .refuse(Some(admitted.tenant), entry, refused)
            .await
            .into_response();
    };
    legal_body(&admitted, uuid, Some(version)).await
}

/// A legal read's answer: the body as stored, its tag and the uuid's state.
async fn legal_body(admitted: &Admitted, uuid: Uuid, version: Option<VersionId>) -> Response {
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    // Store records its own outcomes.
    match blocking(move || store.legal_get(&StoreCaller { key }, uuid, version)).await {
        Ok(read) => {
            let content_type = if read.stored.gzip {
                "application/vcon+gzip"
            } else {
                "application/vcon"
            };
            let state = if read.deleted { "deleted" } else { "live" };
            (
                StatusCode::OK,
                [
                    (CONTENT_TYPE, content_type.to_owned()),
                    (ETAG, entity_tag(read.stored.version)),
                    (VCON_STATE, state.to_owned()),
                ],
                read.stored.body,
            )
                .into_response()
        }
        Err(error) => failed(error).into_response(),
    }
}

/// `GET /legal/vcons/{uuid}/versions`: every kept version, oldest first,
/// as JSON: its id, when it was stored and hidden (RFC 3339, UTC), and
/// whether it is current.
async fn legal_versions(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (admitted, uuid) =
        match legal_admit(&app, &headers, body, Action::LegalVersions, &uuid).await {
            Ok(admitted) => admitted,
            Err(refused) => return refused.into_response(),
        };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    match blocking(move || store.legal_versions(&StoreCaller { key }, uuid)).await {
        Ok(versions) => {
            let time = |at: time::OffsetDateTime| at.format(&Rfc3339).ok();
            let listed: Option<Vec<serde_json::Value>> = versions
                .iter()
                .map(|v| {
                    let hidden_at = match v.hidden_at {
                        Some(at) => Some(time(at)?),
                        None => None,
                    };
                    Some(serde_json::json!({
                        "version": v.version.to_string(),
                        "stored_at": time(v.stored_at)?,
                        "hidden_at": hidden_at,
                        "current": v.current,
                    }))
                })
                .collect();
            match listed {
                Some(listed) => (
                    StatusCode::OK,
                    [(CONTENT_TYPE, "application/json")],
                    serde_json::Value::Array(listed).to_string(),
                )
                    .into_response(),
                None => failed(StoreError::Damaged).into_response(),
            }
        }
        Err(error) => failed(error).into_response(),
    }
}

/// `POST /legal/vcons/{uuid}/restore`: makes a deleted vCon live again and
/// answers 200 with the restored version's tag.
async fn legal_restore(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let (admitted, uuid) = match legal_admit(&app, &headers, body, Action::Restore, &uuid).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    match blocking(move || store.restore(&StoreCaller { key }, uuid)).await {
        Ok(version) => (StatusCode::OK, [(ETAG, entity_tag(version))]).into_response(),
        Err(error) => failed(error).into_response(),
    }
}

/// `PUT /legal/vcons/{uuid}/hold`: puts the uuid under a legal hold.
async fn hold_on(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    hold(app, uuid, headers, body, true).await
}

/// `DELETE /legal/vcons/{uuid}/hold`: releases the uuid's legal hold.
async fn hold_off(
    State(app): State<Arc<App>>,
    UriPath(uuid): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    hold(app, uuid, headers, body, false).await
}

async fn hold(app: Arc<App>, uuid: String, headers: HeaderMap, body: Body, on: bool) -> Response {
    // A release is its own action, so the log tells it from a hold.
    let action = if on { Action::Hold } else { Action::Release };
    let (admitted, uuid) = match legal_admit(&app, &headers, body, action, &uuid).await {
        Ok(admitted) => admitted,
        Err(refused) => return refused.into_response(),
    };
    let (store, key) = (admitted.store.clone(), admitted.caller.key);
    match blocking(move || store.set_hold(&StoreCaller { key }, uuid, on)).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => failed(error).into_response(),
    }
}

/// The admin routes' shared start: an operator key holding the admin
/// scope, then the body within the limits, each refusal recorded as
/// `action`.
async fn admin_admit(
    app: &Arc<App>,
    headers: &HeaderMap,
    body: Body,
    action: Action,
) -> Result<(KeyId, Bytes), Problem> {
    let caller = authenticated(app, headers).await?;
    let entry = |refused| refusal(caller.key, action, refused);
    if access::require(&caller, Scope::Admin).is_err() {
        let refused = Problem::Forbidden;
        return Err(app.refuse(caller.tenant, entry(refused), refused).await);
    }
    match read_body(body, headers, &app.limits).await {
        Ok(body) => Ok((caller.key, body)),
        Err(refused) => Err(app.refuse(None, entry(refused), refused).await),
    }
}

/// An admin body as JSON, or the refusal recorded as `action`.
async fn admin_body<T: DeserializeOwned>(
    app: &Arc<App>,
    key: KeyId,
    action: Action,
    body: &Bytes,
) -> Result<T, Problem> {
    match serde_json::from_slice(body) {
        Ok(read) => Ok(read),
        Err(_) => {
            let refused = Problem::InvalidBody;
            Err(app
                .refuse(None, refusal(key, action, refused), refused)
                .await)
        }
    }
}

/// A retention period in a body: a string `Retention` reads, or null for
/// the server default.
fn period<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Retention>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|text| Retention::parse(&text).ok_or_else(|| D::Error::custom("a bad period")))
        .transpose()
}

/// The body of `POST /admin/tenants`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewTenant {
    name: String,
    #[serde(default, deserialize_with = "period")]
    retention: Option<Retention>,
}

/// The body of `PATCH /admin/tenants/{id}`. The period must be there,
/// null included, so an empty body changes nothing by mistake.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewRetention {
    #[serde(deserialize_with = "period")]
    retention: Option<Retention>,
}

/// A JSON answer.
fn json_answer(status: StatusCode, body: &serde_json::Value) -> Response {
    (
        status,
        [(CONTENT_TYPE, "application/json")],
        body.to_string(),
    )
        .into_response()
}

/// `POST /admin/tenants`: makes a tenant and answers 201 with its id.
async fn tenant_create(State(app): State<Arc<App>>, headers: HeaderMap, body: Body) -> Response {
    let action = Action::TenantCreate;
    let made = async {
        let (key, body) = admin_admit(&app, &headers, body, action).await?;
        let new: NewTenant = admin_body(&app, key, action, &body).await?;
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.create(Actor::Key(key), &new.name, new.retention))
            .await
            .map_err(Problem::Tenant)
    };
    match made.await {
        Ok(id) => json_answer(
            StatusCode::CREATED,
            &serde_json::json!({ "id": id.to_string() }),
        ),
        Err(refused) => refused.into_response(),
    }
}

/// `GET /admin/tenants`: every tenant by name, with its id and its own
/// retention period, null for the server default.
async fn tenant_list(State(app): State<Arc<App>>, headers: HeaderMap, body: Body) -> Response {
    let listed = async {
        let (key, _) = admin_admit(&app, &headers, body, Action::TenantList).await?;
        let tenants = app.tenants.clone();
        blocking(move || tenants.list(Actor::Key(key)))
            .await
            .map_err(Problem::Tenant)
    };
    match listed.await {
        Ok(listed) => {
            let listed: Vec<serde_json::Value> = listed.iter().map(TenantInfo::to_json).collect();
            json_answer(StatusCode::OK, &serde_json::Value::Array(listed))
        }
        Err(refused) => refused.into_response(),
    }
}

/// `PATCH /admin/tenants/{id}`: sets the tenant's retention period, or
/// with null returns it to the server default, and answers 200 with it.
async fn tenant_retention(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::TenantRetention;
    let changed = async {
        let (key, body) = admin_admit(&app, &headers, body, action).await?;
        let Some(id) = TenantId::parse(&id) else {
            let refused = Problem::Tenant(TenantError::NotFound);
            return Err(app
                .refuse(None, refusal(key, action, refused), refused)
                .await);
        };
        let new: NewRetention = admin_body(&app, key, action, &body).await?;
        let tenants = app.tenants.clone();
        blocking(move || tenants.set_retention(Actor::Key(key), id, new.retention))
            .await
            .map_err(Problem::Tenant)?;
        Ok((id, new.retention))
    };
    match changed.await {
        Ok((id, retention)) => json_answer(
            StatusCode::OK,
            &serde_json::json!({
                "id": id.to_string(),
                "retention": retention.map(|r| r.to_string()),
            }),
        ),
        Err(refused) => refused.into_response(),
    }
}

/// `DELETE /admin/tenants/{id}`: removes a tenant that keeps no version,
/// or with `?force=true` one that does, unless a uuid of it is under a
/// legal hold; answers 204.
async fn tenant_remove(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    uri: Uri,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::TenantRemove;
    let removed = async {
        let (key, _) = admin_admit(&app, &headers, body, action).await?;
        let refuse = |refused| app.refuse(None, refusal(key, action, refused), refused);
        let Some(id) = TenantId::parse(&id) else {
            return Err(refuse(Problem::Tenant(TenantError::NotFound)).await);
        };
        let Some(removal) = removal(uri.query()) else {
            return Err(refuse(Problem::BadQuery).await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.remove(Actor::Key(key), id, removal))
            .await
            .map_err(Problem::Tenant)
    };
    match removed.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(refused) => refused.into_response(),
    }
}

/// How far a removal asked over HTTP may go: with no query or
/// `force=false` only an empty tenant, with `force=true` its vCons too.
/// No query overrides a legal hold; that is the command line's alone.
fn removal(query: Option<&str>) -> Option<Removal> {
    match query {
        None | Some("force=false") => Some(Removal::Empty),
        Some("force=true") => Some(Removal::Force),
        Some(_) => None,
    }
}

/// The body of `POST /admin/tenants/{id}/keys`.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NewKeyBody {
    scopes: Vec<String>,
    label: String,
}

/// `POST /admin/tenants/{id}/keys`: makes a key for the tenant and
/// answers 201 with its id and token. The token is shown this once; only
/// its hash is kept.
async fn key_create(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::KeyCreate;
    let made = async {
        let (key, body) = admin_admit(&app, &headers, body, action).await?;
        let refuse = |refused| app.refuse(None, refusal(key, action, refused), refused);
        let Some(id) = TenantId::parse(&id) else {
            return Err(refuse(Problem::Tenant(TenantError::NotFound)).await);
        };
        let new: NewKeyBody = admin_body(&app, key, action, &body).await?;
        let Some(scopes) = Scopes::named(&new.scopes) else {
            return Err(refuse(Problem::Tenant(TenantError::BadScopes)).await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.create_key(Actor::Key(key), Some(id), scopes, &new.label))
            .await
            .map_err(Problem::Tenant)
    };
    match made.await {
        Ok(made) => {
            let body = serde_json::json!({ "id": made.id.to_string(), "token": made.token });
            let mut answer = json_answer(StatusCode::CREATED, &body);
            answer
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            answer
        }
        Err(refused) => refused.into_response(),
    }
}

/// `GET /admin/tenants/{id}/keys`: the tenant's keys, oldest first, with
/// their ids, scopes, labels and times (RFC 3339, UTC), never a token.
async fn key_list(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::KeyList;
    let listed = async {
        let (key, _) = admin_admit(&app, &headers, body, action).await?;
        let Some(id) = TenantId::parse(&id) else {
            let refused = Problem::Tenant(TenantError::NotFound);
            return Err(app
                .refuse(None, refusal(key, action, refused), refused)
                .await);
        };
        let tenants = app.tenants.clone();
        blocking(move || tenants.list_keys(Actor::Key(key), Some(id)))
            .await
            .map_err(Problem::Tenant)
    };
    let keys = match listed.await {
        Ok(keys) => keys,
        Err(refused) => return refused.into_response(),
    };
    let listed: Option<Vec<serde_json::Value>> = keys.iter().map(KeyInfo::to_json).collect();
    match listed {
        Some(listed) => json_answer(StatusCode::OK, &serde_json::Value::Array(listed)),
        None => Problem::Tenant(TenantError::Io).into_response(),
    }
}

/// `DELETE /admin/keys/{id}`: revokes the key, so its next request is
/// refused; 204 again for a key already revoked.
async fn key_revoke(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::KeyRevoke;
    let revoked = async {
        let (key, _) = admin_admit(&app, &headers, body, action).await?;
        let Some(id) = KeyId::parse(&id) else {
            let refused = Problem::UnknownKey;
            return Err(app
                .refuse(None, refusal(key, action, refused), refused)
                .await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.revoke_key(Actor::Key(key), id))
            .await
            .map_err(|error| match error {
                TenantError::NotFound => Problem::UnknownKey,
                error => Problem::Tenant(error),
            })
    };
    match revoked.await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(refused) => refused.into_response(),
    }
}

/// `GET /health`: any valid key may ask; 200 when server.sqlite answers,
/// and 503 for a key whose tenant is being removed.
///
/// Health requests are counted, one record per key, status and minute, in
/// the key's tenant log, or the admin log for an operator key or a tenant
/// whose log cannot be opened; a health request that cannot be counted is
/// not healthy.
async fn health(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let caller = match authenticated(&app, &headers).await {
        Ok(caller) => caller,
        Err(refused) => return refused.into_response(),
    };
    let tenants = app.tenants.clone();
    let (healthy, counted) = blocking(move || {
        // A tenant being removed answers 503 here too, as on every route.
        let store = caller.tenant.map(|t| tenants.store(t));
        let closing = matches!(store, Some(Err(TenantError::Closing)));
        let healthy = !closing && tenants.check().is_ok();
        let entry = Entry {
            actor: Actor::Key(caller.key),
            action: Action::Health,
            target: None,
            version: None,
            status: if healthy { 200 } else { 503 },
        };
        let counted = match store {
            Some(Ok(store)) => store.count(&entry).is_ok(),
            _ => tenants.count_admin(&entry).is_ok(),
        };
        (healthy, counted)
    })
    .await;
    match (healthy, counted) {
        (_, false) => Problem::AuditFailed.into_response(),
        (false, true) => Problem::Unavailable.into_response(),
        (true, true) => (
            StatusCode::OK,
            [(CONTENT_TYPE, "application/json")],
            r#"{"status":"ok"}"#,
        )
            .into_response(),
    }
}

/// `GET /key`: any valid key reads what it may do: its own id, its tenant
/// (null for an operator key) and its scopes, so a client can tell which
/// requests it may make.
///
/// The read is recorded before the answer, in the key's tenant log, or the
/// admin log for an operator key. A key whose tenant is being removed, or
/// whose tenant's log cannot be opened, is refused with 503, recorded in
/// the admin log.
async fn key_info(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let caller = match authenticated(&app, &headers).await {
        Ok(caller) => caller,
        Err(refused) => return refused.into_response(),
    };
    let entry = |status| Entry {
        actor: Actor::Key(caller.key),
        action: Action::KeyInfo,
        target: Some(caller.key.to_string()),
        version: None,
        status,
    };
    let store = match caller.tenant {
        Some(t) => {
            let tenants = app.tenants.clone();
            match blocking(move || tenants.store(t)).await {
                Ok(store) => Some(store),
                Err(_) => {
                    let refused = Problem::Unavailable;
                    let entry = entry(refused.status().as_u16());
                    return app.refuse(None, entry, refused).await.into_response();
                }
            }
        }
        None => None,
    };
    let tenants = app.tenants.clone();
    let record = entry(200);
    let recorded = blocking(move || match store {
        Some(store) => store.log(&record).is_ok(),
        None => tenants.record_alone(&record).is_ok(),
    })
    .await;
    if !recorded {
        return Problem::AuditFailed.into_response();
    }
    let body = serde_json::json!({
        "id": caller.key.to_string(),
        "tenant": caller.tenant.map(|t| t.to_string()),
        "scopes": caller.scopes.names(),
    });
    let mut answer = json_answer(StatusCode::OK, &body);
    answer
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    answer
}

/// The audit record of a request refused before Store saw it.
fn refusal(key: KeyId, action: Action, refused: Problem) -> Entry {
    Entry {
        actor: Actor::Key(key),
        action,
        target: None,
        version: None,
        status: refused.status().as_u16(),
    }
}

/// A version id as a strong entity tag.
fn entity_tag(version: VersionId) -> String {
    format!("\"{version}\"")
}

/// Whether a request's media type is one a vCon may be sent as:
/// `application/vcon`, `application/vcon+gzip` or `application/json`, in
/// any case and with any parameters.
fn is_vcon_media_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or("").trim();
    [
        "application/vcon",
        "application/vcon+gzip",
        "application/json",
    ]
    .iter()
    .any(|taken| essence.eq_ignore_ascii_case(taken))
}

/// A request the pipeline let through, with its body read.
struct Admitted {
    caller: Caller,
    tenant: TenantId,
    store: Arc<Store>,
    body: Bytes,
}

/// The steps every vCon request takes before its route's own work, in
/// order: authenticate, check the scope, open the caller's tenant, read
/// the body within the limits. A refusal at any step is recorded here,
/// since Store never sees the request.
async fn admit(
    app: &Arc<App>,
    headers: &HeaderMap,
    body: Body,
    scope: Scope,
    action: Action,
) -> Result<Admitted, Problem> {
    let caller = authenticated(app, headers).await?;
    let entry = |refused| refusal(caller.key, action, refused);
    let Ok(tenant) = access::tenant_for(&caller, scope) else {
        let refused = Problem::Forbidden;
        return Err(app.refuse(caller.tenant, entry(refused), refused).await);
    };
    let tenants = app.tenants.clone();
    let Ok(store) = blocking(move || tenants.store(tenant)).await else {
        let refused = Problem::Unavailable;
        return Err(app.refuse(None, entry(refused), refused).await);
    };
    match read_body(body, headers, &app.limits).await {
        Ok(body) => Ok(Admitted {
            caller,
            tenant,
            store,
            body,
        }),
        Err(refused) => Err(app.refuse(Some(tenant), entry(refused), refused).await),
    }
}

/// The caller a request's bearer key names: 401 for a missing or bad
/// key, and 503 when the key table cannot be read.
async fn authenticated(app: &Arc<App>, headers: &HeaderMap) -> Result<Caller, Problem> {
    let authorization = headers
        .get(AUTHORIZATION)
        .map(|v| v.to_str().unwrap_or_default().to_owned());
    let tenants = app.tenants.clone();
    blocking(move || access::authenticate(&tenants, authorization.as_deref()))
        .await
        .map_err(|e| match e {
            AuthError::Io => Problem::Unavailable,
            _ => Problem::Unauthorized,
        })
}

impl App {
    /// Records a refusal in the tenant's log, or the admin log when there
    /// is no tenant or its log cannot be opened, and answers it; a refusal
    /// that cannot be recorded answers that the audit failed.
    async fn refuse(&self, tenant: Option<TenantId>, entry: Entry, refused: Problem) -> Problem {
        let tenants = self.tenants.clone();
        let recorded = blocking(move || match tenant.and_then(|t| tenants.store(t).ok()) {
            Some(store) => store.log(&entry).is_ok(),
            None => tenants.record_alone(&entry).is_ok(),
        })
        .await;
        if recorded {
            refused
        } else {
            Problem::AuditFailed
        }
    }
}

/// Reads a body within the size limit and the time limit. A declared
/// length over the limit is refused before anything is read, and a body
/// that passes it is refused as soon as it does.
async fn read_body(body: Body, headers: &HeaderMap, limits: &Limits) -> Result<Bytes, Problem> {
    let declared = headers
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limits.body as u64) {
        return Err(Problem::TooLarge);
    }
    match tokio::time::timeout(limits.body_timeout, axum::body::to_bytes(body, limits.body)).await {
        Err(_) => Err(Problem::Timeout),
        Ok(Ok(body)) => Ok(body),
        Ok(Err(e)) if std::error::Error::source(&e).is_some_and(|s| s.is::<LengthLimitError>()) => {
            Err(Problem::TooLarge)
        }
        Ok(Err(_)) => Err(Problem::BadBody),
    }
}

/// Runs blocking work, SQLite and files, off the async workers.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("blocking work does not panic")
}

/// An error answer: an RFC 9457 problem document with one type per error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Problem {
    Unauthorized,
    Forbidden,
    TooLarge,
    Timeout,
    BadBody,
    Unavailable,
    AuditFailed,
    UnsupportedMediaType,
    /// A uuid in the path is not in RFC 9562 text form.
    BadPathUuid,
    /// A version in the path is not a version id.
    BadPathVersion,
    /// A replacement came without `If-Match`.
    PreconditionRequired,
    /// The body's uuid is not the one in the path.
    PathUuidMismatch,
    /// Format refused the body.
    Invalid(Invalid),
    /// An admin body is not one the route takes.
    InvalidBody,
    /// The query is not one the route takes.
    BadQuery,
    /// No key has the id.
    UnknownKey,
    /// Store refused or failed the request.
    Store(StoreError),
    /// Tenants refused or failed the request.
    Tenant(TenantError),
}

/// Why Format refused a body, one problem type each.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Invalid {
    BadGzip,
    NotJson,
    DuplicateMember,
    AmbiguousForm,
    NoForm,
    MissingUuid,
    BadUuid,
    MissingCreatedAt,
    BadPayload,
    UuidMismatch,
    BadCritical,
    UnsupportedExtension,
}

/// The problem for Format's refusal. Inflating past the limit is a body
/// too large; the others are unprocessable. No problem repeats the body,
/// so unsupported extensions are not named.
fn rejected(rejection: &Rejection) -> Problem {
    Problem::Invalid(match rejection {
        Rejection::TooLarge => return Problem::TooLarge,
        Rejection::BadGzip => Invalid::BadGzip,
        Rejection::NotJson => Invalid::NotJson,
        Rejection::DuplicateKey => Invalid::DuplicateMember,
        Rejection::AmbiguousForm => Invalid::AmbiguousForm,
        Rejection::NoForm => Invalid::NoForm,
        Rejection::MissingUuid => Invalid::MissingUuid,
        Rejection::BadUuid => Invalid::BadUuid,
        Rejection::MissingCreatedAt => Invalid::MissingCreatedAt,
        Rejection::BadPayload => Invalid::BadPayload,
        Rejection::UuidMismatch => Invalid::UuidMismatch,
        Rejection::BadCritical => Invalid::BadCritical,
        Rejection::UnsupportedCritical(_) => Invalid::UnsupportedExtension,
    })
}

/// The problem for Store's refusal or failure.
fn failed(error: StoreError) -> Problem {
    Problem::Store(error)
}

impl Problem {
    fn status(self) -> StatusCode {
        match self {
            Problem::Unauthorized => StatusCode::UNAUTHORIZED,
            Problem::Forbidden => StatusCode::FORBIDDEN,
            Problem::UnknownKey => StatusCode::NOT_FOUND,
            Problem::TooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Problem::Timeout => StatusCode::REQUEST_TIMEOUT,
            Problem::BadBody
            | Problem::BadPathUuid
            | Problem::BadPathVersion
            | Problem::BadQuery => StatusCode::BAD_REQUEST,
            Problem::PreconditionRequired => StatusCode::PRECONDITION_REQUIRED,
            Problem::PathUuidMismatch => StatusCode::UNPROCESSABLE_ENTITY,
            Problem::Unavailable | Problem::AuditFailed => StatusCode::SERVICE_UNAVAILABLE,
            Problem::UnsupportedMediaType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Problem::Invalid(_) | Problem::InvalidBody => StatusCode::UNPROCESSABLE_ENTITY,
            Problem::Store(error) => match error {
                StoreError::Conflict | StoreError::NotDeleted | StoreError::Immutable => {
                    StatusCode::CONFLICT
                }
                StoreError::Gone => StatusCode::GONE,
                StoreError::NotFound => StatusCode::NOT_FOUND,
                StoreError::PreconditionFailed => StatusCode::PRECONDITION_FAILED,
                StoreError::LimitReached | StoreError::AuditFailed => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
                StoreError::Damaged | StoreError::Io => StatusCode::INTERNAL_SERVER_ERROR,
            },
            Problem::Tenant(error) => match error {
                TenantError::NotFound => StatusCode::NOT_FOUND,
                TenantError::NameTaken | TenantError::HoldsVcons | TenantError::UnderHold => {
                    StatusCode::CONFLICT
                }
                TenantError::BadName | TenantError::BadScopes => StatusCode::UNPROCESSABLE_ENTITY,
                TenantError::CliOnly => StatusCode::FORBIDDEN,
                TenantError::LimitReached | TenantError::AuditFailed | TenantError::Closing => {
                    StatusCode::SERVICE_UNAVAILABLE
                }
                TenantError::WrongKey | TenantError::Io => StatusCode::INTERNAL_SERVER_ERROR,
            },
        }
    }

    /// The last part of its type URI, and its title.
    fn name(self) -> (&'static str, &'static str) {
        match self {
            Problem::Unauthorized => ("unauthorized", "A valid bearer key is needed"),
            Problem::Forbidden => ("forbidden", "The key does not allow this"),
            Problem::TooLarge => ("too-large", "The body is over the size limit"),
            Problem::Timeout => ("timeout", "The body did not arrive in time"),
            Problem::BadBody => ("bad-body", "The body could not be read"),
            Problem::BadPathUuid => (
                "bad-path-uuid",
                "The path's uuid is not in RFC 9562 text form",
            ),
            Problem::BadPathVersion => {
                ("bad-path-version", "The path's version is not a version id")
            }
            Problem::PreconditionRequired => (
                "precondition-required",
                "If-Match must name the current version",
            ),
            Problem::PathUuidMismatch => (
                "path-uuid-mismatch",
                "The body's uuid is not the one in the path",
            ),
            Problem::Unavailable => ("unavailable", "The service cannot answer now"),
            Problem::AuditFailed => ("audit-failed", "The audit record could not be written"),
            Problem::UnsupportedMediaType => {
                ("unsupported-media-type", "This media type is not taken")
            }
            Problem::Invalid(invalid) => match invalid {
                Invalid::BadGzip => ("bad-gzip", "The body is marked gzip but is not"),
                Invalid::NotJson => ("not-json", "The body is not one JSON object"),
                Invalid::DuplicateMember => ("duplicate-member", "An object repeats a member"),
                Invalid::AmbiguousForm => {
                    ("ambiguous-form", "The body is both signed and encrypted")
                }
                Invalid::NoForm => ("no-form", "The body is in no vCon form"),
                Invalid::MissingUuid => ("missing-uuid", "The vCon has no uuid"),
                Invalid::BadUuid => ("bad-uuid", "The uuid is not in RFC 9562 text form"),
                Invalid::MissingCreatedAt => ("missing-created-at", "The vCon has no created_at"),
                Invalid::BadPayload => ("bad-payload", "The signed payload cannot be read"),
                Invalid::UuidMismatch => ("uuid-mismatch", "A signature names another uuid"),
                Invalid::BadCritical => (
                    "bad-critical",
                    "critical or must_support is not a list of names",
                ),
                Invalid::UnsupportedExtension => (
                    "unsupported-extension",
                    "An extension the vCon needs is not supported",
                ),
            },
            Problem::Store(error) => match error {
                StoreError::Conflict => ("conflict", "Another vCon holds the uuid"),
                StoreError::Gone => ("gone", "The vCon was deleted or purged"),
                StoreError::NotFound => ("not-found", "No vCon has the uuid"),
                StoreError::NotDeleted => ("not-deleted", "The vCon is not deleted"),
                StoreError::PreconditionFailed => (
                    "precondition-failed",
                    "If-Match does not name the current version",
                ),
                StoreError::Immutable => {
                    ("immutable", "A signed or encrypted vCon is not replaced")
                }
                StoreError::LimitReached => ("limit-reached", "The tenant key must be rotated"),
                StoreError::Damaged => ("damaged", "A stored version is damaged"),
                StoreError::AuditFailed => {
                    ("audit-failed", "The audit record could not be written")
                }
                StoreError::Io => ("internal", "The server failed"),
            },
            Problem::BadQuery => ("bad-query", "The query is not one this route takes"),
            Problem::UnknownKey => ("unknown-key", "No key has the id"),
            Problem::InvalidBody | Problem::Tenant(TenantError::BadName) => {
                ("invalid-body", "The body is not one this route takes")
            }
            Problem::Tenant(error) => match error {
                TenantError::NotFound => ("unknown-tenant", "No tenant has the id"),
                TenantError::NameTaken => ("name-taken", "Another tenant has the name"),
                TenantError::HoldsVcons => (
                    "holds-vcons",
                    "The tenant holds vCons; removing them needs force",
                ),
                TenantError::UnderHold => {
                    ("under-hold", "A vCon of the tenant is under a legal hold")
                }
                TenantError::BadScopes => ("bad-scopes", "A tenant key cannot hold these scopes"),
                TenantError::CliOnly => ("forbidden", "The key does not allow this"),
                TenantError::LimitReached => ("master-key-limit", "The master key must be rotated"),
                TenantError::AuditFailed => {
                    ("audit-failed", "The audit record could not be written")
                }
                TenantError::Closing => ("unavailable", "The service cannot answer now"),
                TenantError::BadName | TenantError::WrongKey | TenantError::Io => {
                    ("internal", "The server failed")
                }
            },
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let (name, title) = self.name();
        let status = self.status();
        let body = serde_json::json!({
            "type": format!("https://vcon-server.com/problems/{name}"),
            "title": title,
            "status": status.as_u16(),
        });
        let mut answer = (
            status,
            [(CONTENT_TYPE, "application/problem+json")],
            body.to_string(),
        )
            .into_response();
        if self == Problem::Unauthorized {
            answer
                .headers_mut()
                .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        answer
    }
}

/// Why the TLS configuration could not be made.
#[derive(Debug)]
pub enum TlsError {
    /// The certificate file could not be read, or holds no certificate.
    Certificate,
    /// The key file could not be read, or holds no key.
    Key,
    /// rustls refused the pair, for instance a key that is not the
    /// certificate's.
    Refused(rustls::Error),
}

/// The server's TLS configuration from a PEM certificate chain and a PEM
/// private key.
pub fn tls_config(certificate: &Path, key: &Path) -> Result<Arc<ServerConfig>, TlsError> {
    let chain: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(certificate)
        .map_err(|_| TlsError::Certificate)?
        .collect::<Result<_, _>>()
        .map_err(|_| TlsError::Certificate)?;
    if chain.is_empty() {
        return Err(TlsError::Certificate);
    }
    let key = PrivateKeyDer::from_pem_file(key).map_err(|_| TlsError::Key)?;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(TlsError::Refused)?
        .with_no_client_auth()
        .with_single_cert(chain, key)
        .map_err(TlsError::Refused)?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}

/// Serves `app` on `listener` over TLS until `shutdown` completes. A
/// connection whose handshake fails, such as plain HTTP, is dropped; the
/// server goes on.
pub async fn serve(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    app: Router,
    shutdown: impl Future<Output = ()>,
) -> io::Result<()> {
    let acceptor = TlsAcceptor::from(tls);
    let mut shutdown = std::pin::pin!(shutdown);
    loop {
        let (tcp, _) = tokio::select! {
            accepted = listener.accept() => accepted?,
            () = &mut shutdown => return Ok(()),
        };
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(app.clone());
        tokio::spawn(async move {
            let Ok(stream) = acceptor.accept(tcp).await else {
                return;
            };
            // The connection's own errors end only the connection.
            let _ = auto::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::path::PathBuf;

    use axum::routing::get;

    use super::*;

    /// A self-signed certificate for localhost and its key, as PEM files.
    struct Certificate {
        _dir: tempfile::TempDir,
        cert: PathBuf,
        key: PathBuf,
        pem: String,
    }

    fn certificate() -> Certificate {
        let dir = tempfile::tempdir().expect("temp dir");
        let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .expect("a certificate");
        let pem = made.cert.pem();
        let (cert, key) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        std::fs::write(&cert, &pem).expect("write");
        std::fs::write(&key, made.signing_key.serialize_pem()).expect("write");
        Certificate {
            _dir: dir,
            cert,
            key,
            pem,
        }
    }

    fn client(c: &Certificate, addr: SocketAddr) -> reqwest::Client {
        reqwest::Client::builder()
            .tls_certs_only([reqwest::Certificate::from_pem(c.pem.as_bytes()).expect("pem")])
            .resolve("localhost", addr)
            .no_proxy()
            .build()
            .expect("a client")
    }

    #[tokio::test]
    async fn the_listener_speaks_tls_only() {
        let c = certificate();
        let tls = tls_config(&c.cert, &c.key).expect("loads");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let app = Router::new().route("/ping", get(|| async { "pong" }));
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve(listener, tls, app, async {
            stopped.await.ok();
        }));
        let plain = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("a client")
            .get(format!("http://{addr}/ping"))
            .send()
            .await;
        assert!(plain.is_err(), "plain HTTP is not served: {plain:?}");
        let answer = client(&c, addr)
            .get(format!("https://localhost:{}/ping", addr.port()))
            .send()
            .await
            .expect("TLS is served, after a plain-HTTP attempt too");
        assert_eq!(answer.status(), 200);
        assert_eq!(answer.version(), reqwest::Version::HTTP_2, "h2 by ALPN");
        assert_eq!(answer.text().await.expect("body"), "pong");
        stop.send(()).expect("stop");
        server.await.expect("joins").expect("serves");
    }

    #[test]
    fn a_missing_or_unusable_certificate_or_key_is_refused() {
        let c = certificate();
        let other = certificate();
        let dir = tempfile::tempdir().expect("temp dir");
        let (missing, empty) = (dir.path().join("missing"), dir.path().join("empty"));
        std::fs::write(&empty, "").expect("write");
        assert!(matches!(
            tls_config(&missing, &c.key),
            Err(TlsError::Certificate)
        ));
        assert!(matches!(
            tls_config(&empty, &c.key),
            Err(TlsError::Certificate)
        ));
        assert!(matches!(tls_config(&c.cert, &missing), Err(TlsError::Key)));
        assert!(matches!(tls_config(&c.cert, &empty), Err(TlsError::Key)));
        assert!(matches!(
            tls_config(&c.cert, &other.key),
            Err(TlsError::Refused(_))
        ));
        assert!(tls_config(&c.cert, &c.key).is_ok());
    }

    pub(crate) mod pipeline {
        use std::net::SocketAddr;

        use serde_json::Value;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        use super::*;
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::clock::SystemClock;
        use crate::envelope::Key;
        use crate::fs::OsFs;
        use crate::ids::TenantId;

        pub(crate) const LIMIT: usize = 64 * 1024;

        /// A server with one tenant, on a free port.
        pub(crate) struct Server {
            _dir: tempfile::TempDir,
            certificate: Certificate,
            addr: SocketAddr,
            pub(crate) tenants: Arc<Tenants>,
            pub(crate) tenant: TenantId,
            stop: Option<tokio::sync::oneshot::Sender<()>>,
        }

        impl Drop for Server {
            fn drop(&mut self) {
                if let Some(stop) = self.stop.take() {
                    let _ = stop.send(());
                }
            }
        }

        pub(crate) async fn server() -> Server {
            server_with(&[]).await
        }

        /// A server that supports the must_support extensions named.
        pub(crate) async fn server_with(supported: &[&str]) -> Server {
            server_from(supported, Retention::Years(7)).await
        }

        /// A server whose tenants that set no retention period keep
        /// vCons for `default`.
        pub(crate) async fn server_keeping(default: Retention) -> Server {
            server_from(&[], default).await
        }

        async fn server_from(supported: &[&str], retention_default: Retention) -> Server {
            let dir = tempfile::tempdir().expect("temp dir");
            let tenants = Arc::new(
                Tenants::open(
                    dir.path(),
                    Key::generate().expect("master key"),
                    Arc::new(OsFs),
                    Arc::new(SystemClock),
                )
                .expect("opens"),
            );
            let tenant = tenants.create(Actor::Cli, "acme", None).expect("a tenant");
            let app = Arc::new(App {
                tenants: tenants.clone(),
                limits: Limits {
                    body: LIMIT,
                    inflated: 1 << 20,
                    body_timeout: Duration::from_millis(300),
                },
                supported: supported.iter().map(|s| (*s).to_owned()).collect(),
                retention_default,
            });
            let certificate = certificate();
            let tls = tls_config(&certificate.cert, &certificate.key).expect("tls");
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            let addr = listener.local_addr().expect("addr");
            let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
            tokio::spawn(serve(listener, tls, router(app), async {
                stopped.await.ok();
            }));
            Server {
                _dir: dir,
                certificate,
                addr,
                tenants,
                tenant,
                stop: Some(stop),
            }
        }

        impl Server {
            pub(crate) fn dir(&self) -> &std::path::Path {
                self._dir.path()
            }

            pub(crate) fn url(&self, path: &str) -> String {
                format!("https://localhost:{}{path}", self.addr.port())
            }

            pub(crate) fn client(&self) -> reqwest::Client {
                client(&self.certificate, self.addr)
            }

            /// A client that shows a redirect instead of following it.
            pub(crate) fn client_not_following(&self) -> reqwest::Client {
                let pem = self.certificate.pem.as_bytes();
                reqwest::Client::builder()
                    .tls_certs_only([reqwest::Certificate::from_pem(pem).expect("pem")])
                    .resolve("localhost", self.addr)
                    .no_proxy()
                    .redirect(reqwest::redirect::Policy::none())
                    .build()
                    .expect("a client")
            }

            pub(crate) fn key(&self, scopes: &[Scope]) -> String {
                self.tenants
                    .create_key(Actor::Cli, Some(self.tenant), Scopes::of(scopes), "test")
                    .expect("a key")
                    .token
            }

            pub(crate) fn tenant_log(&self, action: &str) -> Vec<Value> {
                let store = self.tenants.store(self.tenant).expect("its store");
                store
                    .records()
                    .into_iter()
                    .filter(|r| r["action"] == action)
                    .collect()
            }

            /// Sends a POST /vcons over HTTP/1.1 with a chunked body: the
            /// chunks given, and then, unless `finish`, nothing more while
            /// the connection stays open. Returns the status line.
            async fn chunked(&self, token: &str, chunks: &[usize], finish: bool) -> String {
                let mut body = String::new();
                for &size in chunks {
                    body.push_str(&format!("{size:x}\r\n{}\r\n", "x".repeat(size)));
                }
                if finish {
                    body.push_str("0\r\n\r\n");
                }
                self.raw(token, "transfer-encoding: chunked", body.as_bytes())
                    .await
            }

            /// Sends a POST /vcons over HTTP/1.1 with one header of the
            /// caller's and the bytes given, keeping the connection open.
            /// Returns the status line.
            async fn raw(&self, token: &str, header: &str, body: &[u8]) -> String {
                let mut roots = rustls::RootCertStore::empty();
                for cert in CertificateDer::pem_slice_iter(self.certificate.pem.as_bytes()) {
                    roots.add(cert.expect("pem")).expect("a root");
                }
                let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
                let mut config = rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .expect("versions")
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                config.alpn_protocols = vec![b"http/1.1".to_vec()];
                let tcp = tokio::net::TcpStream::connect(self.addr)
                    .await
                    .expect("connect");
                let mut tls = tokio_rustls::TlsConnector::from(Arc::new(config))
                    .connect("localhost".try_into().expect("name"), tcp)
                    .await
                    .expect("handshake");
                let head = format!(
                    "POST /vcons HTTP/1.1\r\nhost: localhost\r\nauthorization: Bearer {token}\r\n\
                     content-type: application/vcon\r\n{header}\r\n\r\n"
                );
                tls.write_all(head.as_bytes()).await.expect("head");
                // The server may answer and stop reading before all is sent.
                let _ = tls.write_all(body).await;
                let mut answer = vec![0; 4096];
                let read = tokio::time::timeout(Duration::from_secs(10), tls.read(&mut answer))
                    .await
                    .expect("an answer while the body is still open")
                    .expect("read");
                let answer = String::from_utf8_lossy(&answer[..read]).into_owned();
                answer.lines().next().unwrap_or_default().to_owned()
            }
        }

        async fn post(s: &Server, token: Option<&str>, body: Vec<u8>) -> reqwest::Response {
            let mut request = s
                .client()
                .post(s.url("/vcons"))
                .header("content-type", "application/vcon")
                .body(body);
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        /// Checks the answer is a problem document for its status, and
        /// returns its type's last part.
        pub(crate) async fn problem(answer: reqwest::Response) -> String {
            assert_eq!(
                answer.headers()["content-type"],
                "application/problem+json",
                "{answer:?}"
            );
            let status = answer.status().as_u16();
            let body: Value =
                serde_json::from_str(&answer.text().await.expect("text")).expect("JSON");
            assert_eq!(body["status"], status);
            assert!(body["title"].as_str().is_some_and(|t| !t.is_empty()));
            body["type"]
                .as_str()
                .and_then(|t| t.strip_prefix("https://vcon-server.com/problems/"))
                .expect("a type under the server's domain")
                .to_owned()
        }

        #[tokio::test]
        async fn a_request_without_a_valid_key_is_unauthorized() {
            let s = server().await;
            for token in [None, Some("vcs_nonsense")] {
                let answer = post(&s, token, b"{}".to_vec()).await;
                assert_eq!(answer.status(), 401, "{token:?}");
                assert_eq!(answer.headers()["www-authenticate"], "Bearer");
                assert_eq!(problem(answer).await, "unauthorized");
            }
        }

        #[tokio::test]
        async fn a_key_table_that_cannot_be_read_makes_the_service_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants
                .conn_for_test()
                .execute_batch("ALTER TABLE api_key RENAME TO gone")
                .expect("lose the table");
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn a_refusal_that_cannot_be_recorded_answers_that_the_audit_failed() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = post(&s, Some(&reader), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");

            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            s.tenants.break_audit();
            let answer = post(&s, Some(&operator), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        #[tokio::test]
        async fn a_key_without_the_scope_is_forbidden_and_recorded() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            let answer = post(&s, Some(&reader), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 403);
            assert_eq!(problem(answer).await, "forbidden");
            let refused = s.tenant_log("create");
            assert_eq!(refused.len(), 1);
            assert_eq!(refused[0]["status"], 403);

            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            let answer = post(&s, Some(&operator), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 403);
            let admin: Vec<Value> = s
                .tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == "create")
                .collect();
            assert_eq!(admin.len(), 1, "an operator's refusal is in the admin log");
            assert_eq!(admin[0]["status"], 403);
        }

        #[tokio::test]
        async fn a_closing_tenant_is_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants.close(s.tenant);
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn a_declared_length_over_the_limit_is_refused_at_once() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let answer = post(&s, Some(&writer), vec![b'x'; LIMIT + 1]).await;
            assert_eq!(answer.status(), 413);
            assert_eq!(problem(answer).await, "too-large");
            assert_eq!(s.tenant_log("create")[0]["status"], 413);
            let declared = format!("content-length: {}", LIMIT + 1);
            assert_eq!(
                s.raw(&writer, &declared, b"").await,
                "HTTP/1.1 413 Payload Too Large",
                "refused on the declared length, with no byte of the body sent"
            );
        }

        #[tokio::test]
        async fn a_body_over_the_limit_is_refused_before_it_is_read_in_full() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[LIMIT / 2, LIMIT / 2 + 1], false).await;
            assert_eq!(status, "HTTP/1.1 413 Payload Too Large");
        }

        #[tokio::test]
        async fn a_body_that_stops_arriving_times_out() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[10], false).await;
            assert_eq!(status, "HTTP/1.1 408 Request Timeout");
            assert_eq!(s.tenant_log("create")[0]["status"], 408);
        }

        #[tokio::test]
        /// The body reaches Format whole: a body of x's at the limit is
        /// refused as not JSON, not as too large.
        async fn a_request_within_the_limits_goes_on_to_its_route() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let status = s.chunked(&writer, &[LIMIT / 2, LIMIT / 2], true).await;
            assert_eq!(status, "HTTP/1.1 422 Unprocessable Entity");
            let answer = post(&s, Some(&writer), b"{}".to_vec()).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "no-form");
        }
    }

    #[test]
    fn a_removal_query_is_none_or_force_true_or_false_and_nothing_else() {
        assert_eq!(removal(None), Some(Removal::Empty));
        assert_eq!(removal(Some("force=false")), Some(Removal::Empty));
        assert_eq!(removal(Some("force=true")), Some(Removal::Force));
        for query in [
            "",
            "force",
            "force=",
            "force=1",
            "Force=true",
            "force=true&",
        ] {
            assert_eq!(removal(Some(query)), None, "{query:?}");
        }
    }

    #[test]
    fn every_refusal_from_format_has_its_own_problem() {
        let cases = [
            (Rejection::TooLarge, 413, "too-large"),
            (Rejection::BadGzip, 422, "bad-gzip"),
            (Rejection::NotJson, 422, "not-json"),
            (Rejection::DuplicateKey, 422, "duplicate-member"),
            (Rejection::AmbiguousForm, 422, "ambiguous-form"),
            (Rejection::NoForm, 422, "no-form"),
            (Rejection::MissingUuid, 422, "missing-uuid"),
            (Rejection::BadUuid, 422, "bad-uuid"),
            (Rejection::MissingCreatedAt, 422, "missing-created-at"),
            (Rejection::BadPayload, 422, "bad-payload"),
            (Rejection::UuidMismatch, 422, "uuid-mismatch"),
            (Rejection::BadCritical, 422, "bad-critical"),
            (
                Rejection::UnsupportedCritical(vec!["secret-name".to_owned()]),
                422,
                "unsupported-extension",
            ),
        ];
        for (rejection, status, name) in &cases {
            let problem = rejected(rejection);
            assert_eq!(
                (problem.status().as_u16(), problem.name().0),
                (*status, *name),
                "{rejection:?}"
            );
            let answer = format!("{:?}", problem.into_response().into_body());
            assert!(!answer.contains("secret-name"), "the body is not repeated");
        }
    }

    #[test]
    fn every_store_outcome_has_its_problem() {
        let cases = [
            (StoreError::Conflict, 409, "conflict"),
            (StoreError::Gone, 410, "gone"),
            (StoreError::NotFound, 404, "not-found"),
            (StoreError::NotDeleted, 409, "not-deleted"),
            (StoreError::PreconditionFailed, 412, "precondition-failed"),
            (StoreError::Immutable, 409, "immutable"),
            (StoreError::LimitReached, 503, "limit-reached"),
            (StoreError::Damaged, 500, "damaged"),
            (StoreError::AuditFailed, 503, "audit-failed"),
            (StoreError::Io, 500, "internal"),
        ];
        for (error, status, name) in cases {
            let problem = failed(error);
            assert_eq!(
                (problem.status().as_u16(), problem.name().0),
                (status, name),
                "{error:?}"
            );
        }
    }

    #[test]
    fn every_tenants_outcome_has_its_problem() {
        let cases = [
            (TenantError::NotFound, 404, "unknown-tenant"),
            (TenantError::NameTaken, 409, "name-taken"),
            (TenantError::BadName, 422, "invalid-body"),
            (TenantError::HoldsVcons, 409, "holds-vcons"),
            (TenantError::UnderHold, 409, "under-hold"),
            (TenantError::LimitReached, 503, "master-key-limit"),
            (TenantError::AuditFailed, 503, "audit-failed"),
            (TenantError::Closing, 503, "unavailable"),
            (TenantError::CliOnly, 403, "forbidden"),
            (TenantError::BadScopes, 422, "bad-scopes"),
            (TenantError::WrongKey, 500, "internal"),
            (TenantError::Io, 500, "internal"),
        ];
        for (error, status, name) in cases {
            let problem = Problem::Tenant(error);
            assert_eq!(
                (problem.status().as_u16(), problem.name().0),
                (status, name),
                "{error:?}"
            );
        }
    }

    mod vcons {
        use std::io::Write;

        use super::pipeline::{Server, problem, server};
        use crate::access::Scope;
        use crate::store::Caller as StoreCaller;
        use crate::store::testing::vcon;

        const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

        fn uuid(n: u8) -> String {
            format!("01a07da8-c2bb-83e5-b9a2-279e0d16bc{n:02x}")
        }

        async fn post(
            s: &Server,
            token: &str,
            content_type: Option<&str>,
            body: Vec<u8>,
        ) -> reqwest::Response {
            let mut request = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(token)
                .body(body);
            if let Some(content_type) = content_type {
                request = request.header("content-type", content_type);
            }
            request.send().await.expect("an answer")
        }

        fn gzip(body: &[u8]) -> Vec<u8> {
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(body).expect("gzip");
            encoder.finish().expect("gzip")
        }

        fn store_caller() -> StoreCaller {
            StoreCaller {
                key: crate::ids::KeyId([9; 8]),
            }
        }

        #[tokio::test]
        async fn a_new_vcon_is_created_with_its_location_and_tag() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (found, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, Some("application/vcon"), body.clone()).await;
            assert_eq!(answer.status(), 201);
            assert_eq!(answer.headers()["location"], format!("/vcons/{UUID}"));
            let tag = answer.headers()["etag"].to_str().expect("ascii").to_owned();
            let stored = s
                .tenants
                .store(s.tenant)
                .expect("its store")
                .get(&store_caller(), found.uuid)
                .expect("stored");
            assert_eq!(tag, format!("\"{}\"", stored.version), "a strong tag");
            assert_eq!(stored.body, body);

            let again = post(&s, &writer, Some("application/vcon"), body).await;
            assert_eq!(again.status(), 200, "the same bytes are already current");
            assert_eq!(again.headers()["etag"], tag.as_str());
        }

        #[tokio::test]
        async fn another_vcon_with_the_uuid_conflicts() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, first) = vcon(UUID, "first");
            let (_, other) = vcon(UUID, "other");
            assert_eq!(post(&s, &writer, None, first).await.status(), 201);
            let answer = post(&s, &writer, None, other).await;
            assert_eq!(answer.status(), 409);
            assert_eq!(problem(answer).await, "conflict");
            let statuses: Vec<_> = s
                .tenant_log("create")
                .into_iter()
                .map(|r| r["status"].clone())
                .collect();
            assert_eq!(statuses, [201, 409], "Store records its own, once");
        }

        #[tokio::test]
        async fn a_vcon_needing_an_unsupported_extension_is_unprocessable() {
            let s = super::pipeline::server_with(&["known"]).await;
            let writer = s.key(&[Scope::Write]);
            let needing = |n: u8, extension: &str| {
                format!(
                    r#"{{"uuid": "{}", "created_at": "2026-10-23T12:00:00Z", "must_support": ["{extension}"], "parties": []}}"#,
                    uuid(n)
                )
                .into_bytes()
            };
            assert_eq!(
                post(&s, &writer, None, needing(1, "known")).await.status(),
                201
            );
            let answer = post(&s, &writer, None, needing(2, "unknown")).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "unsupported-extension");
        }

        #[tokio::test]
        async fn a_media_type_that_is_not_text_is_unsupported() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, body) = vcon(UUID, "first");
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(&writer)
                .header(
                    "content-type",
                    reqwest::header::HeaderValue::from_bytes(b"application/vcon\xff")
                        .expect("a header value"),
                )
                .body(body)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 415);
        }

        #[tokio::test]
        async fn a_deleted_vcon_is_gone() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (found, body) = vcon(UUID, "first");
            assert_eq!(post(&s, &writer, None, body.clone()).await.status(), 201);
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .delete(
                    &store_caller(),
                    found.uuid,
                    crate::retention::Retention::Years(7),
                )
                .expect("deleted");
            let answer = post(&s, &writer, None, body).await;
            assert_eq!(answer.status(), 410);
            assert_eq!(problem(answer).await, "gone");
        }

        #[tokio::test]
        async fn the_vcon_media_types_json_and_none_are_taken() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let cases: [(Option<&str>, bool); 6] = [
                (None, false),
                (Some("application/vcon"), false),
                (Some("application/vcon+gzip"), true),
                (Some("application/json"), false),
                (Some("Application/VCON; charset=utf-8"), false),
                (Some("APPLICATION/VCON+GZIP ; q=1"), true),
            ];
            for (n, (content_type, gzipped)) in cases.into_iter().enumerate() {
                let (_, body) = vcon(&uuid(n as u8), "typed");
                let body = if gzipped { gzip(&body) } else { body };
                let answer = post(&s, &writer, content_type, body).await;
                assert_eq!(answer.status(), 201, "{content_type:?}");
            }
        }

        #[tokio::test]
        async fn another_media_type_is_unsupported_and_recorded() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let (_, body) = vcon(UUID, "first");
            for content_type in ["text/plain", "application/vcon-json", "application/xml"] {
                let answer = post(&s, &writer, Some(content_type), body.clone()).await;
                assert_eq!(answer.status(), 415, "{content_type}");
                assert_eq!(problem(answer).await, "unsupported-media-type");
            }
            assert_eq!(s.tenant_log("create")[0]["status"], 415);
        }

        #[tokio::test]
        async fn a_body_format_refuses_is_unprocessable_and_recorded() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let answer = post(&s, &writer, None, b"not json".to_vec()).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "not-json");
            let (_, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, Some("application/vcon+gzip"), body).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "bad-gzip");
            let refused = s.tenant_log("create");
            assert_eq!(refused.len(), 2);
            assert_eq!(refused[0]["status"], 422);
        }

        #[tokio::test]
        async fn a_body_that_inflates_past_its_limit_is_too_large() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let bomb = gzip(&vec![b' '; 2 << 20]);
            assert!(bomb.len() < 64 * 1024, "small on the wire");
            let answer = post(&s, &writer, Some("application/vcon+gzip"), bomb).await;
            assert_eq!(answer.status(), 413);
            assert_eq!(problem(answer).await, "too-large");
        }

        #[tokio::test]
        async fn a_create_whose_record_fails_is_unavailable() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let (_, body) = vcon(UUID, "first");
            let answer = post(&s, &writer, None, body).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }
    }

    mod legal {
        use reqwest::Method;
        use serde_json::Value;

        use super::pipeline::{Server, problem, server};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::ids::VersionId;
        use crate::retention::Retention;

        const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
        const NEVER: &str = "0199e5c6-0d7a-7c11-8000-000000000001";

        fn key(s: &Server, scopes: &[Scope]) -> String {
            s.tenants
                .create_key(Actor::Cli, Some(s.tenant), Scopes::of(scopes), "test")
                .expect("a key")
                .token
        }

        fn a4() -> Vec<u8> {
            std::fs::read(format!(
                "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("A.4")
        }

        async fn call(s: &Server, method: Method, path: &str, token: &str) -> reqwest::Response {
            s.client()
                .request(method, s.url(path))
                .bearer_auth(token)
                .send()
                .await
                .expect("an answer")
        }

        /// Stores A.4 and returns its entity tag.
        async fn stored(s: &Server) -> String {
            let writer = key(s, &[Scope::Write]);
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(&writer)
                .header("content-type", "application/vcon")
                .body(a4())
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 201);
            answer.headers()["etag"].to_str().expect("tag").to_owned()
        }

        async fn deleted(s: &Server) {
            let deleter = key(s, &[Scope::Delete]);
            let path = format!("/vcons/{UUID}");
            assert_eq!(call(s, Method::DELETE, &path, &deleter).await.status(), 204);
        }

        /// Every legal route for `uuid`, with `version` for the one that
        /// names a version.
        fn routes(uuid: &str, version: &str) -> Vec<(Method, String)> {
            vec![
                (Method::GET, format!("/legal/vcons/{uuid}")),
                (Method::GET, format!("/legal/vcons/{uuid}/versions")),
                (
                    Method::GET,
                    format!("/legal/vcons/{uuid}/versions/{version}"),
                ),
                (Method::POST, format!("/legal/vcons/{uuid}/restore")),
                (Method::PUT, format!("/legal/vcons/{uuid}/hold")),
                (Method::DELETE, format!("/legal/vcons/{uuid}/hold")),
            ]
        }

        fn bare(tag: &str) -> String {
            tag.trim_matches('"').to_owned()
        }

        #[tokio::test]
        async fn every_legal_route_needs_the_legal_scope() {
            let s = server().await;
            let tag = stored(&s).await;
            let others = key(&s, &[Scope::Write, Scope::Read, Scope::Delete]);
            for (method, path) in routes(UUID, &bare(&tag)) {
                let answer = call(&s, method.clone(), &path, &others).await;
                assert_eq!(answer.status(), 403, "{method} {path}");
            }
        }

        #[tokio::test]
        async fn a_deleted_vcon_reads_back_byte_for_byte_with_a_legal_key() {
            let s = server().await;
            let tag = stored(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let path = format!("/legal/vcons/{UUID}");

            let live = call(&s, Method::GET, &path, &legal).await;
            assert_eq!(live.status(), 200);
            assert_eq!(live.headers()["vcon-state"], "live");

            deleted(&s).await;
            let answer = call(&s, Method::GET, &path, &legal).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["vcon-state"], "deleted");
            assert_eq!(answer.headers()["etag"], tag.as_str());
            assert_eq!(answer.headers()["content-type"], "application/vcon");
            assert_eq!(
                answer.bytes().await.expect("body").as_ref(),
                a4().as_slice()
            );
            assert_eq!(
                s.tenant_log("legal_read").last().expect("recorded")["status"],
                200
            );
        }

        #[tokio::test]
        async fn the_version_list_and_a_named_version() {
            let s = server().await;
            let tag = stored(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let live = call(
                &s,
                Method::GET,
                &format!("/legal/vcons/{UUID}/versions"),
                &legal,
            )
            .await;
            let live: Value =
                serde_json::from_slice(&live.bytes().await.expect("body")).expect("json");
            assert_eq!(live[0]["current"], true, "live, so current");
            assert!(live[0]["hidden_at"].is_null(), "not hidden while live");
            deleted(&s).await;

            let list = call(
                &s,
                Method::GET,
                &format!("/legal/vcons/{UUID}/versions"),
                &legal,
            )
            .await;
            assert_eq!(list.status(), 200);
            assert_eq!(list.headers()["content-type"], "application/json");
            let list: Value =
                serde_json::from_slice(&list.bytes().await.expect("body")).expect("json");
            let versions = list.as_array().expect("a list");
            assert_eq!(versions.len(), 1);
            assert_eq!(versions[0]["version"], bare(&tag));
            assert_eq!(versions[0]["current"], false);
            assert!(
                versions[0]["stored_at"]
                    .as_str()
                    .expect("a time")
                    .ends_with('Z')
            );
            assert!(versions[0]["hidden_at"].is_string(), "hidden by the delete");

            let one = format!("/legal/vcons/{UUID}/versions/{}", bare(&tag));
            let answer = call(&s, Method::GET, &one, &legal).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["etag"], tag.as_str());
            assert_eq!(
                answer.bytes().await.expect("body").as_ref(),
                a4().as_slice()
            );

            let unknown = format!("/legal/vcons/{UUID}/versions/{}", VersionId::generate());
            let answer = call(&s, Method::GET, &unknown, &legal).await;
            assert_eq!(answer.status(), 404);
            assert_eq!(problem(answer).await, "not-found");

            let bad = format!("/legal/vcons/{UUID}/versions/not-a-version");
            let answer = call(&s, Method::GET, &bad, &legal).await;
            assert_eq!(answer.status(), 400);
            assert_eq!(problem(answer).await, "bad-path-version");
            assert_eq!(
                s.tenant_log("legal_read").last().expect("recorded")["status"],
                400
            );
        }

        #[tokio::test]
        async fn restore_answers_its_tag_then_409_once_live() {
            let s = server().await;
            let tag = stored(&s).await;
            deleted(&s).await;
            let token = key(&s, &[Scope::Legal, Scope::Read]);
            let path = format!("/legal/vcons/{UUID}/restore");

            let answer = call(&s, Method::POST, &path, &token).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["etag"], tag.as_str());
            let read = call(&s, Method::GET, &format!("/vcons/{UUID}"), &token).await;
            assert_eq!(read.status(), 200, "live again");

            let again = call(&s, Method::POST, &path, &token).await;
            assert_eq!(again.status(), 409);
            assert_eq!(problem(again).await, "not-deleted");
        }

        #[tokio::test]
        async fn a_hold_is_set_and_released_with_204() {
            let s = server().await;
            stored(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let path = format!("/legal/vcons/{UUID}/hold");
            for method in [Method::PUT, Method::DELETE] {
                let answer = call(&s, method, &path, &legal).await;
                assert_eq!(answer.status(), 204);
                assert!(answer.bytes().await.expect("body").is_empty());
            }
            assert_eq!(s.tenant_log("hold").len(), 1, "the hold");
            assert_eq!(s.tenant_log("release").len(), 1, "the release, as itself");
        }

        #[tokio::test]
        async fn a_held_vcon_survives_a_zero_period_delete_until_released() {
            let s = server().await;
            stored(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let hold = format!("/legal/vcons/{UUID}/hold");
            let read = format!("/legal/vcons/{UUID}");
            assert_eq!(call(&s, Method::PUT, &hold, &legal).await.status(), 204);
            s.tenants
                .set_retention(Actor::Cli, s.tenant, Some(Retention::Days(0)))
                .expect("set");
            deleted(&s).await;
            assert_eq!(
                call(&s, Method::GET, &read, &legal).await.status(),
                200,
                "held"
            );

            assert_eq!(call(&s, Method::DELETE, &hold, &legal).await.status(), 204);
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .purge_due(Retention::Days(0))
                .expect("swept");
            assert_eq!(
                call(&s, Method::GET, &read, &legal).await.status(),
                410,
                "released"
            );
        }

        #[tokio::test]
        async fn a_gzipped_vcon_reads_back_gzipped() {
            use std::io::Write;
            let s = server().await;
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gz.write_all(&a4()).expect("gzip");
            let gzipped = gz.finish().expect("gzip");
            let writer = key(&s, &[Scope::Write]);
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(&writer)
                .header("content-type", "application/vcon+gzip")
                .body(gzipped.clone())
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 201);
            deleted(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let answer = call(&s, Method::GET, &format!("/legal/vcons/{UUID}"), &legal).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["content-type"], "application/vcon+gzip");
            assert_eq!(
                answer.bytes().await.expect("body").as_ref(),
                gzipped.as_slice()
            );
        }

        #[tokio::test]
        async fn a_purged_uuid_is_410_on_every_legal_route() {
            let s = server().await;
            let tag = stored(&s).await;
            s.tenants
                .set_retention(Actor::Cli, s.tenant, Some(Retention::Days(0)))
                .expect("set");
            deleted(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            for (method, path) in routes(UUID, &bare(&tag)) {
                let answer = call(&s, method.clone(), &path, &legal).await;
                assert_eq!(answer.status(), 410, "{method} {path}");
                assert_eq!(problem(answer).await, "gone");
            }
        }

        #[tokio::test]
        async fn a_uuid_never_stored_is_404_and_a_bad_one_400_on_every_legal_route() {
            let s = server().await;
            let legal = key(&s, &[Scope::Legal]);
            let version = VersionId::generate().to_string();
            for (method, path) in routes(NEVER, &version) {
                let answer = call(&s, method.clone(), &path, &legal).await;
                assert_eq!(answer.status(), 404, "{method} {path}");
                assert_eq!(problem(answer).await, "not-found");
            }
            for (method, path) in routes("not-a-uuid", &version) {
                let answer = call(&s, method.clone(), &path, &legal).await;
                assert_eq!(answer.status(), 400, "{method} {path}");
                assert_eq!(problem(answer).await, "bad-path-uuid");
            }
            for action in ["legal_read", "legal_versions", "restore", "hold", "release"] {
                let last = s.tenant_log(action).last().expect("recorded").clone();
                assert_eq!(last["status"], 400, "{action}'s refusal is recorded");
            }
        }

        #[tokio::test]
        async fn damage_is_500_on_every_legal_route() {
            let s = server().await;
            let tag = stored(&s).await;
            deleted(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            let store = s.tenants.store(s.tenant).expect("its store");
            store
                .conn_for_test()
                .execute_batch(
                    "UPDATE version SET wrapped_key = x'00', stored_at = 'x';
                     CREATE TEMP TRIGGER broken BEFORE UPDATE ON vcon
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;",
                )
                .expect("damaged");
            for (method, path) in routes(UUID, &bare(&tag)) {
                let answer = call(&s, method.clone(), &path, &legal).await;
                assert_eq!(answer.status(), 500, "{method} {path}");
            }
        }

        #[tokio::test]
        async fn an_answer_that_cannot_be_recorded_is_503_on_every_legal_route() {
            let s = server().await;
            let tag = stored(&s).await;
            let legal = key(&s, &[Scope::Legal]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            for (method, path) in routes(UUID, &bare(&tag)) {
                let answer = call(&s, method.clone(), &path, &legal).await;
                assert_eq!(answer.status(), 503, "{method} {path}");
                assert_eq!(problem(answer).await, "audit-failed");
            }
        }
    }

    mod admin {
        use reqwest::Method;
        use serde_json::{Value, json};

        use super::pipeline::{Server, problem, server};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use time::format_description::well_known::Rfc3339;

        use crate::access;
        use crate::ids::{KeyId, TenantId};
        use crate::retention::Retention;

        fn operator(s: &Server) -> String {
            s.tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("an operator key")
                .token
        }

        async fn call(
            s: &Server,
            method: Method,
            path: &str,
            token: Option<&str>,
            body: Option<&str>,
        ) -> reqwest::Response {
            let mut request = s.client().request(method, s.url(path));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            if let Some(body) = body {
                request = request
                    .header("content-type", "application/json")
                    .body(body.to_owned());
            }
            request.send().await.expect("an answer")
        }

        async fn json(answer: reqwest::Response) -> Value {
            assert_eq!(answer.headers()["content-type"], "application/json");
            serde_json::from_slice(&answer.bytes().await.expect("body")).expect("JSON")
        }

        /// The admin log's records of `action` written for requests, not
        /// for the tests' own setup from the command line.
        fn admin_log(s: &Server, action: &str) -> Vec<Value> {
            s.tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == action && r["actor"] != "cli")
                .collect()
        }

        fn key_id(token: &str) -> KeyId {
            access::parse(token).expect("a token")
        }

        /// Each admin route, with a body it takes, and its audit action.
        fn routes(
            id: TenantId,
            key: KeyId,
        ) -> Vec<(Method, String, Option<&'static str>, &'static str)> {
            vec![
                (
                    Method::POST,
                    "/admin/tenants".to_owned(),
                    Some(r#"{"name":"beta"}"#),
                    "tenant_create",
                ),
                (
                    Method::GET,
                    "/admin/tenants".to_owned(),
                    None,
                    "tenant_list",
                ),
                (
                    Method::PATCH,
                    format!("/admin/tenants/{id}"),
                    Some(r#"{"retention":"5y"}"#),
                    "tenant_retention",
                ),
                (
                    Method::DELETE,
                    format!("/admin/tenants/{id}"),
                    None,
                    "tenant_remove",
                ),
                (
                    Method::POST,
                    format!("/admin/tenants/{id}/keys"),
                    Some(r#"{"scopes":["read"],"label":"ingest"}"#),
                    "key_create",
                ),
                (
                    Method::GET,
                    format!("/admin/tenants/{id}/keys"),
                    None,
                    "key_list",
                ),
                (
                    Method::DELETE,
                    format!("/admin/keys/{key}"),
                    None,
                    "key_revoke",
                ),
            ]
        }

        #[tokio::test]
        async fn a_tenant_key_is_forbidden_and_no_key_unauthorized_on_every_admin_route() {
            let s = server().await;
            let every = [Scope::Write, Scope::Read, Scope::Delete, Scope::Legal];
            let tenant_key = s.key(&every);
            for (method, path, body, action) in routes(s.tenant, key_id(&tenant_key)) {
                let answer = call(&s, method.clone(), &path, Some(&tenant_key), body).await;
                assert_eq!(answer.status(), 403, "{method} {path}");
                assert_eq!(problem(answer).await, "forbidden");
                let refused: Vec<Value> = s
                    .tenant_log(action)
                    .into_iter()
                    .filter(|r| r["actor"] != "cli")
                    .collect();
                assert_eq!(refused.len(), 1, "{action} refusal in the tenant's log");
                assert_eq!(refused[0]["status"], 403);

                let answer = call(&s, method.clone(), &path, None, body).await;
                assert_eq!(answer.status(), 401, "{method} {path}");
            }
            assert_eq!(s.tenants.list(Actor::Cli).expect("listed").len(), 1);
            assert_eq!(s.tenants.retention(s.tenant), Ok(None), "nothing changed");
            let keys = s.tenants.list_keys(Actor::Cli, Some(s.tenant));
            let keys = keys.expect("listed");
            assert_eq!(keys.len(), 1, "no key made");
            assert_eq!(keys[0].revoked_at, None, "nor revoked");
        }

        #[tokio::test]
        async fn a_tenant_is_created_with_201_and_its_id_and_listed() {
            let s = server().await;
            let op = operator(&s);
            let body = r#"{"name":"beta","retention":"5y"}"#;
            let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(body)).await;
            assert_eq!(answer.status(), 201);
            let made = json(answer).await;
            let id = TenantId::parse(made["id"].as_str().expect("an id")).expect("a tenant id");
            assert_eq!(made, json!({ "id": id.to_string() }));
            assert_eq!(s.tenants.retention(id), Ok(Some(Retention::Years(5))));
            let created = admin_log(&s, "tenant_create");
            assert_eq!(created.last().expect("recorded")["status"], 201);

            let body = r#"{"name":"gamma"}"#;
            let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(body)).await;
            assert_eq!(answer.status(), 201);

            let answer = call(&s, Method::GET, "/admin/tenants", Some(&op), None).await;
            assert_eq!(answer.status(), 200);
            let listed = json(answer).await;
            let gamma = s
                .tenants
                .list(Actor::Cli)
                .expect("listed")
                .into_iter()
                .find(|t| t.name == "gamma")
                .expect("gamma")
                .id;
            assert_eq!(
                listed,
                json!([
                    { "id": s.tenant.to_string(), "name": "acme", "retention": null },
                    { "id": id.to_string(), "name": "beta", "retention": "5y" },
                    { "id": gamma.to_string(), "name": "gamma", "retention": null },
                ])
            );
            assert_eq!(admin_log(&s, "tenant_list")[0]["status"], 200);
        }

        #[tokio::test]
        async fn a_taken_name_is_409() {
            let s = server().await;
            let op = operator(&s);
            let body = r#"{"name":"acme"}"#;
            let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(body)).await;
            assert_eq!(answer.status(), 409);
            assert_eq!(problem(answer).await, "name-taken");
            assert_eq!(s.tenants.list(Actor::Cli).expect("listed").len(), 1);
        }

        #[tokio::test]
        async fn a_create_body_it_does_not_take_is_422_and_recorded_once() {
            let s = server().await;
            let op = operator(&s);
            let bodies = [
                "",
                "not json",
                "[]",
                "{}",
                r#"{"name":7}"#,
                r#"{"name":null}"#,
                r#"{"name":"beta","colour":"red"}"#,
                r#"{"name":"beta","name":"gamma"}"#,
                r#"{"name":"beta","retention":"7x"}"#,
                r#"{"name":"beta","retention":7}"#,
                r#"{"name":" beta"}"#,
                r#"{"name":""}"#,
            ];
            for body in bodies {
                let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(body)).await;
                assert_eq!(answer.status(), 422, "{body}");
                assert_eq!(problem(answer).await, "invalid-body", "{body}");
            }
            let refused = admin_log(&s, "tenant_create");
            assert_eq!(refused.len(), bodies.len(), "one per refusal");
            assert!(refused.iter().all(|r| r["status"] == 422));
            assert_eq!(s.tenants.list(Actor::Cli).expect("listed").len(), 1);
        }

        #[tokio::test]
        async fn a_create_past_the_master_keys_wrap_limit_is_503() {
            let s = server().await;
            let op = operator(&s);
            s.tenants
                .conn_for_test()
                .execute("UPDATE master_key_use SET wraps = ?1", [1_i64 << 32])
                .expect("at the limit");
            let body = r#"{"name":"beta"}"#;
            let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(body)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "master-key-limit");
        }

        #[tokio::test]
        async fn a_retention_change_answers_200_with_the_period() {
            let s = server().await;
            let op = operator(&s);
            let path = format!("/admin/tenants/{}", s.tenant);
            let id = s.tenant.to_string();
            for (body, period, kept) in [
                (
                    r#"{"retention":"30d"}"#,
                    json!("30d"),
                    Some(Retention::Days(30)),
                ),
                (r#"{"retention":null}"#, Value::Null, None),
            ] {
                let answer = call(&s, Method::PATCH, &path, Some(&op), Some(body)).await;
                assert_eq!(answer.status(), 200, "{body}");
                assert_eq!(json(answer).await, json!({ "id": id, "retention": period }));
                assert_eq!(s.tenants.retention(s.tenant), Ok(kept), "{body}");
            }
            let changed = admin_log(&s, "tenant_retention");
            assert_eq!(changed.len(), 2);
            assert!(changed.iter().all(|r| r["status"] == 200));
            assert_eq!(s.tenant_log("retention").len(), 2, "the tenant's log too");
        }

        #[tokio::test]
        async fn a_retention_change_for_no_tenant_is_404_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let body = Some(r#"{"retention":"5y"}"#);
            let unknown = TenantId([7; 16]).to_string();
            for id in [
                unknown.as_str(),
                "acme",
                "0123",
                &s.tenant.to_string().to_uppercase(),
            ] {
                let path = format!("/admin/tenants/{id}");
                let answer = call(&s, Method::PATCH, &path, Some(&op), body).await;
                assert_eq!(answer.status(), 404, "{id}");
                assert_eq!(problem(answer).await, "unknown-tenant", "{id}");
            }
            let refused = admin_log(&s, "tenant_retention");
            assert_eq!(refused.len(), 4);
            assert!(refused.iter().all(|r| r["status"] == 404));
            assert_eq!(s.tenants.retention(s.tenant), Ok(None));
        }

        #[tokio::test]
        async fn a_retention_body_it_does_not_take_is_422_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let path = format!("/admin/tenants/{}", s.tenant);
            let bodies = [
                "",
                "{}",
                r#"{"retention":"7"}"#,
                r#"{"retention":"07y"}"#,
                r#"{"retention":5}"#,
                r#"{"retention":"5y","name":"beta"}"#,
            ];
            for body in bodies {
                let answer = call(&s, Method::PATCH, &path, Some(&op), Some(body)).await;
                assert_eq!(answer.status(), 422, "{body}");
                assert_eq!(problem(answer).await, "invalid-body", "{body}");
            }
            let refused = admin_log(&s, "tenant_retention");
            assert_eq!(refused.len(), bodies.len());
            assert!(refused.iter().all(|r| r["status"] == 422));
            assert_eq!(s.tenants.retention(s.tenant), Ok(None));
        }

        #[tokio::test]
        async fn a_retention_change_for_a_closing_tenant_is_503() {
            let s = server().await;
            let op = operator(&s);
            s.tenants.close(s.tenant);
            let path = format!("/admin/tenants/{}", s.tenant);
            let body = Some(r#"{"retention":"5y"}"#);
            let answer = call(&s, Method::PATCH, &path, Some(&op), body).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn damage_is_500_on_every_admin_route() {
            let s = server().await;
            let op = operator(&s);
            let kept = key_id(&s.key(&[Scope::Read]));
            s.tenants
                .conn_for_test()
                .execute_batch(
                    "UPDATE tenant SET retention = 'x';
                     UPDATE api_key SET created_at = 'x' WHERE tenant IS NOT NULL;
                     CREATE TEMP TRIGGER no_key BEFORE INSERT ON api_key
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;
                     CREATE TEMP TRIGGER no_revoke BEFORE UPDATE ON api_key
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;
                     CREATE TEMP TRIGGER broken BEFORE INSERT ON tenant
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;
                     CREATE TEMP TRIGGER broken_too BEFORE UPDATE ON tenant
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;
                     CREATE TEMP TRIGGER broken_three BEFORE DELETE ON tenant
                     BEGIN SELECT RAISE(ABORT, 'the disk has failed'); END;",
                )
                .expect("damaged");
            for (method, path, body, _) in routes(s.tenant, kept) {
                let answer = call(&s, method.clone(), &path, Some(&op), body).await;
                assert_eq!(answer.status(), 500, "{method} {path}");
                assert_eq!(problem(answer).await, "internal");
            }
        }

        #[tokio::test]
        async fn an_answer_that_cannot_be_recorded_is_503_on_every_admin_route() {
            let s = server().await;
            let op = operator(&s);
            let kept = key_id(&s.key(&[Scope::Read]));
            s.tenants.break_audit();
            for (method, path, body, _) in routes(s.tenant, kept) {
                let answer = call(&s, method.clone(), &path, Some(&op), body).await;
                assert_eq!(answer.status(), 503, "{method} {path}");
                assert_eq!(problem(answer).await, "audit-failed");
            }
            for body in ["{}", r#"{"retention":"5y"}"#] {
                let path = if body == "{}" {
                    "/admin/tenants".to_owned()
                } else {
                    format!("/admin/tenants/{}", TenantId([7; 16]))
                };
                let method = if body == "{}" {
                    Method::POST
                } else {
                    Method::PATCH
                };
                let answer = call(&s, method, &path, Some(&op), Some(body)).await;
                assert_eq!(problem(answer).await, "audit-failed", "a refusal too");
            }
            assert_eq!(
                s.tenants.list(Actor::Cli).err(),
                Some(crate::tenants::TenantError::AuditFailed)
            );
        }

        /// Stores A.4 in the server's tenant with a key of its own.
        async fn stored(s: &Server) {
            let body = std::fs::read(format!(
                "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("A.4");
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(s.key(&[Scope::Write]))
                .header("content-type", "application/vcon")
                .body(body)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 201);
        }

        fn listed(s: &Server) -> Vec<TenantId> {
            let tenants = s.tenants.list(Actor::Cli).expect("listed");
            tenants.into_iter().map(|t| t.id).collect()
        }

        #[tokio::test]
        async fn an_empty_tenant_is_removed_with_204_then_unknown() {
            let s = server().await;
            let op = operator(&s);
            let beta = s.tenants.create(Actor::Cli, "beta", None).expect("made");
            let path = format!("/admin/tenants/{beta}");
            let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
            assert_eq!(answer.status(), 204);
            assert_eq!(listed(&s), vec![s.tenant]);
            let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
            assert_eq!(answer.status(), 404);
            assert_eq!(problem(answer).await, "unknown-tenant");
            let removed = admin_log(&s, "tenant_remove");
            let statuses: Vec<&Value> = removed.iter().map(|r| &r["status"]).collect();
            assert_eq!(statuses, [204, 404]);
        }

        #[tokio::test]
        async fn a_tenant_holding_vcons_is_409_without_force_and_removed_with_it() {
            let s = server().await;
            let op = operator(&s);
            stored(&s).await;
            let reader = s.key(&[Scope::Read]);
            let path = format!("/admin/tenants/{}", s.tenant);
            for query in ["", "?force=false"] {
                let path = format!("{path}{query}");
                let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 409, "{query}");
                assert_eq!(problem(answer).await, "holds-vcons", "{query}");
            }
            assert_eq!(listed(&s), vec![s.tenant], "still there");

            let forced = format!("{path}?force=true");
            let answer = call(&s, Method::DELETE, &forced, Some(&op), None).await;
            assert_eq!(answer.status(), 204);
            assert_eq!(listed(&s), Vec::<TenantId>::new());
            let health = s
                .client()
                .get(s.url("/health"))
                .bearer_auth(&reader)
                .send()
                .await
                .expect("an answer");
            assert_eq!(health.status(), 401, "its keys went with it");
        }

        #[tokio::test]
        async fn a_legal_hold_is_409_even_with_force() {
            let s = server().await;
            let op = operator(&s);
            stored(&s).await;
            let legal = s.key(&[Scope::Legal]);
            let hold = s
                .client()
                .put(s.url("/legal/vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46/hold"))
                .bearer_auth(&legal)
                .send()
                .await
                .expect("an answer");
            assert_eq!(hold.status(), 204);
            let path = format!("/admin/tenants/{}?force=true", s.tenant);
            let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
            assert_eq!(answer.status(), 409);
            assert_eq!(problem(answer).await, "under-hold");
            assert_eq!(listed(&s), vec![s.tenant], "still there");
            let removed = admin_log(&s, "tenant_remove");
            assert_eq!(removed.len(), 1);
            assert_eq!(removed[0]["status"], 409);
            let read = s
                .client()
                .get(s.url("/legal/vcons/01a07da8-c2bb-83e5-b9a2-279e0d16bc46"))
                .bearer_auth(&legal)
                .send()
                .await
                .expect("an answer");
            assert_eq!(read.status(), 200, "the tenant is served again");
        }

        #[tokio::test]
        async fn a_query_it_does_not_take_is_400_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let queries = [
                "force=yes",
                "force=TRUE",
                "force",
                "force=",
                "force=true&force=true",
                "force=true&other=1",
                "other=1",
            ];
            for query in queries {
                let path = format!("/admin/tenants/{}?{query}", s.tenant);
                let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 400, "{query}");
                assert_eq!(problem(answer).await, "bad-query", "{query}");
            }
            let refused = admin_log(&s, "tenant_remove");
            assert_eq!(refused.len(), queries.len());
            assert!(refused.iter().all(|r| r["status"] == 400));
            assert_eq!(listed(&s), vec![s.tenant]);
        }

        #[tokio::test]
        async fn a_removal_of_no_tenant_is_404_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let unknown = TenantId([7; 16]).to_string();
            for id in [unknown.as_str(), "acme"] {
                let path = format!("/admin/tenants/{id}?force=true");
                let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 404, "{id}");
                assert_eq!(problem(answer).await, "unknown-tenant", "{id}");
            }
            let refused = admin_log(&s, "tenant_remove");
            assert_eq!(refused.len(), 2);
            assert!(refused.iter().all(|r| r["status"] == 404));
        }

        #[tokio::test]
        async fn a_removal_of_a_closing_tenant_is_503() {
            let s = server().await;
            let op = operator(&s);
            s.tenants.close(s.tenant);
            let path = format!("/admin/tenants/{}", s.tenant);
            let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        async fn health(s: &Server, token: &str) -> u16 {
            let answer = s.client().get(s.url("/health")).bearer_auth(token).send();
            answer.await.expect("an answer").status().as_u16()
        }

        /// Issues a key over HTTP and returns its id and token.
        async fn issued(s: &Server, op: &str, body: &str) -> (String, String) {
            let path = format!("/admin/tenants/{}/keys", s.tenant);
            let answer = call(s, Method::POST, &path, Some(op), Some(body)).await;
            assert_eq!(answer.status(), 201, "{body}");
            assert_eq!(answer.headers()["cache-control"], "no-store");
            let made = json(answer).await;
            let id = made["id"].as_str().expect("an id").to_owned();
            let token = made["token"].as_str().expect("a token").to_owned();
            assert_eq!(made, json!({ "id": id, "token": token }), "nothing else");
            assert_eq!(key_id(&token).to_string(), id, "the token names its key");
            (id, token)
        }

        async fn key_list(s: &Server, op: &str) -> (String, Value) {
            let path = format!("/admin/tenants/{}/keys", s.tenant);
            let answer = call(s, Method::GET, &path, Some(op), None).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["content-type"], "application/json");
            let text = answer.text().await.expect("text");
            let listed = serde_json::from_str(&text).expect("JSON");
            (text, listed)
        }

        #[tokio::test]
        async fn a_key_is_issued_with_201_and_its_token_shown_once() {
            let s = server().await;
            let op = operator(&s);
            let body = r#"{"scopes":["read","write"],"label":"ingest"}"#;
            let (id, token) = issued(&s, &op, body).await;
            assert_eq!(health(&s, &token).await, 200, "the token works");
            let (again, other) = issued(&s, &op, body).await;
            assert_ne!((&again, &other), (&id, &token), "each key is new");

            let (text, listed) = key_list(&s, &op).await;
            for token in [&token, &other] {
                // vcs_<key id>_<secret>: the secret is base64url, whose
                // alphabet includes `_`, so it is all after the second `_`.
                let secret = token.splitn(3, '_').nth(2).expect("a secret");
                assert_eq!(secret.len(), 43, "the whole 32-byte secret");
                assert!(!text.contains(secret), "no token is listed");
            }
            let first = &listed[0];
            assert_eq!(first["id"], id.as_str());
            assert_eq!(
                first["scopes"],
                json!(["write", "read"]),
                "in the fixed order"
            );
            assert_eq!(first["label"], "ingest");
            assert_eq!(first["revoked_at"], Value::Null);
            let stored = s.tenants.list_keys(Actor::Cli, Some(s.tenant));
            let stored = stored.expect("listed")[0].created_at;
            let stored = stored.format(&Rfc3339).expect("RFC 3339");
            assert_eq!(first["created_at"], stored.as_str());
            let fields: Vec<&String> = first.as_object().expect("an object").keys().collect();
            assert_eq!(
                fields,
                [
                    "created_at",
                    "id",
                    "label",
                    "revoked_at",
                    "scopes",
                    "tenant"
                ]
            );
            assert_eq!(first["tenant"], s.tenant.to_string().as_str());

            let made = admin_log(&s, "key_create");
            assert_eq!(made.len(), 2);
            assert_eq!(made[0]["status"], 201);
            assert_eq!(made[0]["target"], id.as_str());
            let copied = s.tenant_log("key_create");
            assert_eq!(copied.len(), 2, "the tenant's log shows it");
            assert_eq!(copied[0]["target"], id.as_str());
            assert_eq!(admin_log(&s, "key_list")[0]["status"], 200);
        }

        #[tokio::test]
        async fn a_revoked_key_answers_401_and_the_tenants_log_shows_it() {
            let s = server().await;
            let op = operator(&s);
            let (id, token) = issued(&s, &op, r#"{"scopes":["read"],"label":"a"}"#).await;
            let (_, kept) = issued(&s, &op, r#"{"scopes":["read"],"label":"b"}"#).await;
            let path = format!("/admin/keys/{id}");
            for _ in 0..2 {
                let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 204, "again too");
            }
            assert_eq!(health(&s, &token).await, 401);
            assert_eq!(health(&s, &kept).await, 200, "only that key");
            let (_, listed) = key_list(&s, &op).await;
            assert!(listed[0]["revoked_at"].is_string(), "{listed}");
            assert_eq!(listed[1]["revoked_at"], Value::Null);
            let revoked = s.tenant_log("key_revoke");
            assert_eq!(revoked.len(), 2);
            assert!(
                revoked
                    .iter()
                    .all(|r| r["status"] == 204 && r["target"] == id.as_str())
            );
            assert_eq!(admin_log(&s, "key_revoke").len(), 2);
        }

        #[tokio::test]
        async fn scopes_a_tenant_key_cannot_hold_are_422_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let path = format!("/admin/tenants/{}/keys", s.tenant);
            let scopes = [
                "[]",
                r#"["admin"]"#,
                r#"["read","admin"]"#,
                r#"["superuser"]"#,
                r#"["read","superuser"]"#,
                r#"["Read"]"#,
                r#"["read","read"]"#,
            ];
            for scopes in scopes {
                let body = format!(r#"{{"scopes":{scopes},"label":"x"}}"#);
                let answer = call(&s, Method::POST, &path, Some(&op), Some(&body)).await;
                assert_eq!(answer.status(), 422, "{scopes}");
                assert_eq!(problem(answer).await, "bad-scopes", "{scopes}");
            }
            let refused = admin_log(&s, "key_create");
            assert_eq!(refused.len(), scopes.len(), "each recorded once");
            assert!(refused.iter().all(|r| r["status"] == 422));
            let keys = s.tenants.list_keys(Actor::Cli, Some(s.tenant));
            assert_eq!(keys.expect("listed").len(), 0, "no key made");
        }

        #[tokio::test]
        async fn a_key_body_it_does_not_take_is_422_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let path = format!("/admin/tenants/{}/keys", s.tenant);
            let bodies = [
                "",
                "{}",
                r#"{"scopes":["read"]}"#,
                r#"{"label":"x"}"#,
                r#"{"scopes":"read","label":"x"}"#,
                r#"{"scopes":["read"],"label":7}"#,
                r#"{"scopes":["read"],"label":"x","tenant":"y"}"#,
            ];
            for body in bodies {
                let answer = call(&s, Method::POST, &path, Some(&op), Some(body)).await;
                assert_eq!(answer.status(), 422, "{body}");
                assert_eq!(problem(answer).await, "invalid-body", "{body}");
            }
            let refused = admin_log(&s, "key_create");
            assert_eq!(refused.len(), bodies.len());
            assert!(refused.iter().all(|r| r["status"] == 422));
        }

        #[tokio::test]
        async fn key_routes_for_no_tenant_or_no_key_are_404_and_recorded() {
            let s = server().await;
            let op = operator(&s);
            let body = Some(r#"{"scopes":["read"],"label":"x"}"#);
            let unknown = TenantId([7; 16]).to_string();
            for id in [unknown.as_str(), "acme"] {
                let path = format!("/admin/tenants/{id}/keys");
                let answer = call(&s, Method::POST, &path, Some(&op), body).await;
                assert_eq!(answer.status(), 404, "POST {id}");
                assert_eq!(problem(answer).await, "unknown-tenant");
                let answer = call(&s, Method::GET, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 404, "GET {id}");
                assert_eq!(problem(answer).await, "unknown-tenant");
            }
            let unknown_key = KeyId([7; 8]).to_string();
            for id in [unknown_key.as_str(), "acme", &unknown_key.to_uppercase()] {
                let path = format!("/admin/keys/{id}");
                let answer = call(&s, Method::DELETE, &path, Some(&op), None).await;
                assert_eq!(answer.status(), 404, "DELETE {id}");
                assert_eq!(problem(answer).await, "unknown-key");
            }
            for (action, count) in [("key_create", 2), ("key_list", 2), ("key_revoke", 3)] {
                let refused = admin_log(&s, action);
                assert_eq!(refused.len(), count, "{action}");
                assert!(refused.iter().all(|r| r["status"] == 404), "{action}");
            }
        }

        #[tokio::test]
        async fn requests_during_a_removal_get_503_then_401() {
            let s = server().await;
            let op = operator(&s);
            stored(&s).await;
            let uuid = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";
            let writer = s.key(&[Scope::Write]);
            let reader = s.key(&[Scope::Read]);
            let legal = s.key(&[Scope::Legal]);
            let a4 = std::fs::read(format!(
                "{}/../vcon/tests/fixtures/ietf/ab_call_ext_rec.vcon",
                env!("CARGO_MANIFEST_DIR")
            ))
            .expect("A.4");
            let requests = || {
                let client = s.client();
                [
                    client
                        .post(s.url("/vcons"))
                        .bearer_auth(&writer)
                        .header("content-type", "application/vcon")
                        .body(a4.clone()),
                    client
                        .get(s.url(&format!("/vcons/{uuid}")))
                        .bearer_auth(&reader),
                    client
                        .get(s.url(&format!("/legal/vcons/{uuid}")))
                        .bearer_auth(&legal),
                    client.get(s.url("/health")).bearer_auth(&reader),
                ]
            };

            // A request still holding the tenant keeps the removal draining.
            let in_flight = s.tenants.store(s.tenant).expect("its store");
            let removal = {
                let request = s
                    .client()
                    .delete(s.url(&format!("/admin/tenants/{}?force=true", s.tenant)))
                    .bearer_auth(&op);
                tokio::spawn(async move { request.send().await.expect("an answer").status() })
            };
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
            while s.tenants.store(s.tenant).is_ok() {
                assert!(
                    tokio::time::Instant::now() < deadline,
                    "the removal never began"
                );
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            for request in requests() {
                let answer = request.send().await.expect("an answer");
                let url = answer.url().path().to_owned();
                assert_eq!(answer.status(), 503, "{url} while removing");
                assert_eq!(problem(answer).await, "unavailable", "{url}");
            }
            assert!(!removal.is_finished(), "still draining");

            drop(in_flight);
            assert_eq!(removal.await.expect("joined"), 204);
            for request in requests() {
                let answer = request.send().await.expect("an answer");
                let url = answer.url().path().to_owned();
                assert_eq!(answer.status(), 401, "{url} once removed");
            }
        }

        #[tokio::test]
        async fn a_key_for_a_closing_tenant_is_503() {
            let s = server().await;
            let op = operator(&s);
            s.tenants.close(s.tenant);
            let path = format!("/admin/tenants/{}/keys", s.tenant);
            let body = Some(r#"{"scopes":["read"],"label":"x"}"#);
            let answer = call(&s, Method::POST, &path, Some(&op), body).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn a_body_over_the_limit_is_413() {
            let s = server().await;
            let op = operator(&s);
            let body = format!(r#"{{"name":"{}"}}"#, "x".repeat(super::pipeline::LIMIT));
            let answer = call(&s, Method::POST, "/admin/tenants", Some(&op), Some(&body)).await;
            assert_eq!(answer.status(), 413);
            assert_eq!(problem(answer).await, "too-large");
        }
    }

    mod by_uuid {
        use super::pipeline::{Server, problem, server, server_keeping};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::ids::TenantId;
        use crate::retention::Retention;

        const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

        fn fixture(name: &str) -> Vec<u8> {
            let path = format!(
                "{}/../vcon/tests/fixtures/ietf/{name}",
                env!("CARGO_MANIFEST_DIR")
            );
            std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
        }

        fn gzip(body: &[u8]) -> Vec<u8> {
            use std::io::Write;
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(body).expect("gzip");
            encoder.finish().expect("gzip")
        }

        /// A key of this tenant's with these scopes.
        fn key_in(s: &Server, tenant: TenantId, scopes: &[Scope]) -> String {
            s.tenants
                .create_key(Actor::Cli, Some(tenant), Scopes::of(scopes), "test")
                .expect("a key")
                .token
        }

        async fn get(s: &Server, token: &str, uuid: &str) -> reqwest::Response {
            s.client()
                .get(s.url(&format!("/vcons/{uuid}")))
                .bearer_auth(token)
                .send()
                .await
                .expect("an answer")
        }

        async fn post(s: &Server, token: &str, content_type: &str, body: Vec<u8>) -> String {
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(token)
                .header("content-type", content_type)
                .body(body)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 201);
            answer.headers()["etag"].to_str().expect("ascii").to_owned()
        }

        /// A.4, A.6 and A.7 share one uuid, so each runs in its own tenant.
        #[tokio::test]
        async fn the_core_examples_come_back_byte_for_byte_plain_and_gzipped() {
            let s = server().await;
            let files = [
                "ab_call_ext_rec.vcon",
                "ab_call_ext_rec_signed.vcon",
                "ab_call_ext_rec_encrypted.vcon",
            ];
            for (n, file) in files.into_iter().enumerate() {
                for gzipped in [false, true] {
                    let tenant = s
                        .tenants
                        .create(Actor::Cli, &format!("t{n}{gzipped}"), None)
                        .expect("a tenant");
                    let token = key_in(&s, tenant, &[Scope::Write, Scope::Read]);
                    let plain = fixture(file);
                    let (sent, content_type) = if gzipped {
                        (gzip(&plain), "application/vcon+gzip")
                    } else {
                        (plain, "application/vcon")
                    };
                    let tag = post(&s, &token, content_type, sent.clone()).await;
                    let answer = get(&s, &token, UUID).await;
                    assert_eq!(answer.status(), 200, "{file} {gzipped}");
                    assert_eq!(answer.headers()["content-type"], content_type);
                    assert_eq!(answer.headers()["etag"], tag.as_str());
                    assert!(answer.headers().get("content-encoding").is_none());
                    let got = answer.bytes().await.expect("body");
                    assert_eq!(got.as_ref(), sent.as_slice(), "{file} {gzipped}");
                }
            }
        }

        #[tokio::test]
        async fn a_uuid_is_found_in_either_case() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read]);
            post(
                &s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            let answer = get(&s, &token, &UUID.to_uppercase()).await;
            assert_eq!(answer.status(), 200);
        }

        #[tokio::test]
        async fn a_uuid_never_stored_is_not_found_and_a_deleted_one_is_gone() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read]);
            let answer = get(&s, &token, UUID).await;
            assert_eq!(answer.status(), 404);
            assert_eq!(problem(answer).await, "not-found");
            post(
                &s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .delete(
                    &crate::store::Caller {
                        key: crate::ids::KeyId([9; 8]),
                    },
                    UUID.parse().expect("uuid"),
                    crate::retention::Retention::Years(7),
                )
                .expect("deleted");
            let answer = get(&s, &token, UUID).await;
            assert_eq!(answer.status(), 410);
            assert_eq!(problem(answer).await, "gone");
        }

        #[tokio::test]
        async fn a_path_uuid_not_in_text_form_is_a_bad_request_and_recorded() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Read]);
            let simple = UUID.replace('-', "");
            for bad in [
                "not-a-uuid".to_owned(),
                simple.clone(),
                format!("%7B{UUID}%7D"),
                format!("urn:uuid:{UUID}"),
                format!("{UUID}0"),
            ] {
                let answer = get(&s, &token, &bad).await;
                assert_eq!(answer.status(), 400, "{bad}");
                assert_eq!(problem(answer).await, "bad-path-uuid");
            }
            let refused = s.tenant_log("read");
            assert_eq!(refused.len(), 5);
            assert_eq!(refused[0]["status"], 400);
        }

        #[tokio::test]
        async fn a_key_without_read_is_forbidden() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write]);
            let answer = get(&s, &token, UUID).await;
            assert_eq!(answer.status(), 403);
        }

        #[tokio::test]
        async fn a_read_whose_record_fails_sends_nothing() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read]);
            post(
                &s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = get(&s, &token, UUID).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        #[tokio::test]
        async fn a_version_whose_file_is_missing_is_damaged() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read]);
            let tag = post(
                &s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            let object = s
                .dir()
                .join("tenants")
                .join(s.tenant.to_string())
                .join("objects")
                .join(tag.trim_matches('"'));
            std::fs::remove_file(&object).expect("remove the object file");
            let answer = get(&s, &token, UUID).await;
            assert_eq!(answer.status(), 500);
            assert_eq!(problem(answer).await, "damaged");
        }

        async fn put(
            s: &Server,
            token: &str,
            uuid: &str,
            if_match: Option<&str>,
            body: Vec<u8>,
        ) -> reqwest::Response {
            let mut request = s
                .client()
                .put(s.url(&format!("/vcons/{uuid}")))
                .bearer_auth(token)
                .header("content-type", "application/vcon")
                .body(body);
            if let Some(tag) = if_match {
                request = request.header("if-match", tag);
            }
            request.send().await.expect("an answer")
        }

        /// A writer and reader in the fixture's tenant, with A.4 stored.
        async fn stored_a4(s: &Server) -> (String, String) {
            let token = key_in(s, s.tenant, &[Scope::Write, Scope::Read]);
            let tag = post(
                s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            (token, tag)
        }

        /// An unsigned vCon may be replaced by its signed form, and a signed
        /// one is replaced by nothing.
        #[tokio::test]
        async fn a4_then_a6_by_put_and_a7_after_a6_conflicts() {
            let s = server().await;
            let (token, a4) = stored_a4(&s).await;
            let a6 = fixture("ab_call_ext_rec_signed.vcon");
            let answer = put(&s, &token, UUID, Some(&a4), a6.clone()).await;
            assert_eq!(answer.status(), 200);
            let new_tag = answer.headers()["etag"].to_str().expect("ascii").to_owned();
            assert_ne!(new_tag, a4);
            let read = get(&s, &token, UUID).await;
            assert_eq!(read.headers()["etag"], new_tag.as_str());
            assert_eq!(read.bytes().await.expect("body").as_ref(), a6.as_slice());

            let a7 = fixture("ab_call_ext_rec_encrypted.vcon");
            let answer = put(&s, &token, UUID, Some(&new_tag), a7).await;
            assert_eq!(answer.status(), 409);
            assert_eq!(problem(answer).await, "immutable");
        }

        #[tokio::test]
        async fn a_put_without_if_match_is_refused_and_recorded() {
            let s = server().await;
            let (token, _) = stored_a4(&s).await;
            let a6 = fixture("ab_call_ext_rec_signed.vcon");
            let answer = put(&s, &token, UUID, None, a6).await;
            assert_eq!(answer.status(), 428);
            assert_eq!(problem(answer).await, "precondition-required");
            assert_eq!(s.tenant_log("replace")[0]["status"], 428);
        }

        #[tokio::test]
        async fn an_if_match_that_does_not_name_the_current_version_fails() {
            let s = server().await;
            let (token, a4) = stored_a4(&s).await;
            let a6 = fixture("ab_call_ext_rec_signed.vcon");
            let weak = format!("W/{a4}");
            let listed = format!("{a4}, \"00000000000000000000000000000000\"");
            let stale = format!("\"{}\"", crate::ids::VersionId([7; 16]));
            for tag in [
                "*",
                weak.as_str(),
                listed.as_str(),
                "garbage",
                stale.as_str(),
            ] {
                let answer = put(&s, &token, UUID, Some(tag), a6.clone()).await;
                assert_eq!(answer.status(), 412, "{tag}");
                assert_eq!(problem(answer).await, "precondition-failed");
            }
            let current = get(&s, &token, UUID).await;
            assert_eq!(current.headers()["etag"], a4.as_str(), "nothing replaced");
        }

        #[tokio::test]
        async fn a_body_naming_another_uuid_is_unprocessable() {
            let s = server().await;
            let (token, a4) = stored_a4(&s).await;
            let (_, other) =
                crate::store::testing::vcon("01a07da8-c2bb-83e5-b9a2-279e0d16bc47", "x");
            let answer = put(&s, &token, UUID, Some(&a4), other).await;
            assert_eq!(answer.status(), 422);
            assert_eq!(problem(answer).await, "path-uuid-mismatch");
            assert_eq!(s.tenant_log("replace")[0]["status"], 422);
        }

        #[tokio::test]
        async fn a_put_to_a_uuid_never_stored_or_deleted_or_badly_written() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read]);
            let tag = format!("\"{}\"", crate::ids::VersionId([7; 16]));
            let a4 = fixture("ab_call_ext_rec.vcon");
            let answer = put(&s, &token, UUID, Some(&tag), a4.clone()).await;
            assert_eq!(answer.status(), 404);
            assert_eq!(problem(answer).await, "not-found");

            let answer = put(&s, &token, "not-a-uuid", Some(&tag), a4.clone()).await;
            assert_eq!(answer.status(), 400);
            assert_eq!(problem(answer).await, "bad-path-uuid");

            let a4_tag = post(&s, &token, "application/vcon", a4.clone()).await;
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .delete(
                    &crate::store::Caller {
                        key: crate::ids::KeyId([9; 8]),
                    },
                    UUID.parse().expect("uuid"),
                    crate::retention::Retention::Years(7),
                )
                .expect("deleted");
            let answer = put(&s, &token, UUID, Some(&a4_tag), a4).await;
            assert_eq!(answer.status(), 410);
            assert_eq!(problem(answer).await, "gone");
        }

        #[tokio::test]
        async fn a_put_needs_write_and_a_vcon_media_type() {
            let s = server().await;
            let (_, a4) = stored_a4(&s).await;
            let reader = key_in(&s, s.tenant, &[Scope::Read]);
            let a6 = fixture("ab_call_ext_rec_signed.vcon");
            let answer = put(&s, &reader, UUID, Some(&a4), a6.clone()).await;
            assert_eq!(answer.status(), 403);
            let writer = key_in(&s, s.tenant, &[Scope::Write]);
            let answer = s
                .client()
                .put(s.url(&format!("/vcons/{UUID}")))
                .bearer_auth(&writer)
                .header("content-type", "text/plain")
                .header("if-match", &a4)
                .body(a6)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 415);
        }

        async fn delete(s: &Server, token: &str, uuid: &str) -> reqwest::Response {
            s.client()
                .delete(s.url(&format!("/vcons/{uuid}")))
                .bearer_auth(token)
                .send()
                .await
                .expect("an answer")
        }

        #[tokio::test]
        async fn a_delete_answers_204_again_and_again() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Write, Scope::Read, Scope::Delete]);
            post(
                &s,
                &token,
                "application/vcon",
                fixture("ab_call_ext_rec.vcon"),
            )
            .await;
            for _ in 0..2 {
                let answer = delete(&s, &token, UUID).await;
                assert_eq!(answer.status(), 204);
                assert!(answer.bytes().await.expect("body").is_empty());
            }
            assert_eq!(get(&s, &token, UUID).await.status(), 410);
            let deletes = s.tenant_log("delete");
            assert_eq!(deletes.len(), 2, "Store records each");
        }

        #[tokio::test]
        async fn a_delete_of_a_uuid_never_stored_or_badly_written() {
            let s = server().await;
            let token = key_in(&s, s.tenant, &[Scope::Delete]);
            let answer = delete(&s, &token, UUID).await;
            assert_eq!(answer.status(), 404);
            assert_eq!(problem(answer).await, "not-found");
            let answer = delete(&s, &token, "not-a-uuid").await;
            assert_eq!(answer.status(), 400);
            assert_eq!(problem(answer).await, "bad-path-uuid");
            assert_eq!(
                s.tenant_log("delete").last().expect("recorded")["status"],
                400
            );
        }

        #[tokio::test]
        async fn a_delete_needs_the_delete_scope_and_its_record() {
            let s = server().await;
            let (writer, _) = stored_a4(&s).await;
            let answer = delete(&s, &writer, UUID).await;
            assert_eq!(answer.status(), 403);
            let deleter = key_in(&s, s.tenant, &[Scope::Delete]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = delete(&s, &deleter, UUID).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        /// The purge records of `s`'s tenant whose actor is `token`'s key.
        fn purged_by(s: &Server, token: &str) -> usize {
            s.tenant_log("purge")
                .iter()
                .filter(|r| r["actor"] == token[4..20])
                .count()
        }

        #[tokio::test]
        async fn a_delete_erases_at_once_under_the_tenants_zero_period() {
            let s = server().await;
            s.tenants
                .set_retention(Actor::Cli, s.tenant, Some(Retention::Days(0)))
                .expect("set");
            let (_, _) = stored_a4(&s).await;
            let deleter = key_in(&s, s.tenant, &[Scope::Delete]);
            assert_eq!(delete(&s, &deleter, UUID).await.status(), 204);
            assert_eq!(purged_by(&s, &deleter), 1);
        }

        #[tokio::test]
        async fn a_delete_uses_the_server_default_when_the_tenant_sets_none() {
            let s = server_keeping(Retention::Days(0)).await;
            let (_, _) = stored_a4(&s).await;
            let deleter = key_in(&s, s.tenant, &[Scope::Delete]);
            assert_eq!(delete(&s, &deleter, UUID).await.status(), 204);
            assert_eq!(purged_by(&s, &deleter), 1);
        }

        #[tokio::test]
        async fn a_delete_keeps_a_young_vcon_for_legal_reads() {
            let s = server().await;
            let (_, _) = stored_a4(&s).await;
            let deleter = key_in(&s, s.tenant, &[Scope::Delete]);
            assert_eq!(delete(&s, &deleter, UUID).await.status(), 204);
            assert_eq!(purged_by(&s, &deleter), 0);
        }

        #[tokio::test]
        async fn a_delete_whose_period_cannot_be_read_deletes_nothing() {
            let s = server().await;
            let (_, _) = stored_a4(&s).await;
            s.tenants
                .conn_for_test()
                .execute(
                    "UPDATE tenant SET retention = '7w' WHERE id = ?1",
                    [s.tenant.0],
                )
                .expect("damaged");
            let token = key_in(&s, s.tenant, &[Scope::Delete, Scope::Read]);
            let answer = delete(&s, &token, UUID).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
            assert_eq!(get(&s, &token, UUID).await.status(), 200);
            assert_eq!(
                s.tenant_log("delete").last().expect("recorded")["status"],
                503
            );
        }

        async fn health(s: &Server, token: Option<&str>) -> reqwest::Response {
            let mut request = s.client().get(s.url("/health"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        #[tokio::test]
        async fn health_needs_a_key_of_any_scope_and_is_counted() {
            let s = server().await;
            assert_eq!(health(&s, None).await.status(), 401);
            let reader = key_in(&s, s.tenant, &[Scope::Read]);
            for _ in 0..2 {
                let answer = health(&s, Some(&reader)).await;
                assert_eq!(answer.status(), 200);
                assert_eq!(answer.headers()["content-type"], "application/json");
            }
            let counted = s.tenant_log("health");
            assert_eq!(counted.len(), 1, "one record a minute");
            assert_eq!(counted[0]["count"], 2);
            assert_eq!(counted[0]["status"], 200);

            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            assert_eq!(health(&s, Some(&operator)).await.status(), 200);
            let admin: Vec<_> = s
                .tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == "health")
                .collect();
            assert_eq!(admin.len(), 1);
            assert_eq!(admin[0]["actor"], operator[4..20]);
        }

        #[tokio::test]
        async fn health_is_unavailable_when_the_catalog_does_not_answer() {
            let s = server().await;
            let reader = key_in(&s, s.tenant, &[Scope::Read]);
            s.tenants
                .conn_for_test()
                .execute_batch("ALTER TABLE tenant RENAME TO gone")
                .expect("lose the table");
            let answer = health(&s, Some(&reader)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }

        #[tokio::test]
        async fn health_whose_count_fails_is_unavailable() {
            let s = server().await;
            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            s.tenants.break_audit();
            let answer = health(&s, Some(&operator)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }
    }

    /// `GET /openapi.json`: the description, to any valid key.
    mod openapi_route {
        use super::pipeline::{Server, problem, server};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;

        async fn get(s: &Server, token: Option<&str>) -> reqwest::Response {
            let mut request = s.client().get(s.url("/openapi.json"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        fn admin_openapi_records(s: &Server) -> usize {
            let mut out = Vec::new();
            s.tenants.export_admin(None, &mut out).expect("export");
            String::from_utf8(out)
                .expect("UTF-8")
                .lines()
                .filter(|l| l.contains("\"action\":\"openapi\""))
                .count()
        }

        #[tokio::test]
        async fn any_valid_key_gets_the_description_as_compiled_and_it_is_recorded() {
            let s = server().await;
            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("an operator key")
                .token;
            for token in [s.key(&[Scope::Read]), s.key(&[Scope::Write]), operator] {
                let answer = get(&s, Some(&token)).await;
                assert_eq!(answer.status(), 200);
                assert_eq!(answer.headers()["content-type"], "application/json");
                assert_eq!(answer.text().await.expect("text"), super::super::OPENAPI);
            }
            let recorded = s.tenant_log("openapi");
            assert_eq!(recorded.len(), 2, "the two tenant keys");
            assert!(recorded.iter().all(|r| r["status"] == 200));
            assert_eq!(admin_openapi_records(&s), 1, "the operator key");
        }

        #[tokio::test]
        async fn no_key_is_401_and_a_read_that_cannot_be_recorded_503() {
            let s = server().await;
            let answer = get(&s, None).await;
            assert_eq!(answer.status(), 401);
            assert_eq!(problem(answer).await, "unauthorized");
            let reader = s.key(&[Scope::Read]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = get(&s, Some(&reader)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        #[tokio::test]
        async fn a_key_of_a_closing_tenant_is_503() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            s.tenants.close(s.tenant);
            let answer = get(&s, Some(&reader)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
        }
    }

    /// `GET /audit` and `GET /admin/audit`: a log a page at a time, as the
    /// command line exports it.
    mod audit_routes {
        use super::pipeline::{Server, problem, server};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::store::testing::vcon;

        fn operator(s: &Server) -> String {
            s.tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("an operator key")
                .token
        }

        async fn get(s: &Server, path: &str, token: Option<&str>) -> reqwest::Response {
            let mut request = s.client().get(s.url(path));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        async fn body(answer: reqwest::Response) -> Vec<u8> {
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["content-type"], "application/jsonl");
            answer.bytes().await.expect("body").to_vec()
        }

        /// The tenant's log as `audit export --tenant` writes it.
        fn tenant_export(s: &Server, since: Option<i64>) -> Vec<u8> {
            let mut out = Vec::new();
            s.tenants
                .store(s.tenant)
                .expect("its store")
                .export(since, &mut out)
                .expect("export");
            out
        }

        fn admin_export(s: &Server) -> Vec<u8> {
            let mut out = Vec::new();
            s.tenants.export_admin(None, &mut out).expect("export");
            out
        }

        async fn three_stored(s: &Server) {
            let writer = s.key(&[Scope::Write]);
            for n in 1..=3 {
                let answer = s
                    .client()
                    .post(s.url("/vcons"))
                    .bearer_auth(&writer)
                    .header("content-type", "application/vcon")
                    .body(vcon(&format!("01a07da8-c2bb-83e5-b9a2-{n:012x}"), "x").1)
                    .send()
                    .await
                    .expect("an answer");
                assert_eq!(answer.status(), 201);
            }
        }

        #[tokio::test]
        async fn the_tenant_log_is_its_export_byte_for_byte_a_page_at_a_time() {
            let s = server().await;
            three_stored(&s).await;
            let auditor = s.key(&[Scope::Audit]);
            let before = tenant_export(&s, None);
            assert_eq!(body(get(&s, "/audit", Some(&auditor)).await).await, before);
            let lines: Vec<&[u8]> = before.split_inclusive(|b| *b == b'\n').collect();
            assert_eq!(
                body(get(&s, "/audit?after=1&limit=2", Some(&auditor)).await).await,
                lines[1..3].concat()
            );
            // The reads are recorded, each after its own page.
            let reads = s.tenant_log("audit_read");
            assert_eq!(reads.len(), 2);
            assert!(reads.iter().all(|r| r["status"] == 200));
            let after = 3;
            let page = body(get(&s, &format!("/audit?after={after}"), Some(&auditor)).await).await;
            assert_eq!(page, tenant_export(&s, Some(after))[..page.len()]);
        }

        #[tokio::test]
        async fn the_admin_log_is_its_export_byte_for_byte() {
            let s = server().await;
            let op = operator(&s);
            let before = admin_export(&s);
            assert!(!before.is_empty(), "the tenant's creation, at least");
            assert_eq!(body(get(&s, "/admin/audit", Some(&op)).await).await, before);
            let lines: Vec<&[u8]> = before.split_inclusive(|b| *b == b'\n').collect();
            assert_eq!(
                body(get(&s, "/admin/audit?limit=1", Some(&op)).await).await,
                lines[0]
            );
            let mut recorded = Vec::new();
            s.tenants.export_admin(None, &mut recorded).expect("export");
            let reads = String::from_utf8(recorded)
                .expect("UTF-8")
                .lines()
                .filter(|l| l.contains("\"action\":\"audit_read\""))
                .count();
            assert_eq!(reads, 2);
        }

        #[tokio::test]
        async fn the_tenant_log_needs_a_tenant_key_with_audit() {
            let s = server().await;
            assert_eq!(get(&s, "/audit", None).await.status(), 401);
            for scopes in [
                &[Scope::Read][..],
                &[Scope::Legal],
                &[Scope::Write, Scope::Delete],
            ] {
                let answer = get(&s, "/audit", Some(&s.key(scopes))).await;
                assert_eq!(answer.status(), 403, "{scopes:?}");
                assert_eq!(problem(answer).await, "forbidden");
            }
            let answer = get(&s, "/audit", Some(&operator(&s))).await;
            assert_eq!(answer.status(), 403, "an operator key");
            let refused = s.tenant_log("audit_read");
            assert_eq!(refused.len(), 3, "the tenant keys' refusals");
            assert!(refused.iter().all(|r| r["status"] == 403));
        }

        #[tokio::test]
        async fn the_admin_log_needs_an_operator_key() {
            let s = server().await;
            assert_eq!(get(&s, "/admin/audit", None).await.status(), 401);
            let every = s.key(&[
                Scope::Write,
                Scope::Read,
                Scope::Delete,
                Scope::Legal,
                Scope::Audit,
            ]);
            let answer = get(&s, "/admin/audit", Some(&every)).await;
            assert_eq!(answer.status(), 403);
            assert_eq!(problem(answer).await, "forbidden");
            assert_eq!(s.tenant_log("audit_read").len(), 1, "recorded");
        }

        #[tokio::test]
        async fn a_query_the_routes_do_not_take_is_a_bad_query_and_recorded() {
            let s = server().await;
            let auditor = s.key(&[Scope::Audit]);
            let op = operator(&s);
            let bad = [
                "?since=3",
                "?after=-1",
                "?limit=0",
                "?limit=10001",
                "?after=1&after=2",
            ];
            for query in bad {
                for (path, token) in [("/audit", &auditor), ("/admin/audit", &op)] {
                    let answer = get(&s, &format!("{path}{query}"), Some(token)).await;
                    assert_eq!(answer.status(), 400, "{path}{query}");
                    assert_eq!(problem(answer).await, "bad-query");
                }
            }
            let refused = s.tenant_log("audit_read");
            assert_eq!(refused.len(), bad.len());
            assert!(refused.iter().all(|r| r["status"] == 400));
            let mut admin = Vec::new();
            s.tenants.export_admin(None, &mut admin).expect("export");
            let admin_refused = String::from_utf8(admin)
                .expect("UTF-8")
                .lines()
                .filter(|l| l.contains("\"action\":\"audit_read\"") && l.contains("\"status\":400"))
                .count();
            assert_eq!(admin_refused, bad.len());
        }

        #[tokio::test]
        async fn a_read_that_cannot_be_recorded_is_503() {
            let s = server().await;
            let auditor = s.key(&[Scope::Audit]);
            let op = operator(&s);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = get(&s, "/audit", Some(&auditor)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
            s.tenants.break_audit();
            let answer = get(&s, "/admin/audit", Some(&op)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }
    }

    /// `GET /vcons`: a tenant's live vCons, a page at a time.
    mod list {
        use std::collections::BTreeSet;

        use serde_json::Value;

        use super::pipeline::{Server, problem, server};
        use crate::access::{Scope, Scopes};
        use crate::audit::Actor;
        use crate::ids::TenantId;
        use crate::store::testing::vcon;

        fn uuid(n: u32) -> String {
            format!("01a07da8-c2bb-83e5-b9a2-{n:012x}")
        }

        fn key_of(s: &Server, tenant: TenantId, scopes: &[Scope]) -> String {
            s.tenants
                .create_key(Actor::Cli, Some(tenant), Scopes::of(scopes), "test")
                .expect("a key")
                .token
        }

        /// Stores vCon `n` with `writer` and returns its tag, unquoted.
        async fn stored(s: &Server, writer: &str, n: u32) -> String {
            let answer = s
                .client()
                .post(s.url("/vcons"))
                .bearer_auth(writer)
                .header("content-type", "application/vcon")
                .body(vcon(&uuid(n), "listed").1)
                .send()
                .await
                .expect("an answer");
            assert_eq!(answer.status(), 201);
            answer.headers()["etag"]
                .to_str()
                .expect("text")
                .trim_matches('"')
                .to_owned()
        }

        async fn list(s: &Server, token: Option<&str>, query: &str) -> reqwest::Response {
            let mut request = s.client().get(s.url(&format!("/vcons{query}")));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        async fn page(s: &Server, reader: &str, query: &str) -> Value {
            let answer = list(s, Some(reader), query).await;
            assert_eq!(answer.status(), 200, "{query}");
            assert_eq!(answer.headers()["content-type"], "application/json");
            serde_json::from_str(&answer.text().await.expect("text")).expect("JSON")
        }

        fn uuids(page: &Value) -> Vec<String> {
            page["vcons"]
                .as_array()
                .expect("a list")
                .iter()
                .map(|v| v["uuid"].as_str().expect("a uuid").to_owned())
                .collect()
        }

        /// Every vCon the reader's pages list, following `next`.
        async fn every_page(s: &Server, reader: &str, limit: u32) -> Vec<String> {
            let mut seen = Vec::new();
            let mut query = format!("?limit={limit}");
            for _ in 0..100 {
                let page = page(s, reader, &query).await;
                seen.extend(uuids(&page));
                match page["next"].as_str() {
                    Some(cursor) => query = format!("?limit={limit}&cursor={cursor}"),
                    None => return seen,
                }
            }
            panic!("the cursors end")
        }

        #[tokio::test]
        async fn the_list_is_newest_first_with_each_tag_and_time() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            let mut tags = Vec::new();
            for n in 1..=3 {
                tags.push(stored(&s, &writer, n).await);
            }
            let reader = s.key(&[Scope::Read]);
            let page = page(&s, &reader, "").await;
            assert_eq!(uuids(&page), [3, 2, 1].map(uuid));
            assert_eq!(page["next"], Value::Null);
            for (listed, tag) in page["vcons"]
                .as_array()
                .expect("a list")
                .iter()
                .zip(tags.iter().rev())
            {
                assert_eq!(listed["version"], tag.as_str());
                assert_eq!(listed["stored_at"], listed["updated_at"]);
                let at = listed["stored_at"].as_str().expect("a time");
                assert!(at.ends_with('Z'), "{at}");
            }
            let recorded = s.tenant_log("list");
            assert_eq!(recorded.len(), 1);
            assert_eq!(recorded[0]["status"], 200);
        }

        #[tokio::test]
        async fn paging_returns_each_vcon_once_and_a_deleted_one_never() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            for n in 1..=7 {
                stored(&s, &writer, n).await;
            }
            let deleted = s
                .client()
                .delete(s.url(&format!("/vcons/{}", uuid(4))))
                .bearer_auth(s.key(&[Scope::Delete]))
                .send()
                .await
                .expect("an answer");
            assert_eq!(deleted.status(), 204);
            let reader = s.key(&[Scope::Read]);
            for limit in [1, 2, 3, 6, 7, 100] {
                assert_eq!(
                    every_page(&s, &reader, limit).await,
                    [7, 6, 5, 3, 2, 1].map(uuid),
                    "limit {limit}"
                );
            }
        }

        #[tokio::test]
        async fn a_tenant_lists_its_own_vcons_never_another_tenants() {
            let s = server().await;
            let beta = s.tenants.create(Actor::Cli, "beta", None).expect("made");
            let acme_writer = s.key(&[Scope::Write]);
            let beta_writer = key_of(&s, beta, &[Scope::Write]);
            for n in [1, 3, 5] {
                stored(&s, &acme_writer, n).await;
            }
            // vCon 3 is held by both tenants, each its own.
            for n in [2, 3, 4] {
                stored(&s, &beta_writer, n).await;
            }
            let acme = s.key(&[Scope::Read]);
            let beta_reader = key_of(&s, beta, &[Scope::Read]);
            assert_eq!(every_page(&s, &acme, 1).await, [5, 3, 1].map(uuid));
            assert_eq!(every_page(&s, &beta_reader, 1).await, [4, 3, 2].map(uuid));
            // A cursor from the other tenant's list only sets a place.
            let beta_page = page(&s, &beta_reader, "?limit=1").await;
            let cursor = beta_page["next"].as_str().expect("a cursor");
            let mixed = page(&s, &acme, &format!("?cursor={cursor}")).await;
            let acme_uuids: BTreeSet<String> = [5, 3, 1].map(uuid).into_iter().collect();
            assert!(uuids(&mixed).iter().all(|u| acme_uuids.contains(u)));
        }

        #[tokio::test]
        async fn since_and_until_bound_the_first_store_time() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            stored(&s, &writer, 1).await;
            let reader = s.key(&[Scope::Read]);
            let first = page(&s, &reader, "").await;
            let at = first["vcons"][0]["stored_at"]
                .as_str()
                .expect("a time")
                .to_owned();
            let at = at.replace(':', "%3A");
            assert_eq!(
                uuids(&page(&s, &reader, &format!("?since={at}")).await),
                vec![uuid(1)]
            );
            assert_eq!(
                uuids(&page(&s, &reader, &format!("?until={at}")).await),
                Vec::<String>::new()
            );
        }

        #[tokio::test]
        async fn a_query_the_route_does_not_take_is_a_bad_query_and_recorded() {
            let s = server().await;
            let reader = s.key(&[Scope::Read]);
            let bad = [
                "?offset=2",
                "?limit=0",
                "?limit=1001",
                "?limit=ten",
                "?limit=1&limit=2",
                "?cursor=nope",
                "?since=yesterday",
                "?until=2026-10-25",
            ];
            for query in bad {
                let answer = list(&s, Some(&reader), query).await;
                assert_eq!(answer.status(), 400, "{query}");
                assert_eq!(problem(answer).await, "bad-query", "{query}");
            }
            let recorded = s.tenant_log("list");
            assert_eq!(recorded.len(), bad.len());
            assert!(recorded.iter().all(|r| r["status"] == 400));
        }

        #[tokio::test]
        async fn the_list_needs_a_tenant_key_with_read() {
            let s = server().await;
            let answer = list(&s, None, "").await;
            assert_eq!(answer.status(), 401);
            for scopes in [&[Scope::Write][..], &[Scope::Delete], &[Scope::Legal]] {
                let answer = list(&s, Some(&s.key(scopes)), "").await;
                assert_eq!(answer.status(), 403, "{scopes:?}");
                assert_eq!(problem(answer).await, "forbidden");
            }
            let operator = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("an operator key")
                .token;
            let answer = list(&s, Some(&operator), "").await;
            assert_eq!(answer.status(), 403);
            let recorded = s.tenant_log("list");
            assert_eq!(recorded.len(), 3, "the tenant keys' refusals");
            assert!(recorded.iter().all(|r| r["status"] == 403));
        }

        #[tokio::test]
        async fn damage_is_500_and_a_list_that_cannot_be_recorded_503() {
            let s = server().await;
            let writer = s.key(&[Scope::Write]);
            stored(&s, &writer, 1).await;
            let reader = s.key(&[Scope::Read]);
            let store = s.tenants.store(s.tenant).expect("its store");
            store
                .conn_for_test()
                .execute("DELETE FROM version", [])
                .expect("damaged");
            let answer = list(&s, Some(&reader), "").await;
            assert_eq!(answer.status(), 500);
            assert_eq!(problem(answer).await, "damaged");
            store.break_audit();
            let answer = list(&s, Some(&reader), "").await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }
    }

    /// `GET /key`: a key learns its own id, tenant and scopes, and the
    /// request is recorded before the answer goes out.
    mod key_info {
        use serde_json::Value;

        use super::pipeline::{Server, problem, server};
        use crate::access::{self, Scope, Scopes};
        use crate::audit::Actor;

        async fn key(s: &Server, token: Option<&str>) -> reqwest::Response {
            let mut request = s.client().get(s.url("/key"));
            if let Some(token) = token {
                request = request.bearer_auth(token);
            }
            request.send().await.expect("an answer")
        }

        /// What the key may do, and nothing else: its id, tenant and scopes.
        fn own(token: &str, tenant: Option<String>, scopes: &[&str]) -> Value {
            let id = access::parse(token).expect("a token").to_string();
            serde_json::json!({ "id": id, "tenant": tenant, "scopes": scopes })
        }

        #[tokio::test]
        async fn a_tenant_key_reads_its_own_record_and_the_read_is_recorded() {
            let s = server().await;
            let token = s.key(&[Scope::Read, Scope::Delete]);
            let answer = key(&s, Some(&token)).await;
            assert_eq!(answer.status(), 200);
            assert_eq!(answer.headers()["content-type"], "application/json");
            assert_eq!(answer.headers()["cache-control"], "no-store");
            let body: Value =
                serde_json::from_slice(&answer.bytes().await.expect("body")).expect("JSON");
            let tenant = Some(s.tenant.to_string());
            assert_eq!(body, own(&token, tenant, &["read", "delete"]));
            let records = s.tenant_log("key_info");
            assert_eq!(records.len(), 1, "one record per request");
            assert_eq!(records[0]["status"], 200);
            assert_eq!(records[0]["actor"], token[4..20]);
            assert_eq!(records[0]["target"], token[4..20], "the key read");
        }

        #[tokio::test]
        async fn an_operator_key_reads_its_record_in_the_admin_log() {
            let s = server().await;
            let token = s
                .tenants
                .create_key(Actor::Cli, None, Scopes::of(&[Scope::Admin]), "op")
                .expect("made")
                .token;
            let answer = key(&s, Some(&token)).await;
            assert_eq!(answer.status(), 200);
            let body: Value =
                serde_json::from_slice(&answer.bytes().await.expect("body")).expect("JSON");
            assert_eq!(body, own(&token, None, &["admin"]));
            let records: Vec<_> = s
                .tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == "key_info")
                .collect();
            assert_eq!(records.len(), 1);
            assert_eq!(records[0]["status"], 200);
            assert_eq!(records[0]["target"], token[4..20]);
            assert!(s.tenant_log("key_info").is_empty(), "no tenant's log");
        }

        #[tokio::test]
        async fn no_key_or_a_bad_one_is_401() {
            let s = server().await;
            for token in [None, Some("vcs_nonsense")] {
                let answer = key(&s, token).await;
                assert_eq!(answer.status(), 401, "{token:?}");
                assert_eq!(problem(answer).await, "unauthorized");
            }
        }

        #[tokio::test]
        async fn a_read_that_cannot_be_recorded_shows_nothing() {
            let s = server().await;
            let token = s.key(&[Scope::Read]);
            s.tenants.store(s.tenant).expect("its store").break_audit();
            let answer = key(&s, Some(&token)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "audit-failed");
        }

        #[tokio::test]
        async fn a_key_whose_tenant_is_closing_is_503() {
            let s = server().await;
            let token = s.key(&[Scope::Read]);
            s.tenants.close(s.tenant);
            let answer = key(&s, Some(&token)).await;
            assert_eq!(answer.status(), 503);
            assert_eq!(problem(answer).await, "unavailable");
            let refused: Vec<_> = s
                .tenants
                .admin_records()
                .into_iter()
                .filter(|r| r["action"] == "key_info")
                .collect();
            assert_eq!(refused.len(), 1, "the refusal is recorded");
            assert_eq!(refused[0]["status"], 503);
        }
    }

    /// The console's files are served through the API's own router, with
    /// its headers, and the API itself never asks a browser to keep or
    /// share anything.
    mod console {
        use super::pipeline::server;
        use crate::access::Scope;

        #[tokio::test]
        async fn the_router_serves_the_console_without_a_key() {
            let s = server().await;
            let files = [
                ("/console/", "index.html", "text/html; charset=utf-8"),
                (
                    "/console/console.js",
                    "console.js",
                    "text/javascript; charset=utf-8",
                ),
                (
                    "/console/console.css",
                    "console.css",
                    "text/css; charset=utf-8",
                ),
            ];
            for (path, file, media_type) in files {
                let answer = s.client().get(s.url(path)).send().await.expect("an answer");
                assert_eq!(answer.status(), 200, "{path}");
                assert_eq!(answer.headers()["content-type"], media_type, "{path}");
                for (name, value) in crate::console::HEADERS {
                    assert_eq!(answer.headers()[name], value, "{path} {name}");
                }
                let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("console")
                    .join(file);
                let source = std::fs::read(source).expect("the source file");
                assert_eq!(answer.bytes().await.expect("body"), source, "{path}");
            }
        }

        /// Every answer under /console carries the headers, a refusal too.
        #[tokio::test]
        async fn every_other_console_answer_carries_the_headers() {
            let s = server().await;
            let client = s.client_not_following();
            let answers = [
                (client.get(s.url("/console/other.js")), 404),
                (client.get(s.url("/console/a/b")), 404),
                (client.post(s.url("/console/")), 405),
                (client.get(s.url("/console")), 308),
            ];
            for (request, status) in answers {
                let answer = request.send().await.expect("an answer");
                assert_eq!(answer.status(), status, "{answer:?}");
                for (name, value) in crate::console::HEADERS {
                    assert_eq!(answer.headers()[name], value, "{status} {name}");
                }
                if status == 308 {
                    assert_eq!(answer.headers()["location"], "/console/");
                }
            }
            let elsewhere = client
                .get(s.url("/consoles"))
                .send()
                .await
                .expect("an answer");
            assert_eq!(elsewhere.status(), 404);
            assert!(
                !elsewhere.headers().contains_key("content-security-policy"),
                "only console answers carry its headers"
            );
        }

        #[tokio::test]
        async fn no_answer_sets_a_cookie_or_allows_another_origin() {
            let s = server().await;
            let token = s.key(&[Scope::Read]);
            for path in ["/console/", "/health", "/key", "/vcons/nonsense"] {
                let answer = s
                    .client()
                    .get(s.url(path))
                    .bearer_auth(&token)
                    .header("origin", "https://elsewhere.example.com")
                    .send()
                    .await
                    .expect("an answer");
                let headers = answer.headers();
                assert!(!headers.contains_key("set-cookie"), "{path}");
                assert!(
                    !headers.contains_key("access-control-allow-origin"),
                    "{path}"
                );
            }
        }
    }
}

/// The website documents every problem type at the URL the type names, so
/// a client following a `type` finds what it means. These tests hold the
/// pages to the code.
#[cfg(test)]
mod site_pages {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use super::*;

    /// One of every problem the API can answer. Each list sits behind an
    /// exhaustive match, so a new kind of problem fails to compile here
    /// until it is listed, and then fails the test until it has its page.
    pub(super) fn every_problem() -> Vec<Problem> {
        let mut all = vec![
            Problem::Unauthorized,
            Problem::Forbidden,
            Problem::TooLarge,
            Problem::Timeout,
            Problem::BadBody,
            Problem::Unavailable,
            Problem::AuditFailed,
            Problem::UnsupportedMediaType,
            Problem::BadPathUuid,
            Problem::BadPathVersion,
            Problem::PreconditionRequired,
            Problem::PathUuidMismatch,
            Problem::InvalidBody,
            Problem::BadQuery,
            Problem::UnknownKey,
        ];
        for problem in &all {
            match problem {
                Problem::Unauthorized
                | Problem::Forbidden
                | Problem::TooLarge
                | Problem::Timeout
                | Problem::BadBody
                | Problem::Unavailable
                | Problem::AuditFailed
                | Problem::UnsupportedMediaType
                | Problem::BadPathUuid
                | Problem::BadPathVersion
                | Problem::PreconditionRequired
                | Problem::PathUuidMismatch
                | Problem::InvalidBody
                | Problem::BadQuery
                | Problem::UnknownKey
                | Problem::Invalid(_)
                | Problem::Store(_)
                | Problem::Tenant(_) => {}
            }
        }
        let invalid = [
            Invalid::BadGzip,
            Invalid::NotJson,
            Invalid::DuplicateMember,
            Invalid::AmbiguousForm,
            Invalid::NoForm,
            Invalid::MissingUuid,
            Invalid::BadUuid,
            Invalid::MissingCreatedAt,
            Invalid::BadPayload,
            Invalid::UuidMismatch,
            Invalid::BadCritical,
            Invalid::UnsupportedExtension,
        ];
        for kind in invalid {
            match kind {
                Invalid::BadGzip
                | Invalid::NotJson
                | Invalid::DuplicateMember
                | Invalid::AmbiguousForm
                | Invalid::NoForm
                | Invalid::MissingUuid
                | Invalid::BadUuid
                | Invalid::MissingCreatedAt
                | Invalid::BadPayload
                | Invalid::UuidMismatch
                | Invalid::BadCritical
                | Invalid::UnsupportedExtension => all.push(Problem::Invalid(kind)),
            }
        }
        let store = [
            StoreError::Conflict,
            StoreError::Gone,
            StoreError::NotFound,
            StoreError::NotDeleted,
            StoreError::PreconditionFailed,
            StoreError::Immutable,
            StoreError::LimitReached,
            StoreError::Damaged,
            StoreError::AuditFailed,
            StoreError::Io,
        ];
        for error in store {
            match error {
                StoreError::Conflict
                | StoreError::Gone
                | StoreError::NotFound
                | StoreError::NotDeleted
                | StoreError::PreconditionFailed
                | StoreError::Immutable
                | StoreError::LimitReached
                | StoreError::Damaged
                | StoreError::AuditFailed
                | StoreError::Io => all.push(Problem::Store(error)),
            }
        }
        let tenant = [
            TenantError::NotFound,
            TenantError::NameTaken,
            TenantError::BadName,
            TenantError::HoldsVcons,
            TenantError::UnderHold,
            TenantError::LimitReached,
            TenantError::AuditFailed,
            TenantError::Closing,
            TenantError::CliOnly,
            TenantError::BadScopes,
            TenantError::WrongKey,
            TenantError::Io,
        ];
        for error in tenant {
            match error {
                TenantError::NotFound
                | TenantError::NameTaken
                | TenantError::BadName
                | TenantError::HoldsVcons
                | TenantError::UnderHold
                | TenantError::LimitReached
                | TenantError::AuditFailed
                | TenantError::Closing
                | TenantError::CliOnly
                | TenantError::BadScopes
                | TenantError::WrongKey
                | TenantError::Io => all.push(Problem::Tenant(error)),
            }
        }
        all
    }

    fn pages() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../website/content/problems")
    }

    #[test]
    fn every_problem_type_has_its_page_with_its_status_and_title() {
        for problem in every_problem() {
            let (name, title) = problem.name();
            let page = pages().join(format!("{name}.md"));
            let text = std::fs::read_to_string(&page)
                .unwrap_or_else(|e| panic!("{problem:?}: {}: {e}", page.display()));
            let status = problem.status().as_u16();
            for line in [
                format!("title = \"{name}\""),
                format!("status = {status}"),
                format!("problem_title = \"{title}\""),
            ] {
                assert!(
                    text.lines().any(|l| l == line),
                    "{}: no line `{line}`",
                    page.display()
                );
            }
        }
    }

    /// No page outlives its problem type.
    #[test]
    fn every_problem_page_is_a_problem_type() {
        let names: BTreeSet<String> = every_problem()
            .into_iter()
            .map(|p| p.name().0.to_owned())
            .collect();
        let pages: BTreeSet<String> = std::fs::read_dir(pages())
            .expect("the problem pages")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .filter(|f| f != "_index.md")
            .map(|f| f.strip_suffix(".md").unwrap_or(&f).to_owned())
            .collect();
        assert_eq!(pages, names);
    }
}

/// Reading the description, for the tests that hold it to the code.
#[cfg(test)]
mod description {
    use std::sync::OnceLock;

    use serde_json::Value;

    fn parsed() -> &'static Value {
        static PARSED: OnceLock<Value> = OnceLock::new();
        PARSED.get_or_init(|| serde_json::from_str(super::OPENAPI).expect("openapi.json is JSON"))
    }

    /// Whether the description lists `status` for `method` on `route`, a
    /// route template as the router writes it. The router's `{*name}`, the
    /// rest of a path, is OpenAPI's `{name}`.
    pub(super) fn lists(method: &str, route: &str, status: u16) -> bool {
        let path = route.replace("{*", "{");
        parsed()["paths"][path.as_str()][method.to_ascii_lowercase()]["responses"]
            .get(status.to_string())
            .is_some()
    }

    /// Whether the operation needs a key: it does unless it says
    /// `security: []`.
    pub(super) fn keyed(method: &str, path: &str) -> bool {
        parsed()["paths"][path][method.to_ascii_lowercase()]["security"]
            .as_array()
            .is_none_or(|s| !s.is_empty())
    }

    /// Every path and its methods, as the description names them.
    pub(super) fn operations() -> Vec<(String, String)> {
        let mut all = Vec::new();
        for (path, item) in parsed()["paths"].as_object().expect("paths") {
            for method in item.as_object().expect("a path item").keys() {
                if ["get", "put", "post", "delete", "patch"].contains(&method.as_str()) {
                    all.push((method.to_ascii_uppercase(), path.clone()));
                }
            }
        }
        all
    }

    pub(super) fn document() -> &'static Value {
        parsed()
    }
}

/// The description against the code: the published copy, the version, the
/// problem types and every route and method. Statuses are checked on every
/// answer the unit tests produce, in `log_request`.
#[cfg(test)]
mod openapi_description {
    use std::collections::BTreeSet;

    use reqwest::Method;

    use super::description::{document, keyed, lists, operations};
    use super::site_pages::every_problem;
    use super::tests::pipeline::server;
    use super::*;

    #[test]
    fn the_website_publishes_the_compiled_description() {
        let published = std::fs::read_to_string(format!(
            "{}/../../website/static/openapi.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("the website's copy");
        assert!(published == OPENAPI, "website/static/openapi.json differs");
    }

    #[test]
    fn it_is_openapi_3_1_for_this_version() {
        let doc = document();
        assert!(
            doc["openapi"]
                .as_str()
                .is_some_and(|v| v.starts_with("3.1.")),
            "{}",
            doc["openapi"]
        );
        assert_eq!(doc["info"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(doc["info"]["title"].as_str().is_some_and(|t| !t.is_empty()));
    }

    #[test]
    fn its_problem_types_are_the_ones_the_api_answers() {
        let described: BTreeSet<String> =
            document()["components"]["schemas"]["Problem"]["properties"]["type"]["enum"]
                .as_array()
                .expect("the Problem type list")
                .iter()
                .map(|t| t.as_str().expect("a URL").to_owned())
                .collect();
        let answered: BTreeSet<String> = every_problem()
            .into_iter()
            .map(|p| format!("https://vcon-server.com/problems/{}", p.name().0))
            .collect();
        assert_eq!(described, answered);
    }

    /// A path template with sample values, so it reaches its route.
    fn concrete(path: &str) -> String {
        path.replace("{uuid}", "01a07da8-c2bb-83e5-b9a2-279e0d16bc46")
            .replace("{version}", "00112233445566778899aabbccddeeff")
            .replace("{id}", "00112233445566778899aabbccddeeff")
            .replace("{other}", "no-such-file")
    }

    /// Every operation the description names is routed (one that needs a
    /// key asks for it; one that needs none answers a status it lists), and
    /// every other method on its paths is not (405).
    #[tokio::test]
    async fn every_described_route_is_routed_and_no_other_method() {
        let s = server().await;
        let described: BTreeSet<(String, String)> = operations().into_iter().collect();
        assert!(!described.is_empty());
        let paths: BTreeSet<&String> = described.iter().map(|(_, p)| p).collect();
        for path in paths {
            for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
                // Not following redirects, so each answer is the route's own.
                let answer = s
                    .client_not_following()
                    .request(
                        Method::from_bytes(method.as_bytes()).expect("a method"),
                        s.url(&concrete(path)),
                    )
                    .send()
                    .await
                    .expect("an answer");
                let status = answer.status().as_u16();
                if !described.contains(&(method.to_owned(), path.clone())) {
                    assert_eq!(status, 405, "{method} {path}");
                } else if keyed(method, path) {
                    assert_eq!(status, 401, "{method} {path}");
                } else {
                    assert!(lists(method, path, status), "{method} {path}: {status}");
                }
            }
        }
    }
}

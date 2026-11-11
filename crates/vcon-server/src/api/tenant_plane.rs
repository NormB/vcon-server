//! The routes of a tenant's administrator: `/tenant/settings`,
//! `/tenant/keys` and `/tenant/memberships`.
//!
//! Each needs a key of a tenant that holds `tenant_admin`, and works in
//! that key's tenant alone: no route takes a tenant's id. A refusal before
//! the route's own work is recorded here, in the tenant's log;
//! [`crate::tenants::Tenants`] records every outcome of the work itself, in
//! the admin log and the tenant's.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path as UriPath, State};
use axum::http::header::CACHE_CONTROL;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use serde::de::DeserializeOwned;

use super::{App, NewKeyBody, Problem, admit, blocking, json_answer, refusal};
use crate::access::{Scope, Scopes};
use crate::audit::{Action, Actor};
use crate::ids::{KeyId, TenantId};
use crate::tenants::TenantError;

/// A request admitted to one of these routes: the key that asked, its
/// tenant, and the body.
struct Asked {
    key: KeyId,
    tenant: TenantId,
    body: Bytes,
}

impl Asked {
    fn who(&self) -> Actor {
        Actor::Key(self.key)
    }
}

/// Admits a request as every tenant route is admitted, for a key that
/// holds `tenant_admin`.
async fn asked(
    app: &Arc<App>,
    headers: &HeaderMap,
    body: Body,
    action: Action,
) -> Result<Asked, Problem> {
    let admitted = admit(app, headers, body, Scope::TenantAdmin, action).await?;
    Ok(Asked {
        key: admitted.caller.key,
        tenant: admitted.tenant,
        body: admitted.body,
    })
}

/// The body as JSON of the kind the route takes, or the refusal, recorded
/// as `action` in the tenant's log.
async fn json_body<T: DeserializeOwned>(
    app: &Arc<App>,
    asked: &Asked,
    action: Action,
) -> Result<T, Problem> {
    match serde_json::from_slice(&asked.body) {
        Ok(read) => Ok(read),
        Err(_) => {
            let refused = Problem::InvalidBody;
            let entry = refusal(asked.key, action, refused);
            Err(app.refuse(Some(asked.tenant), entry, refused).await)
        }
    }
}

/// `GET /tenant/settings`: every setting of the key's tenant, each name
/// with what was set or its default.
pub(super) async fn settings_read(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let read = async {
        let asked = asked(&app, &headers, body, Action::SettingsRead).await?;
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.settings(asked.who(), asked.tenant))
            .await
            .map_err(Problem::Tenant)
    };
    match read.await {
        Ok(settings) => json_answer(StatusCode::OK, &settings.to_json()),
        Err(refused) => refused.into_response(),
    }
}

/// `PUT /tenant/settings`: changes the settings the body names, each to
/// its new value or with null to its default, and answers every setting
/// as it is after the change.
pub(super) async fn settings_set(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::SettingsSet;
    let set = async {
        let asked = asked(&app, &headers, body, action).await?;
        let change: serde_json::Value = json_body(&app, &asked, action).await?;
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.set_settings(asked.who(), asked.tenant, &change))
            .await
            .map_err(Problem::Tenant)
    };
    match set.await {
        Ok(settings) => json_answer(StatusCode::OK, &settings.to_json()),
        Err(refused) => refused.into_response(),
    }
}

/// `POST /tenant/keys`: makes a key for the caller's tenant, with scopes
/// the tenant's role cap allows and never `tenant_admin`, and answers 201
/// with its id and token. The token is shown this once.
pub(super) async fn key_create(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::KeyCreate;
    let made = async {
        let asked = asked(&app, &headers, body, action).await?;
        let new: NewKeyBody = json_body(&app, &asked, action).await?;
        let Some(scopes) = Scopes::named(&new.scopes) else {
            let refused = Problem::Tenant(TenantError::BadScopes);
            let entry = refusal(asked.key, action, refused);
            return Err(app.refuse(Some(asked.tenant), entry, refused).await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.create_tenant_key(asked.who(), asked.tenant, scopes, &new.label))
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

/// `GET /tenant/keys`: the keys of the caller's tenant, oldest first, with
/// their ids, scopes, labels and times, never a token.
pub(super) async fn key_list(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let listed = async {
        let asked = asked(&app, &headers, body, Action::KeyList).await?;
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.tenant_keys(asked.who(), asked.tenant))
            .await
            .map_err(Problem::Tenant)
    };
    let keys = match listed.await {
        Ok(keys) => keys,
        Err(refused) => return refused.into_response(),
    };
    let listed: Option<Vec<serde_json::Value>> = keys.iter().map(|key| key.to_json()).collect();
    match listed {
        Some(listed) => json_answer(StatusCode::OK, &serde_json::Value::Array(listed)),
        None => Problem::Tenant(TenantError::Io).into_response(),
    }
}

/// `DELETE /tenant/keys/{id}`: revokes a key of the caller's tenant; 204
/// again for a key already revoked. A key of another tenant, an operator
/// key and an id no key has are all 404 `unknown-key`.
pub(super) async fn key_revoke(
    State(app): State<Arc<App>>,
    UriPath(id): UriPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let action = Action::KeyRevoke;
    let revoked = async {
        let asked = asked(&app, &headers, body, action).await?;
        let Some(id) = KeyId::parse(&id) else {
            let refused = Problem::UnknownKey;
            let entry = refusal(asked.key, action, refused);
            return Err(app.refuse(Some(asked.tenant), entry, refused).await);
        };
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.revoke_tenant_key(asked.who(), asked.tenant, id))
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

/// `GET /tenant/memberships`: the memberships in the caller's tenant,
/// oldest first: each user's id and label, the roles granted, and the
/// scopes they give under the tenant's role cap.
pub(super) async fn member_list(
    State(app): State<Arc<App>>,
    headers: HeaderMap,
    body: Body,
) -> Response {
    let listed = async {
        let asked = asked(&app, &headers, body, Action::MembershipList).await?;
        let tenants = app.tenants.clone();
        // Tenants records its own outcomes.
        blocking(move || tenants.members(asked.who(), asked.tenant))
            .await
            .map_err(Problem::Tenant)
    };
    let members = match listed.await {
        Ok(members) => members,
        Err(refused) => return refused.into_response(),
    };
    let listed: Option<Vec<serde_json::Value>> =
        members.iter().map(|member| member.to_json()).collect();
    match listed {
        Some(listed) => json_answer(StatusCode::OK, &serde_json::Value::Array(listed)),
        None => Problem::Tenant(TenantError::Io).into_response(),
    }
}

//! The admin tools, for operator keys: the same work as the `/admin/`
//! routes. Tenants records each outcome, and a refusal before Tenants is
//! recorded in the admin log by the gate, as the routes record theirs.

use rmcp::model::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Hints, Outcome, Running, Server, Spec, arguments, budget, fence, tool};
use crate::access::{Caller, Scopes};
use crate::api::{self, Problem};
use crate::audit::{Action, Actor, AuditQuery};
use crate::ids::{KeyId, TenantId};
use crate::retention::Retention;
use crate::tenants::{KeyInfo, Removal, TenantError, TenantInfo};

/// The admin tools.
pub(super) fn specs() -> Vec<Spec> {
    let read = Hints {
        read_only: true,
        idempotent: true,
        ..Hints::NONE
    };
    let erase = Hints {
        destructive: true,
        ..Hints::NONE
    };
    vec![
        spec::<TenantCreateArgs>(
            "admin_tenant_create",
            "Makes a tenant with this name (1 to 128 characters) and, if given, its own \
             retention period (such as 7y, 18m or 90d; the server default when left out or \
             null), and returns its id.",
            Hints::NONE,
            Action::TenantCreate,
            tenant_create,
        ),
        spec::<PageArgs>(
            "admin_tenant_list",
            "Returns every tenant, by name: its id, its name between untrusted-vcon-data \
             markers, and its own retention period (null for the server default); from \
             cursor, within max_bytes, with next_cursor when more follow.",
            read,
            Action::TenantList,
            tenant_list,
        ),
        spec::<TenantRetentionArgs>(
            "admin_tenant_retention",
            "Sets the tenant's own retention period, or with null returns it to the server \
             default, and returns the tenant's id and its period.",
            Hints {
                idempotent: true,
                ..Hints::NONE
            },
            Action::TenantRetention,
            tenant_retention,
        ),
        spec::<TenantRemoveArgs>(
            "admin_tenant_remove",
            "Removes a tenant that keeps no vCon, or with force one that does unless a vCon \
             of it is under a legal hold, destroying its keys and data, and returns its id.",
            erase,
            Action::TenantRemove,
            tenant_remove,
        ),
        spec::<KeyCreateArgs>(
            "admin_key_create",
            "Makes a key for the tenant holding these scopes (write, read, delete, legal, \
             audit) with this label, and returns its id and its token. The token is returned \
             this once; only its hash is kept.",
            Hints::NONE,
            Action::KeyCreate,
            key_create,
        ),
        spec::<KeyListArgs>(
            "admin_key_list",
            "Returns the tenant's keys, oldest first: each id, tenant, scopes, label between \
             untrusted-vcon-data markers, and times made and revoked (RFC 3339); never a \
             token. From cursor, within max_bytes, with next_cursor when more follow.",
            read,
            Action::KeyList,
            key_list,
        ),
        spec::<KeyRevokeArgs>(
            "admin_key_revoke",
            "Revokes the key with this id, so its next request is refused, and returns the \
             id; a key already revoked answers the same.",
            Hints {
                destructive: true,
                idempotent: true,
                ..Hints::NONE
            },
            Action::KeyRevoke,
            key_revoke,
        ),
        spec::<AuditArgs>(
            "admin_audit_read",
            "Returns records of the admin log after the sequence number after (0, the \
             default, is the start), oldest first, at most limit of them (1 to 10000, default \
             1000), within max_bytes: each record as audit export --admin writes it, and \
             next_cursor, the seq of the last record returned.",
            Hints {
                read_only: true,
                ..Hints::NONE
            },
            Action::AuditRead,
            audit_read,
        ),
    ]
}

fn spec<A: rmcp::schemars::JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
    hints: Hints,
    action: Action,
    run: super::Run,
) -> Spec {
    Spec {
        tool: tool::<A>(name, description, hints),
        action,
        run,
    }
}

/// A tenant's id, or the refusal the admin routes give a malformed one:
/// unknown-tenant.
fn tenant_id(text: &str) -> Result<TenantId, Outcome> {
    TenantId::parse(text).ok_or(Outcome::Refused(Problem::Tenant(TenantError::NotFound)))
}

/// A retention period as text, null for the server default.
fn period(text: Option<&str>) -> Result<Option<Retention>, Outcome> {
    text.map(|t| Retention::parse(t).ok_or(Outcome::Arguments))
        .transpose()
}

/// What Tenants refused or failed, as the admin routes answer it.
fn answered(error: TenantError) -> Outcome {
    Outcome::Answered(Problem::Tenant(error))
}

/// A page of `rows` from `cursor`, a row index an earlier page gave, under
/// `key`, fitting `room`, with the provenance sentence.
fn rows_page(
    key: &str,
    rows: &[Value],
    cursor: Option<&str>,
    room: usize,
) -> Result<Value, Outcome> {
    let start = cursor
        .map(|c| c.parse::<usize>().map_err(|_| Outcome::Arguments))
        .transpose()?
        .unwrap_or(0);
    let rows = rows.get(start..).ok_or(Outcome::Arguments)?;
    let page = |n: usize| {
        json!({
            key: rows[..n],
            "truncated": n < rows.len(),
            "next_cursor": (n < rows.len()).then(|| (start + n).to_string()),
            "provenance": fence::PROVENANCE,
        })
    };
    let n = budget::rows_that_fit(rows.len(), |n| budget::size(&page(n)) <= room);
    Ok(page(n))
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct TenantCreateArgs {
    /// The tenant's name: 1 to 128 characters, no control character, no white
    /// space at either end.
    name: String,
    /// Its own retention period, such as 7y, 18m or 90d; the server default
    /// when left out or null.
    retention: Option<String>,
}

fn tenant_create<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: TenantCreateArgs = arguments(args)?;
        let retention = period(args.retention.as_deref())?;
        let tenants = server.app.tenants.clone();
        let id =
            api::blocking(move || tenants.create(Actor::Key(caller.key), &args.name, retention))
                .await
                .map_err(answered)?;
        Ok(json!({"id": id.to_string()}))
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct PageArgs {
    /// The next_cursor a previous page gave.
    cursor: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn tenant_list<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: PageArgs = arguments(args)?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let tenants = server.app.tenants.clone();
        let listed = api::blocking(move || tenants.list(Actor::Key(caller.key)))
            .await
            .map_err(answered)?;
        let rows: Vec<Value> = listed
            .iter()
            .map(|t: &TenantInfo| {
                let mut row = t.to_json();
                row["name"] = fence::fence_field(&t.name).into();
                row
            })
            .collect();
        rows_page("tenants", &rows, args.cursor.as_deref(), room)
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct TenantRetentionArgs {
    /// The tenant's id.
    id: String,
    /// The period, such as 7y, 18m or 90d, or null for the server default;
    /// it must be given.
    retention: Value,
}

fn tenant_retention<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: TenantRetentionArgs = arguments(args)?;
        let retention = match &args.retention {
            Value::Null => None,
            Value::String(text) => period(Some(text))?,
            _ => return Err(Outcome::Arguments),
        };
        let id = tenant_id(&args.id)?;
        let tenants = server.app.tenants.clone();
        api::blocking(move || tenants.set_retention(Actor::Key(caller.key), id, retention))
            .await
            .map_err(answered)?;
        Ok(json!({"id": id.to_string(), "retention": retention.map(|r| r.to_string())}))
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct TenantRemoveArgs {
    /// The tenant's id.
    id: String,
    /// Remove it even while it keeps vCons; a legal hold still refuses.
    force: Option<bool>,
}

fn tenant_remove<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: TenantRemoveArgs = arguments(args)?;
        let id = tenant_id(&args.id)?;
        // Nothing over the network overrides a legal hold.
        let removal = if args.force == Some(true) {
            Removal::Force
        } else {
            Removal::Empty
        };
        let tenants = server.app.tenants.clone();
        api::blocking(move || tenants.remove(Actor::Key(caller.key), id, removal))
            .await
            .map_err(answered)?;
        Ok(json!({"id": id.to_string()}))
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct KeyCreateArgs {
    /// The tenant the key belongs to.
    tenant: String,
    /// The scopes it holds: write, read, delete, legal, audit.
    scopes: Vec<String>,
    /// Who holds the key.
    label: String,
}

fn key_create<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: KeyCreateArgs = arguments(args)?;
        let tenant = tenant_id(&args.tenant)?;
        let scopes = Scopes::named(&args.scopes)
            .ok_or(Outcome::Refused(Problem::Tenant(TenantError::BadScopes)))?;
        let tenants = server.app.tenants.clone();
        let made = api::blocking(move || {
            tenants.create_key(Actor::Key(caller.key), Some(tenant), scopes, &args.label)
        })
        .await
        .map_err(answered)?;
        Ok(json!({"id": made.id.to_string(), "token": made.token}))
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct KeyListArgs {
    /// The tenant whose keys to list.
    tenant: String,
    /// The next_cursor a previous page gave.
    cursor: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn key_list<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: KeyListArgs = arguments(args)?;
        let tenant = tenant_id(&args.tenant)?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let tenants = server.app.tenants.clone();
        let keys = api::blocking(move || tenants.list_keys(Actor::Key(caller.key), Some(tenant)))
            .await
            .map_err(answered)?;
        let rows: Vec<Value> = keys
            .iter()
            .map(|k: &KeyInfo| {
                let mut row = k.to_json()?;
                row["label"] = fence::fence_field(&k.label).into();
                Some(row)
            })
            .collect::<Option<_>>()
            .ok_or_else(|| answered(TenantError::Io))?;
        rows_page("keys", &rows, args.cursor.as_deref(), room)
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct KeyRevokeArgs {
    /// The key's id.
    id: String,
}

fn key_revoke<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: KeyRevokeArgs = arguments(args)?;
        let id = KeyId::parse(&args.id).ok_or(Outcome::Refused(Problem::UnknownKey))?;
        let tenants = server.app.tenants.clone();
        api::blocking(move || tenants.revoke_key(Actor::Key(caller.key), id))
            .await
            .map_err(|error| match error {
                TenantError::NotFound => Outcome::Answered(Problem::UnknownKey),
                error => answered(error),
            })?;
        Ok(json!({"id": id.to_string()}))
    })
}

#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct AuditArgs {
    /// Records after this sequence number; 0 when left out.
    after: Option<u64>,
    /// How many records at most: 1 to 10000, 1000 when left out.
    limit: Option<u64>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn audit_read<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: AuditArgs = arguments(args)?;
        let query = AuditQuery::new(args.after, args.limit)
            .map_err(|_| Outcome::Refused(Problem::BadQuery))?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let tenants = server.app.tenants.clone();
        let lines = api::blocking(move || tenants.audit_page(Actor::Key(caller.key), &query))
            .await
            .map_err(answered)?;
        super::audit::records(lines, room)
    })
}

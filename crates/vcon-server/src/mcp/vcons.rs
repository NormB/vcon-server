//! The vCon tools, for tenant keys: the same work as the `/vcons` routes.

use rmcp::model::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Hints, Outcome, Running, Server, Spec, arguments, budget, opened, text_result, tool};
use crate::access::{Caller, Scope};
use crate::api::{self, Problem};
use crate::audit::Action;
use crate::ids::VersionId;
use crate::store::{Created, Cursor, ListQuery};

/// The vCon tools.
pub(super) fn specs() -> Vec<Spec> {
    let read = Hints {
        read_only: true,
        idempotent: true,
        ..Hints::NONE
    };
    vec![
        Spec {
            tool: tool::<GetArgs>(
                "vcon_get",
                "Returns the current version of the vCon with this uuid in the key's tenant: \
                 its uuid, version id, whether it was stored gzipped, and its JSON text \
                 between untrusted-vcon-data markers, from cursor up to max_bytes, with \
                 next_cursor when more follows.",
                read,
            ),
            action: Action::Read,
            run: get,
        },
        Spec {
            tool: tool::<ListArgs>(
                "vcon_list",
                "Returns the key's tenant's live vCons, newest stored first: each uuid, \
                 current version id, first stored time and current version's time, at most \
                 limit of them (1 to 1000, default 100), stored between since and until \
                 (RFC 3339), after cursor, within max_bytes, with next_cursor when more \
                 follow.",
                read,
            ),
            action: Action::List,
            run: list,
        },
        Spec {
            tool: tool::<StoreArgs>(
                "vcon_store",
                "Stores a new vCon, given as its JSON text, in the key's tenant, and returns \
                 its uuid, version id and created: true; the same bytes already current give \
                 created: false and the current version.",
                Hints {
                    idempotent: true,
                    ..Hints::NONE
                },
            ),
            action: Action::Create,
            run: store,
        },
        Spec {
            tool: tool::<ReplaceArgs>(
                "vcon_replace",
                "Replaces the current version of the vCon with this uuid in the key's \
                 tenant, which version must name, with the JSON text given, and returns the \
                 uuid and the new version id. The replaced version is kept for legal reads.",
                Hints::NONE,
            ),
            action: Action::Replace,
            run: replace,
        },
        Spec {
            tool: tool::<UuidArgs>(
                "vcon_delete",
                "Deletes the vCon with this uuid in the key's tenant, hiding it from vCon \
                 reads at once, and returns the uuid; a legal key can still read or restore \
                 it until its retention period ends, and a deleted uuid answers the same \
                 again.",
                Hints {
                    destructive: true,
                    idempotent: true,
                    ..Hints::NONE
                },
            ),
            action: Action::Delete,
            run: delete,
        },
    ]
}

/// The arguments of `vcon_store`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct StoreArgs {
    /// The vCon's JSON text, stored byte for byte as given.
    vcon: String,
}

fn store<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: StoreArgs = arguments(args)?;
        let body = axum::body::Bytes::from(args.vcon);
        let found = api::read_vcon(&server.app, body.clone(), None)
            .await
            .map_err(Outcome::Refused)?;
        let at = opened(server, caller, Scope::Write).await?;
        // Store records its own outcome.
        let made = api::blocking(move || at.store.create(&at.who, &found, &body))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        let (version, created) = match made {
            Created::New(version) => (version, true),
            Created::Same(version) => (version, false),
        };
        Ok(json!({
            "uuid": found.uuid.to_string(),
            "version": version.to_string(),
            "created": created,
        }))
    })
}

/// The arguments of `vcon_replace`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct ReplaceArgs {
    /// The vCon's uuid, in RFC 9562 text form; the new text must hold it.
    uuid: String,
    /// The current version's id, as vcon_get or vcon_store gave it.
    version: String,
    /// The new version's JSON text, stored byte for byte as given.
    vcon: String,
}

fn replace<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: ReplaceArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let current = VersionId::parse(&args.version).ok_or(Outcome::Arguments)?;
        let body = axum::body::Bytes::from(args.vcon);
        let found = api::read_vcon(&server.app, body.clone(), None)
            .await
            .map_err(Outcome::Refused)?;
        if found.uuid != uuid {
            return Err(Outcome::Refused(Problem::PathUuidMismatch));
        }
        let at = opened(server, caller, Scope::Write).await?;
        let version = api::blocking(move || at.store.replace(&at.who, current, &found, &body))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        Ok(json!({"uuid": uuid.to_string(), "version": version.to_string()}))
    })
}

/// The arguments of a tool that takes only a uuid.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct UuidArgs {
    /// The vCon's uuid, in RFC 9562 text form.
    pub(super) uuid: String,
}

fn delete<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: UuidArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let at = opened(server, caller, Scope::Delete).await?;
        // The tenant's own period, else the server's, read now, as
        // DELETE /vcons/{uuid} reads it; unread, nothing is deleted.
        let tenants = server.app.tenants.clone();
        let tenant = caller.tenant.ok_or(Outcome::Refused(Problem::Forbidden))?;
        let retention = api::blocking(move || tenants.retention(tenant))
            .await
            .map_err(|_| Outcome::Refused(Problem::Unavailable))?
            .unwrap_or(server.app.retention_default);
        api::blocking(move || at.store.delete(&at.who, uuid, retention))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        Ok(json!({"uuid": uuid.to_string()}))
    })
}

/// The arguments of `vcon_get`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct GetArgs {
    /// The vCon's uuid, in RFC 9562 text form.
    uuid: String,
    /// The next_cursor a previous page of this vCon gave.
    cursor: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn get<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: GetArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let at = opened(server, caller, Scope::Read).await?;
        // Store records the read, before any of its text is returned.
        let stored = api::blocking(move || at.store.get(&at.who, uuid))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        let text = super::vcon_text(&stored, server.app.limits.inflated)?;
        let head = json!({
            "uuid": uuid.to_string(),
            "version": stored.version.to_string(),
            "gzip": stored.gzip,
        });
        text_result(head, &text, args.cursor.as_deref(), room)
    })
}

/// The arguments of `vcon_list`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct ListArgs {
    /// How many vCons at most: 1 to 1000, 100 when left out.
    limit: Option<u64>,
    /// The next_cursor a previous page gave.
    cursor: Option<String>,
    /// Only vCons first stored at or after this time (RFC 3339).
    since: Option<String>,
    /// Only vCons first stored before this time (RFC 3339).
    until: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn list<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: ListArgs = arguments(args)?;
        // The values' one rule, as GET /vcons reads them.
        let query = ListQuery::new(
            args.limit,
            args.cursor.as_deref(),
            args.since.as_deref(),
            args.until.as_deref(),
        )
        .map_err(|_| Outcome::Refused(Problem::BadQuery))?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let at = opened(server, caller, Scope::Read).await?;
        let listing = api::blocking(move || at.store.list(&at.who, &query))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        let rows = match listing.to_json()["vcons"].take() {
            Value::Array(rows) => rows,
            _ => Vec::new(),
        };
        let page = |kept: usize| {
            let (next, truncated) = if kept < rows.len() {
                (Some(Cursor::after(&listing.vcons[kept - 1])), true)
            } else {
                (listing.next, false)
            };
            json!({
                "vcons": rows[..kept],
                "truncated": truncated,
                "next_cursor": next.map(|c| c.to_string()),
            })
        };
        let kept = budget::rows_that_fit(rows.len(), |n| budget::size(&page(n)) <= room);
        Ok(page(kept))
    })
}

//! The legal tools, for tenant keys with the legal scope: the same work as
//! the `/legal/` routes.

use rmcp::model::JsonObject;
use serde::Deserialize;
use serde_json::json;

use super::{Hints, Outcome, Running, Server, Spec, arguments, budget, opened, text_result, tool};
use crate::access::{Caller, Scope};
use crate::api::{self, Problem};
use crate::audit::Action;
use crate::ids::VersionId;
use crate::store::StoreError;

use super::vcons::UuidArgs;

/// The legal tools.
pub(super) fn specs() -> Vec<Spec> {
    let read = Hints {
        read_only: true,
        idempotent: true,
        ..Hints::NONE
    };
    vec![
        Spec {
            tool: tool::<GetArgs>(
                "legal_get",
                "Returns a kept version of the vCon with this uuid in the key's tenant, \
                 deleted or not: the version named, else the current one, else the latest \
                 kept. Gives its uuid, version id, state (live or deleted), whether it was \
                 stored gzipped, and its JSON text between untrusted-vcon-data markers, from \
                 cursor up to max_bytes, with next_cursor when more follows.",
                read,
            ),
            action: Action::LegalRead,
            run: get,
        },
        Spec {
            tool: tool::<VersionsArgs>(
                "legal_versions",
                "Returns every kept version of the vCon with this uuid in the key's tenant, \
                 oldest first: each version id, when it was stored and hidden (RFC 3339, \
                 null while current), and whether it is current; from cursor, within \
                 max_bytes, with next_cursor when more follow.",
                read,
            ),
            action: Action::LegalVersions,
            run: versions,
        },
        Spec {
            tool: tool::<UuidArgs>(
                "legal_restore",
                "Makes the deleted vCon with this uuid in the key's tenant live again, with \
                 its latest kept version current, and returns the uuid and that version id.",
                Hints::NONE,
            ),
            action: Action::Restore,
            run: restore,
        },
        Spec {
            tool: tool::<UuidArgs>(
                "legal_hold",
                "Puts the vCon with this uuid in the key's tenant under a legal hold, so no \
                 version of it is purged until the hold is released, and returns the uuid \
                 and held: true.",
                Hints {
                    idempotent: true,
                    ..Hints::NONE
                },
            ),
            action: Action::Hold,
            run: hold,
        },
        Spec {
            tool: tool::<UuidArgs>(
                "legal_release",
                "Releases the legal hold on the vCon with this uuid in the key's tenant, and \
                 returns the uuid and held: false.",
                Hints {
                    idempotent: true,
                    ..Hints::NONE
                },
            ),
            // A refusal is recorded as DELETE /legal/vcons/{uuid}/hold records
            // it; Store records the release itself as release.
            action: Action::Hold,
            run: release,
        },
    ]
}

fn restore<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: UuidArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let at = opened(server, caller, Scope::Legal).await?;
        let version = api::blocking(move || at.store.restore(&at.who, uuid))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        Ok(json!({"uuid": uuid.to_string(), "version": version.to_string()}))
    })
}

/// Puts on or releases a uuid's hold.
async fn set_hold(
    server: &Server,
    caller: Caller,
    args: JsonObject,
    on: bool,
) -> Result<serde_json::Value, Outcome> {
    let args: UuidArgs = arguments(args)?;
    let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
    let at = opened(server, caller, Scope::Legal).await?;
    api::blocking(move || at.store.set_hold(&at.who, uuid, on))
        .await
        .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
    Ok(json!({"uuid": uuid.to_string(), "held": on}))
}

fn hold<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(set_hold(server, caller, args, true))
}

fn release<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(set_hold(server, caller, args, false))
}

/// The arguments of `legal_get`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct GetArgs {
    /// The vCon's uuid, in RFC 9562 text form.
    uuid: String,
    /// A kept version's id, from legal_versions.
    version: Option<String>,
    /// The next_cursor a previous page of this version gave.
    cursor: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn get<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: GetArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let version = args
            .version
            .as_deref()
            .map(|v| VersionId::parse(v).ok_or(Outcome::Arguments))
            .transpose()?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let at = opened(server, caller, Scope::Legal).await?;
        let read = api::blocking(move || at.store.legal_get(&at.who, uuid, version))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        let text = super::vcon_text(&read.stored, server.app.limits.inflated)?;
        let head = json!({
            "uuid": uuid.to_string(),
            "version": read.stored.version.to_string(),
            "state": if read.deleted { "deleted" } else { "live" },
            "gzip": read.stored.gzip,
        });
        text_result(head, &text, args.cursor.as_deref(), room)
    })
}

/// The arguments of `legal_versions`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct VersionsArgs {
    /// The vCon's uuid, in RFC 9562 text form.
    uuid: String,
    /// The next_cursor a previous page gave.
    cursor: Option<String>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn versions<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: VersionsArgs = arguments(args)?;
        let uuid = vcon::parse_uuid(&args.uuid).ok_or(Outcome::Arguments)?;
        let start = args
            .cursor
            .as_deref()
            .map(|c| c.parse::<usize>().map_err(|_| Outcome::Arguments))
            .transpose()?
            .unwrap_or(0);
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let at = opened(server, caller, Scope::Legal).await?;
        let kept = api::blocking(move || at.store.legal_versions(&at.who, uuid))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        // The rendering GET /legal/vcons/{uuid}/versions answers.
        let rows = api::versions_json(&kept)
            .ok_or(Outcome::Answered(Problem::Store(StoreError::Damaged)))?;
        let rows = rows.get(start..).ok_or(Outcome::Arguments)?;
        let page = |n: usize| {
            json!({
                "uuid": uuid.to_string(),
                "versions": rows[..n],
                "truncated": n < rows.len(),
                "next_cursor": (n < rows.len()).then(|| (start + n).to_string()),
            })
        };
        let n = budget::rows_that_fit(rows.len(), |n| budget::size(&page(n)) <= room);
        Ok(page(n))
    })
}

//! The audit tool, for tenant keys with the audit scope: the same work as
//! `GET /audit`.

use rmcp::model::JsonObject;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Hints, Outcome, Running, Server, Spec, arguments, budget, opened, tool};
use crate::access::{Caller, Scope};
use crate::api::{self, Problem};
use crate::audit::{Action, AuditQuery};
use crate::store::StoreError;

/// The audit tool.
pub(super) fn specs() -> Vec<Spec> {
    vec![Spec {
        tool: tool::<ReadArgs>(
            "audit_read",
            "Returns records of the key's tenant's audit log after the sequence number \
             after (0, the default, is the start), oldest first, at most limit of them \
             (1 to 10000, default 1000), within max_bytes: each record as audit export \
             writes it, and next_cursor, the seq of the last record returned.",
            Hints {
                read_only: true,
                ..Hints::NONE
            },
        ),
        action: Action::AuditRead,
        run: read,
    }]
}

/// The arguments of `audit_read`.
#[derive(Debug, Clone, Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
#[serde(deny_unknown_fields)]
pub(super) struct ReadArgs {
    /// Records after this sequence number; 0 when left out.
    after: Option<u64>,
    /// How many records at most: 1 to 10000, 1000 when left out.
    limit: Option<u64>,
    /// The most bytes the result may hold; the server's ceiling applies.
    max_bytes: Option<u64>,
}

fn read<'a>(server: &'a Server, caller: Caller, args: JsonObject) -> Running<'a> {
    Box::pin(async move {
        let args: ReadArgs = arguments(args)?;
        // The values' one rule, as GET /audit reads them.
        let query = AuditQuery::new(args.after, args.limit)
            .map_err(|_| Outcome::Refused(Problem::BadQuery))?;
        let room = budget::resolve(args.max_bytes, server.app.mcp.response_bytes);
        let at = opened(server, caller, Scope::Audit).await?;
        let lines = api::blocking(move || at.store.audit_page(&at.who, &query))
            .await
            .map_err(|e| Outcome::Answered(Problem::Store(e)))?;
        records(lines, room)
    })
}

/// A page of an audit log, the JSON lines Store or Tenants gave: the
/// records, cut from the end to fit `room`, and the seq of the last one.
pub(super) fn records(lines: Vec<u8>, room: usize) -> Result<Value, Outcome> {
    let records: Vec<Value> = String::from_utf8(lines)
        .ok()
        .and_then(|text| {
            text.lines()
                .map(|line| serde_json::from_str(line).ok())
                .collect::<Option<Vec<Value>>>()
        })
        .ok_or(Outcome::Answered(Problem::Store(StoreError::Damaged)))?;
    let page = |n: usize| {
        json!({
            "records": records[..n],
            "truncated": n < records.len(),
            "next_cursor": n.checked_sub(1).map(|last| records[last]["seq"].to_string()),
        })
    };
    let n = budget::rows_that_fit(records.len(), |n| budget::size(&page(n)) <= room);
    Ok(page(n))
}

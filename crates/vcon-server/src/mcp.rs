//! The MCP server: the HTTP API's operations as MCP tools for AI agents.
//!
//! It is served at `/mcp` on the TLS listener, in rmcp's stateless mode with
//! JSON answers, behind the same authentication as every route. The stdio
//! command in [`stdio`] forwards to it, so both transports pass one gate.

mod admin;
mod audit;
pub mod budget;
pub mod fence;
mod gate;
mod legal;
pub mod stdio;
mod vcons;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::handler::server::common::schema_for_type;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, Extensions, Implementation,
    JsonObject, ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    ToolAnnotations,
};
use rmcp::service::{RequestContext, RoleServer};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, ServerHandler};
use serde::de::DeserializeOwned;
use serde_json::Value;

pub use gate::scope;

use crate::access::{self, Caller};
use crate::api::{self, App, Problem};
use crate::audit::{Action, Actor, Entry};
use crate::ids::TenantId;

/// What every client is told the server holds.
const INSTRUCTIONS: &str = "vcon-server stores vCons, encrypted at rest, and \
keeps an audit log of every action. Its tools act only within the tenant of \
the API key that calls them, and only within that key's scopes; each call is \
recorded in the audit log.";

/// The limits from configuration that the MCP server applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Tool calls one key may make in a one-second window.
    pub calls_per_second: u32,
    /// Tool calls in flight on the server.
    pub concurrent: u32,
    /// The largest result, in bytes.
    pub response_bytes: usize,
}

impl Limits {
    /// sipnab's defaults for the same limits.
    pub const DEFAULT: Limits = Limits {
        calls_per_second: 100,
        concurrent: 100,
        response_bytes: 1 << 20,
    };
}

/// The hints a tool states besides `openWorldHint: false`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hints {
    pub(crate) read_only: bool,
    pub(crate) destructive: bool,
    pub(crate) idempotent: bool,
}

impl Hints {
    pub(crate) const NONE: Hints = Hints {
        read_only: false,
        destructive: false,
        idempotent: false,
    };
}

/// A tool named `name` taking `A` as its arguments, with `hints`.
pub(crate) fn tool<A: rmcp::schemars::JsonSchema + 'static>(
    name: &'static str,
    description: &'static str,
    hints: Hints,
) -> Tool {
    Tool::new(name, description, schema_for_type::<A>()).with_annotations(
        ToolAnnotations::new()
            .read_only(hints.read_only)
            .destructive(hints.destructive)
            .idempotent(hints.idempotent)
            .open_world(false),
    )
}

/// Why a tool did not answer with a result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[must_use]
pub(crate) enum Outcome {
    /// Its arguments do not fit its type, or a value in them names nothing
    /// the tool can read; the gate records invalid-arguments.
    Arguments,
    /// The tool refused before the layer it calls saw the call; the gate
    /// records the problem, and the result carries it.
    Refused(Problem),
    /// The layer the tool calls refused or failed, and recorded it; the
    /// result carries the problem.
    Answered(Problem),
}

impl std::fmt::Display for Outcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Outcome::Arguments => f.write_str("the arguments do not fit the tool"),
            Outcome::Refused(problem) => write!(f, "refused: {}", problem.name().1),
            Outcome::Answered(problem) => write!(f, "answered: {}", problem.name().1),
        }
    }
}

impl std::error::Error for Outcome {}

/// A tool's running call.
pub(crate) type Running<'a> = Pin<Box<dyn Future<Output = Result<Value, Outcome>> + Send + 'a>>;

/// What a tool does with its caller and arguments.
pub(crate) type Run = for<'a> fn(&'a Server, Caller, JsonObject) -> Running<'a>;

/// One registered tool: what clients are shown, the action it is recorded
/// under, and what it runs.
#[derive(Debug, Clone)]
pub(crate) struct Spec {
    /// The tool as `tools/list` shows it.
    pub(crate) tool: Tool,
    /// The action its calls are recorded under.
    pub(crate) action: Action,
    /// What a call runs.
    pub(crate) run: Run,
}

/// A tool's arguments as its type, or the refusal when they do not fit.
pub(crate) fn arguments<A: DeserializeOwned>(arguments: JsonObject) -> Result<A, Outcome> {
    serde_json::from_value(Value::Object(arguments)).map_err(|_| Outcome::Arguments)
}

/// The MCP server's routes: `/mcp`, behind authentication and counting,
/// with every tool.
pub fn router(app: Arc<App>) -> Router {
    router_with(app, tools())
}

/// Every tool, by area.
pub(crate) fn tools() -> Vec<Spec> {
    let mut all = vcons::specs();
    all.extend(legal::specs());
    all.extend(audit::specs());
    all.extend(admin::specs());
    all
}

/// The caller's Store and its caller for Store, for a tool needing `scope`;
/// a tenant being removed is unavailable.
pub(crate) struct Opened {
    pub(crate) store: Arc<crate::store::Store>,
    pub(crate) who: crate::store::Caller,
}

impl std::fmt::Debug for Opened {
    // The store holds the tenant's keys; only the caller is shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("who", &self.who)
            .finish_non_exhaustive()
    }
}

pub(crate) async fn opened(
    server: &Server,
    caller: Caller,
    scope: access::Scope,
) -> Result<Opened, Outcome> {
    let tenant =
        access::tenant_for(&caller, scope).map_err(|_| Outcome::Refused(Problem::Forbidden))?;
    let tenants = server.app.tenants.clone();
    let store = api::blocking(move || tenants.store(tenant))
        .await
        .map_err(|_| Outcome::Refused(Problem::Unavailable))?;
    Ok(Opened {
        store,
        who: crate::store::Caller { key: caller.key },
    })
}

/// A stored version's JSON text, inflated within `limit` when it was
/// stored gzipped, with the characters the fence never holds raw escaped.
pub(crate) fn vcon_text(stored: &crate::store::Stored, limit: u64) -> Result<String, Outcome> {
    use std::io::Read;
    let damaged = || Outcome::Answered(Problem::Store(crate::store::StoreError::Damaged));
    let text = if stored.gzip {
        let mut inflated = Vec::new();
        flate2::read::MultiGzDecoder::new(stored.body.as_slice())
            .take(limit.saturating_add(1))
            .read_to_end(&mut inflated)
            .map_err(|_| damaged())?;
        if inflated.len() as u64 > limit {
            return Err(Outcome::Answered(Problem::TooLarge));
        }
        inflated
    } else {
        stored.body.clone()
    };
    let text = String::from_utf8(text).map_err(|_| damaged())?;
    Ok(fence::escape_json(&text))
}

/// A result holding a piece of `text`: `head`'s members, the piece fenced,
/// whether more follows and where, and the provenance sentence; the piece
/// from `cursor`, a byte offset an earlier page gave, as long as `room`
/// allows.
pub(crate) fn text_result(
    head: Value,
    text: &str,
    cursor: Option<&str>,
    room: usize,
) -> Result<Value, Outcome> {
    let start = match cursor {
        None => 0,
        Some(cursor) => cursor
            .parse::<usize>()
            .ok()
            .filter(|&at| at <= text.len() && text.is_char_boundary(at))
            .ok_or(Outcome::Arguments)?,
    };
    let build = |piece: &str, next: Option<usize>| {
        let mut result = head.clone();
        result["vcon"] = fence::fence(piece).into();
        result["truncated"] = next.is_some().into();
        result["next_cursor"] = next.map(|at| at.to_string()).into();
        result["provenance"] = fence::PROVENANCE.into();
        result
    };
    let (piece, next) = budget::piece(text, start, |piece, more| {
        // A following page's cursor is at most the text's length.
        budget::size(&build(piece, more.then_some(text.len()))) <= room
    });
    Ok(build(piece, next))
}

/// The MCP server's routes with the tools in `specs`.
pub(crate) fn router_with(app: Arc<App>, specs: Vec<Spec>) -> Router {
    let server = Server {
        app: app.clone(),
        specs: Arc::new(specs),
        gate: Arc::new(gate::Gate::new(app.mcp)),
    };
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_sse_keep_alive(None)
        // TLS and the bearer key already stop a page that rebinds a name to
        // this server, and a deployment's host name is its own.
        .disable_allowed_hosts()
        // A web page sends an Origin header; no page may call /mcp.
        .enforce_origin_validation()
        .with_max_request_body_bytes(app.limits.body);
    let service: StreamableHttpService<Server, LocalSessionManager> =
        StreamableHttpService::new(move || Ok(server.clone()), Default::default(), config);
    // POST only: GET and DELETE answer 405 before any key is asked for, and
    // the layer wraps this one route, never the router's fallback.
    Router::new().route(
        "/mcp",
        axum::routing::post_service(service)
            .route_layer(axum::middleware::from_fn_with_state(app, admit)),
    )
}

/// The MCP request handler: the tools, and the gate every call passes.
#[derive(Clone)]
pub(crate) struct Server {
    pub(crate) app: Arc<App>,
    specs: Arc<Vec<Spec>>,
    gate: Arc<gate::Gate>,
}

impl std::fmt::Debug for Server {
    // The app holds keys and stores; the tools and the gate are shown.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let tools: Vec<&str> = self.specs.iter().map(|spec| &*spec.tool.name).collect();
        f.debug_struct("Server")
            .field("tools", &tools)
            .field("gate", &self.gate)
            .finish_non_exhaustive()
    }
}

/// The caller a request to `/mcp` was admitted with.
fn caller_of(extensions: &Extensions) -> Option<Caller> {
    extensions
        .get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<Caller>())
        .copied()
}

/// Whether `caller` may call the tool, by the scope rule.
fn allows(caller: &Caller, tool: &Tool) -> bool {
    scope(&tool.name, tool.annotations.as_ref())
        .is_some_and(|scope| access::require(caller, scope).is_ok())
}

impl Server {
    /// Records a refusal in the caller's log and returns it, or audit-failed
    /// when the record cannot be written.
    async fn recorded(&self, caller: Caller, action: Action, problem: Problem) -> ErrorData {
        let entry = refusal(caller, action, problem);
        gate::refused(self.app.refuse(caller.tenant, entry, problem).await, None)
    }

    /// Counts a refusal in the caller's log and returns it, or audit-failed
    /// when the count cannot be written.
    async fn counted(
        &self,
        caller: Caller,
        action: Action,
        problem: Problem,
        retry: Option<std::time::Duration>,
    ) -> ErrorData {
        if count(&self.app, caller.tenant, refusal(caller, action, problem)).await {
            gate::refused(problem, retry)
        } else {
            gate::refused(Problem::AuditFailed, None)
        }
    }
}

/// The record of a call the gate refused.
fn refusal(caller: Caller, action: Action, problem: Problem) -> Entry {
    Entry {
        actor: Actor::Key(caller.key),
        action,
        target: None,
        version: None,
        status: problem.status().as_u16(),
    }
}

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        // rmcp's default names rmcp itself; the server names itself.
        info.server_info = Implementation::new("vcon-server", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(INSTRUCTIONS.to_owned());
        info
    }

    /// The tools the caller's key may call, by the scope rule.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let caller = caller_of(&context.extensions);
        let mut tools: Vec<Tool> = self
            .specs
            .iter()
            .filter(|spec| caller.is_some_and(|caller| allows(&caller, &spec.tool)))
            .map(|spec| spec.tool.clone())
            .collect();
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ListToolsResult::with_all_items(tools))
    }

    /// The gate every tool call passes, in order: the key's rate, a free
    /// slot, a known tool, the tool's scope, then the tool with arguments
    /// that fit it. Each refusal is recorded, or counted for rate and
    /// capacity, before it is sent.
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let Some(caller) = caller_of(&context.extensions) else {
            // Every request reaches here through admit, which sets it.
            return Err(gate::refused(Problem::Unauthorized, None));
        };
        let spec = self
            .specs
            .iter()
            .find(|spec| spec.tool.name == request.name);
        let action = spec.map_or(Action::Mcp, |spec| spec.action);
        let limit = self.gate.limits.calls_per_second;
        if let Err(retry) = self.gate.windows.take(caller.key, limit, Instant::now()) {
            return Err(self
                .counted(caller, action, Problem::RateLimited, Some(retry))
                .await);
        }
        let Some(_slot) = self.gate.slot() else {
            return Err(self
                .counted(caller, action, Problem::AtCapacity, None)
                .await);
        };
        let Some(spec) = spec else {
            return Err(self.recorded(caller, action, Problem::UnknownTool).await);
        };
        if !allows(&caller, &spec.tool) {
            return Err(self.recorded(caller, action, Problem::Forbidden).await);
        }
        // The tool runs in its own task, so a panic in it is answered and
        // recorded as internal instead of leaving the request unanswered.
        let (run, server, arguments) = (
            spec.run,
            self.clone(),
            request.arguments.unwrap_or_default(),
        );
        let ran = tokio::spawn(async move { (run)(&server, caller, arguments).await }).await;
        let Ok(ran) = ran else {
            let failed = Problem::Store(crate::store::StoreError::Io);
            let entry = refusal(caller, action, failed);
            let problem = self.app.refuse(caller.tenant, entry, failed).await;
            return Ok(CallToolResult::structured_error(problem.document()).into());
        };
        match ran {
            Ok(value) => Ok(CallToolResult::structured(value).into()),
            Err(Outcome::Arguments) => Err(self
                .recorded(caller, action, Problem::InvalidArguments)
                .await),
            Err(Outcome::Refused(problem)) => {
                let entry = refusal(caller, action, problem);
                let problem = self.app.refuse(caller.tenant, entry, problem).await;
                Ok(CallToolResult::structured_error(problem.document()).into())
            }
            Err(Outcome::Answered(problem)) => {
                Ok(CallToolResult::structured_error(problem.document()).into())
            }
        }
    }
}

/// Counts `entry` in `tenant`'s log, or in the admin log for an operator key
/// or a tenant whose log cannot be opened; false when it cannot be.
async fn count(app: &Arc<App>, tenant: Option<TenantId>, entry: Entry) -> bool {
    let tenants = app.tenants.clone();
    api::blocking(move || match tenant.and_then(|t| tenants.store(t).ok()) {
        Some(store) => store.count(&entry).is_ok(),
        None => tenants.count_admin(&entry).is_ok(),
    })
    .await
}

/// Every request to `/mcp`, before rmcp reads it: the bearer key is
/// authenticated as on every route, the caller's tenant opened, the body
/// read within the limits, and the caller passed on inside the request.
/// Each answered request is then counted in the key's log; one that cannot
/// be counted answers audit-failed.
async fn admit(State(app): State<Arc<App>>, request: Request, next: Next) -> Response {
    let (mut parts, body) = request.into_parts();
    let caller = match api::authenticated(&app, &parts.headers).await {
        Ok(caller) => caller,
        Err(refused) => return refused.into_response(),
    };
    // A tenant being removed answers 503, as on every route, and is counted
    // in the admin log, since its own may already be gone.
    let tenants = app.tenants.clone();
    let closing = caller
        .tenant
        .is_some_and(|tenant| tenants.store(tenant).is_err());
    let answer = if closing {
        Problem::Unavailable.into_response()
    } else {
        match api::read_body(body, &parts.headers, &app.limits).await {
            Ok(body) => {
                parts.extensions.insert(caller);
                next.run(Request::from_parts(parts, Body::from(body))).await
            }
            Err(refused) => refused.into_response(),
        }
    };
    let entry = Entry {
        actor: Actor::Key(caller.key),
        action: Action::Mcp,
        target: None,
        version: None,
        status: answer.status().as_u16(),
    };
    if count(&app, caller.tenant, entry).await {
        answer
    } else {
        Problem::AuditFailed.into_response()
    }
}

#[cfg(test)]
mod tests;

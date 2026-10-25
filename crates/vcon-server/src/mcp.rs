//! The MCP server: the HTTP API's operations as MCP tools for AI agents.
//!
//! It is served at `/mcp` on the TLS listener, in rmcp's stateless mode with
//! JSON answers, behind the same authentication as every route. The stdio
//! command in [`stdio`] forwards to it, so both transports pass one gate.

pub mod stdio;

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use rmcp::ServerHandler;
use rmcp::model::{Implementation, ServerCapabilities, ServerConfig};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};

use crate::api::{self, App, Problem};
use crate::audit::{Action, Actor, Entry};

/// What every client is told the server holds.
const INSTRUCTIONS: &str = "vcon-server stores vCons, encrypted at rest, and \
keeps an audit log of every action. Its tools act only within the tenant of \
the API key that calls them, and only within that key's scopes; each call is \
recorded in the audit log.";

/// The MCP server's routes: `/mcp`, behind authentication and counting.
pub fn router(app: Arc<App>) -> Router {
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
        StreamableHttpService::new(|| Ok(Server), Default::default(), config);
    // POST only: GET and DELETE answer 405 before any key is asked for, and
    // the layer wraps this one route, never the router's fallback.
    Router::new().route(
        "/mcp",
        axum::routing::post_service(service)
            .route_layer(axum::middleware::from_fn_with_state(app, admit)),
    )
}

/// The MCP request handler.
#[derive(Debug, Clone, Copy)]
struct Server;

impl ServerHandler for Server {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::new(ServerCapabilities::builder().enable_tools().build());
        // rmcp's default names rmcp itself; the server names itself.
        info.server_info = Implementation::new("vcon-server", env!("CARGO_PKG_VERSION"));
        info.instructions = Some(INSTRUCTIONS.to_owned());
        info
    }
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
    let open = caller.tenant.map(|tenant| tenants.store(tenant));
    let (answer, store) = match open {
        Some(Err(_)) => (Problem::Unavailable.into_response(), None),
        opened => {
            let store = opened.and_then(Result::ok);
            let answer = match api::read_body(body, &parts.headers, &app.limits).await {
                Ok(body) => {
                    parts.extensions.insert(caller);
                    next.run(Request::from_parts(parts, Body::from(body))).await
                }
                Err(refused) => refused.into_response(),
            };
            (answer, store)
        }
    };
    let entry = Entry {
        actor: Actor::Key(caller.key),
        action: Action::Mcp,
        target: None,
        version: None,
        status: answer.status().as_u16(),
    };
    let tenants = app.tenants.clone();
    let counted = api::blocking(move || match store {
        Some(store) => store.count(&entry).is_ok(),
        None => tenants.count_admin(&entry).is_ok(),
    })
    .await;
    if counted {
        answer
    } else {
        Problem::AuditFailed.into_response()
    }
}

#[cfg(test)]
mod tests;

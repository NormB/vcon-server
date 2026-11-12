//! The routes of a session: `GET /session` and `POST /session/logout`.
//!
//! A request by session carries the cookie `__Host-vcs_session` and no
//! key. It is checked in this order, and the first check that fails
//! answers: a key beside the cookie (400 `key-and-session`); the cookie
//! (401 `no-session`); where the request comes from (403 `bad-origin`); the
//! session in the registry (401 `no-session`); its CSRF token, on every
//! request but the hand-out (403 `bad-csrf`); and the listener's plane
//! (403 `wrong-listener`). The origin is checked before the registry is
//! read, so a request from elsewhere learns nothing about a cookie. Each
//! refusal is counted in the audit as `auth_failed`, and a session at the
//! other plane's listener as `wrong_listener`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CACHE_CONTROL, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{App, Problem, blocking, json_answer};
use crate::audit::Action;
use crate::session::{self, CSRF_HEADER, Cookie};
use crate::tenants::{Proof, SessionError, SessionInfo, TenantError};

/// A request admitted by its session: the id its cookie carries and the
/// session as it is after this request was seen.
struct BySession {
    id: String,
    session: SessionInfo,
}

/// A refusal, with the header that removes the cookie from the browser
/// when the cookie it sent names no live session.
fn refusal(refused: Problem, clear: bool) -> Response {
    let mut answer = refused.into_response();
    if clear {
        answer
            .headers_mut()
            .insert(SET_COOKIE, session::clear_cookie());
    }
    answer
}

/// A refusal before a route's work: the problem, and whether its answer
/// removes the cookie from the browser.
type Refused = (Problem, bool);

/// Counts a refusal that knows no session.
async fn refuse_unknown(app: &Arc<App>, refused: Problem, clear: bool) -> Refused {
    let tenants = app.tenants.clone();
    let status = refused.status().as_u16();
    blocking(move || tenants.count_session(Action::AuthFailed, status, None)).await;
    (refused, clear)
}

/// Admits a request by its session, as the module's text orders the
/// checks. `hand_out` is true for `GET /session` alone.
async fn by_session(
    app: &Arc<App>,
    headers: &HeaderMap,
    hand_out: bool,
) -> Result<BySession, Refused> {
    let found = session::cookie(headers);
    if headers.contains_key(AUTHORIZATION) {
        // A key is not a session, and a key beside a cookie is refused on
        // every route.
        let refused = match found {
            Cookie::Absent => Problem::NoSession,
            Cookie::Bad | Cookie::Id(_) => Problem::KeyAndSession,
        };
        return Err(refuse_unknown(app, refused, false).await);
    }
    let id = match found {
        Cookie::Absent => return Err(refuse_unknown(app, Problem::NoSession, false).await),
        Cookie::Bad => return Err(refuse_unknown(app, Problem::NoSession, true).await),
        Cookie::Id(id) => id.to_owned(),
    };
    if !session::from_origin(headers, app.origin.as_ref(), hand_out) {
        return Err(refuse_unknown(app, Problem::BadOrigin, false).await);
    }
    let token = headers
        .get(CSRF_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let (tenants, plane, asked) = (app.tenants.clone(), app.plane, id.clone());
    // Tenants counts its own refusals.
    let checked = blocking(move || {
        let proof = if hand_out {
            Proof::Handout
        } else {
            Proof::Csrf(token.as_deref())
        };
        let session = tenants.session(&asked, proof)?;
        if plane.serves(session.tenant) {
            Ok(Ok(session))
        } else {
            let known = Some((session.user, session.tenant));
            tenants.count_session(Action::WrongListener, 403, known);
            Ok(Err(Problem::WrongListener))
        }
    })
    .await;
    match checked {
        Ok(Ok(session)) => Ok(BySession { id, session }),
        Ok(Err(refused)) => Err((refused, false)),
        Err(SessionError::NoSession) => Err((Problem::NoSession, true)),
        Err(SessionError::BadCsrf) => Err((Problem::BadCsrf, false)),
        Err(SessionError::Registry(TenantError::Io)) => Err((Problem::Unavailable, false)),
        Err(SessionError::Registry(error)) => Err((Problem::Tenant(error), false)),
        // `Tenants::session` refuses in no other way.
        Err(_) => Err((Problem::Tenant(TenantError::Io), false)),
    }
}

/// `GET /session`: the hand-out. It answers the session of the request's
/// cookie: its user, tenant, scopes, authentication methods and limits,
/// and the CSRF token every other request by this session must carry. It
/// needs `Sec-Fetch-Site: same-origin`, which a browser sets and page
/// script cannot, and takes no CSRF token, since the page asks for it
/// here. The answer is never to be stored. Reads are counted in the audit
/// as `session_read`.
pub(super) async fn session_read(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let admitted = match by_session(&app, &headers, true).await {
        Ok(admitted) => admitted,
        Err((refused, clear)) => return refusal(refused, clear),
    };
    let tenants = app.tenants.clone();
    let known = Some((admitted.session.user, admitted.session.tenant));
    blocking(move || tenants.count_session(Action::SessionRead, 200, known)).await;
    let token = session::csrf_token(&admitted.id);
    let Some(shown) = admitted.session.to_json(&token) else {
        return Problem::Tenant(TenantError::Io).into_response();
    };
    let mut answer = json_answer(StatusCode::OK, &shown);
    answer
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    answer
}

/// `POST /session/logout`: ends the session of the request's cookie and
/// answers 204 with the header that removes the cookie from the browser.
/// It needs the `Origin` of the listener's own pages and the session's
/// CSRF token. The body is not read. The end is recorded in the audit as
/// `session_end`.
pub(super) async fn session_end(State(app): State<Arc<App>>, headers: HeaderMap) -> Response {
    let admitted = match by_session(&app, &headers, false).await {
        Ok(admitted) => admitted,
        Err((refused, clear)) => return refusal(refused, clear),
    };
    let tenants = app.tenants.clone();
    // Tenants records its own outcome.
    match blocking(move || tenants.end_session(&admitted.id)).await {
        Ok(_) => {
            let mut answer = StatusCode::NO_CONTENT.into_response();
            answer
                .headers_mut()
                .insert(SET_COOKIE, session::clear_cookie());
            answer
        }
        // Ended between the check and here: by its limit or by another
        // request.
        Err(SessionError::NoSession) => refusal(Problem::NoSession, true),
        Err(SessionError::Registry(error)) => Problem::Tenant(error).into_response(),
        // `Tenants::end_session` refuses in no other way.
        Err(_) => Problem::Tenant(TenantError::Io).into_response(),
    }
}

//! The sessions of users who signed in.
//!
//! A session is one row of the registry's `session` table, found by the
//! SHA-256 of its id. It is for one membership of its user, a tenant's or
//! the operator's, and holds the scopes that membership gave when the
//! session started. It ends at its idle limit unless a request is seen, at
//! its absolute limit whatever is seen, when its user ends it, and when the
//! membership could have changed: `users` and `role_cap` remove sessions in
//! the transaction that changes a membership, a user or a role cap.
//!
//! No sign-in method is built yet. [`Tenants::start_session`] is the seam
//! one will call once it has checked who the user is.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::role_cap::stored_cap;
use super::{RTx, TenantError, Tenants, commit};
use crate::access::Scopes;
use crate::audit::{Action, Actor, Entry};
use crate::backend::SessionRow;
use crate::clock::{from_nanos, nanos};
use crate::ids::{TenantId, UserId};
use crate::roles::Roles;
use crate::session::{amr_text, csrf_token, hash, new_secret, same, well_formed};
use crate::settings::{Name, Settings, Value};

const NANOS_PER_SECOND: i64 = 1_000_000_000;

/// A session as its user's browser may see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub user: UserId,
    /// The tenant the session works in; None on the operator plane.
    pub tenant: Option<TenantId>,
    /// The scopes the membership gave when the session started.
    pub scopes: Scopes,
    /// The authentication methods the sign-in showed, joined with spaces.
    pub amr: String,
    pub created_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    /// When the session ends unless a request is seen before; never after
    /// `expires_at`.
    pub idle_expires_at: OffsetDateTime,
    /// When the session ends whatever is seen.
    pub expires_at: OffsetDateTime,
}

impl SessionInfo {
    /// The session as `GET /session` answers it, with the CSRF token
    /// given: user, tenant (null on the operator plane), scopes,
    /// authentication methods and the times in RFC 3339. None when a time
    /// cannot be written so.
    #[must_use]
    pub fn to_json(&self, csrf: &str) -> Option<serde_json::Value> {
        let time = |at: OffsetDateTime| at.format(&Rfc3339).ok();
        Some(serde_json::json!({
            "user": self.user.to_string(),
            "tenant": self.tenant.map(|t| t.to_string()),
            "scopes": self.scopes.names(),
            "amr": self.amr.split(' ').collect::<Vec<_>>(),
            "created_at": time(self.created_at)?,
            "last_seen_at": time(self.last_seen_at)?,
            "idle_expires_at": time(self.idle_expires_at)?,
            "expires_at": time(self.expires_at)?,
            "csrf_token": csrf,
        }))
    }
}

/// A session just started or just given a new id: the id for the cookie,
/// the CSRF token for the page, and the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Started {
    /// The session's id. It is shown this once and kept nowhere.
    pub id: String,
    /// The session's CSRF token, `crate::session::csrf_token` of the id.
    pub csrf: String,
    pub session: SessionInfo,
}

/// What a request by session shows besides its cookie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Proof<'a> {
    /// The hand-out, `GET /session`: no token, since the page asks for it.
    Handout,
    /// Any other request: the `X-CSRF-Token` header's value, if it has one.
    Csrf(Option<&'a str>),
}

/// Why a call about a session was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionError {
    /// No user has the id.
    NoUser,
    /// The user is deactivated.
    Deactivated,
    /// The user has no membership there, or one whose roles give no scope
    /// under the tenant's role cap.
    NoMembership,
    /// The authentication methods are not ones a session keeps.
    BadAmr,
    /// No live session has the id: it never was, or it has ended.
    NoSession,
    /// The CSRF token is missing or is not the session's.
    BadCsrf,
    /// The registry failed.
    Registry(TenantError),
}

impl From<TenantError> for SessionError {
    fn from(e: TenantError) -> SessionError {
        SessionError::Registry(e)
    }
}

impl SessionError {
    /// The status a refused start is recorded with.
    fn status(self) -> u16 {
        match self {
            SessionError::BadCsrf | SessionError::NoMembership => 403,
            SessionError::NoSession => 401,
            SessionError::NoUser => 404,
            SessionError::Deactivated => 409,
            SessionError::BadAmr => 422,
            SessionError::Registry(_) => 500,
        }
    }
}

/// `seconds` after `at`, in the registry's nanoseconds.
fn after(at: i64, seconds: i64) -> i64 {
    at.saturating_add(seconds.saturating_mul(NANOS_PER_SECOND))
}

/// A row as its user's browser may see it. Scopes or a time that do not
/// read back are damage, not a session with none.
fn info(row: &SessionRow) -> Result<SessionInfo, TenantError> {
    let time = |at| from_nanos(at).ok_or(TenantError::Io);
    Ok(SessionInfo {
        user: row.user,
        tenant: row.tenant,
        scopes: Scopes::parse(&row.scopes).ok_or(TenantError::Io)?,
        amr: row.amr.clone(),
        created_at: time(row.created_at)?,
        last_seen_at: time(row.last_seen_at)?,
        idle_expires_at: time(row.idle_expires_at.min(row.expires_at))?,
        expires_at: time(row.expires_at)?,
    })
}

/// What a record of a session names: its user and where it works.
fn target(user: UserId, tenant: Option<TenantId>) -> String {
    let place = tenant.map_or_else(|| "operator".to_owned(), |t| t.to_string());
    format!("{user}/{place}")
}

/// A limit of a tenant's settings, in seconds.
fn limit(settings: &Settings, name: Name) -> i64 {
    match settings.get(name) {
        Value::Seconds(seconds) => i64::from(seconds),
        // `Settings::get` answers a limit's name with seconds.
        Value::Flag(_) => 0,
    }
}

impl Tenants {
    /// Starts a session for a user's membership in `tenant`, or as an
    /// operator for None, after a sign-in that showed the authentication
    /// methods `amr`. The caller has checked who the user is.
    ///
    /// The session always has a new id, so an id a browser held before
    /// sign-in is never the id after it. Its scopes are those of the
    /// membership's roles under the tenant's role cap, fixed here. Its
    /// limits are the tenant's settings `session.idle_seconds` and
    /// `session.absolute_seconds` as they are now, or the defaults on the
    /// operator plane. Sessions that have ended are removed from the table
    /// here. The start, and a refusal, are recorded under the user in the
    /// admin log, and a tenant's start in the tenant's log too.
    pub fn start_session(
        &self,
        user: UserId,
        tenant: Option<TenantId>,
        amr: &[&str],
    ) -> Result<Started, SessionError> {
        let entry = |status| Entry {
            actor: Actor::User(user),
            action: Action::SessionStart,
            target: Some(target(user, tenant)),
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        let found = match amr_text(amr) {
            None => Err(SessionError::BadAmr),
            Some(amr) => membership_scopes(&mut tx, user, tenant).map(|found| (amr, found)),
        };
        let (amr, (scopes, settings)) = match found {
            Ok(found) => found,
            Err(SessionError::Registry(error)) => return Err(error.into()),
            Err(refused) => {
                self.record(&mut tx, &entry(refused.status()))?;
                commit(tx)?;
                return Err(refused);
            }
        };
        let now = nanos(self.clock.now());
        let id = new_secret();
        let csrf = csrf_token(&id);
        let idle_seconds = limit(&settings, Name::SessionIdle);
        let row = SessionRow {
            id_hash: hash(&id).to_vec(),
            user,
            tenant,
            scopes: scopes.to_string(),
            created_at: now,
            last_seen_at: now,
            idle_expires_at: after(now, idle_seconds),
            expires_at: after(now, limit(&settings, Name::SessionAbsolute)),
            idle_seconds,
            amr,
            csrf_hash: hash(&csrf).to_vec(),
        };
        tx.delete_sessions_ended(now)?;
        tx.insert_session(&row)?;
        let session = info(&row)?;
        let done = entry(201);
        self.record(&mut tx, &done)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        if let Some(t) = tenant {
            self.copy_to_tenant(t, &done)?;
        }
        Ok(Started { id, csrf, session })
    }

    /// The live session a request's cookie names, seen now: its idle limit
    /// moves to its idle time from now. A session at or past either limit
    /// has ended and its row is removed. A request that is not the
    /// hand-out needs the session's CSRF token, compared by hash in
    /// constant time; one refused for its token does not count as seen.
    /// A refusal is counted in the audit as `auth_failed`: with no actor
    /// when no live session has the id, under the user for a token.
    pub fn session(&self, id: &str, proof: Proof<'_>) -> Result<SessionInfo, SessionError> {
        let mut tx = self.registry.immediate()?;
        let now = nanos(self.clock.now());
        let Some(row) = live(&mut tx, id, now)? else {
            commit(tx)?;
            self.count_session(Action::AuthFailed, 401, None);
            return Err(SessionError::NoSession);
        };
        if let Proof::Csrf(token) = proof
            && !token.is_some_and(|token| same(&hash(token), &row.csrf_hash))
        {
            drop(tx);
            self.count_session(Action::AuthFailed, 403, Some((row.user, row.tenant)));
            return Err(SessionError::BadCsrf);
        }
        let seen = SessionRow {
            last_seen_at: now,
            idle_expires_at: after(now, row.idle_seconds),
            ..row
        };
        let session = info(&seen)?;
        tx.touch_session(&seen.id_hash, seen.last_seen_at, seen.idle_expires_at)?;
        commit(tx)?;
        Ok(session)
    }

    /// Ends the live session with this id and answers what it was. The
    /// caller has checked the request as [`Tenants::session`] does. The
    /// end is recorded under the user in the admin log, and a tenant's in
    /// the tenant's log too.
    pub fn end_session(&self, id: &str) -> Result<SessionInfo, SessionError> {
        let mut tx = self.registry.immediate()?;
        let Some(row) = live(&mut tx, id, nanos(self.clock.now()))? else {
            commit(tx)?;
            return Err(SessionError::NoSession);
        };
        let session = info(&row)?;
        tx.delete_session(&row.id_hash)?;
        let done = Entry {
            actor: Actor::User(row.user),
            action: Action::SessionEnd,
            target: Some(target(row.user, row.tenant)),
            version: None,
            status: 204,
        };
        self.record(&mut tx, &done)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        if let Some(t) = row.tenant {
            self.copy_to_tenant(t, &done)?;
        }
        Ok(session)
    }

    /// Gives the live session with this id a new id and CSRF token after a
    /// step-up that showed the authentication methods `amr`, which replace
    /// those it had. The session keeps its user, scopes, start and
    /// absolute limit, and is seen now. The old id reaches nothing from
    /// here on. Nothing is recorded here: the step-up records itself.
    pub fn step_up_session(&self, id: &str, amr: &[&str]) -> Result<Started, SessionError> {
        let amr = amr_text(amr).ok_or(SessionError::BadAmr)?;
        let mut tx = self.registry.immediate()?;
        let now = nanos(self.clock.now());
        let Some(row) = live(&mut tx, id, now)? else {
            commit(tx)?;
            return Err(SessionError::NoSession);
        };
        let id = new_secret();
        let csrf = csrf_token(&id);
        tx.delete_session(&row.id_hash)?;
        let row = SessionRow {
            id_hash: hash(&id).to_vec(),
            last_seen_at: now,
            idle_expires_at: after(now, row.idle_seconds),
            amr,
            csrf_hash: hash(&csrf).to_vec(),
            ..row
        };
        tx.insert_session(&row)?;
        let session = info(&row)?;
        commit(tx)?;
        Ok(Started { id, csrf, session })
    }

    /// Counts one request by session in the minute it came: under the
    /// user, in the tenant's log for a session in a tenant, or the admin
    /// log for an operator's or when the tenant's log cannot be opened;
    /// with no actor, in the admin log, when no session is known. A
    /// failure to count changes nothing for the request.
    pub(crate) fn count_session(
        &self,
        action: Action,
        status: u16,
        known: Option<(UserId, Option<TenantId>)>,
    ) {
        let entry = |actor| Entry {
            actor,
            action,
            target: None,
            version: None,
            status,
        };
        if let Some((user, Some(tenant))) = known
            && self
                .store(tenant)
                .is_ok_and(|store| store.count(&entry(Actor::User(user))).is_ok())
        {
            return;
        }
        let actor = known.map_or(Actor::Unparsed, |(user, _)| Actor::User(user));
        let _ = self.count_admin(&entry(actor));
    }
}

/// The session with this id unless it has ended; one found ended is
/// removed, for the caller to commit.
fn live(tx: &mut RTx<'_>, id: &str, now: i64) -> Result<Option<SessionRow>, TenantError> {
    if !well_formed(id) {
        return Ok(None);
    }
    let Some(row) = tx.session(&hash(id))? else {
        return Ok(None);
    };
    if row.idle_expires_at <= now || row.expires_at <= now {
        tx.delete_session(&row.id_hash)?;
        return Ok(None);
    }
    Ok(Some(row))
}

/// The scopes a user's membership gives now, and the settings a session
/// for it takes its limits from.
fn membership_scopes(
    tx: &mut RTx<'_>,
    user: UserId,
    tenant: Option<TenantId>,
) -> Result<(Scopes, Settings), SessionError> {
    match tx.user(user)? {
        None => return Err(SessionError::NoUser),
        Some(row) if row.deactivated_at.is_some() => return Err(SessionError::Deactivated),
        Some(_) => {}
    }
    let Some(held) = tx.membership(user, tenant)? else {
        return Err(SessionError::NoMembership);
    };
    // Roles, a cap or settings that do not read back are damage.
    let roles = Roles::parse(&held.roles).ok_or(TenantError::Io)?;
    let (cap, settings) = match tenant {
        None => (None, Settings::default()),
        Some(t) => {
            let Some(row) = tx.tenant(t)? else {
                return Err(SessionError::NoMembership);
            };
            let settings = Settings::stored(&tx.settings(t)?).ok_or(TenantError::Io)?;
            (stored_cap(row.role_cap.as_deref())?, settings)
        }
    };
    let scopes = roles.within(cap).scopes();
    if scopes.is_empty() {
        return Err(SessionError::NoMembership);
    }
    Ok((scopes, settings))
}

#[cfg(test)]
mod tests;

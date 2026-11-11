//! The users in server.sqlite and their memberships: made, listed,
//! deactivated, granted and revoked here. A user is a person who signs in;
//! a membership gives the user roles in one tenant, or as an operator.
//!
//! Only the command line reaches these calls: no request does. Every call
//! is recorded in the admin log, and a change to a tenant membership is
//! copied to the tenant's log. A record names ids and role names, never a
//! label, an issuer or a subject.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::role_cap::stored_cap;
use super::{Slot, TenantError, Tenants, commit, good_name};
use crate::access::Scopes;
use crate::audit::{Action, Actor, Entry};
use crate::backend::{MembershipRow, UserRow};
use crate::clock::{from_nanos, nanos};
use crate::ids::{TenantId, UserId};
use crate::roles::Roles;

/// Who an identity provider says a user is: the provider's issuer and the
/// subject it gives the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub issuer: String,
    pub subject: String,
}

impl Identity {
    /// Whether the identity is one an OpenID Connect provider can give:
    /// an `https://` issuer of at most 255 bytes with no query, fragment,
    /// white space or control character, and a subject of 1 to 255 ASCII
    /// characters, none a control, with no space at either end.
    #[must_use]
    pub fn good(&self) -> bool {
        let issuer = self.issuer.strip_prefix("https://").is_some_and(|rest| {
            !rest.is_empty()
                && !rest.starts_with('/')
                && self.issuer.len() <= 255
                && !self
                    .issuer
                    .chars()
                    .any(|c| c.is_control() || c.is_whitespace() || c == '?' || c == '#')
        });
        let subject = (1..=255).contains(&self.subject.len())
            && self
                .subject
                .chars()
                .all(|c| c.is_ascii() && !c.is_ascii_control())
            && self.subject.trim() == self.subject;
        issuer && subject
    }
}

/// One membership as `list_users` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipInfo {
    /// None for an operator membership.
    pub tenant: Option<TenantId>,
    /// The roles granted, as the membership's row keeps them.
    pub roles: Roles,
    /// What the membership lets its user do: the scopes of those of its
    /// roles that the tenant's role cap allows.
    pub scopes: Scopes,
    pub granted_at: OffsetDateTime,
}

/// A user as `list_users` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInfo {
    pub id: UserId,
    pub label: String,
    /// None for a local account.
    pub identity: Option<Identity>,
    pub created_at: OffsetDateTime,
    pub deactivated_at: Option<OffsetDateTime>,
    /// The user's memberships, oldest first; none once deactivated.
    pub memberships: Vec<MembershipInfo>,
}

impl UserInfo {
    /// The user as the command line shows it: id, label, issuer and
    /// subject (both null for a local account), times in RFC 3339, and
    /// each membership's tenant (null for an operator membership), roles,
    /// scopes and time. None when a time cannot be written so.
    #[must_use]
    pub fn to_json(&self) -> Option<serde_json::Value> {
        let time = |at: OffsetDateTime| at.format(&Rfc3339).ok();
        let deactivated_at = match self.deactivated_at {
            Some(at) => Some(time(at)?),
            None => None,
        };
        let memberships = self
            .memberships
            .iter()
            .map(|m| {
                Some(serde_json::json!({
                    "tenant": m.tenant.map(|t| t.to_string()),
                    "roles": m.roles.names(),
                    "scopes": m.scopes.names(),
                    "granted_at": time(m.granted_at)?,
                }))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(serde_json::json!({
            "id": self.id.to_string(),
            "label": self.label,
            "issuer": self.identity.as_ref().map(|i| &i.issuer),
            "subject": self.identity.as_ref().map(|i| &i.subject),
            "created_at": time(self.created_at)?,
            "deactivated_at": deactivated_at,
            "memberships": memberships,
        }))
    }
}

/// Why a call about users or memberships was refused or failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserError {
    /// Only the command line may do this.
    CliOnly,
    /// The label breaks the naming rule a tenant's name follows.
    BadLabel,
    /// The issuer or subject is not one a provider can give
    /// ([`Identity::good`]).
    BadIdentity,
    /// An active user has the identity.
    IdentityTaken,
    /// No user has the id.
    NoUser,
    /// The user is deactivated.
    Deactivated,
    /// No tenant has the id.
    NoTenant,
    /// The tenant is being removed.
    Closing,
    /// Roles a membership of its kind cannot hold: an operator membership
    /// holds `operator` alone, a tenant membership one or more of the
    /// others.
    BadRoles,
    /// A role named is outside the tenant's role cap.
    OverCap,
    /// Roles that cannot be a tenant's role cap: a cap is one or more of
    /// the roles a tenant membership may hold.
    BadCap,
    /// The user has no such membership.
    NoMembership,
    /// The registry failed, or the audit record could not be written.
    Registry(TenantError),
}

impl From<TenantError> for UserError {
    fn from(e: TenantError) -> UserError {
        UserError::Registry(e)
    }
}

impl UserError {
    /// The status a refusal is recorded under.
    pub(super) fn status(self) -> u16 {
        match self {
            UserError::CliOnly => 403,
            UserError::NoUser | UserError::NoTenant | UserError::NoMembership => 404,
            UserError::IdentityTaken | UserError::Deactivated => 409,
            UserError::BadCap
            | UserError::BadLabel
            | UserError::BadIdentity
            | UserError::BadRoles
            | UserError::OverCap => 422,
            UserError::Closing => 503,
            UserError::Registry(_) => 500,
        }
    }
}

/// The target of a membership record: the user, then the tenant or
/// `operator`, then the roles the call named, if it named any.
fn membership_target(user: UserId, tenant: Option<TenantId>, roles: Option<Roles>) -> String {
    let place = tenant.map_or_else(|| "operator".to_owned(), |t| t.to_string());
    match roles {
        Some(roles) => format!("{user}/{place}/{roles}"),
        None => format!("{user}/{place}"),
    }
}

impl Tenants {
    /// Makes a user, active and with no membership. `identity` is who an
    /// identity provider says the user is; None makes a local account.
    pub fn add_user(
        &self,
        who: Actor,
        label: &str,
        identity: Option<&Identity>,
    ) -> Result<UserId, UserError> {
        let entry = |status, target: Option<String>| Entry {
            actor: who,
            action: Action::UserAdd,
            target,
            version: None,
            status,
        };
        let shape = if who != Actor::Cli {
            Some(UserError::CliOnly)
        } else if !good_name(label) {
            Some(UserError::BadLabel)
        } else if identity.is_some_and(|identity| !identity.good()) {
            Some(UserError::BadIdentity)
        } else {
            None
        };
        let mut tx = self.registry.immediate()?;
        let refused = match (shape, identity) {
            (Some(refused), _) => Some(refused),
            (None, Some(identity)) if tx.identity_taken(&identity.issuer, &identity.subject)? => {
                Some(UserError::IdentityTaken)
            }
            _ => None,
        };
        if let Some(error) = refused {
            self.record(&mut tx, &entry(error.status(), None))?;
            commit(tx)?;
            return Err(error);
        }
        let id = UserId::generate();
        tx.insert_user(&UserRow {
            id,
            label: label.to_owned(),
            identity: identity.map(|i| (i.issuer.clone(), i.subject.clone())),
            created_at: nanos(self.clock.now()),
            deactivated_at: None,
        })?;
        self.record(&mut tx, &entry(201, Some(id.to_string())))?;
        commit(tx)?;
        Ok(id)
    }

    /// Every user, oldest first, each with its memberships.
    pub fn list_users(&self, who: Actor) -> Result<Vec<UserInfo>, UserError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::UserList,
            target: None,
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        if who != Actor::Cli {
            self.record(&mut tx, &entry(403))?;
            commit(tx)?;
            return Err(UserError::CliOnly);
        }
        let time = |at| from_nanos(at).ok_or(TenantError::Io);
        let caps = tx
            .tenants()?
            .into_iter()
            .map(|row| Ok((row.id, stored_cap(row.role_cap.as_deref())?)))
            .collect::<Result<Vec<_>, TenantError>>()?;
        let cap_of = |tenant: Option<TenantId>| {
            caps.iter()
                .find(|(id, _)| Some(*id) == tenant)
                .and_then(|(_, cap)| *cap)
        };
        let memberships = tx.memberships()?;
        let users = tx
            .users()?
            .into_iter()
            .map(|row| {
                let held = memberships
                    .iter()
                    .filter(|m| m.user == row.id)
                    .map(|m| {
                        // Roles that do not read back are damage.
                        let roles = Roles::parse(&m.roles).ok_or(TenantError::Io)?;
                        Ok(MembershipInfo {
                            tenant: m.tenant,
                            roles,
                            scopes: roles.within(cap_of(m.tenant)).scopes(),
                            granted_at: time(m.granted_at)?,
                        })
                    })
                    .collect::<Result<Vec<_>, TenantError>>()?;
                Ok(UserInfo {
                    id: row.id,
                    label: row.label,
                    identity: row
                        .identity
                        .map(|(issuer, subject)| Identity { issuer, subject }),
                    created_at: time(row.created_at)?,
                    deactivated_at: row.deactivated_at.map(time).transpose()?,
                    memberships: held,
                })
            })
            .collect::<Result<Vec<_>, TenantError>>()?;
        self.record(&mut tx, &entry(200))?;
        commit(tx)?;
        Ok(users)
    }

    /// Deactivates a user and removes its memberships; its row stays, for
    /// the logs that name its id, and its identity is free for a new user.
    /// Deactivating it again changes nothing. A tenant's log gets a copy
    /// of the record for each membership in it that was removed.
    pub fn deactivate_user(&self, who: Actor, id: UserId) -> Result<(), UserError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::UserDeactivate,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        let refused = if who != Actor::Cli {
            Some(UserError::CliOnly)
        } else if tx.user(id)?.is_none() {
            Some(UserError::NoUser)
        } else {
            None
        };
        if let Some(error) = refused {
            self.record(&mut tx, &entry(error.status()))?;
            commit(tx)?;
            return Err(error);
        }
        let tenants: Vec<TenantId> = tx
            .memberships()?
            .into_iter()
            .filter(|m| m.user == id)
            .filter_map(|m| m.tenant)
            .collect();
        tx.deactivate_user(id, nanos(self.clock.now()))?;
        tx.delete_memberships_of_user(id)?;
        let done = entry(204);
        self.record(&mut tx, &done)?;
        // The copies open each tenant's Store, which reads the registry.
        commit(tx)?;
        let mut copied = Ok(());
        for tenant in tenants {
            copied = copied.and(self.copy_to_tenant(tenant, &done));
        }
        Ok(copied?)
    }

    /// Gives a user roles in `tenant`, or as an operator for None, and
    /// returns the roles the membership then holds: those it held and
    /// those named. A membership is made by its first grant.
    pub fn grant_membership(
        &self,
        who: Actor,
        user: UserId,
        tenant: Option<TenantId>,
        roles: Roles,
    ) -> Result<Roles, UserError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::MembershipGrant,
            target: Some(membership_target(user, tenant, Some(roles))),
            version: None,
            status,
        };
        let closing = tenant.is_some_and(|t| matches!(self.stores().get(&t), Some(Slot::Closing)));
        let mut tx = self.registry.immediate()?;
        let refused = if who != Actor::Cli {
            Some(UserError::CliOnly)
        } else if !roles.fit(tenant.is_none()) {
            Some(UserError::BadRoles)
        } else {
            match (tx.user(user)?, tenant) {
                (None, _) => Some(UserError::NoUser),
                (Some(row), _) if row.deactivated_at.is_some() => Some(UserError::Deactivated),
                (Some(_), Some(t)) => match tx.tenant(t)? {
                    None => Some(UserError::NoTenant),
                    Some(_) if closing => Some(UserError::Closing),
                    // The cap is asked of the roles this call names; a
                    // cap that does not read back is damage, not no cap.
                    Some(row)
                        if !roles
                            .outside(stored_cap(row.role_cap.as_deref())?)
                            .is_empty() =>
                    {
                        Some(UserError::OverCap)
                    }
                    Some(_) => None,
                },
                (Some(_), None) => None,
            }
        };
        if let Some(error) = refused {
            self.record(&mut tx, &entry(error.status()))?;
            commit(tx)?;
            return Err(error);
        }
        let held = match tx.membership(user, tenant)? {
            Some(row) => {
                // Roles that do not read back are damage, not none.
                let held = Roles::parse(&row.roles).ok_or(TenantError::Io)?.with(roles);
                tx.set_roles(&MembershipRow {
                    roles: held.to_string(),
                    ..row
                })?;
                held
            }
            None => {
                tx.insert_membership(&MembershipRow {
                    user,
                    tenant,
                    roles: roles.to_string(),
                    granted_at: nanos(self.clock.now()),
                })?;
                roles
            }
        };
        let done = entry(200);
        self.record(&mut tx, &done)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        if let Some(t) = tenant {
            self.copy_to_tenant(t, &done)?;
        }
        Ok(held)
    }

    /// Takes roles from a user's membership in `tenant`, or as an operator
    /// for None, and returns the roles it still holds; None for `roles`
    /// takes them all. A membership left with no role is removed.
    pub fn revoke_membership(
        &self,
        who: Actor,
        user: UserId,
        tenant: Option<TenantId>,
        roles: Option<Roles>,
    ) -> Result<Roles, UserError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::MembershipRevoke,
            target: Some(membership_target(user, tenant, roles)),
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        let found = tx.membership(user, tenant)?;
        let refused = if who != Actor::Cli {
            Some(UserError::CliOnly)
        } else if roles.is_some_and(Roles::is_empty) {
            Some(UserError::BadRoles)
        } else if tx.user(user)?.is_none() {
            Some(UserError::NoUser)
        } else if found.is_none() {
            Some(UserError::NoMembership)
        } else {
            None
        };
        let row = match (refused, found) {
            (None, Some(row)) => row,
            (refused, _) => {
                let error = refused.unwrap_or(UserError::NoMembership);
                self.record(&mut tx, &entry(error.status()))?;
                commit(tx)?;
                return Err(error);
            }
        };
        // Roles that do not read back are damage, not none.
        let held = Roles::parse(&row.roles).ok_or(TenantError::Io)?;
        let left = roles.map_or(Roles::default(), |taken| held.without(taken));
        if left.is_empty() {
            tx.delete_membership(user, tenant)?;
        } else {
            tx.set_roles(&MembershipRow {
                roles: left.to_string(),
                ..row
            })?;
        }
        let done = entry(200);
        self.record(&mut tx, &done)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        if let Some(t) = tenant {
            self.copy_to_tenant(t, &done)?;
        }
        Ok(left)
    }
}

#[cfg(test)]
mod tests {
    // Users and memberships, by kind: what must work, what must be refused,
    // and the limits and odd inputs.

    use serde_json::{Value, json};
    use time::macros::datetime;

    use super::{Identity, MembershipInfo, UserError, UserInfo};
    use crate::audit::Actor;
    use crate::audit::testing::exported;
    use crate::ids::{KeyId, TenantId, UserId};
    use crate::roles::{Role, Roles};
    use crate::tenants::tests::{Fixture, fixture};
    use crate::tenants::{Removal, Slot, TenantError};

    const CLI: Actor = Actor::Cli;

    fn sso(subject: &str) -> Identity {
        Identity {
            issuer: "https://idp.example.com".to_owned(),
            subject: subject.to_owned(),
        }
    }

    fn tenant(f: &Fixture, name: &str) -> TenantId {
        f.tenants.create(CLI, name, None).expect("made")
    }

    fn local(f: &Fixture, label: &str) -> UserId {
        f.tenants.add_user(CLI, label, None).expect("a local user")
    }

    fn roles(of: &[Role]) -> Roles {
        Roles::of(of)
    }

    const OPERATOR: &[Role] = &[Role::Operator];

    /// The admin log's records of `action`: target and status.
    fn admin(f: &Fixture, action: &str) -> Vec<(Value, Value)> {
        records(&exported(&f.conn(), None), action)
    }

    /// A tenant's log's records of `action`: target and status.
    fn in_tenant(f: &Fixture, tenant: TenantId, action: &str) -> Vec<(Value, Value)> {
        records(&exported(&f.tenant_catalog(tenant), None), action)
    }

    fn records(log: &[Value], action: &str) -> Vec<(Value, Value)> {
        log.iter()
            .filter(|r| r["action"] == action)
            .map(|r| (r["target"].clone(), r["status"].clone()))
            .collect()
    }

    fn count(f: &Fixture, sql: &str) -> i64 {
        f.conn().query_row(sql, [], |r| r.get(0)).expect(sql)
    }

    fn listed(f: &Fixture) -> Vec<UserInfo> {
        f.tenants.list_users(CLI).expect("the users")
    }

    /// The one listed user with this id.
    fn user(f: &Fixture, id: UserId) -> UserInfo {
        listed(f)
            .into_iter()
            .find(|u| u.id == id)
            .expect("the user is listed")
    }

    fn held(f: &Fixture, id: UserId, tenant: Option<TenantId>) -> Option<Roles> {
        user(f, id)
            .memberships
            .iter()
            .find(|m| m.tenant == tenant)
            .map(|m| m.roles)
    }

    // What must work.

    #[test]
    fn a_local_user_is_made_active_with_no_identity_and_no_membership() {
        let f = fixture();
        let id = local(&f, "Night shift lead");
        assert_eq!(
            user(&f, id),
            UserInfo {
                id,
                label: "Night shift lead".to_owned(),
                identity: None,
                created_at: datetime!(2026-10-23 12:00 UTC),
                deactivated_at: None,
                memberships: vec![],
            },
            "the user as listed"
        );
    }

    #[test]
    fn a_user_with_an_identity_keeps_its_issuer_and_subject() {
        let f = fixture();
        let id = f
            .tenants
            .add_user(CLI, "Auditor", Some(&sso("subject-1")))
            .expect("made");
        assert_eq!(
            user(&f, id).identity,
            Some(sso("subject-1")),
            "the identity as given"
        );
    }

    #[test]
    fn adding_a_user_is_recorded_by_its_id_and_nothing_else_of_it() {
        let f = fixture();
        let id = f
            .tenants
            .add_user(CLI, "Auditor Label", Some(&sso("subject-secret")))
            .expect("made");
        assert_eq!(
            admin(&f, "user_add"),
            [(json!(id.to_string()), json!(201))],
            "one record, of the id"
        );
        let log = format!("{:?}", exported(&f.conn(), None));
        for kept_out in ["Auditor Label", "subject-secret", "idp.example.com"] {
            assert!(!log.contains(kept_out), "{kept_out} is in no record");
        }
    }

    #[test]
    fn users_are_listed_oldest_first_and_the_list_is_recorded() {
        let f = fixture();
        let first = local(&f, "First");
        f.clock.set(datetime!(2026-10-23 12:01 UTC));
        let second = local(&f, "Second");
        f.tenants
            .grant_membership(CLI, second, None, roles(OPERATOR))
            .expect("granted");
        assert_eq!(
            listed(&f)
                .iter()
                .map(|u| u.memberships.len())
                .collect::<Vec<_>>(),
            [0, 1],
            "each user is listed with its own memberships"
        );
        assert_eq!(
            listed(&f).iter().map(|u| u.id).collect::<Vec<_>>(),
            [first, second],
            "in the order they were made"
        );
        assert_eq!(
            admin(&f, "user_list"),
            vec![(Value::Null, json!(200)); 2],
            "a record for each of the two lists"
        );
    }

    #[test]
    fn a_first_grant_makes_the_membership_with_its_roles_and_time() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let now = f
            .tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Auditor]))
            .expect("granted");
        assert_eq!(now, roles(&[Role::Reader, Role::Auditor]), "the roles held");
        assert_eq!(
            user(&f, id).memberships,
            [MembershipInfo {
                tenant: Some(t),
                roles: roles(&[Role::Reader, Role::Auditor]),
                scopes: crate::access::Scopes::of(&[
                    crate::access::Scope::Read,
                    crate::access::Scope::Audit,
                ]),
                granted_at: datetime!(2026-10-23 13:00 UTC),
            }],
            "the membership as listed"
        );
    }

    #[test]
    fn a_later_grant_adds_to_the_roles_held_and_keeps_the_first_time() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("first");
        f.clock.set(datetime!(2026-10-23 13:00 UTC));
        let now = f
            .tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Legal]))
            .expect("second");
        assert_eq!(now, roles(&[Role::Reader, Role::Legal]), "both roles");
        let memberships = user(&f, id).memberships;
        assert_eq!(memberships.len(), 1, "still one membership");
        assert_eq!(
            memberships[0].granted_at,
            datetime!(2026-10-23 12:00 UTC),
            "the time of the first grant"
        );
    }

    #[test]
    fn an_operator_membership_holds_the_operator_role() {
        let f = fixture();
        let id = local(&f, "Operator");
        f.tenants
            .grant_membership(CLI, id, None, roles(OPERATOR))
            .expect("granted");
        assert_eq!(held(&f, id, None), Some(roles(OPERATOR)), "as listed");
        assert_eq!(
            admin(&f, "membership_grant"),
            [(json!(format!("{id}/operator/operator")), json!(200))],
            "the record names the user, the place and the roles"
        );
    }

    #[test]
    fn a_tenant_grant_is_recorded_in_the_admin_log_and_the_tenants() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Producer]))
            .expect("granted");
        let record = [(json!(format!("{id}/{t}/reader,producer")), json!(200))];
        assert_eq!(admin(&f, "membership_grant"), record, "the admin log");
        assert_eq!(
            in_tenant(&f, t, "membership_grant"),
            record,
            "the tenant's log"
        );
    }

    #[test]
    fn revoking_some_roles_leaves_the_others() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Legal]))
            .expect("granted");
        let left = f
            .tenants
            .revoke_membership(CLI, id, Some(t), Some(roles(&[Role::Legal])))
            .expect("revoked");
        assert_eq!(left, roles(&[Role::Reader]), "what is left");
        assert_eq!(held(&f, id, Some(t)), Some(left), "as listed");
        let record = [(json!(format!("{id}/{t}/legal")), json!(200))];
        assert_eq!(admin(&f, "membership_revoke"), record, "the admin log");
        assert_eq!(
            in_tenant(&f, t, "membership_revoke"),
            record,
            "the tenant's log"
        );
    }

    #[test]
    fn revoking_with_no_roles_named_removes_the_membership() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Legal]))
            .expect("granted");
        let left = f
            .tenants
            .revoke_membership(CLI, id, Some(t), None)
            .expect("revoked");
        assert!(left.is_empty(), "nothing is left");
        assert_eq!(held(&f, id, Some(t)), None, "no membership is listed");
        assert_eq!(
            admin(&f, "membership_revoke"),
            [(json!(format!("{id}/{t}")), json!(200))],
            "the record names no roles"
        );
    }

    #[test]
    fn deactivating_a_user_stamps_it_and_removes_its_memberships() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Leaver"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("tenant");
        f.tenants
            .grant_membership(CLI, id, None, roles(OPERATOR))
            .expect("operator");
        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        f.tenants.deactivate_user(CLI, id).expect("deactivated");
        let shown = user(&f, id);
        assert_eq!(
            shown.deactivated_at,
            Some(datetime!(2026-10-23 14:00 UTC)),
            "when"
        );
        assert_eq!(shown.memberships, [], "no membership is left");
        let record = [(json!(id.to_string()), json!(204))];
        assert_eq!(admin(&f, "user_deactivate"), record, "the admin log");
        assert_eq!(
            in_tenant(&f, t, "user_deactivate"),
            record,
            "the log of the tenant the user was a member of"
        );
    }

    #[test]
    fn removing_a_tenant_removes_the_memberships_in_it_and_no_others() {
        let f = fixture();
        let (gone, kept) = (tenant(&f, "gone"), tenant(&f, "kept"));
        let id = local(&f, "Member");
        for (minute, place) in [Some(gone), Some(kept), None].into_iter().enumerate() {
            let minute = u8::try_from(minute).expect("a small number");
            f.clock
                .set(datetime!(2026-10-23 12:00 UTC) + time::Duration::minutes(minute.into()));
            let given = if place.is_some() {
                roles(&[Role::Reader])
            } else {
                roles(OPERATOR)
            };
            f.tenants
                .grant_membership(CLI, id, place, given)
                .expect("granted");
        }
        f.tenants
            .remove(CLI, gone, Removal::Empty)
            .expect("removed");
        assert_eq!(
            user(&f, id)
                .memberships
                .iter()
                .map(|m| m.tenant)
                .collect::<Vec<_>>(),
            [Some(kept), None],
            "the memberships left"
        );
    }

    #[test]
    fn users_and_memberships_are_there_after_a_restart() {
        let mut f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Auditor]))
            .expect("granted");
        let before = listed(&f);
        f.restart();
        assert_eq!(listed(&f), before, "what the registry kept");
    }

    #[test]
    fn a_user_is_shown_as_json_with_null_for_what_it_has_not() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::TenantAdmin, Role::Reader]))
            .expect("granted");
        assert_eq!(
            user(&f, id).to_json(),
            Some(json!({
                "id": id.to_string(),
                "label": "Reader",
                "issuer": null,
                "subject": null,
                "created_at": "2026-10-23T12:00:00Z",
                "deactivated_at": null,
                "memberships": [{
                    "tenant": t.to_string(),
                    "roles": ["reader", "tenant-admin"],
                    "scopes": ["read", "tenant_admin"],
                    "granted_at": "2026-10-23T12:00:00Z",
                }],
            })),
            "the line the command line prints"
        );
    }

    // What must be refused.

    /// Every call, made by `who`, with its error and the action it records.
    fn each_call(
        f: &Fixture,
        who: Actor,
        id: UserId,
        t: TenantId,
    ) -> Vec<(&'static str, Option<UserError>)> {
        vec![
            ("user_add", f.tenants.add_user(who, "Label", None).err()),
            ("user_list", f.tenants.list_users(who).err()),
            ("user_deactivate", f.tenants.deactivate_user(who, id).err()),
            (
                "membership_grant",
                f.tenants
                    .grant_membership(who, id, Some(t), roles(&[Role::Reader]))
                    .err(),
            ),
            (
                "membership_grant",
                f.tenants
                    .grant_membership(who, id, None, roles(OPERATOR))
                    .err(),
            ),
            (
                "membership_revoke",
                f.tenants.revoke_membership(who, id, Some(t), None).err(),
            ),
        ]
    }

    #[test]
    fn a_key_may_make_none_of_these_calls_and_each_refusal_is_recorded() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        let before = listed(&f);
        let key = Actor::Key(KeyId([7; 8]));
        let already = exported(&f.conn(), None).len();
        let calls = each_call(&f, key, id, t);
        let log = exported(&f.conn(), None);
        assert_eq!(log.len(), already + calls.len(), "one record for each call");
        for ((action, error), record) in calls.iter().zip(&log[already..]) {
            assert_eq!(*error, Some(UserError::CliOnly), "{action} by a key");
            assert_eq!(
                (&record["action"], &record["status"], &record["actor"]),
                (
                    &json!(action),
                    &json!(403),
                    &json!(KeyId([7; 8]).to_string())
                ),
                "the refusal's record"
            );
        }
        assert_eq!(listed(&f), before, "nothing changed");
    }

    #[test]
    fn a_bad_label_is_refused_and_recorded_with_no_target() {
        let f = fixture();
        for label in ["", " outer", "outer ", "new\nline", &"x".repeat(129)] {
            assert_eq!(
                f.tenants.add_user(CLI, label, None),
                Err(UserError::BadLabel),
                "label {label:?}"
            );
        }
        assert_eq!(
            admin(&f, "user_add"),
            vec![(Value::Null, json!(422)); 5],
            "each refusal's record"
        );
        assert_eq!(count(&f, "SELECT count(*) FROM user_account"), 0, "no user");
    }

    #[test]
    fn an_identity_no_provider_can_give_is_refused() {
        let f = fixture();
        let long = format!("https://{}", "a".repeat(248));
        for (issuer, subject) in [
            ("http://idp.example.com", "s"),
            ("idp.example.com", "s"),
            ("https://", "s"),
            ("https:///path", "s"),
            ("https://idp.example.com?x=1", "s"),
            ("https://idp.example.com#x", "s"),
            ("https://idp.example.com /a", "s"),
            ("https://idp.example.com\n", "s"),
            (long.as_str(), "s"),
            ("https://idp.example.com", ""),
            ("https://idp.example.com", " s"),
            ("https://idp.example.com", "s\u{7}"),
            ("https://idp.example.com", "s\u{e9}"),
        ] {
            let identity = Identity {
                issuer: issuer.to_owned(),
                subject: subject.to_owned(),
            };
            assert_eq!(
                f.tenants.add_user(CLI, "Label", Some(&identity)),
                Err(UserError::BadIdentity),
                "issuer {issuer:?}, subject {subject:?}"
            );
        }
        assert_eq!(count(&f, "SELECT count(*) FROM user_account"), 0, "no user");
    }

    #[test]
    fn an_identity_an_active_user_has_is_refused() {
        let f = fixture();
        f.tenants
            .add_user(CLI, "First", Some(&sso("subject-1")))
            .expect("made");
        assert_eq!(
            f.tenants.add_user(CLI, "Second", Some(&sso("subject-1"))),
            Err(UserError::IdentityTaken),
            "the same issuer and subject"
        );
        assert_eq!(
            admin(&f, "user_add").last(),
            Some(&(Value::Null, json!(409))),
            "the refusal's record"
        );
        assert_eq!(
            count(&f, "SELECT count(*) FROM user_account"),
            1,
            "one user"
        );
    }

    #[test]
    fn deactivating_an_unknown_user_is_refused_and_recorded() {
        let f = fixture();
        let nobody = UserId([9; 16]);
        assert_eq!(
            f.tenants.deactivate_user(CLI, nobody),
            Err(UserError::NoUser),
            "no user has the id"
        );
        assert_eq!(
            admin(&f, "user_deactivate"),
            [(json!(nobody.to_string()), json!(404))],
            "the refusal's record"
        );
    }

    #[test]
    fn a_grant_to_an_unknown_user_or_tenant_is_refused() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        let reader = roles(&[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, UserId([9; 16]), Some(t), reader),
            Err(UserError::NoUser),
            "an unknown user"
        );
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(TenantId([9; 16])), reader),
            Err(UserError::NoTenant),
            "an unknown tenant"
        );
        assert_eq!(
            admin(&f, "membership_grant")
                .iter()
                .map(|r| r.1.clone())
                .collect::<Vec<_>>(),
            [json!(404), json!(404)],
            "each refusal's record"
        );
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 0, "none made");
    }

    #[test]
    fn a_grant_to_a_deactivated_user_is_refused() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Leaver"));
        f.tenants.deactivate_user(CLI, id).expect("deactivated");
        for place in [Some(t), None] {
            let given = if place.is_some() {
                roles(&[Role::Reader])
            } else {
                roles(OPERATOR)
            };
            assert_eq!(
                f.tenants.grant_membership(CLI, id, place, given),
                Err(UserError::Deactivated),
                "in {place:?}"
            );
        }
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 0, "none made");
        assert_eq!(
            admin(&f, "membership_grant").last().map(|r| r.1.clone()),
            Some(json!(409)),
            "the refusal's status"
        );
    }

    #[test]
    fn roles_that_do_not_fit_the_membership_are_refused() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        for (place, given) in [
            (Some(t), roles(OPERATOR)),
            (Some(t), roles(&[Role::Reader, Role::Operator])),
            (Some(t), roles(&[])),
            (None, roles(&[Role::Reader])),
            (None, roles(&[Role::Operator, Role::TenantAdmin])),
            (None, roles(&[])),
        ] {
            assert_eq!(
                f.tenants.grant_membership(CLI, id, place, given),
                Err(UserError::BadRoles),
                "{given} in {place:?}"
            );
        }
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 0, "none made");
        assert_eq!(
            admin(&f, "membership_grant").len(),
            6,
            "each refusal is recorded"
        );
    }

    #[test]
    fn revoking_a_membership_that_is_not_there_is_refused() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        assert_eq!(
            f.tenants.revoke_membership(CLI, id, Some(t), None),
            Err(UserError::NoMembership),
            "no membership in the tenant"
        );
        assert_eq!(
            f.tenants.revoke_membership(CLI, id, None, None),
            Err(UserError::NoMembership),
            "no operator membership"
        );
        assert_eq!(
            f.tenants
                .revoke_membership(CLI, UserId([9; 16]), Some(t), None),
            Err(UserError::NoUser),
            "an unknown user"
        );
        assert_eq!(
            admin(&f, "membership_revoke")
                .iter()
                .map(|r| r.1.clone())
                .collect::<Vec<_>>(),
            [json!(404), json!(404), json!(404)],
            "each refusal's record"
        );
        assert_eq!(
            in_tenant(&f, t, "membership_revoke"),
            [],
            "a refusal is not copied to the tenant"
        );
    }

    #[test]
    fn revoking_an_empty_set_of_roles_is_refused_and_changes_nothing() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        assert_eq!(
            f.tenants
                .revoke_membership(CLI, id, Some(t), Some(roles(&[]))),
            Err(UserError::BadRoles),
            "no role named is not every role"
        );
        assert_eq!(
            held(&f, id, Some(t)),
            Some(roles(&[Role::Reader])),
            "the membership is as it was"
        );
    }

    #[test]
    fn a_call_whose_record_cannot_be_written_changes_nothing() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        let rows = |f: &Fixture| {
            (
                count(
                    f,
                    "SELECT count(*) FROM user_account WHERE deactivated_at IS NULL",
                ),
                count(f, "SELECT count(*) FROM membership WHERE roles = 'reader'"),
            )
        };
        f.tenants.break_audit();
        let failed = Some(UserError::Registry(TenantError::AuditFailed));
        for (action, error) in each_call(&f, CLI, id, t) {
            assert_eq!(error, failed, "{action}");
            assert_eq!(rows(&f), (1, 1), "after {action}");
        }
    }

    #[test]
    fn a_grant_in_a_tenant_being_removed_is_refused() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants.stores().insert(t, Slot::Closing);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Reader])),
            Err(UserError::Closing),
            "while the tenant closes"
        );
        assert_eq!(
            admin(&f, "membership_grant").last().map(|r| r.1.clone()),
            Some(json!(503)),
            "the refusal's status"
        );
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 0, "none made");
    }

    #[test]
    fn stored_roles_that_do_not_read_back_are_damage_not_no_roles() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        for damaged in ["owner", "", "reader,reader", "producer,reader"] {
            f.conn()
                .execute("UPDATE membership SET roles = ?1", [damaged])
                .expect("damage");
            let io = UserError::Registry(TenantError::Io);
            assert_eq!(
                f.tenants.list_users(CLI).err(),
                Some(io),
                "list, {damaged:?}"
            );
            assert_eq!(
                f.tenants
                    .grant_membership(CLI, id, Some(t), roles(&[Role::Legal])),
                Err(io),
                "grant, {damaged:?}"
            );
            assert_eq!(
                f.tenants.revoke_membership(CLI, id, Some(t), None),
                Err(io),
                "revoke, {damaged:?}"
            );
        }
        assert_eq!(
            count(&f, "SELECT count(*) FROM membership"),
            1,
            "still there"
        );
    }

    // Limits and odd inputs.

    #[test]
    fn a_label_is_1_to_128_characters_and_two_users_may_share_one() {
        let f = fixture();
        let longest = "\u{e9}".repeat(128);
        for label in ["x", longest.as_str(), "Same", "Same"] {
            f.tenants.add_user(CLI, label, None).expect(label);
        }
        assert_eq!(listed(&f).len(), 4, "every user");
    }

    #[test]
    fn an_issuer_may_be_255_bytes_with_a_port_and_a_path() {
        let f = fixture();
        let longest = format!("https://{}", "a".repeat(247));
        assert_eq!(longest.len(), 255, "the limit");
        for issuer in [
            longest.as_str(),
            "https://idp.example.com:8443/realms/main",
            "https://[2001:db8::1]/",
        ] {
            let identity = Identity {
                issuer: issuer.to_owned(),
                subject: "subject | with spaces".to_owned(),
            };
            f.tenants
                .add_user(CLI, "Label", Some(&identity))
                .expect(issuer);
        }
        assert_eq!(listed(&f).len(), 3, "every user");
    }

    #[test]
    fn a_subject_may_be_255_characters_and_no_more() {
        let f = fixture();
        f.tenants
            .add_user(CLI, "Label", Some(&sso(&"s".repeat(255))))
            .expect("255 characters");
        assert_eq!(
            f.tenants
                .add_user(CLI, "Label", Some(&sso(&"s".repeat(256)))),
            Err(UserError::BadIdentity),
            "256 characters"
        );
    }

    #[test]
    fn one_subject_under_two_issuers_is_two_users() {
        let f = fixture();
        let other = Identity {
            issuer: "https://other.example.com".to_owned(),
            subject: "subject-1".to_owned(),
        };
        let first = f.tenants.add_user(CLI, "First", Some(&sso("subject-1")));
        let second = f.tenants.add_user(CLI, "Second", Some(&other));
        assert!(first.is_ok() && second.is_ok(), "{first:?} {second:?}");
        assert_ne!(first, second, "two ids");
    }

    #[test]
    fn a_deactivated_users_identity_may_be_given_to_a_new_user() {
        let f = fixture();
        let old = f
            .tenants
            .add_user(CLI, "First", Some(&sso("subject-1")))
            .expect("made");
        f.tenants.deactivate_user(CLI, old).expect("deactivated");
        let new = f
            .tenants
            .add_user(CLI, "Again", Some(&sso("subject-1")))
            .expect("the identity is free");
        assert_ne!(old, new, "a new id, so the logs tell the two apart");
        assert_eq!(listed(&f).len(), 2, "both rows are kept");
    }

    #[test]
    fn deactivating_twice_keeps_the_first_time_and_records_both() {
        let f = fixture();
        let id = local(&f, "Leaver");
        f.clock.set(datetime!(2026-10-23 14:00 UTC));
        f.tenants.deactivate_user(CLI, id).expect("first");
        f.clock.set(datetime!(2026-10-23 15:00 UTC));
        f.tenants.deactivate_user(CLI, id).expect("again");
        assert_eq!(
            user(&f, id).deactivated_at,
            Some(datetime!(2026-10-23 14:00 UTC)),
            "the first time"
        );
        assert_eq!(admin(&f, "user_deactivate").len(), 2, "both calls");
    }

    #[test]
    fn granting_a_role_already_held_changes_nothing_and_is_recorded() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        for _ in 0..2 {
            let now = f
                .tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
                .expect("granted");
            assert_eq!(now, roles(&[Role::Reader]), "the one role");
        }
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 1, "one row");
        assert_eq!(admin(&f, "membership_grant").len(), 2, "both calls");
    }

    #[test]
    fn revoking_a_role_not_held_leaves_the_membership_as_it_was() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        let left = f
            .tenants
            .revoke_membership(CLI, id, Some(t), Some(roles(&[Role::Legal])))
            .expect("nothing to take");
        assert_eq!(left, roles(&[Role::Reader]), "what is left");
    }

    #[test]
    fn revoking_the_last_role_by_name_removes_the_membership() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        let left = f
            .tenants
            .revoke_membership(CLI, id, Some(t), Some(roles(&[Role::Reader, Role::Legal])))
            .expect("revoked");
        assert!(left.is_empty(), "nothing is left");
        assert_eq!(count(&f, "SELECT count(*) FROM membership"), 0, "no row");
    }

    #[test]
    fn a_user_holds_memberships_in_two_tenants_and_as_operator_apart() {
        let f = fixture();
        let (a, b) = (tenant(&f, "acme"), tenant(&f, "beta"));
        let id = local(&f, "Member");
        f.tenants
            .grant_membership(CLI, id, Some(a), roles(&[Role::Reader]))
            .expect("a");
        f.tenants
            .grant_membership(CLI, id, Some(b), roles(&[Role::Auditor]))
            .expect("b");
        f.tenants
            .grant_membership(CLI, id, None, roles(OPERATOR))
            .expect("operator");
        f.tenants
            .revoke_membership(CLI, id, Some(a), None)
            .expect("revoked in a");
        assert_eq!(held(&f, id, Some(a)), None, "gone in a");
        assert_eq!(held(&f, id, Some(b)), Some(roles(&[Role::Auditor])), "b");
        assert_eq!(held(&f, id, None), Some(roles(OPERATOR)), "operator");
        assert_eq!(
            in_tenant(&f, b, "membership_revoke"),
            [],
            "the other tenant's log has no record of it"
        );
    }

    #[test]
    fn a_change_of_roles_touches_one_membership_only() {
        let f = fixture();
        let (a, b) = (tenant(&f, "acme"), tenant(&f, "beta"));
        let (id, other) = (local(&f, "Member"), local(&f, "Other"));
        let reader = roles(&[Role::Reader]);
        for (user, place) in [(id, Some(a)), (id, Some(b)), (other, Some(a))] {
            f.tenants
                .grant_membership(CLI, user, place, reader)
                .expect("granted");
        }
        f.tenants
            .grant_membership(CLI, id, None, roles(OPERATOR))
            .expect("operator");
        f.tenants
            .grant_membership(CLI, id, Some(a), roles(&[Role::Legal]))
            .expect("a second role in one tenant");
        f.tenants
            .revoke_membership(CLI, id, Some(a), Some(reader))
            .expect("the first role taken there");
        assert_eq!(
            held(&f, id, Some(a)),
            Some(roles(&[Role::Legal])),
            "changed"
        );
        assert_eq!(
            held(&f, id, Some(b)),
            Some(reader),
            "the user's other tenant"
        );
        assert_eq!(
            held(&f, id, None),
            Some(roles(OPERATOR)),
            "the operator role"
        );
        assert_eq!(
            held(&f, other, Some(a)),
            Some(reader),
            "another user in the same tenant"
        );
    }

    #[test]
    fn an_operator_membership_is_copied_to_no_tenants_log() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Operator"));
        f.tenants
            .grant_membership(CLI, id, None, roles(OPERATOR))
            .expect("granted");
        f.tenants
            .revoke_membership(CLI, id, None, None)
            .expect("revoked");
        let log = exported(&f.tenant_catalog(t), None);
        assert!(
            log.iter().all(|r| !r["action"]
                .as_str()
                .is_some_and(|a| a.starts_with("membership"))),
            "the tenant's log: {log:?}"
        );
    }

    #[test]
    fn a_user_with_no_membership_deactivates_with_no_tenant_copy() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Leaver"));
        f.tenants.deactivate_user(CLI, id).expect("deactivated");
        assert_eq!(in_tenant(&f, t, "user_deactivate"), [], "no copy");
    }

    #[test]
    fn an_empty_registry_lists_no_users() {
        let f = fixture();
        assert_eq!(listed(&f), [], "no user yet");
    }

    #[test]
    fn every_role_set_is_written_in_one_order_and_read_back() {
        for bits in 1..128_u8 {
            let set = Roles::of(
                &Role::ALL
                    .into_iter()
                    .enumerate()
                    .filter(|(n, _)| bits >> n & 1 == 1)
                    .map(|(_, role)| role)
                    .collect::<Vec<_>>(),
            );
            assert_eq!(Roles::parse(&set.to_string()), Some(set), "{set}");
        }
        assert_eq!(
            Roles::named(&["legal".to_owned(), "reader".to_owned(), "legal".to_owned()])
                .map(|r| r.to_string()),
            Some("reader,legal".to_owned()),
            "names in any order, with a repeat"
        );
        assert_eq!(Roles::named(&["owner".to_owned()]), None, "no such role");
        assert_eq!(Roles::parse(""), None, "an empty row is damage");
    }
}

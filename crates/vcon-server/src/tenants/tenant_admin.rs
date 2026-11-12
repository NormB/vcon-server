//! What a tenant's administrator does in its own tenant: read and change
//! the tenant's settings, make, list and revoke the tenant's keys, and
//! list the tenant's memberships.
//!
//! The routes under `/tenant` reach these calls with a key that holds the
//! scope `tenant_admin`; the route checks the scope and names the key's
//! own tenant, so no call here reaches another tenant. Each call is
//! recorded in the admin log, in the registry transaction that does its
//! work, and copied to the tenant's log.
//!
//! The settings are the fixed list of [`crate::settings`]. A key made
//! here holds scopes within those of the tenant's role cap
//! ([`Tenants::set_role_cap`]) and never `admin` or `tenant_admin`: only
//! the operator makes a key that administers a tenant.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::keys::{KeyInfo, NewKey, key_info};
use super::role_cap::stored_cap;
use super::{KeyLife, RTx, Slot, TenantError, Tenants, commit};
use crate::access::Scopes;
use crate::audit::{Action, Actor, Entry};
use crate::backend::TenantRow;
use crate::clock::{from_nanos, nanos};
use crate::ids::{KeyId, TenantId, UserId};
use crate::roles::{Role, Roles};
use crate::settings::{self, Settings};

/// One membership in a tenant, as the tenant's administrator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberInfo {
    pub user: UserId,
    /// The user's label.
    pub label: String,
    /// The roles granted, as the membership's row keeps them.
    pub roles: Roles,
    /// What the membership lets its user do: the scopes of those of its
    /// roles that the tenant's role cap allows.
    pub scopes: Scopes,
    pub granted_at: OffsetDateTime,
}

impl MemberInfo {
    /// The member with its user's id, its label, its roles and scopes by
    /// name, and the time of the grant (RFC 3339, UTC). None when the time
    /// cannot be written so.
    #[must_use]
    pub fn to_json(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "user": self.user.to_string(),
            "label": self.label,
            "roles": self.roles.names(),
            "scopes": self.scopes.names(),
            "granted_at": self.granted_at.format(&Rfc3339).ok()?,
        }))
    }
}

/// The scopes a key made by a tenant's administrator may hold under the
/// tenant's role cap: those of the roles the cap allows, or of every
/// tenant role with no cap, and never `tenant_admin`.
#[must_use]
pub(crate) fn key_scopes(cap: Option<Roles>) -> Scopes {
    Roles::TENANT
        .within(cap)
        .without(Roles::of(&[Role::TenantAdmin]))
        .scopes()
}

/// The settings a tenant's rows make. Rows the list does not read are
/// damage.
fn stored_settings(tx: &mut RTx<'_>, tenant: TenantId) -> Result<Settings, TenantError> {
    Settings::stored(&tx.settings(tenant)?).ok_or(TenantError::Io)
}

impl Tenants {
    /// Starts a call on a tenant: the registry transaction and the
    /// tenant's row. A tenant that is not there or is being removed is
    /// refused here, and the refusal recorded in the admin log alone: there
    /// is no tenant's log to copy it to.
    fn begin(
        &self,
        tenant: TenantId,
        entry: impl Fn(u16) -> Entry,
    ) -> Result<(RTx<'_>, TenantRow), TenantError> {
        let closing = matches!(self.stores().get(&tenant), Some(Slot::Closing));
        let mut tx = self.registry.immediate()?;
        let (status, refused) = match tx.tenant(tenant)? {
            None => (404, TenantError::NotFound),
            Some(_) if closing => (503, TenantError::Closing),
            Some(row) => return Ok((tx, row)),
        };
        self.record(&mut tx, &entry(status))?;
        commit(tx)?;
        Err(refused)
    }

    /// Ends a call: its record in the admin log with its work, then the
    /// copy in the tenant's log. The copy opens the tenant's Store, which
    /// reads the registry, so it follows the commit.
    fn end(&self, mut tx: RTx<'_>, tenant: TenantId, entry: &Entry) -> Result<(), TenantError> {
        self.record(&mut tx, entry)?;
        commit(tx)?;
        self.copy_to_tenant(tenant, entry)
    }

    /// The tenant's settings: each name with what was set, or its default.
    pub fn settings(&self, who: Actor, tenant: TenantId) -> Result<Settings, TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::SettingsRead,
            target: Some(tenant.to_string()),
            version: None,
            status,
        };
        let (mut tx, _) = self.begin(tenant, entry)?;
        let settings = stored_settings(&mut tx, tenant)?;
        self.end(tx, tenant, &entry(200))?;
        Ok(settings)
    }

    /// Changes the tenant's settings as `change` says: a JSON object of
    /// names, each with its new value or null for its default
    /// ([`Settings::changed`]). Gives the settings after the change. A
    /// refused change stores nothing. The record names the tenant and the
    /// names changed, never a value.
    pub fn set_settings(
        &self,
        who: Actor,
        tenant: TenantId,
        change: &serde_json::Value,
    ) -> Result<Settings, TenantError> {
        let entry = |status, names: &str| Entry {
            actor: who,
            action: Action::SettingsSet,
            target: Some(if names.is_empty() {
                tenant.to_string()
            } else {
                format!("{tenant}/{names}")
            }),
            version: None,
            status,
        };
        let (mut tx, _) = self.begin(tenant, |status| entry(status, ""))?;
        let (after, changes) = match stored_settings(&mut tx, tenant)?.changed(change) {
            Ok(changed) => changed,
            Err(error) => {
                self.end(tx, tenant, &entry(422, ""))?;
                return Err(TenantError::BadSetting(error));
            }
        };
        for (name, value) in &changes {
            match value {
                Some(value) => tx.put_setting(tenant, name.as_str(), &value.text())?,
                None => tx.delete_setting(tenant, name.as_str())?,
            }
        }
        self.end(tx, tenant, &entry(200, &settings::named(&changes)))?;
        Ok(after)
    }

    /// Makes a key for the tenant with the default life, as
    /// [`Tenants::create_tenant_key_for`] makes one with
    /// [`KeyLife::Default`].
    pub fn create_tenant_key(
        &self,
        who: Actor,
        tenant: TenantId,
        scopes: Scopes,
        label: &str,
    ) -> Result<NewKey, TenantError> {
        self.create_tenant_key_for(who, tenant, scopes, label, KeyLife::Default)
    }

    /// Makes a key for the tenant, as its administrator may: with at
    /// least one scope, all among those the tenant's role cap gives a key and
    /// never `tenant_admin`. The key expires as `life` says; a life the key
    /// cannot have is refused after its scopes are.
    pub fn create_tenant_key_for(
        &self,
        who: Actor,
        tenant: TenantId,
        scopes: Scopes,
        label: &str,
        life: KeyLife,
    ) -> Result<NewKey, TenantError> {
        let entry = |status, target: String| Entry {
            actor: who,
            action: Action::KeyCreate,
            target: Some(target),
            version: None,
            status,
        };
        let (mut tx, row) = self.begin(tenant, |status| entry(status, tenant.to_string()))?;
        let allowed = key_scopes(stored_cap(row.role_cap.as_deref())?);
        if scopes.is_empty() || !scopes.within(allowed) {
            self.end(tx, tenant, &entry(422, tenant.to_string()))?;
            return Err(TenantError::BadScopes);
        }
        let now = nanos(self.clock.now());
        if let Err(refused) = life.expires_at(now, self.no_expiry_allowed()) {
            self.end(tx, tenant, &entry(422, tenant.to_string()))?;
            return Err(refused);
        }
        let made = self.insert_key(&mut tx, Some(tenant), scopes, label, life)?;
        self.end(tx, tenant, &entry(201, made.id.to_string()))?;
        Ok(made)
    }

    /// The tenant's keys, oldest first.
    pub fn tenant_keys(&self, who: Actor, tenant: TenantId) -> Result<Vec<KeyInfo>, TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::KeyList,
            target: Some(tenant.to_string()),
            version: None,
            status,
        };
        let (mut tx, _) = self.begin(tenant, entry)?;
        let keys = tx
            .keys(Some(tenant))?
            .into_iter()
            .map(key_info)
            .collect::<Result<Vec<_>, TenantError>>()?;
        self.end(tx, tenant, &entry(200))?;
        Ok(keys)
    }

    /// Revokes a key of the tenant; revoking it again changes nothing. A
    /// key of another tenant, an operator key and an id no key has are all
    /// not found, and nothing is revoked.
    pub fn revoke_tenant_key(
        &self,
        who: Actor,
        tenant: TenantId,
        id: KeyId,
    ) -> Result<(), TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::KeyRevoke,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let (mut tx, _) = self.begin(tenant, entry)?;
        if tx.key(id)?.is_none_or(|row| row.tenant != Some(tenant)) {
            self.end(tx, tenant, &entry(404))?;
            return Err(TenantError::NotFound);
        }
        tx.revoke_key(id, nanos(self.clock.now()))?;
        self.end(tx, tenant, &entry(204))
    }

    /// The tenant's memberships, oldest first: each user with its label,
    /// the roles granted and the scopes they give under the tenant's role
    /// cap.
    pub fn members(&self, who: Actor, tenant: TenantId) -> Result<Vec<MemberInfo>, TenantError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::MembershipList,
            target: Some(tenant.to_string()),
            version: None,
            status,
        };
        let (mut tx, row) = self.begin(tenant, entry)?;
        let cap = stored_cap(row.role_cap.as_deref())?;
        let users = tx.users()?;
        let mut held: Vec<_> = tx
            .memberships()?
            .into_iter()
            .filter(|m| m.tenant == Some(tenant))
            .collect();
        held.sort_by_key(|m| (m.granted_at, m.user.0));
        let members = held
            .into_iter()
            .map(|m| {
                // A membership with no user, or roles that do not read
                // back, is damage.
                let user = users.iter().find(|user| user.id == m.user);
                let roles = Roles::parse(&m.roles).ok_or(TenantError::Io)?;
                Ok(MemberInfo {
                    user: m.user,
                    label: user.ok_or(TenantError::Io)?.label.clone(),
                    roles,
                    scopes: roles.within(cap).scopes(),
                    granted_at: from_nanos(m.granted_at).ok_or(TenantError::Io)?,
                })
            })
            .collect::<Result<Vec<_>, TenantError>>()?;
        self.end(tx, tenant, &entry(200))?;
        Ok(members)
    }
}

#[cfg(test)]
mod tests {
    // By kind: what must work, what must be refused, and the limits and
    // odd inputs.

    use serde_json::{Value, json};

    use super::*;
    use crate::access::{Scope, Scopes};
    use crate::audit::Actor;
    use crate::audit::testing::exported;
    use crate::ids::{KeyId, TenantId, UserId};
    use crate::roles::{Role, Roles};
    use crate::settings::{Name, SettingError, Settings, Value as Held};
    use crate::tenants::tests::{Fixture, fixture};
    use crate::tenants::{Removal, Slot, TenantError};

    const CLI: Actor = Actor::Cli;

    fn tenant(f: &Fixture, name: &str) -> TenantId {
        f.tenants.create(CLI, name, None).expect("made")
    }

    /// A key of the tenant's administrator, made by the operator, and the
    /// actor its requests are.
    fn administrator(f: &Fixture, tenant: TenantId) -> (KeyId, Actor) {
        let key = f
            .tenants
            .create_key(
                CLI,
                Some(tenant),
                Scopes::of(&[Scope::TenantAdmin]),
                "admin",
            )
            .expect("the operator makes the administrator's key")
            .id;
        (key, Actor::Key(key))
    }

    /// A tenant and its administrator.
    fn administered(f: &Fixture, name: &str) -> (TenantId, Actor) {
        let t = tenant(f, name);
        (t, administrator(f, t).1)
    }

    fn cap(f: &Fixture, tenant: TenantId, of: &[Role]) {
        f.tenants
            .set_role_cap(CLI, tenant, Some(Roles::of(of)))
            .expect("the cap is set");
    }

    fn member(f: &Fixture, label: &str, tenant: Option<TenantId>, of: &[Role]) -> UserId {
        let user = f.tenants.add_user(CLI, label, None).expect("a local user");
        f.tenants
            .grant_membership(CLI, user, tenant, Roles::of(of))
            .expect("granted");
        user
    }

    /// The records of `action` in a log: actor, target and status.
    fn records(log: &[Value], action: &str) -> Vec<(Value, Value, Value)> {
        log.iter()
            .filter(|r| r["action"] == action)
            .map(|r| (r["actor"].clone(), r["target"].clone(), r["status"].clone()))
            .collect()
    }

    fn admin(f: &Fixture, action: &str) -> Vec<(Value, Value, Value)> {
        records(&exported(&f.conn(), None), action)
    }

    fn in_tenant(f: &Fixture, tenant: TenantId, action: &str) -> Vec<(Value, Value, Value)> {
        records(&exported(&f.tenant_catalog(tenant), None), action)
    }

    /// The last record of `action` in the admin log: target and status.
    fn last(f: &Fixture, action: &str) -> (Value, Value) {
        let (_, target, status) = admin(f, action).pop().expect("a record");
        (target, status)
    }

    fn actor(who: Actor) -> Value {
        match who {
            Actor::Key(key) => json!(key.to_string()),
            _ => json!("cli"),
        }
    }

    /// The tenant's rows of the settings table.
    fn stored(f: &Fixture, tenant: TenantId) -> Vec<(String, String)> {
        let conn = f.conn();
        let mut rows = conn
            .prepare("SELECT name, value FROM tenant_setting WHERE tenant = ?1 ORDER BY name")
            .expect("the table");
        rows.query_map([tenant.0], |r| Ok((r.get(0)?, r.get(1)?)))
            .expect("rows")
            .collect::<Result<_, _>>()
            .expect("text")
    }

    fn pairs(of: &[(&str, &str)]) -> Vec<(String, String)> {
        of.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn scopes(of: &[Scope]) -> Scopes {
        Scopes::of(of)
    }

    /// Whether the key is revoked, as a request's check reads it.
    fn revoked(f: &Fixture, key: KeyId) -> bool {
        f.tenants
            .stored_key(key)
            .expect("read")
            .expect("the key")
            .revoked
    }

    // What must work: settings.

    #[test]
    fn a_tenant_that_set_nothing_reads_every_default() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        assert_eq!(
            f.tenants.settings(who, t),
            Ok(Settings::default()),
            "the defaults"
        );
        assert_eq!(stored(&f, t), vec![], "and no row");
    }

    #[test]
    fn a_change_is_kept_and_read_back() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let change = json!({ "masking.legal": false, "session.idle_seconds": 300 });
        let after = f.tenants.set_settings(who, t, &change).expect("set");
        assert_eq!(
            after.get(Name::Masking(Role::Legal)),
            Held::Flag(false),
            "the answer holds the change"
        );
        assert_eq!(f.tenants.settings(who, t), Ok(after), "and a read gives it");
        assert_eq!(
            stored(&f, t),
            pairs(&[("masking.legal", "false"), ("session.idle_seconds", "300")]),
            "one row for each name set"
        );
    }

    #[test]
    fn a_read_is_recorded_in_the_admin_log_and_the_tenants() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.settings(who, t).expect("read");
        let record = vec![(actor(who), json!(t.to_string()), json!(200))];
        assert_eq!(admin(&f, "settings_read"), record, "the admin log");
        assert_eq!(in_tenant(&f, t, "settings_read"), record, "the tenant's");
    }

    #[test]
    fn a_change_is_recorded_with_its_names_and_no_value() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let change = json!({ "session.idle_seconds": 300, "masking.legal": false });
        f.tenants.set_settings(who, t, &change).expect("set");
        let record = vec![(
            actor(who),
            json!(format!("{t}/masking.legal,session.idle_seconds")),
            json!(200),
        )];
        assert_eq!(admin(&f, "settings_set"), record, "the admin log");
        assert_eq!(in_tenant(&f, t, "settings_set"), record, "the tenant's");
    }

    #[test]
    fn a_null_removes_the_row_and_gives_the_default_again() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants
            .set_settings(
                who,
                t,
                &json!({ "masking.legal": false, "masking.reader": false }),
            )
            .expect("set");
        let after = f
            .tenants
            .set_settings(who, t, &json!({ "masking.legal": null }))
            .expect("unset");
        assert_eq!(
            after.get(Name::Masking(Role::Legal)),
            Held::Flag(true),
            "the default"
        );
        assert_eq!(
            stored(&f, t),
            pairs(&[("masking.reader", "false")]),
            "only the other row is left"
        );
    }

    #[test]
    fn a_later_change_builds_on_the_earlier_ones() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants
            .set_settings(
                who,
                t,
                &json!({ "session.absolute_seconds": 600 , "session.idle_seconds": 300 }),
            )
            .expect("first");
        let after = f
            .tenants
            .set_settings(who, t, &json!({ "session.idle_seconds": 600 }))
            .expect("second: 600 is within the absolute limit already set");
        assert_eq!(after.get(Name::SessionIdle), Held::Seconds(600), "idle");
        assert_eq!(
            after.get(Name::SessionAbsolute),
            Held::Seconds(600),
            "absolute, from the first change"
        );
    }

    #[test]
    fn each_tenant_has_its_own_settings() {
        let f = fixture();
        let (a, who_a) = administered(&f, "acme");
        let (b, who_b) = administered(&f, "beta");
        f.tenants
            .set_settings(who_a, a, &json!({ "masking.reader": false }))
            .expect("acme's");
        assert_eq!(
            f.tenants.settings(who_b, b),
            Ok(Settings::default()),
            "beta's are untouched"
        );
        assert_eq!(in_tenant(&f, b, "settings_set"), vec![], "nor is its log");
    }

    #[test]
    fn settings_are_there_after_a_restart() {
        let mut f = fixture();
        let (t, who) = administered(&f, "acme");
        let after = f
            .tenants
            .set_settings(who, t, &json!({ "session.idle_seconds": 120 }))
            .expect("set");
        f.restart();
        assert_eq!(
            f.tenants.settings(who, t),
            Ok(after),
            "what the registry kept"
        );
    }

    // What must work: keys.

    #[test]
    fn an_administrator_makes_a_key_for_its_tenant() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let made = f
            .tenants
            .create_tenant_key(who, t, scopes(&[Scope::Read]), "reports")
            .expect("made");
        let key = f.tenants.stored_key(made.id).expect("read").expect("kept");
        assert_eq!(key.tenant, Some(t), "a key of the tenant");
        assert_eq!(key.scopes, scopes(&[Scope::Read]), "with the scopes asked");
        assert!(!made.token.is_empty(), "and its token, shown this once");
    }

    #[test]
    fn making_a_key_is_recorded_in_both_logs_under_the_new_keys_id() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let made = f
            .tenants
            .create_tenant_key(who, t, scopes(&[Scope::Write]), "producer")
            .expect("made");
        let record = (actor(who), json!(made.id.to_string()), json!(201));
        assert_eq!(
            admin(&f, "key_create").last(),
            Some(&record),
            "the admin log"
        );
        assert_eq!(
            in_tenant(&f, t, "key_create").last(),
            Some(&record),
            "the tenant's"
        );
    }

    #[test]
    fn a_key_may_hold_every_scope_the_cap_gives() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        cap(&f, t, &[Role::Reader, Role::Producer, Role::TenantAdmin]);
        let asked = scopes(&[Scope::Read, Scope::Write]);
        let made = f
            .tenants
            .create_tenant_key(who, t, asked, "both")
            .expect("within the cap");
        let key = f.tenants.stored_key(made.id).expect("read").expect("kept");
        assert_eq!(key.scopes, asked, "both scopes");
    }

    #[test]
    fn an_administrator_lists_its_tenants_keys_and_no_others() {
        let f = fixture();
        let (a, who) = administered(&f, "acme");
        let (b, _) = administered(&f, "beta");
        // A later time, so the order below does not hang on the keys' ids.
        f.clock.set(time::macros::datetime!(2026-10-23 12:05 UTC));
        let made = f
            .tenants
            .create_tenant_key(who, a, scopes(&[Scope::Read]), "reports")
            .expect("made");
        let listed = f.tenants.tenant_keys(who, a).expect("listed");
        assert_eq!(listed.len(), 2, "the administrator's key and the new one");
        assert!(
            listed.iter().all(|key| key.tenant == Some(a)),
            "none of {b}'s: {listed:?}"
        );
        assert_eq!(
            listed.last().map(|key| (key.id, key.label.as_str())),
            Some((made.id, "reports")),
            "oldest first"
        );
    }

    #[test]
    fn listing_keys_is_recorded_in_both_logs() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.tenant_keys(who, t).expect("listed");
        let record = vec![(actor(who), json!(t.to_string()), json!(200))];
        assert_eq!(admin(&f, "key_list"), record, "the admin log");
        assert_eq!(in_tenant(&f, t, "key_list"), record, "the tenant's");
    }

    #[test]
    fn an_administrator_revokes_a_key_of_its_tenant() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let made = f
            .tenants
            .create_key(CLI, Some(t), scopes(&[Scope::Read]), "old")
            .expect("made by the operator");
        assert_eq!(
            f.tenants.revoke_tenant_key(who, t, made.id),
            Ok(()),
            "revoked"
        );
        assert!(revoked(&f, made.id), "its next request is refused");
        let record = (actor(who), json!(made.id.to_string()), json!(204));
        assert_eq!(
            admin(&f, "key_revoke").last(),
            Some(&record),
            "the admin log"
        );
        assert_eq!(
            in_tenant(&f, t, "key_revoke").last(),
            Some(&record),
            "the tenant's"
        );
    }

    // What must work: memberships.

    #[test]
    fn an_administrator_lists_its_tenants_memberships() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let reader = member(&f, "Reader", Some(t), &[Role::Reader]);
        f.clock.set(time::macros::datetime!(2026-10-23 12:05 UTC));
        let both = member(&f, "Both", Some(t), &[Role::Legal, Role::Auditor]);
        let listed = f.tenants.members(who, t).expect("listed");
        let shown: Vec<(UserId, &str, Roles, Scopes)> = listed
            .iter()
            .map(|m| (m.user, m.label.as_str(), m.roles, m.scopes))
            .collect();
        assert_eq!(
            shown,
            [
                (
                    reader,
                    "Reader",
                    Roles::of(&[Role::Reader]),
                    scopes(&[Scope::Read])
                ),
                (
                    both,
                    "Both",
                    Roles::of(&[Role::Legal, Role::Auditor]),
                    scopes(&[Scope::Legal, Scope::Audit])
                ),
            ],
            "each member with its label, roles and scopes, oldest first"
        );
    }

    #[test]
    fn a_members_scopes_are_those_the_cap_still_gives() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        member(&f, "Both", Some(t), &[Role::Legal, Role::Reader]);
        cap(&f, t, &[Role::Reader]);
        let listed = f.tenants.members(who, t).expect("listed");
        assert_eq!(
            listed[0].roles,
            Roles::of(&[Role::Legal, Role::Reader]),
            "the roles as granted"
        );
        assert_eq!(
            listed[0].scopes,
            scopes(&[Scope::Read]),
            "the scopes under the cap"
        );
    }

    #[test]
    fn listing_memberships_is_recorded_in_both_logs() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.members(who, t).expect("listed");
        let record = vec![(actor(who), json!(t.to_string()), json!(200))];
        assert_eq!(admin(&f, "membership_list"), record, "the admin log");
        assert_eq!(in_tenant(&f, t, "membership_list"), record, "the tenant's");
    }

    #[test]
    fn a_member_is_shown_as_json_with_names_and_a_time() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let user = member(&f, "Reader", Some(t), &[Role::Reader, Role::Auditor]);
        let listed = f.tenants.members(who, t).expect("listed");
        let shown = listed[0].to_json().expect("a time that can be written");
        assert_eq!(shown["user"], json!(user.to_string()), "the user's id");
        assert_eq!(shown["label"], json!("Reader"), "the label");
        assert_eq!(shown["roles"], json!(["reader", "auditor"]), "role names");
        assert_eq!(shown["scopes"], json!(["read", "audit"]), "scope names");
        assert!(
            shown["granted_at"]
                .as_str()
                .is_some_and(|at| at.ends_with('Z')),
            "RFC 3339, UTC: {}",
            shown["granted_at"]
        );
        assert_eq!(
            shown.as_object().map(serde_json::Map::len),
            Some(5),
            "and nothing else: {shown}"
        );
    }

    // What must be refused: settings.

    #[test]
    fn a_name_outside_the_list_is_refused_and_recorded() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        assert_eq!(
            f.tenants
                .set_settings(who, t, &json!({ "bulk_export": false })),
            Err(TenantError::BadSetting(SettingError::UnknownName)),
            "not a setting"
        );
        assert_eq!(stored(&f, t), vec![], "nothing is stored");
        assert_eq!(
            last(&f, "settings_set"),
            (json!(t.to_string()), json!(422)),
            "the refusal, with no name from the body"
        );
        assert_eq!(
            in_tenant(&f, t, "settings_set"),
            vec![(actor(who), json!(t.to_string()), json!(422))],
            "the tenant's log holds what its administrator was refused"
        );
    }

    #[test]
    fn a_limit_outside_its_bounds_is_refused_and_nothing_is_stored() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let change = json!({ "masking.reader": false, "session.idle_seconds": 30 });
        assert_eq!(
            f.tenants.set_settings(who, t, &change),
            Err(TenantError::BadSetting(SettingError::OutOfBounds)),
            "30 s is under the bound"
        );
        assert_eq!(
            stored(&f, t),
            vec![],
            "not even the good name of the change"
        );
    }

    #[test]
    fn an_idle_limit_above_the_absolute_limit_set_earlier_is_refused() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants
            .set_settings(
                who,
                t,
                &json!({ "session.absolute_seconds": 300, "session.idle_seconds": 120 }),
            )
            .expect("first");
        assert_eq!(
            f.tenants
                .set_settings(who, t, &json!({ "session.idle_seconds": 600 })),
            Err(TenantError::BadSetting(SettingError::IdleOverAbsolute)),
            "against the stored absolute limit"
        );
        assert_eq!(
            stored(&f, t),
            pairs(&[
                ("session.absolute_seconds", "300"),
                ("session.idle_seconds", "120")
            ]),
            "the rows are as the first change left them"
        );
    }

    #[test]
    fn settings_of_a_tenant_that_is_not_there_are_not_found() {
        let f = fixture();
        let gone = TenantId::generate();
        let who = Actor::Key(KeyId::generate());
        assert_eq!(
            f.tenants.settings(who, gone),
            Err(TenantError::NotFound),
            "read"
        );
        assert_eq!(
            f.tenants
                .set_settings(who, gone, &json!({ "masking.reader": false })),
            Err(TenantError::NotFound),
            "set"
        );
        assert_eq!(
            last(&f, "settings_read"),
            (json!(gone.to_string()), json!(404)),
            "the read's refusal"
        );
        assert_eq!(
            last(&f, "settings_set"),
            (json!(gone.to_string()), json!(404)),
            "the change's refusal"
        );
        assert_eq!(stored(&f, gone), vec![], "nothing is stored for no tenant");
    }

    #[test]
    fn a_change_for_a_tenant_being_removed_is_refused() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.stores().insert(t, Slot::Closing);
        assert_eq!(
            f.tenants
                .set_settings(who, t, &json!({ "masking.reader": false })),
            Err(TenantError::Closing),
            "while the tenant closes"
        );
        assert_eq!(stored(&f, t), vec![], "nothing was set");
        assert_eq!(
            last(&f, "settings_set"),
            (json!(t.to_string()), json!(503)),
            "recorded"
        );
    }

    #[test]
    fn a_stored_row_the_list_does_not_read_is_damage_not_a_default() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.conn()
            .execute(
                "INSERT INTO tenant_setting VALUES (?1, 'masking.reader', 'off')",
                [t.0],
            )
            .expect("a row by hand");
        assert_eq!(f.tenants.settings(who, t), Err(TenantError::Io), "read");
        assert_eq!(
            f.tenants
                .set_settings(who, t, &json!({ "masking.legal": false })),
            Err(TenantError::Io),
            "a change does not build on damage"
        );
        assert_eq!(
            stored(&f, t),
            pairs(&[("masking.reader", "off")]),
            "and writes nothing beside it"
        );
    }

    #[test]
    fn a_change_whose_record_cannot_be_written_is_not_made() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.break_audit();
        assert_eq!(
            f.tenants
                .set_settings(who, t, &json!({ "masking.reader": false })),
            Err(TenantError::AuditFailed),
            "no record, no change"
        );
        assert_eq!(stored(&f, t), vec![], "nothing was set");
        assert_eq!(
            f.tenants.settings(who, t),
            Err(TenantError::AuditFailed),
            "nor is a read answered without its record"
        );
    }

    // What must be refused: keys.

    #[test]
    fn an_administrator_cannot_make_another_administrators_key() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let before = f.tenants.tenant_keys(who, t).expect("listed").len();
        for asked in [
            scopes(&[Scope::TenantAdmin]),
            scopes(&[Scope::Read, Scope::TenantAdmin]),
        ] {
            assert!(
                matches!(
                    f.tenants.create_tenant_key(who, t, asked, "more"),
                    Err(TenantError::BadScopes)
                ),
                "only the operator gives tenant_admin: {asked}"
            );
        }
        assert_eq!(
            last(&f, "key_create"),
            (json!(t.to_string()), json!(422)),
            "recorded"
        );
        assert_eq!(
            f.tenants.tenant_keys(who, t).expect("listed").len(),
            before,
            "no key was made"
        );
    }

    #[test]
    fn an_administrator_cannot_make_an_operator_key() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        for asked in [
            scopes(&[Scope::Admin]),
            scopes(&[Scope::Read, Scope::Admin]),
        ] {
            assert!(
                matches!(
                    f.tenants.create_tenant_key(who, t, asked, "more"),
                    Err(TenantError::BadScopes)
                ),
                "admin is the operator's: {asked}"
            );
        }
    }

    #[test]
    fn an_administrators_key_expires_in_90_days_unless_a_life_is_asked() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants
            .create_tenant_key(who, t, scopes(&[Scope::Read]), "default")
            .expect("made");
        f.tenants
            .create_tenant_key_for(who, t, scopes(&[Scope::Read]), "week", KeyLife::Days(7))
            .expect("made");
        f.tenants
            .create_tenant_key_for(who, t, scopes(&[Scope::Read]), "forever", KeyLife::Never)
            .expect("made");
        let keys = f.tenants.tenant_keys(who, t).expect("listed");
        let life = |label: &str| {
            let key = keys.iter().find(|k| k.label == label).expect("the key");
            key.expires_at.map(|at| (at - key.created_at).whole_days())
        };
        assert_eq!(life("default"), Some(90), "no life asked");
        assert_eq!(life("week"), Some(7), "seven days asked");
        assert_eq!(life("forever"), None, "no expiry asked");
    }

    #[test]
    fn an_expiry_the_configuration_refuses_is_refused_and_recorded_in_both_logs() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.allow_no_expiry(false);
        let before = f.tenants.tenant_keys(who, t).expect("listed").len();
        assert!(
            matches!(
                f.tenants.create_tenant_key_for(
                    who,
                    t,
                    scopes(&[Scope::Read]),
                    "forever",
                    KeyLife::Never
                ),
                Err(TenantError::BadExpiry)
            ),
            "no expiry where keys.no_expiry is false"
        );
        assert!(
            matches!(
                f.tenants.create_tenant_key_for(
                    who,
                    t,
                    scopes(&[Scope::Read]),
                    "long",
                    KeyLife::Days(366)
                ),
                Err(TenantError::BadExpiry)
            ),
            "more than 365 days"
        );
        let refusal = (actor(who), json!(t.to_string()), json!(422));
        assert_eq!(
            in_tenant(&f, t, "key_create")
                .iter()
                .filter(|record| **record == refusal)
                .count(),
            2,
            "both refusals are in the tenant's log, under the administrator's key"
        );
        assert_eq!(
            admin(&f, "key_create")
                .iter()
                .filter(|record| **record == refusal)
                .count(),
            2,
            "and in the admin log"
        );
        assert_eq!(
            f.tenants.tenant_keys(who, t).expect("listed").len(),
            before,
            "no key was made"
        );
    }

    #[test]
    fn a_key_with_no_scope_is_refused() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        assert!(
            matches!(
                f.tenants
                    .create_tenant_key(who, t, Scopes::default(), "nothing"),
                Err(TenantError::BadScopes)
            ),
            "a key holds at least one scope"
        );
    }

    #[test]
    fn a_scope_outside_the_tenants_role_cap_is_refused() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        cap(&f, t, &[Role::Reader, Role::TenantAdmin]);
        for asked in [
            scopes(&[Scope::Write]),
            scopes(&[Scope::Read, Scope::Legal]),
        ] {
            assert!(
                matches!(
                    f.tenants.create_tenant_key(who, t, asked, "more"),
                    Err(TenantError::BadScopes)
                ),
                "the cap gives read alone: {asked}"
            );
        }
        assert_eq!(
            last(&f, "key_create"),
            (json!(t.to_string()), json!(422)),
            "recorded"
        );
    }

    #[test]
    fn a_key_of_another_tenant_is_not_found_and_not_revoked() {
        let f = fixture();
        let (a, who) = administered(&f, "acme");
        let (b, _) = administered(&f, "beta");
        let theirs = f
            .tenants
            .create_key(CLI, Some(b), scopes(&[Scope::Read]), "beta's")
            .expect("made");
        assert_eq!(
            f.tenants.revoke_tenant_key(who, a, theirs.id),
            Err(TenantError::NotFound),
            "as if there were no such key"
        );
        assert!(!revoked(&f, theirs.id), "the key still works");
        assert_eq!(
            last(&f, "key_revoke"),
            (json!(theirs.id.to_string()), json!(404)),
            "recorded in the admin log"
        );
        assert_eq!(
            in_tenant(&f, a, "key_revoke"),
            vec![(actor(who), json!(theirs.id.to_string()), json!(404))],
            "and in the log of the tenant whose administrator asked"
        );
        assert_eq!(
            in_tenant(&f, b, "key_revoke"),
            vec![],
            "nothing reaches the other tenant's log"
        );
    }

    #[test]
    fn an_operator_key_is_not_found_and_not_revoked() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let operator = f
            .tenants
            .create_key(CLI, None, scopes(&[Scope::Admin]), "operator")
            .expect("made");
        assert_eq!(
            f.tenants.revoke_tenant_key(who, t, operator.id),
            Err(TenantError::NotFound),
            "not a key of the tenant"
        );
        assert!(!revoked(&f, operator.id), "the operator's key still works");
    }

    #[test]
    fn a_key_nobody_has_is_not_found() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let unknown = KeyId::generate();
        assert_eq!(
            f.tenants.revoke_tenant_key(who, t, unknown),
            Err(TenantError::NotFound),
            "no such key"
        );
        assert_eq!(
            last(&f, "key_revoke"),
            (json!(unknown.to_string()), json!(404)),
            "recorded"
        );
    }

    #[test]
    fn keys_of_a_tenant_that_is_not_there_are_not_found() {
        let f = fixture();
        let gone = TenantId::generate();
        let who = Actor::Key(KeyId::generate());
        assert!(
            matches!(
                f.tenants
                    .create_tenant_key(who, gone, scopes(&[Scope::Read]), "x"),
                Err(TenantError::NotFound)
            ),
            "made"
        );
        assert_eq!(
            f.tenants.tenant_keys(who, gone).map(|keys| keys.len()),
            Err(TenantError::NotFound),
            "listed"
        );
    }

    #[test]
    fn a_key_for_a_tenant_being_removed_is_refused() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        f.tenants.stores().insert(t, Slot::Closing);
        assert!(
            matches!(
                f.tenants
                    .create_tenant_key(who, t, scopes(&[Scope::Read]), "late"),
                Err(TenantError::Closing)
            ),
            "while the tenant closes"
        );
    }

    // What must be refused: memberships.

    #[test]
    fn memberships_of_a_tenant_that_is_not_there_are_not_found() {
        let f = fixture();
        let gone = TenantId::generate();
        let who = Actor::Key(KeyId::generate());
        assert_eq!(
            f.tenants.members(who, gone).map(|members| members.len()),
            Err(TenantError::NotFound),
            "no tenant"
        );
        assert_eq!(
            last(&f, "membership_list"),
            (json!(gone.to_string()), json!(404)),
            "recorded"
        );
    }

    #[test]
    fn a_list_whose_record_cannot_be_written_is_not_answered() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        member(&f, "Reader", Some(t), &[Role::Reader]);
        f.tenants.break_audit();
        assert_eq!(
            f.tenants.members(who, t).map(|members| members.len()),
            Err(TenantError::AuditFailed),
            "memberships"
        );
        assert_eq!(
            f.tenants.tenant_keys(who, t).map(|keys| keys.len()),
            Err(TenantError::AuditFailed),
            "keys"
        );
    }

    // Limits and odd inputs.

    #[test]
    fn an_empty_change_is_answered_and_recorded_with_no_name() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        assert_eq!(
            f.tenants.set_settings(who, t, &json!({})),
            Ok(Settings::default()),
            "nothing to change"
        );
        assert_eq!(stored(&f, t), vec![], "no row");
        assert_eq!(
            last(&f, "settings_set"),
            (json!(t.to_string()), json!(200)),
            "the tenant alone as the target"
        );
    }

    #[test]
    fn removing_a_tenant_removes_its_settings() {
        let f = fixture();
        let (a, who_a) = administered(&f, "acme");
        let (b, who_b) = administered(&f, "beta");
        f.tenants
            .set_settings(who_a, a, &json!({ "masking.reader": false }))
            .expect("acme's");
        f.tenants
            .set_settings(who_b, b, &json!({ "masking.legal": false }))
            .expect("beta's");
        f.tenants.remove(CLI, a, Removal::Empty).expect("removed");
        assert_eq!(stored(&f, a), vec![], "none of acme's rows is left");
        assert_eq!(
            stored(&f, b),
            pairs(&[("masking.legal", "false")]),
            "beta's stay"
        );
    }

    #[test]
    fn with_no_cap_a_key_may_hold_all_five_scopes_of_a_tenants_data() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let all = scopes(&[
            Scope::Write,
            Scope::Read,
            Scope::Delete,
            Scope::Legal,
            Scope::Audit,
        ]);
        let made = f
            .tenants
            .create_tenant_key(who, t, all, "everything")
            .expect("no cap: every tenant role's scope");
        let key = f.tenants.stored_key(made.id).expect("read").expect("kept");
        assert_eq!(key.scopes, all, "all five");
    }

    #[test]
    fn a_cap_of_the_administrators_role_alone_lets_no_key_be_made() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        cap(&f, t, &[Role::TenantAdmin]);
        for scope in [
            Scope::Write,
            Scope::Read,
            Scope::Delete,
            Scope::Legal,
            Scope::Audit,
        ] {
            assert!(
                matches!(
                    f.tenants
                        .create_tenant_key(who, t, scopes(&[scope]), "none"),
                    Err(TenantError::BadScopes)
                ),
                "{} is outside the cap",
                scopes(&[scope])
            );
        }
    }

    #[test]
    fn a_cap_narrowed_later_leaves_the_keys_made_before_it() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let made = f
            .tenants
            .create_tenant_key(who, t, scopes(&[Scope::Write]), "producer")
            .expect("made with no cap");
        cap(&f, t, &[Role::Reader]);
        let key = f.tenants.stored_key(made.id).expect("read").expect("kept");
        assert_eq!(
            (key.scopes, key.revoked),
            (scopes(&[Scope::Write]), false),
            "the cap bounds new keys; the administrator revokes old ones"
        );
    }

    #[test]
    fn the_operator_is_not_bound_by_the_cap_when_it_makes_a_key() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        assert!(
            f.tenants
                .create_key(CLI, Some(t), scopes(&[Scope::Write]), "producer")
                .is_ok(),
            "the cap bounds the tenant's administrator, not the operator"
        );
    }

    #[test]
    fn an_administrator_may_revoke_its_own_key() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let (key, who) = administrator(&f, t);
        assert_eq!(f.tenants.revoke_tenant_key(who, t, key), Ok(()), "revoked");
        assert!(revoked(&f, key), "its next request is refused");
    }

    #[test]
    fn revoking_a_key_again_changes_nothing_and_is_not_an_error() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let made = f
            .tenants
            .create_tenant_key(who, t, scopes(&[Scope::Read]), "old")
            .expect("made");
        assert_eq!(f.tenants.revoke_tenant_key(who, t, made.id), Ok(()), "once");
        assert_eq!(
            f.tenants.revoke_tenant_key(who, t, made.id),
            Ok(()),
            "twice"
        );
        assert!(revoked(&f, made.id), "still revoked");
    }

    #[test]
    fn a_tenant_with_no_membership_lists_none() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        assert_eq!(
            f.tenants.members(who, t).map(|members| members.len()),
            Ok(0),
            "an empty list, not an error"
        );
    }

    #[test]
    fn memberships_elsewhere_are_not_listed() {
        let f = fixture();
        let (a, who) = administered(&f, "acme");
        let (b, _) = administered(&f, "beta");
        member(&f, "Theirs", Some(b), &[Role::Reader]);
        member(&f, "Operator", None, &[Role::Operator]);
        let ours = member(&f, "Ours", Some(a), &[Role::Reader]);
        let listed: Vec<UserId> = f
            .tenants
            .members(who, a)
            .expect("listed")
            .iter()
            .map(|m| m.user)
            .collect();
        assert_eq!(listed, [ours], "acme's member alone");
    }

    #[test]
    fn a_deactivated_user_is_no_longer_a_member() {
        let f = fixture();
        let (t, who) = administered(&f, "acme");
        let user = member(&f, "Leaver", Some(t), &[Role::Reader]);
        f.tenants.deactivate_user(CLI, user).expect("deactivated");
        assert_eq!(
            f.tenants.members(who, t).map(|members| members.len()),
            Ok(0),
            "deactivation removed the membership"
        );
    }

    #[test]
    fn a_user_in_two_tenants_is_listed_in_each_with_that_tenants_roles() {
        let f = fixture();
        let (a, who_a) = administered(&f, "acme");
        let (b, who_b) = administered(&f, "beta");
        let user = member(&f, "Both", Some(a), &[Role::Reader]);
        f.tenants
            .grant_membership(CLI, user, Some(b), Roles::of(&[Role::Legal]))
            .expect("granted in beta too");
        let roles_in = |who, t| f.tenants.members(who, t).expect("listed")[0].roles;
        assert_eq!(roles_in(who_a, a), Roles::of(&[Role::Reader]), "in acme");
        assert_eq!(roles_in(who_b, b), Roles::of(&[Role::Legal]), "in beta");
    }

    #[test]
    fn scopes_within_another_set_are_told_from_those_outside_it() {
        let held = scopes(&[Scope::Read, Scope::Write]);
        assert!(scopes(&[Scope::Read]).within(held), "one of two");
        assert!(held.within(held), "a set is within itself");
        assert!(
            Scopes::default().within(held),
            "the empty set is within any"
        );
        assert!(
            !scopes(&[Scope::Read, Scope::Legal]).within(held),
            "one scope outside"
        );
        assert!(!held.within(Scopes::default()), "nothing is within none");
    }

    #[test]
    fn the_scopes_a_cap_lets_a_key_hold_never_include_tenant_admin() {
        assert_eq!(
            key_scopes(None),
            scopes(&[
                Scope::Write,
                Scope::Read,
                Scope::Delete,
                Scope::Legal,
                Scope::Audit
            ]),
            "no cap: the five scopes of a tenant's data"
        );
        assert_eq!(
            key_scopes(Some(Roles::of(&[Role::Auditor, Role::TenantAdmin]))),
            scopes(&[Scope::Audit]),
            "a cap's scopes, less tenant_admin"
        );
        assert_eq!(
            key_scopes(Some(Roles::of(&[Role::TenantAdmin]))),
            Scopes::default(),
            "none"
        );
    }
}

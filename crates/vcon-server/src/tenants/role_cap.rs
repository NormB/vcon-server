//! A tenant's role cap: the roles a membership in the tenant may be given,
//! set by the operator on the host. A tenant with no cap allows every
//! tenant role.
//!
//! A grant of a role outside the cap is refused
//! ([`Tenants::grant_membership`]). A cap set after a role was granted
//! leaves the membership's row as it was: the role stops counting, so the
//! membership's scopes are those of the roles the cap still holds
//! ([`crate::roles::Roles::within`]), and counts again if the cap is
//! widened.
//!
//! Only the command line reaches these calls. Each is recorded in the
//! admin log, and a change is copied to the tenant's log.

use super::users::UserError;
use super::{Slot, TenantError, Tenants, commit};
use crate::audit::{Action, Actor, Entry};
use crate::ids::TenantId;
use crate::roles::Roles;

/// The cap a tenant's row keeps: None for no cap. Text that is not a
/// cap as [`Roles`] writes one is damage, not no cap.
pub(super) fn stored_cap(text: Option<&str>) -> Result<Option<Roles>, TenantError> {
    text.map(|text| {
        Roles::parse(text)
            .filter(|cap| cap.caps())
            .ok_or(TenantError::Io)
    })
    .transpose()
}

impl Tenants {
    /// Sets the roles a membership in the tenant may be given, or with
    /// None removes the cap, so every tenant role is allowed. Memberships
    /// keep the roles they hold; a role outside the cap stops counting.
    pub fn set_role_cap(
        &self,
        who: Actor,
        id: TenantId,
        cap: Option<Roles>,
    ) -> Result<(), UserError> {
        let named = cap.map_or_else(|| "all".to_owned(), |cap| cap.to_string());
        let entry = |status| Entry {
            actor: who,
            action: Action::RoleCapSet,
            target: Some(format!("{id}/{named}")),
            version: None,
            status,
        };
        let closing = matches!(self.stores().get(&id), Some(Slot::Closing));
        let text = cap.map(|cap| cap.to_string());
        let mut tx = self.registry.immediate()?;
        let refused = if who != Actor::Cli {
            Some(UserError::CliOnly)
        } else if cap.is_some_and(|cap| !cap.caps()) {
            Some(UserError::BadCap)
        } else if tx.tenant(id)?.is_none() {
            Some(UserError::NoTenant)
        } else if closing {
            Some(UserError::Closing)
        } else {
            None
        };
        if let Some(error) = refused {
            self.record(&mut tx, &entry(error.status()))?;
            commit(tx)?;
            return Err(error);
        }
        tx.set_role_cap(id, text.as_deref())?;
        let done = entry(200);
        self.record(&mut tx, &done)?;
        // The copy opens the tenant's Store, which reads the registry.
        commit(tx)?;
        self.copy_to_tenant(id, &done)?;
        Ok(())
    }

    /// The tenant's role cap; None when it has none.
    pub fn role_cap(&self, who: Actor, id: TenantId) -> Result<Option<Roles>, UserError> {
        let entry = |status| Entry {
            actor: who,
            action: Action::RoleCapRead,
            target: Some(id.to_string()),
            version: None,
            status,
        };
        let mut tx = self.registry.immediate()?;
        let found = if who == Actor::Cli {
            tx.tenant(id)?.ok_or(UserError::NoTenant)
        } else {
            Err(UserError::CliOnly)
        };
        let row = match found {
            Ok(row) => row,
            Err(error) => {
                self.record(&mut tx, &entry(error.status()))?;
                commit(tx)?;
                return Err(error);
            }
        };
        let cap = stored_cap(row.role_cap.as_deref())?;
        self.record(&mut tx, &entry(200))?;
        commit(tx)?;
        Ok(cap)
    }
}

#[cfg(test)]
mod tests {
    // The role cap, by kind: what must work, what must be refused, and
    // the limits and odd inputs.

    use serde_json::{Value, json};

    use crate::access::{Scope, Scopes};
    use crate::audit::Actor;
    use crate::audit::testing::exported;
    use crate::ids::{KeyId, TenantId, UserId};
    use crate::roles::{Role, Roles};
    use crate::tenants::tests::{Fixture, fixture};
    use crate::tenants::users::UserError;
    use crate::tenants::{Removal, Slot, TenantError};

    const CLI: Actor = Actor::Cli;

    fn tenant(f: &Fixture, name: &str) -> TenantId {
        f.tenants.create(CLI, name, None).expect("made")
    }

    fn local(f: &Fixture, label: &str) -> UserId {
        f.tenants.add_user(CLI, label, None).expect("a local user")
    }

    fn roles(of: &[Role]) -> Roles {
        Roles::of(of)
    }

    fn cap(f: &Fixture, tenant: TenantId, of: &[Role]) {
        f.tenants
            .set_role_cap(CLI, tenant, Some(roles(of)))
            .expect("the cap is set");
    }

    /// The records of `action` in a log: target and status.
    fn records(log: &[Value], action: &str) -> Vec<(Value, Value)> {
        log.iter()
            .filter(|r| r["action"] == action)
            .map(|r| (r["target"].clone(), r["status"].clone()))
            .collect()
    }

    fn admin(f: &Fixture, action: &str) -> Vec<(Value, Value)> {
        records(&exported(&f.conn(), None), action)
    }

    fn in_tenant(f: &Fixture, tenant: TenantId, action: &str) -> Vec<(Value, Value)> {
        records(&exported(&f.tenant_catalog(tenant), None), action)
    }

    /// The column as the registry keeps it.
    fn stored(f: &Fixture, tenant: TenantId) -> Option<String> {
        f.conn()
            .query_row(
                "SELECT role_cap FROM tenant WHERE id = ?1",
                [tenant.0],
                |r| r.get(0),
            )
            .expect("the tenant's row")
    }

    /// The roles and scopes `user list` shows for a user's membership.
    fn shown(f: &Fixture, user: UserId, tenant: Option<TenantId>) -> (Roles, Scopes) {
        let users = f.tenants.list_users(CLI).expect("the users");
        let user = users.iter().find(|u| u.id == user).expect("the user");
        let membership = user
            .memberships
            .iter()
            .find(|m| m.tenant == tenant)
            .expect("the membership");
        (membership.roles, membership.scopes)
    }

    // What must work.

    #[test]
    fn a_new_tenant_has_no_cap() {
        let f = fixture();
        let t = tenant(&f, "acme");
        assert_eq!(f.tenants.role_cap(CLI, t), Ok(None), "no cap");
        assert_eq!(stored(&f, t), None, "the column is NULL");
    }

    #[test]
    fn a_cap_is_set_and_read_back() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Auditor, Role::Reader]);
        assert_eq!(
            f.tenants.role_cap(CLI, t),
            Ok(Some(roles(&[Role::Reader, Role::Auditor]))),
            "the cap"
        );
        assert_eq!(
            stored(&f, t).as_deref(),
            Some("reader,auditor"),
            "written as a membership's roles are"
        );
    }

    #[test]
    fn setting_a_cap_is_recorded_in_the_admin_log_and_the_tenants() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader, Role::Producer]);
        let record = vec![(json!(format!("{t}/reader,producer")), json!(200))];
        assert_eq!(admin(&f, "role_cap_set"), record, "the admin log");
        assert_eq!(in_tenant(&f, t, "role_cap_set"), record, "the tenant's log");
    }

    #[test]
    fn reading_a_cap_is_recorded_in_the_admin_log_only() {
        let f = fixture();
        let t = tenant(&f, "acme");
        f.tenants.role_cap(CLI, t).expect("read");
        assert_eq!(
            admin(&f, "role_cap_read"),
            [(json!(t.to_string()), json!(200))],
            "the admin log"
        );
        assert_eq!(in_tenant(&f, t, "role_cap_read"), [], "no copy");
    }

    #[test]
    fn a_grant_within_the_cap_is_made() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader, Role::Auditor]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Reader])),
            Ok(roles(&[Role::Reader])),
            "one role of the cap"
        );
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Auditor])),
            Ok(roles(&[Role::Reader, Role::Auditor])),
            "the whole cap"
        );
    }

    #[test]
    fn a_memberships_scopes_are_those_of_its_roles() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        let given = roles(&[Role::Reader, Role::Legal, Role::TenantAdmin]);
        f.tenants
            .grant_membership(CLI, id, Some(t), given)
            .expect("granted");
        assert_eq!(
            shown(&f, id, Some(t)),
            (
                given,
                Scopes::of(&[Scope::Read, Scope::Legal, Scope::TenantAdmin])
            ),
            "the union of the roles' scopes"
        );
    }

    #[test]
    fn an_operator_memberships_scope_is_admin() {
        let f = fixture();
        let id = local(&f, "Operator");
        f.tenants
            .grant_membership(CLI, id, None, roles(&[Role::Operator]))
            .expect("granted");
        assert_eq!(
            shown(&f, id, None),
            (roles(&[Role::Operator]), Scopes::of(&[Scope::Admin])),
            "operator gives admin"
        );
    }

    #[test]
    fn a_cap_set_later_takes_the_scopes_of_the_roles_it_leaves_out() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        let given = roles(&[Role::Reader, Role::Records]);
        f.tenants
            .grant_membership(CLI, id, Some(t), given)
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            shown(&f, id, Some(t)),
            (given, Scopes::of(&[Scope::Read])),
            "the roles stay in the row; delete no longer counts"
        );
    }

    #[test]
    fn widening_the_cap_gives_the_scopes_back() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        let given = roles(&[Role::Reader, Role::Records]);
        f.tenants
            .grant_membership(CLI, id, Some(t), given)
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        cap(&f, t, &[Role::Reader, Role::Records]);
        assert_eq!(
            shown(&f, id, Some(t)).1,
            Scopes::of(&[Scope::Read, Scope::Delete]),
            "both count again"
        );
    }

    #[test]
    fn removing_the_cap_allows_every_tenant_role_again() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader]);
        f.tenants.set_role_cap(CLI, t, None).expect("removed");
        assert_eq!(f.tenants.role_cap(CLI, t), Ok(None), "no cap");
        assert_eq!(stored(&f, t), None, "the column is NULL again");
        assert_eq!(
            f.tenants.grant_membership(CLI, id, Some(t), Roles::TENANT),
            Ok(Roles::TENANT),
            "every tenant role"
        );
        assert_eq!(
            admin(&f, "role_cap_set").last(),
            Some(&(json!(format!("{t}/all")), json!(200))),
            "the record of a removed cap"
        );
    }

    #[test]
    fn a_cap_is_there_after_a_restart() {
        let mut f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Legal]);
        f.restart();
        assert_eq!(
            f.tenants.role_cap(CLI, t),
            Ok(Some(roles(&[Role::Legal]))),
            "what the registry kept"
        );
    }

    #[test]
    fn a_user_is_shown_with_each_memberships_scopes() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Auditor, Role::Producer]))
            .expect("granted");
        cap(&f, t, &[Role::Auditor]);
        let users = f.tenants.list_users(CLI).expect("the users");
        let line = users[0].to_json().expect("a line");
        assert_eq!(
            line["memberships"][0]["roles"],
            json!(["producer", "auditor"]),
            "the roles granted"
        );
        assert_eq!(
            line["memberships"][0]["scopes"],
            json!(["audit"]),
            "the scopes that count under the cap"
        );
    }

    #[test]
    fn revoking_a_role_outside_the_cap_still_works() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Records]))
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .revoke_membership(CLI, id, Some(t), Some(roles(&[Role::Records]))),
            Ok(roles(&[Role::Reader])),
            "the role the cap leaves out is taken from the row"
        );
    }

    // What must be refused.

    #[test]
    fn a_grant_outside_the_cap_is_refused_and_recorded() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Records])),
            Err(UserError::OverCap),
            "a role the cap leaves out"
        );
        assert_eq!(
            admin(&f, "membership_grant"),
            [(json!(format!("{id}/{t}/records")), json!(422))],
            "the refusal's record"
        );
        assert_eq!(
            in_tenant(&f, t, "membership_grant"),
            [],
            "a refusal is not copied to the tenant"
        );
    }

    #[test]
    fn a_grant_partly_outside_the_cap_gives_none_of_its_roles() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Reader, Role::Legal])),
            Err(UserError::OverCap),
            "one role in the cap, one outside"
        );
        let users = f.tenants.list_users(CLI).expect("the users");
        assert!(users[0].memberships.is_empty(), "no membership was made");
    }

    #[test]
    fn a_grant_outside_the_cap_leaves_a_membership_as_it_was() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader, Role::Auditor]);
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::TenantAdmin])),
            Err(UserError::OverCap),
            "tenant-admin is outside"
        );
        assert_eq!(
            shown(&f, id, Some(t)).0,
            roles(&[Role::Reader]),
            "the roles held before"
        );
    }

    #[test]
    fn a_key_may_not_set_or_read_a_cap() {
        let f = fixture();
        let t = tenant(&f, "acme");
        let key = Actor::Key(KeyId([7; 8]));
        assert_eq!(
            f.tenants.set_role_cap(key, t, Some(roles(&[Role::Reader]))),
            Err(UserError::CliOnly),
            "set by a key"
        );
        assert_eq!(
            f.tenants.role_cap(key, t),
            Err(UserError::CliOnly),
            "read by a key"
        );
        assert_eq!(stored(&f, t), None, "nothing was set");
        assert_eq!(
            admin(&f, "role_cap_set"),
            [(json!(format!("{t}/reader")), json!(403))],
            "the refused set"
        );
        assert_eq!(
            admin(&f, "role_cap_read"),
            [(json!(t.to_string()), json!(403))],
            "the refused read"
        );
    }

    #[test]
    fn a_cap_naming_operator_is_refused() {
        let f = fixture();
        let t = tenant(&f, "acme");
        for given in [
            roles(&[Role::Operator]),
            roles(&[Role::Reader, Role::Operator]),
        ] {
            assert_eq!(
                f.tenants.set_role_cap(CLI, t, Some(given)),
                Err(UserError::BadCap),
                "{given}"
            );
        }
        assert_eq!(stored(&f, t), None, "nothing was set");
        assert_eq!(
            admin(&f, "role_cap_set")
                .iter()
                .map(|r| r.1.clone())
                .collect::<Vec<_>>(),
            [json!(422), json!(422)],
            "each refusal is recorded"
        );
    }

    #[test]
    fn a_cap_of_no_roles_is_refused() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants.set_role_cap(CLI, t, Some(Roles::default())),
            Err(UserError::BadCap),
            "an empty cap would allow no membership"
        );
        assert_eq!(
            stored(&f, t).as_deref(),
            Some("reader"),
            "the cap set before"
        );
    }

    #[test]
    fn a_cap_for_an_unknown_tenant_is_refused() {
        let f = fixture();
        let unknown = TenantId([9; 16]);
        assert_eq!(
            f.tenants
                .set_role_cap(CLI, unknown, Some(roles(&[Role::Reader]))),
            Err(UserError::NoTenant),
            "set"
        );
        assert_eq!(
            f.tenants.set_role_cap(CLI, unknown, None),
            Err(UserError::NoTenant),
            "removed"
        );
        assert_eq!(
            f.tenants.role_cap(CLI, unknown),
            Err(UserError::NoTenant),
            "read"
        );
        assert_eq!(
            admin(&f, "role_cap_set")
                .iter()
                .map(|r| r.1.clone())
                .collect::<Vec<_>>(),
            [json!(404), json!(404)],
            "each refused change is recorded"
        );
        assert_eq!(
            admin(&f, "role_cap_read"),
            [(json!(unknown.to_string()), json!(404))],
            "the refused read"
        );
    }

    #[test]
    fn a_cap_for_a_tenant_being_removed_is_refused() {
        let f = fixture();
        let t = tenant(&f, "acme");
        f.tenants.stores().insert(t, Slot::Closing);
        assert_eq!(
            f.tenants.set_role_cap(CLI, t, Some(roles(&[Role::Reader]))),
            Err(UserError::Closing),
            "while the tenant closes"
        );
        assert_eq!(stored(&f, t), None, "nothing was set");
        assert_eq!(
            admin(&f, "role_cap_set").last().map(|r| r.1.clone()),
            Some(json!(503)),
            "the refusal's status"
        );
    }

    #[test]
    fn a_cap_whose_record_cannot_be_written_is_not_set() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        f.tenants.break_audit();
        let failed = Err(UserError::Registry(TenantError::AuditFailed));
        assert_eq!(
            f.tenants.set_role_cap(CLI, t, Some(roles(&[Role::Legal]))),
            failed,
            "set"
        );
        assert_eq!(f.tenants.set_role_cap(CLI, t, None), failed, "removed");
        assert_eq!(
            f.tenants.role_cap(CLI, t).map(|_| ()),
            failed,
            "a read that cannot be recorded gives nothing"
        );
        assert_eq!(
            stored(&f, t).as_deref(),
            Some("reader"),
            "the cap set before"
        );
    }

    #[test]
    fn a_stored_cap_that_does_not_read_back_is_damage_not_no_cap() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Reader]))
            .expect("granted");
        let io = UserError::Registry(TenantError::Io);
        for damaged in ["owner", "", "auditor,reader", "reader,reader", "operator"] {
            f.conn()
                .execute("UPDATE tenant SET role_cap = ?1", [damaged])
                .expect("damage");
            assert_eq!(f.tenants.role_cap(CLI, t), Err(io), "read, {damaged:?}");
            assert_eq!(
                f.tenants
                    .grant_membership(CLI, id, Some(t), roles(&[Role::Legal])),
                Err(io),
                "grant, {damaged:?}"
            );
            assert_eq!(
                f.tenants.list_users(CLI).err(),
                Some(io),
                "list, {damaged:?}"
            );
        }
    }

    #[test]
    fn a_cap_does_not_let_a_tenant_membership_hold_operator() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Operator])),
            Err(UserError::BadRoles),
            "operator is refused as before, whatever the cap"
        );
    }

    #[test]
    fn a_grant_to_an_unknown_user_is_refused_before_the_cap_is_read() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, UserId([8; 16]), Some(t), roles(&[Role::Legal])),
            Err(UserError::NoUser),
            "the user is looked for first"
        );
    }

    // Limits and odd inputs.

    #[test]
    fn a_cap_of_every_tenant_role_is_kept_as_a_cap() {
        let f = fixture();
        let t = tenant(&f, "acme");
        f.tenants
            .set_role_cap(CLI, t, Some(Roles::TENANT))
            .expect("set");
        assert_eq!(
            stored(&f, t).as_deref(),
            Some("reader,producer,records,legal,auditor,tenant-admin"),
            "all six, not NULL"
        );
        assert_eq!(
            f.tenants.role_cap(CLI, t),
            Ok(Some(Roles::TENANT)),
            "read back as a cap"
        );
    }

    #[test]
    fn setting_the_same_cap_twice_changes_nothing_and_records_both() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        cap(&f, t, &[Role::Reader]);
        assert_eq!(stored(&f, t).as_deref(), Some("reader"), "the cap");
        assert_eq!(admin(&f, "role_cap_set").len(), 2, "both are recorded");
    }

    #[test]
    fn removing_a_cap_that_is_not_there_is_not_an_error() {
        let f = fixture();
        let t = tenant(&f, "acme");
        assert_eq!(
            f.tenants.set_role_cap(CLI, t, None),
            Ok(()),
            "no cap before"
        );
        assert_eq!(stored(&f, t), None, "and none after");
    }

    #[test]
    fn a_cap_holds_in_its_own_tenant_only() {
        let f = fixture();
        let (a, b, id) = (tenant(&f, "acme"), tenant(&f, "beta"), local(&f, "Reader"));
        cap(&f, a, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(b), roles(&[Role::Legal])),
            Ok(roles(&[Role::Legal])),
            "the other tenant has no cap"
        );
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(a), roles(&[Role::Legal])),
            Err(UserError::OverCap),
            "the capped tenant refuses"
        );
        assert_eq!(f.tenants.role_cap(CLI, b), Ok(None), "beta's cap");
    }

    #[test]
    fn a_tenants_cap_does_not_touch_an_operator_membership() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Both"));
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, None, roles(&[Role::Operator])),
            Ok(roles(&[Role::Operator])),
            "an operator membership is in no tenant"
        );
        assert_eq!(
            shown(&f, id, None).1,
            Scopes::of(&[Scope::Admin]),
            "and keeps its scope"
        );
    }

    #[test]
    fn a_membership_wholly_outside_the_cap_has_no_scope_and_keeps_its_row() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Legal]))
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            shown(&f, id, Some(t)),
            (roles(&[Role::Legal]), Scopes::default()),
            "the role is kept and gives nothing"
        );
    }

    #[test]
    fn a_grant_of_a_role_already_held_outside_the_cap_is_refused_too() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Legal]))
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Legal])),
            Err(UserError::OverCap),
            "the cap is asked of the roles named, held or not"
        );
    }

    #[test]
    fn a_grant_within_the_cap_keeps_roles_held_outside_it() {
        let f = fixture();
        let (t, id) = (tenant(&f, "acme"), local(&f, "Reader"));
        f.tenants
            .grant_membership(CLI, id, Some(t), roles(&[Role::Legal]))
            .expect("granted");
        cap(&f, t, &[Role::Reader]);
        assert_eq!(
            f.tenants
                .grant_membership(CLI, id, Some(t), roles(&[Role::Reader])),
            Ok(roles(&[Role::Reader, Role::Legal])),
            "the row holds both"
        );
        assert_eq!(
            shown(&f, id, Some(t)).1,
            Scopes::of(&[Scope::Read]),
            "only the role in the cap counts"
        );
    }

    #[test]
    fn a_removed_tenants_cap_goes_with_its_row() {
        let f = fixture();
        let t = tenant(&f, "acme");
        cap(&f, t, &[Role::Reader]);
        f.tenants.remove(CLI, t, Removal::Empty).expect("removed");
        assert_eq!(
            f.tenants.role_cap(CLI, t),
            Err(UserError::NoTenant),
            "no tenant, no cap"
        );
    }

    #[test]
    fn two_tenants_keep_their_own_caps() {
        let f = fixture();
        let (a, b) = (tenant(&f, "acme"), tenant(&f, "beta"));
        cap(&f, a, &[Role::Reader]);
        cap(&f, b, &[Role::Legal, Role::Auditor]);
        f.tenants.set_role_cap(CLI, a, None).expect("removed");
        assert_eq!(f.tenants.role_cap(CLI, a), Ok(None), "acme");
        assert_eq!(
            f.tenants.role_cap(CLI, b),
            Ok(Some(roles(&[Role::Legal, Role::Auditor]))),
            "beta"
        );
    }
}

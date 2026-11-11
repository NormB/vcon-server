//! The roles a membership holds: what a user is in a tenant, or as an
//! operator. This is the one list of role names; a membership's row keeps
//! its roles as [`Roles`] writes them.

use std::fmt;

use crate::access::{Scope, Scopes};

/// One role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Reads a tenant's vCons.
    Reader,
    /// Stores vCons in a tenant.
    Producer,
    /// Deletes a tenant's vCons.
    Records,
    /// Reads deleted versions and sets legal holds in a tenant.
    Legal,
    /// Reads a tenant's audit log.
    Auditor,
    /// Administers a tenant.
    TenantAdmin,
    /// Administers the server; the one role of an operator membership.
    Operator,
}

impl Role {
    /// Every role, in the order a set is written.
    pub const ALL: [Role; 7] = [
        Role::Reader,
        Role::Producer,
        Role::Records,
        Role::Legal,
        Role::Auditor,
        Role::TenantAdmin,
        Role::Operator,
    ];

    /// The role's name, as the command line takes it and a row keeps it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Role::Reader => "reader",
            Role::Producer => "producer",
            Role::Records => "records",
            Role::Legal => "legal",
            Role::Auditor => "auditor",
            Role::TenantAdmin => "tenant-admin",
            Role::Operator => "operator",
        }
    }

    /// The role with this name, if there is one.
    #[must_use]
    pub fn named(name: &str) -> Option<Role> {
        Role::ALL.into_iter().find(|role| role.name() == name)
    }

    /// What the role lets its holder do: the one table of roles to
    /// scopes. A key holds scopes directly; a membership holds roles, and
    /// its scopes are those of its roles ([`Roles::scopes`]).
    #[must_use]
    pub fn scopes(self) -> Scopes {
        Scopes::of(match self {
            Role::Reader => &[Scope::Read],
            Role::Producer => &[Scope::Write],
            Role::Records => &[Scope::Delete],
            Role::Legal => &[Scope::Legal],
            Role::Auditor => &[Scope::Audit],
            Role::TenantAdmin => &[Scope::TenantAdmin],
            Role::Operator => &[Scope::Admin],
        })
    }

    fn bit(self) -> u8 {
        match self {
            Role::Reader => 1,
            Role::Producer => 1 << 1,
            Role::Records => 1 << 2,
            Role::Legal => 1 << 3,
            Role::Auditor => 1 << 4,
            Role::TenantAdmin => 1 << 5,
            Role::Operator => 1 << 6,
        }
    }
}

/// A set of roles. It is written as its roles' names in the order of
/// [`Role::ALL`], joined by commas; the empty set is written empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Roles(u8);

impl Roles {
    /// Every role a tenant membership may hold: all but `operator`.
    pub const TENANT: Roles = Roles((1 << 6) - 1);

    /// The scopes of the set: the union of its roles' scopes
    /// ([`Role::scopes`]).
    #[must_use]
    pub fn scopes(self) -> Scopes {
        Role::ALL
            .into_iter()
            .filter(|role| self.contains(*role))
            .fold(Scopes::default(), |held, role| held.with(role.scopes()))
    }

    /// The roles of this set that count in a tenant whose cap is `cap`:
    /// those the cap holds too, or all of them where the tenant has no
    /// cap.
    #[must_use]
    pub fn within(self, cap: Option<Roles>) -> Roles {
        cap.map_or(self, |cap| Roles(self.0 & cap.0))
    }

    /// The roles of this set that a tenant whose cap is `cap` does not
    /// allow; none where the tenant has no cap.
    #[must_use]
    pub fn outside(self, cap: Option<Roles>) -> Roles {
        cap.map_or(Roles::default(), |cap| self.without(cap))
    }

    /// Whether the set may be a tenant's cap: one or more of the roles a
    /// tenant membership may hold.
    #[must_use]
    pub fn caps(self) -> bool {
        self.fit(false)
    }

    /// The set of `roles`; a role given twice is held once.
    #[must_use]
    pub fn of(roles: &[Role]) -> Roles {
        Roles(roles.iter().fold(0, |bits, role| bits | role.bit()))
    }

    /// The set of the roles `names` name, in any order and with repeats;
    /// None if one is not a role's name.
    #[must_use]
    pub fn named(names: &[String]) -> Option<Roles> {
        let roles: Vec<Role> = names
            .iter()
            .map(|name| Role::named(name))
            .collect::<Option<_>>()?;
        Some(Roles::of(&roles))
    }

    /// Reads a set as [`Roles`] writes it, and nothing else: a row's roles
    /// in another order, with a repeat, or empty, are damage.
    #[must_use]
    pub fn parse(text: &str) -> Option<Roles> {
        let roles: Vec<Role> = text.split(',').map(Role::named).collect::<Option<_>>()?;
        let read = Roles::of(&roles);
        (read.to_string() == text).then_some(read)
    }

    /// Whether the set holds `role`.
    #[must_use]
    pub fn contains(self, role: Role) -> bool {
        self.0 & role.bit() != 0
    }

    /// Whether the set holds no role.
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The roles of either set.
    #[must_use]
    pub fn with(self, other: Roles) -> Roles {
        Roles(self.0 | other.0)
    }

    /// The roles of this set that `other` does not hold.
    #[must_use]
    pub fn without(self, other: Roles) -> Roles {
        Roles(self.0 & !other.0)
    }

    /// The names of the set's roles, in the order of [`Role::ALL`].
    #[must_use]
    pub fn names(self) -> Vec<&'static str> {
        Role::ALL
            .into_iter()
            .filter(|role| self.contains(*role))
            .map(Role::name)
            .collect()
    }

    /// Whether a membership may hold exactly this set: an operator
    /// membership holds `operator` and nothing else, and a tenant
    /// membership holds one or more of the other roles.
    #[must_use]
    pub fn fit(self, operator: bool) -> bool {
        if operator {
            self == Roles::of(&[Role::Operator])
        } else {
            !self.is_empty() && !self.contains(Role::Operator)
        }
    }
}

impl fmt::Display for Roles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.names().join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(of: &[Scope]) -> Scopes {
        Scopes::of(of)
    }

    /// The table as the plan states it, written out a second time so a
    /// change to one copy alone fails.
    const TABLE: [(Role, Scope); 7] = [
        (Role::Reader, Scope::Read),
        (Role::Producer, Scope::Write),
        (Role::Records, Scope::Delete),
        (Role::Legal, Scope::Legal),
        (Role::Auditor, Scope::Audit),
        (Role::TenantAdmin, Scope::TenantAdmin),
        (Role::Operator, Scope::Admin),
    ];

    #[test]
    fn a_reader_reads() {
        assert_eq!(Role::Reader.scopes(), scopes(&[Scope::Read]), "reader");
    }

    #[test]
    fn a_producer_writes() {
        assert_eq!(Role::Producer.scopes(), scopes(&[Scope::Write]), "producer");
    }

    #[test]
    fn records_deletes() {
        assert_eq!(Role::Records.scopes(), scopes(&[Scope::Delete]), "records");
    }

    #[test]
    fn legal_holds_the_legal_scope() {
        assert_eq!(Role::Legal.scopes(), scopes(&[Scope::Legal]), "legal");
    }

    #[test]
    fn an_auditor_reads_the_audit_log() {
        assert_eq!(Role::Auditor.scopes(), scopes(&[Scope::Audit]), "auditor");
    }

    #[test]
    fn a_tenant_admin_holds_tenant_admin_and_no_vcon_scope() {
        assert_eq!(
            Role::TenantAdmin.scopes(),
            scopes(&[Scope::TenantAdmin]),
            "tenant-admin"
        );
    }

    #[test]
    fn an_operator_holds_admin() {
        assert_eq!(Role::Operator.scopes(), scopes(&[Scope::Admin]), "operator");
    }

    #[test]
    fn the_table_has_one_line_for_each_role_and_each_scope() {
        assert_eq!(
            TABLE.map(|(role, _)| role),
            Role::ALL,
            "every role, in the order of Role::ALL"
        );
        for scope in Scope::ALL {
            let givers: Vec<Role> = Role::ALL
                .into_iter()
                .filter(|role| role.scopes().contains(scope))
                .collect();
            assert_eq!(givers.len(), 1, "{scope:?} is given by {givers:?}");
        }
        for (role, scope) in TABLE {
            assert_eq!(role.scopes(), scopes(&[scope]), "{role:?}");
        }
    }

    #[test]
    fn a_sets_scopes_are_the_union_of_its_roles() {
        assert_eq!(
            Roles::of(&[Role::Reader, Role::Auditor]).scopes(),
            scopes(&[Scope::Read, Scope::Audit]),
            "two roles"
        );
        assert_eq!(
            Roles::of(&[Role::Producer, Role::Reader, Role::Records])
                .scopes()
                .to_string(),
            "write,read,delete",
            "three roles, written in the scopes' order"
        );
    }

    #[test]
    fn every_subset_of_the_roles_gives_the_union_of_the_table() {
        for bits in 0..(1_u8 << 7) {
            let held: Vec<Role> = Role::ALL
                .into_iter()
                .enumerate()
                .filter(|(place, _)| bits & (1 << place) != 0)
                .map(|(_, role)| role)
                .collect();
            let want: Vec<Scope> = TABLE
                .into_iter()
                .filter(|(role, _)| held.contains(role))
                .map(|(_, scope)| scope)
                .collect();
            assert_eq!(Roles::of(&held).scopes(), scopes(&want), "{held:?}");
        }
    }

    #[test]
    fn the_empty_set_gives_no_scope() {
        assert!(Roles::default().scopes().is_empty(), "no role, no scope");
    }

    #[test]
    fn a_role_named_twice_gives_its_scope_once() {
        assert_eq!(
            Roles::of(&[Role::Legal, Role::Legal]).scopes(),
            scopes(&[Scope::Legal]),
            "legal twice"
        );
    }

    #[test]
    fn every_tenant_role_together_gives_every_scope_but_admin() {
        let all = Roles::TENANT.scopes();
        for scope in Scope::ALL {
            assert_eq!(all.contains(scope), scope != Scope::Admin, "{scope:?}");
        }
    }

    #[test]
    fn the_tenant_roles_are_every_role_but_operator() {
        assert_eq!(
            Roles::TENANT.names(),
            [
                "reader",
                "producer",
                "records",
                "legal",
                "auditor",
                "tenant-admin"
            ],
            "the six roles of a tenant membership"
        );
        assert!(Roles::TENANT.fit(false), "they fit a tenant membership");
    }

    #[test]
    fn no_tenant_role_gives_admin_and_operator_gives_no_tenant_scope() {
        for role in Role::ALL {
            assert_eq!(
                role.scopes().contains(Scope::Admin),
                role == Role::Operator,
                "{role:?}"
            );
        }
        assert_eq!(
            Role::Operator.scopes().names(),
            ["admin"],
            "operator gives admin alone"
        );
    }

    #[test]
    fn with_no_cap_every_role_counts() {
        let held = Roles::of(&[Role::Reader, Role::TenantAdmin]);
        assert_eq!(held.within(None), held, "no cap");
        assert_eq!(held.outside(None), Roles::default(), "nothing is outside");
    }

    #[test]
    fn under_a_cap_only_the_roles_in_it_count() {
        let held = Roles::of(&[Role::Reader, Role::Records, Role::Auditor]);
        let cap = Roles::of(&[Role::Reader, Role::Producer, Role::Auditor]);
        assert_eq!(
            held.within(Some(cap)),
            Roles::of(&[Role::Reader, Role::Auditor]),
            "the roles both hold"
        );
        assert_eq!(
            held.outside(Some(cap)),
            Roles::of(&[Role::Records]),
            "the role the cap leaves out"
        );
        assert_eq!(
            held.within(Some(cap)).scopes().to_string(),
            "read,audit",
            "and the scopes are those of the roles that count"
        );
    }

    #[test]
    fn a_cap_holding_none_of_the_roles_leaves_no_role_and_no_scope() {
        let held = Roles::of(&[Role::Legal]);
        let cap = Roles::of(&[Role::Reader]);
        assert!(held.within(Some(cap)).is_empty(), "no role counts");
        assert!(held.within(Some(cap)).scopes().is_empty(), "no scope");
        assert_eq!(held.outside(Some(cap)), held, "all of it is outside");
    }

    #[test]
    fn a_cap_wider_than_the_roles_changes_nothing() {
        let held = Roles::of(&[Role::Legal]);
        assert_eq!(held.within(Some(Roles::TENANT)), held, "every tenant role");
    }

    #[test]
    fn a_cap_is_a_set_a_tenant_membership_could_hold() {
        assert!(Roles::of(&[Role::Reader]).caps(), "one tenant role");
        assert!(Roles::TENANT.caps(), "every tenant role");
        assert!(!Roles::default().caps(), "no role");
        assert!(!Roles::of(&[Role::Operator]).caps(), "operator");
        assert!(
            !Roles::of(&[Role::Reader, Role::Operator]).caps(),
            "operator with a tenant role"
        );
    }

    #[test]
    fn a_roles_name_is_not_its_scopes_name() {
        assert_eq!(Role::named("tenant_admin"), None, "the scope's spelling");
        assert_eq!(Scope::named("tenant-admin"), None, "the role's spelling");
        assert_eq!(Role::named("read"), None, "a scope's name");
        assert_eq!(Scope::named("reader"), None, "a role's name");
    }
}

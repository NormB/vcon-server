//! The roles a membership holds: what a user is in a tenant, or as an
//! operator. This is the one list of role names; a membership's row keeps
//! its roles as [`Roles`] writes them.

use std::fmt;

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

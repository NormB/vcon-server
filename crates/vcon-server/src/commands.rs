//! The commands other than `serve`: tenants, keys and the audit logs. Each
//! takes the data directory as its work shares it, opens the registry with
//! recovery when it runs alone and without when it may run beside a server,
//! and prints what the matching admin route answers: one JSON object a
//! line.

use std::fmt;
use std::io::{self, Write};
use std::sync::Arc;

use crate::access::{Scope, Scopes};
use crate::audit::{Actor, AuditError};
use crate::backend;
use crate::clock::{Clock, SystemClock};
use crate::config::{Config, StorageError};
use crate::datadir::{self, LockError, Sharing, Work};
use crate::envelope::{Key, KeyError};
use crate::fs::OsFs;
use crate::ids::{KeyId, TenantId, UserId};
use crate::retention::Retention;
use crate::roles::{Role, Roles};
use crate::store::ExamineError;
use crate::tenants::{
    self, BackupError, Identity, KeyLife, OpenFailure, RegistryError, Removal, TenantError,
    TenantExamination, Tenants, Unexamined, UserError,
};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A command, as written on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    TenantCreate {
        name: String,
        retention: Option<String>,
    },
    TenantList,
    /// `retention` is a period, or `default` for the server's.
    TenantSetRetention {
        id: String,
        retention: String,
    },
    TenantRemove {
        id: String,
        force: bool,
        override_holds: bool,
    },
    /// `metadata` is `none`, `counts` or `full`.
    TenantSetMetadata {
        id: String,
        metadata: String,
    },
    /// Sets the roles a membership in the tenant may be given; `roles`
    /// None removes the cap.
    TenantSetRoleCap {
        id: String,
        roles: Option<Vec<String>>,
    },
    /// Prints the tenant's role cap.
    TenantRoleCap {
        id: String,
    },
    /// Moves every tenant's sealed objects from the data directory to the
    /// bucket `storage.objects = "s3"` names. Runs only while no server
    /// does.
    StorageMoveObjects,
    /// Writes the tenant's kept versions as Parquet files to `to`, a
    /// directory or an `s3://bucket/prefix`, beside a running server.
    /// `since` and `until` are RFC 3339 times.
    ExportParquet {
        tenant: String,
        to: String,
        since: Option<String>,
        until: Option<String>,
    },
    KeyCreate {
        owner: Owner,
        label: String,
        /// When the key expires: by default, in a number of days, or never.
        life: KeyLife,
    },
    KeyList {
        tenant: Option<String>,
    },
    KeyRevoke {
        id: String,
    },
    /// Makes a user. `identity` is the issuer and subject an identity
    /// provider knows the user by; None makes a local account.
    UserAdd {
        label: String,
        identity: Option<(String, String)>,
    },
    UserList,
    UserDeactivate {
        id: String,
    },
    /// Gives a user roles in a tenant, or the operator role.
    MembershipGrant {
        user: String,
        place: Place,
    },
    /// Takes roles from a user's membership; with no role named, all of
    /// them.
    MembershipRevoke {
        user: String,
        place: Place,
    },
    AuditExport {
        log: Log,
        since: Option<i64>,
    },
    /// Rotates a tenant's key.
    RekeyTenant {
        id: String,
    },
    /// Rotates the master key to the one in `new_master`.
    RekeyMaster {
        new_master: std::path::PathBuf,
    },
    /// Examines every tenant and reports what needs attention.
    Check,
    /// Writes a consistent copy of the data directory to `to`, which must
    /// not exist, beside a running server.
    Backup {
        to: std::path::PathBuf,
    },
}

/// Whose key `key create` makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Tenant { id: String, scopes: Vec<String> },
    Operator,
}

/// Where a membership is: in a tenant, with the roles the command names,
/// or as an operator, whose one role the command does not name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    Tenant { id: String, roles: Vec<String> },
    Operator,
}

/// Which audit log `audit export` writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Log {
    Tenant(String),
    Admin,
}

impl Command {
    /// The work this command does in the data directory.
    pub fn work(&self) -> Work {
        match self {
            Command::TenantCreate { .. } => Work::TenantCreate,
            Command::TenantList => Work::TenantList,
            Command::TenantSetRetention { .. } => Work::TenantSetRetention,
            Command::TenantRemove { .. } => Work::TenantRemove,
            Command::TenantSetMetadata { .. } => Work::TenantSetMetadata,
            Command::TenantSetRoleCap { .. } => Work::TenantSetRoleCap,
            Command::TenantRoleCap { .. } => Work::TenantRoleCap,
            Command::ExportParquet { .. } => Work::ExportParquet,
            Command::StorageMoveObjects => Work::StorageMoveObjects,
            Command::KeyCreate { .. } => Work::KeyCreate,
            Command::KeyList { .. } => Work::KeyList,
            Command::KeyRevoke { .. } => Work::KeyRevoke,
            Command::UserAdd { .. } => Work::UserAdd,
            Command::UserList => Work::UserList,
            Command::UserDeactivate { .. } => Work::UserDeactivate,
            Command::MembershipGrant { .. } => Work::MembershipGrant,
            Command::MembershipRevoke { .. } => Work::MembershipRevoke,
            Command::AuditExport { .. } => Work::AuditExport,
            Command::RekeyTenant { .. } => Work::RekeyTenant,
            Command::RekeyMaster { .. } => Work::RekeyMaster,
            Command::Check => Work::Check,
            Command::Backup { .. } => Work::Backup,
        }
    }
}

/// Why a command did not complete.
#[derive(Debug)]
pub enum CommandError {
    DataDir(LockError),
    MasterKey(KeyError),
    /// server.sqlite was made under another master key.
    WrongMasterKey,
    /// A period that is neither like 7y, 18m or 90d nor, where allowed,
    /// `default`.
    BadPeriod(String),
    Tenant(TenantError),
    /// The registry could not be opened.
    Open(RegistryError),
    /// No key has this id.
    UnknownKey,
    /// A user or membership command was refused.
    User(UserError),
    /// The output could not be written.
    Output(io::Error),
    /// The new master key is the one already in use.
    SameMasterKey,
    /// `check` found this many problems; its report says which.
    CheckFound(usize),
    Backup(BackupError),
    /// The storage the configuration names cannot be used.
    Storage(StorageError),
    /// Not `none`, `counts` or `full`.
    BadLevel(String),
    /// Not an RFC 3339 time.
    BadTime(String),
    /// Neither a directory nor an `s3://bucket/prefix`.
    BadTarget(String),
    /// An `s3://` target with no `[export.s3]` or `[storage.s3]` table, or
    /// in a build without S3.
    NoS3,
    Export(crate::export::ExportError),
    /// Moving a tenant's objects to the bucket stopped; what was moved
    /// stays moved, and running the command again moves the rest.
    Move {
        tenant: TenantId,
        why: String,
    },
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::DataDir(e) => write!(f, "{e}"),
            CommandError::MasterKey(KeyError::Refused(why)) => {
                write!(f, "the master key cannot be read: {why}")
            }
            CommandError::MasterKey(e) => write!(f, "the master key cannot be read: {e:?}"),
            CommandError::Open(e) => write!(
                f,
                "the registry cannot be opened: {}",
                crate::tenants::with_causes(e)
            ),
            CommandError::WrongMasterKey | CommandError::Tenant(TenantError::WrongKey) => {
                f.write_str("the master key is not the one server.sqlite was made under")
            }
            CommandError::BadPeriod(text) => write!(
                f,
                "{text:?} is not a period such as 7y, 18m or 90d (or, for set-retention, default)"
            ),
            CommandError::UnknownKey => f.write_str("no key has this id"),
            CommandError::User(e) => f.write_str(match e {
                UserError::CliOnly => "only the command line may do this",
                UserError::BadLabel => {
                    "a user label is 1 to 128 characters, with no control characters and no space at either end"
                }
                UserError::BadIdentity => {
                    "an issuer is an https:// URL of at most 255 bytes with no query or fragment, and a subject is 1 to 255 ASCII characters with no space at either end"
                }
                UserError::IdentityTaken => "an active user has this issuer and subject",
                UserError::NoUser => "no user has this id",
                UserError::Deactivated => "the user is deactivated",
                UserError::NoTenant => "no tenant has this id",
                UserError::Closing => "the tenant is being removed",
                UserError::BadRoles => {
                    "a tenant membership's roles are one or more of reader, producer, records, legal, auditor and tenant-admin; an operator membership holds operator alone"
                }
                UserError::OverCap => "a role named is outside the tenant's role cap",
                UserError::BadCap => {
                    "a role cap is one or more of reader, producer, records, legal, auditor and tenant-admin"
                }
                UserError::NoMembership => "the user has no such membership",
                UserError::Registry(_) => "the data could not be read or written",
            }),
            CommandError::Output(e) => write!(f, "the output could not be written: {e}"),
            CommandError::SameMasterKey => {
                f.write_str("the new master key is the one already in use")
            }
            CommandError::CheckFound(n) => write!(f, "check found {n} problem(s); see its report"),
            CommandError::Storage(e) => write!(f, "the storage cannot be used: {e}"),
            CommandError::BadLevel(text) => write!(
                f,
                "{text:?} is not a metadata level: none, counts or full"
            ),
            CommandError::BadTime(text) => write!(
                f,
                "{text:?} is not an RFC 3339 time such as 2026-11-05T00:00:00Z"
            ),
            CommandError::BadTarget(text) => write!(
                f,
                "{text:?} is not a directory or an s3://bucket/prefix"
            ),
            CommandError::NoS3 => f.write_str(
                "an s3:// target needs an [export.s3] or [storage.s3] table, in a build with the s3 feature",
            ),
            CommandError::Export(e) => write!(f, "{e}"),
            CommandError::Move { tenant, why } => write!(
                f,
                "moving tenant {tenant}'s objects to the bucket stopped: {why}; what moved stays \
                 moved, and running storage move-objects again moves the rest"
            ),
            CommandError::Backup(e) => f.write_str(match e {
                BackupError::Exists => {
                    "the backup's path already exists; a backup is written to a new directory"
                }
                BackupError::KeptChanging => {
                    "tenants were removed or versions purged during each try; try again"
                }
                BackupError::Io => "the backup could not be written, or the data could not be read",
                BackupError::AuditFailed => "the audit record could not be written",
            }),
            CommandError::Tenant(e) => f.write_str(match e {
                TenantError::NotFound => "no tenant has this id",
                TenantError::NameTaken => "another tenant has this name",
                TenantError::BadName => {
                    "a tenant name is 1 to 128 characters, with no control characters and no space at either end"
                }
                TenantError::HoldsVcons => "the tenant holds vCons; add --force to remove them too",
                TenantError::UnderHold => {
                    "a vCon of the tenant is under a legal hold; only --override-holds removes it"
                }
                TenantError::BadScopes => {
                    "a tenant key's scopes are one or more of write, read, delete, legal, audit and tenant_admin, each named once"
                }
                TenantError::BadExpiry => {
                    "a key expires in 1 to 365 days, or never where keys.no_expiry allows it"
                }
                TenantError::LimitReached => {
                    "the master key has wrapped all the tenant keys it may; run rekey master"
                }
                TenantError::AuditFailed => "the audit record could not be written",
                TenantError::Closing => "the tenant is being removed",
                TenantError::CliOnly => "only the command line may do this",
                TenantError::BadSetting(_) => {
                    "a setting's name or value is not one the tenant may set"
                }
                TenantError::WrongKey => unreachable!("matched above"),
                TenantError::Io => "the data could not be read or written",
            }),
        }
    }
}

/// Runs `command` against the data directory `config` names, writing its
/// answer to `out`.
/// `vcon-server migrate`: makes or brings up to date the schema of the
/// PostgreSQL catalog `config` names, and prints what it did. It uses no
/// data directory and no master key.
///
/// # Errors
///
/// [`CommandError::Storage`] when the catalog is local, this build has no
/// PostgreSQL support, or the migration was refused.
pub fn migrate(config: &Config, out: &mut dyn Write) -> Result<(), CommandError> {
    let done = backend::migrate_for_command(&config.catalog).map_err(CommandError::Storage)?;
    line(out, &done)
}

pub fn run(config: &Config, command: &Command, out: &mut dyn Write) -> Result<(), CommandError> {
    let sharing = command.work().sharing();
    let _lock = datadir::lock(&config.data_dir, sharing).map_err(CommandError::DataDir)?;
    let master = Key::load_master(&config.master_key).map_err(CommandError::MasterKey)?;
    backend::catalog_for_command(&config.catalog).map_err(CommandError::Storage)?;
    // A master rotation renames over server.sqlite, so nothing may hold it
    // open: it runs before the registry is opened.
    if let Command::RekeyMaster { new_master } = command {
        return rekey_master(config, &master, new_master, out);
    }
    // A command beside a running server must not run start-up recovery: it
    // would take the server's work in progress for a crash's leftovers. Nor
    // does `check`, which reports what recovery would remove.
    let open = if sharing == Sharing::Beside || matches!(command, Command::Check) {
        Tenants::open_beside_in
    } else {
        Tenants::open_in
    };
    let objects =
        backend::objects_for(&config.storage, &config.data_dir).map_err(CommandError::Storage)?;
    // The bucket the objects move to, for `storage move-objects`.
    #[cfg(feature = "s3")]
    let bucket = match &objects {
        backend::ObjectPlace::S3(s3) => Some(s3.clone()),
        backend::ObjectPlace::Local => None,
    };
    let tenants = open(
        &config.data_dir,
        master,
        Arc::new(OsFs),
        Arc::new(SystemClock),
        objects,
    )
    .map_err(|e| match e {
        RegistryError::WrongKey => CommandError::WrongMasterKey,
        other => CommandError::Open(other),
    })?;
    tenants.allow_no_expiry(config.keys_no_expiry);
    let who = Actor::Cli;
    match command {
        Command::TenantCreate { name, retention } => {
            let retention = retention.as_deref().map(period).transpose()?;
            let id = tenants.create(who, name, retention)?;
            line(out, &serde_json::json!({ "id": id.to_string() }))
        }
        Command::TenantList => {
            for tenant in tenants.list(who)? {
                line(out, &tenant.to_json())?;
            }
            Ok(())
        }
        Command::TenantSetRetention { id, retention } => {
            let retention = match retention.as_str() {
                "default" => None,
                text => Some(period(text)?),
            };
            let id = tenant_id(id)?;
            tenants.set_retention(who, id, retention)?;
            line(
                out,
                &serde_json::json!({
                    "id": id.to_string(),
                    "retention": retention.map(|r| r.to_string()),
                }),
            )
        }
        Command::TenantRemove {
            id,
            force,
            override_holds,
        } => {
            let removal = match (force, override_holds) {
                (_, true) => Removal::ForceOverridingHolds,
                (true, false) => Removal::Force,
                (false, false) => Removal::Empty,
            };
            Ok(tenants.remove(who, tenant_id(id)?, removal)?)
        }
        Command::KeyCreate { owner, label, life } => {
            let (tenant, scopes) = match owner {
                Owner::Tenant { id, scopes } => (
                    Some(tenant_id(id)?),
                    Scopes::named(scopes).ok_or(CommandError::Tenant(TenantError::BadScopes))?,
                ),
                Owner::Operator => (None, Scopes::of(&[Scope::Admin])),
            };
            let made = tenants.create_key_for(who, tenant, scopes, label, *life)?;
            let shown = made
                .to_json()
                .ok_or(CommandError::Tenant(TenantError::Io))?;
            line(out, &shown)
        }
        Command::KeyList { tenant } => {
            let tenant = tenant.as_deref().map(tenant_id).transpose()?;
            for key in tenants.list_keys(who, tenant)? {
                let shown = key.to_json().ok_or(CommandError::Tenant(TenantError::Io))?;
                line(out, &shown)?;
            }
            Ok(())
        }
        Command::KeyRevoke { id } => {
            let id = KeyId::parse(id).ok_or(CommandError::UnknownKey)?;
            tenants.revoke_key(who, id).map_err(|e| match e {
                TenantError::NotFound => CommandError::UnknownKey,
                other => CommandError::Tenant(other),
            })
        }
        Command::UserAdd { label, identity } => {
            let identity = identity.as_ref().map(|(issuer, subject)| Identity {
                issuer: issuer.clone(),
                subject: subject.clone(),
            });
            let id = tenants.add_user(who, label, identity.as_ref())?;
            line(out, &serde_json::json!({ "id": id.to_string() }))
        }
        Command::UserList => {
            for user in tenants.list_users(who)? {
                let shown = user
                    .to_json()
                    .ok_or(CommandError::Tenant(TenantError::Io))?;
                line(out, &shown)?;
            }
            Ok(())
        }
        Command::UserDeactivate { id } => Ok(tenants.deactivate_user(who, user_id(id)?)?),
        Command::MembershipGrant { user, place } => {
            let user = user_id(user)?;
            let (tenant, roles) = match place {
                Place::Tenant { id, roles } => (Some(tenant_id(id)?), named_roles(roles)?),
                Place::Operator => (None, Roles::of(&[Role::Operator])),
            };
            let held = tenants.grant_membership(who, user, tenant, roles)?;
            membership_line(out, user, tenant, held)
        }
        Command::MembershipRevoke { user, place } => {
            let user = user_id(user)?;
            // No role named takes every role the membership holds.
            let (tenant, roles) = match place {
                Place::Tenant { id, roles } if roles.is_empty() => (Some(tenant_id(id)?), None),
                Place::Tenant { id, roles } => (Some(tenant_id(id)?), Some(named_roles(roles)?)),
                Place::Operator => (None, None),
            };
            let left = tenants.revoke_membership(who, user, tenant, roles)?;
            membership_line(out, user, tenant, left)
        }
        Command::TenantSetMetadata { id, metadata } => {
            let level = crate::export::Metadata::parse(metadata)
                .ok_or_else(|| CommandError::BadLevel(metadata.clone()))?;
            let id = tenant_id(id)?;
            tenants.set_metadata(who, id, level)?;
            line(
                out,
                &serde_json::json!({ "id": id.to_string(), "metadata": level.as_str() }),
            )
        }
        Command::TenantSetRoleCap { id, roles } => {
            // A name that is no role's is refused as a cap that names
            // operator is: neither can be a cap.
            let cap = roles
                .as_ref()
                .map(|names| Roles::named(names).ok_or(CommandError::User(UserError::BadCap)))
                .transpose()?;
            let id = tenant_id(id)?;
            tenants.set_role_cap(who, id, cap)?;
            role_cap_line(out, id, cap)
        }
        Command::TenantRoleCap { id } => {
            let id = tenant_id(id)?;
            let cap = tenants.role_cap(who, id)?;
            role_cap_line(out, id, cap)
        }
        Command::ExportParquet {
            tenant,
            to,
            since,
            until,
        } => export_parquet(
            config,
            &tenants,
            tenant,
            to,
            since.as_deref(),
            until.as_deref(),
            out,
        ),
        Command::RekeyTenant { id } => Ok(tenants.rekey_tenant(who, tenant_id(id)?)?),
        Command::RekeyMaster { .. } => unreachable!("rotated before the registry is opened"),
        Command::Check => check(config, &tenants, out),
        #[cfg(feature = "s3")]
        Command::StorageMoveObjects => match bucket {
            Some(s3) => move_objects(config, &tenants, &s3, out),
            None => Err(not_a_bucket()),
        },
        #[cfg(not(feature = "s3"))]
        Command::StorageMoveObjects => Err(not_a_bucket()),
        Command::Backup { to } => {
            let done = tenants.backup(who, to).map_err(CommandError::Backup)?;
            line(
                out,
                &serde_json::json!({
                    "tenants": done.tenants,
                    "versions": done.versions,
                    "tries": done.tries,
                }),
            )
        }
        Command::AuditExport { log, since } => {
            let exported = match log {
                Log::Admin => tenants.export_admin(*since, out),
                Log::Tenant(id) => tenants.store(tenant_id(id)?)?.export(*since, out),
            };
            exported.map_err(|e| match e {
                AuditError::Write(e) => CommandError::Output(e),
                AuditError::Sqlite(_) => CommandError::Tenant(TenantError::Io),
            })
        }
    }
}

impl From<TenantError> for CommandError {
    fn from(e: TenantError) -> CommandError {
        CommandError::Tenant(e)
    }
}

impl From<UserError> for CommandError {
    /// A registry failure is the error every other command gives for it.
    fn from(e: UserError) -> CommandError {
        match e {
            UserError::Registry(e) => CommandError::Tenant(e),
            other => CommandError::User(other),
        }
    }
}

/// A user id as written; text that is no id names no user.
fn user_id(text: &str) -> Result<UserId, CommandError> {
    UserId::parse(text).ok_or(CommandError::User(UserError::NoUser))
}

/// The roles `names` name; a name that is no role's is refused as roles
/// no membership can hold.
fn named_roles(names: &[String]) -> Result<Roles, CommandError> {
    Roles::named(names).ok_or(CommandError::User(UserError::BadRoles))
}

/// The line a role cap command prints: the tenant, and the roles of its
/// cap or null for no cap.
fn role_cap_line(
    out: &mut dyn Write,
    tenant: TenantId,
    cap: Option<Roles>,
) -> Result<(), CommandError> {
    line(
        out,
        &serde_json::json!({
            "id": tenant.to_string(),
            "role_cap": cap.map(Roles::names),
        }),
    )
}

/// The line a membership command prints: the user, the tenant (null for
/// an operator membership) and the roles the membership now holds.
fn membership_line(
    out: &mut dyn Write,
    user: UserId,
    tenant: Option<TenantId>,
    roles: Roles,
) -> Result<(), CommandError> {
    line(
        out,
        &serde_json::json!({
            "user": user.to_string(),
            "tenant": tenant.map(|t| t.to_string()),
            "roles": roles.names(),
        }),
    )
}

/// Exports a tenant's kept versions as Parquet files and prints what was
/// written. Every argument is checked before anything is written.
fn export_parquet(
    config: &Config,
    tenants: &Tenants,
    tenant: &str,
    to: &str,
    since: Option<&str>,
    until: Option<&str>,
    out: &mut dyn Write,
) -> Result<(), CommandError> {
    use crate::export::target::Place;
    use crate::export::{self, Plan};
    let time = |text: Option<&str>| {
        text.map(|t| {
            OffsetDateTime::parse(t, &Rfc3339)
                .map(crate::clock::nanos)
                .map_err(|_| CommandError::BadTime(t.to_owned()))
        })
        .transpose()
    };
    let (since, until) = (time(since)?, time(until)?);
    let place = Place::parse(to).ok_or_else(|| CommandError::BadTarget(to.to_owned()))?;
    let id = tenant_id(tenant)?;
    let level = tenants.metadata(id)?;
    let target: Box<dyn export::Target> = match place {
        Place::Dir(dir) => Box::new(export::DirTarget::new(&dir, Arc::new(OsFs))),
        Place::S3 { bucket, prefix } => s3_target(config, &bucket, &prefix)?,
    };
    let store = tenants.store(id)?;
    let plan = Plan {
        level,
        since,
        until,
        sizes: export::SIZES,
        limits: vcon::read::Limits::new(config.inflated_limit),
    };
    let done = export::export(&store, &plan, target.as_ref(), SystemClock.now())
        .map_err(CommandError::Export)?;
    line(
        out,
        &serde_json::json!({
            "tenant": id.to_string(),
            "metadata": level.as_str(),
            "to": target.describe(),
            "manifest": done.manifest,
            "seq": done.seq,
            "files": done.files.len(),
            "rows": done.rows,
            "bytes": done.bytes,
            "skipped": done.skipped,
            "unread": done.unread,
        }),
    )
}

/// The bucket target `--to` names, reached through `[export.s3]`, else
/// `[storage.s3]`'s service and keys.
#[cfg(feature = "s3")]
fn s3_target(
    config: &Config,
    bucket: &str,
    prefix: &str,
) -> Result<Box<dyn crate::export::Target>, CommandError> {
    use crate::config::{ExportS3, ObjectStore};
    let (service, table) = match (&config.export.s3, &config.storage.objects) {
        (Some(service), _) => (service.clone(), "export.s3"),
        (None, ObjectStore::S3(storage)) => (ExportS3::from(storage), "storage.s3"),
        (None, ObjectStore::Local) => return Err(CommandError::NoS3),
    };
    let target = crate::export::s3::S3Target::connect(&service.for_bucket(bucket, prefix), table)
        .map_err(CommandError::Storage)?;
    Ok(Box::new(target))
}

#[cfg(not(feature = "s3"))]
fn s3_target(
    _config: &Config,
    _bucket: &str,
    _prefix: &str,
) -> Result<Box<dyn crate::export::Target>, CommandError> {
    Err(CommandError::NoS3)
}

/// Rotates the master key and says what the operator does next.
fn rekey_master(
    config: &Config,
    master: &Key,
    new_master: &std::path::Path,
    out: &mut dyn Write,
) -> Result<(), CommandError> {
    let new =
        Key::load_master_named(new_master, "--new-master").map_err(CommandError::MasterKey)?;
    if new.id() == master.id() {
        return Err(CommandError::SameMasterKey);
    }
    tenants::rekey_master(&config.data_dir, master, &new, SystemClock.now()).map_err(
        |e| match e {
            TenantError::WrongKey => CommandError::WrongMasterKey,
            other => CommandError::Tenant(other),
        },
    )?;
    line(
        out,
        &serde_json::json!({
            "master_key_id": new.id().to_string(),
            "next": "point master_key in the configuration at the new key file, start the server, then destroy every copy of the old master key",
        }),
    )
}

/// Examines every tenant, prints a line for each and a summary, and fails
/// when anything needs the operator.
/// The refusal of `storage move-objects` with the objects kept in the data
/// directory.
fn not_a_bucket() -> CommandError {
    CommandError::Storage(StorageError(
        "storage move-objects moves the objects to the bucket that storage.objects = \"s3\" \
         names; this configuration keeps them in the data directory"
            .to_owned(),
    ))
}

/// `storage move-objects`: each tenant's object files, under its directory
/// in the data directory, moved to the bucket, as `backend::migrate` does;
/// one line per tenant with what moved and what `check` then finds missing,
/// and a total.
#[cfg(feature = "s3")]
fn move_objects(
    config: &Config,
    tenants: &Tenants,
    s3: &Arc<crate::backend::s3::S3>,
    out: &mut dyn Write,
) -> Result<(), CommandError> {
    use crate::backend::migrate::MoveError;
    let mut total = 0;
    for id in tenants.ids()? {
        let dir = config
            .data_dir
            .join("tenants")
            .join(id.to_string())
            .join("objects");
        let moved = if dir.exists() {
            let from = crate::backend::local::LocalObjects::new(dir, Arc::new(OsFs));
            crate::backend::migrate::move_objects(&from, &s3.tenant(id)).map_err(|e| {
                let why = match e {
                    MoveError::From(e) => {
                        format!("an object file could not be read or removed: {e}")
                    }
                    MoveError::To(e) => format!("the bucket could not be written or read: {e}"),
                    MoveError::Differs(version) => format!(
                        "version {version} read back from the bucket differs from its file, \
                         which stays in the data directory"
                    ),
                };
                CommandError::Move { tenant: id, why }
            })?
        } else {
            0
        };
        let missing = tenants
            .store(id)?
            .check()
            .map_err(|e| CommandError::Move {
                tenant: id,
                why: format!("the objects in the bucket could not be checked: {e:?}"),
            })?
            .len();
        total += moved;
        line(
            out,
            &serde_json::json!({ "tenant": id.to_string(), "moved": moved, "missing": missing }),
        )?;
    }
    line(out, &serde_json::json!({ "moved": total }))
}

fn check(config: &Config, tenants: &Tenants, out: &mut dyn Write) -> Result<(), CommandError> {
    let now = SystemClock.now();
    let found = tenants.examine(config.retention_default, now)?;
    let time = |at: Option<OffsetDateTime>| at.and_then(|at| at.format(&Rfc3339).ok());
    let mut total = 0;
    for tenant in &found.tenants {
        let named = problems(
            tenant,
            config.purge_interval,
            now,
            config.storage.parquet.is_some(),
        );
        total += named.len();
        let store = tenant.store.as_ref().ok();
        line(
            out,
            &serde_json::json!({
                "tenant": tenant.id.to_string(),
                "name": tenant.name,
                "versions": store.map(|s| s.versions),
                "packed": store.map(|s| s.packed),
                "damaged": store.map(|s| s.damaged.iter().map(ToString::to_string).collect::<Vec<_>>()),
                "leftovers": store.map(|s| &s.leftovers),
                "past_due": store.map(|s| s.due),
                "oldest_past_due": time(store.and_then(|s| s.oldest_due_since)),
                "purges_awaiting_rotation": store.map(|s| s.purges_since_key),
                "oldest_purge_awaiting_rotation": time(store.and_then(|s| s.oldest_purge_since_key)),
                "rotation_stopped": tenant.rotation_stopped,
                "problems": named,
            }),
        )?;
    }
    line(
        out,
        &serde_json::json!({
            "tenants": found.tenants.len(),
            "removed_tenants_awaiting_master_rotation": found.removed_since_master,
            "oldest_removal_awaiting_master_rotation": time(found.oldest_removed_since_master),
            "problems": total,
        }),
    )?;
    match total {
        0 => Ok(()),
        n => Err(CommandError::CheckFound(n)),
    }
}

/// What in one tenant's examination needs the operator. A stalled sweep is
/// one whose oldest due version has waited longer than `purge_interval`;
/// a shorter wait is the next sweep's work. Packed versions are a problem
/// when the configuration has no `[storage.parquet]` (`parquet` false),
/// since `serve` then refuses to start. Leftover files and work waiting
/// for rotation are reported but are not problems.
pub fn problems(
    tenant: &TenantExamination,
    purge_interval: std::time::Duration,
    now: OffsetDateTime,
    parquet: bool,
) -> Vec<String> {
    let mut found = Vec::new();
    if tenant.rotation_stopped {
        found.push("a key rotation was stopped part way; run rekey tenant again".to_owned());
    }
    match &tenant.store {
        Err(Unexamined::Open(OpenFailure::WrongKey)) => {
            found.push("its catalog does not open under its tenant key".to_owned());
        }
        Err(Unexamined::Open(OpenFailure::Access { path, error })) => {
            found.push(format!(
                "its catalog cannot be opened: {}: {error}",
                path.display()
            ));
        }
        Err(Unexamined::Open(OpenFailure::Refused(e))) => {
            found.push(format!("its catalog cannot be opened: {e:?}"));
        }
        Err(Unexamined::Examine(ExamineError::Objects { kind, os_error })) => {
            let number = os_error
                .map(|n| format!(" (os error {n})"))
                .unwrap_or_default();
            found.push(format!("its objects cannot be read: {kind}{number}"));
        }
        Err(Unexamined::Examine(ExamineError::Failed(e))) => {
            found.push(format!("it could not be examined: {e:?}"));
        }
        Ok(store) => {
            if store.packed > 0 && !parquet {
                found.push(format!(
                    "{} version(s) are packed into Parquet files, and the configuration has no \
                     [storage.parquet]: serve refuses to start until the table is put back",
                    store.packed
                ));
            }
            for version in &store.damaged {
                found.push(format!("version {version} is missing or does not open"));
            }
            if let Some(since) = store.oldest_due_since
                && now - since > purge_interval
            {
                found.push(format!(
                    "{} version(s) due for purge, the oldest longer than purge_interval: the sweep is not keeping up",
                    store.due
                ));
            }
        }
    }
    found
}

/// A period as a command line writes it.
fn period(text: &str) -> Result<Retention, CommandError> {
    Retention::parse(text).ok_or_else(|| CommandError::BadPeriod(text.to_owned()))
}

/// A tenant id as a command line writes it; one that does not parse names
/// no tenant.
fn tenant_id(text: &str) -> Result<TenantId, CommandError> {
    TenantId::parse(text).ok_or(CommandError::Tenant(TenantError::NotFound))
}

/// Writes one JSON object as a line.
fn line(out: &mut dyn Write, value: &serde_json::Value) -> Result<(), CommandError> {
    writeln!(out, "{value}").map_err(CommandError::Output)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;
    use crate::store::Caller;
    use crate::store::testing::vcon;

    const UUID: &str = "01a07da8-c2bb-83e5-b9a2-279e0d16bc46";

    struct Setup {
        _dir: tempfile::TempDir,
        config: Config,
    }

    fn chmod(path: &std::path::Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }

    fn setup() -> Setup {
        let dir = tempfile::tempdir().expect("temp dir");
        let data = dir.path().join("data");
        std::fs::create_dir(&data).expect("data dir");
        let master = dir.path().join("master.key");
        std::fs::write(&master, [0x5a; 32]).expect("master key file");
        chmod(&master, 0o600);
        let config = crate::config::parse(&format!(
            "listen = \"127.0.0.1:1\"\ndata_dir = {data:?}\nmaster_key = {master:?}\n\
             [tls]\ncert = \"c\"\nkey = \"k\"\n"
        ))
        .expect("a configuration");
        Setup { _dir: dir, config }
    }

    impl Setup {
        /// Runs `command` and parses each line it prints.
        fn run(&self, command: Command) -> Result<Vec<Value>, CommandError> {
            let mut out = Vec::new();
            run(&self.config, &command, &mut out)?;
            Ok(String::from_utf8(out)
                .expect("text")
                .lines()
                .map(|line| serde_json::from_str(line).expect("a JSON line"))
                .collect())
        }

        fn create(&self, name: &str) -> String {
            let out = self
                .run(Command::TenantCreate {
                    name: name.to_owned(),
                    retention: None,
                })
                .expect("made");
            out[0]["id"].as_str().expect("an id").to_owned()
        }

        /// The registry as a test reads it, with the command's locks free.
        fn tenants(&self) -> Tenants {
            Tenants::open_beside(
                &self.config.data_dir,
                Key::load_master(&self.config.master_key).expect("master"),
                Arc::new(OsFs),
                Arc::new(SystemClock),
            )
            .expect("opens")
        }

        fn tenant_dirs(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.config.data_dir.join("tenants"))
                .expect("tenants dir")
                .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    /// `key create` for a tenant key that reads, with this life.
    fn key_with(s: &Setup, tenant: &str, life: KeyLife) -> Result<Vec<Value>, CommandError> {
        s.run(Command::KeyCreate {
            owner: Owner::Tenant {
                id: tenant.to_owned(),
                scopes: vec!["read".to_owned()],
            },
            label: "dated".to_owned(),
            life,
        })
    }

    fn rfc3339(value: &Value) -> time::OffsetDateTime {
        time::OffsetDateTime::parse(
            value.as_str().expect("a time as text"),
            &time::format_description::well_known::Rfc3339,
        )
        .expect("RFC 3339")
    }

    #[test]
    fn key_create_prints_the_expiry_of_the_key_it_makes() {
        let s = setup();
        let tenant = s.create("acme");
        let made = key_with(&s, &tenant, KeyLife::Default).expect("made");
        let listed = s.run(Command::KeyList { tenant: None }).expect("listed");
        assert_eq!(
            rfc3339(&made[0]["expires_at"]) - rfc3339(&listed[0]["created_at"]),
            time::Duration::days(90),
            "the default life"
        );
        assert_eq!(
            made[0]["expires_at"], listed[0]["expires_at"],
            "the list shows the same expiry"
        );
        assert!(listed[0]["last_used_at"].is_null(), "a key never used");
    }

    #[test]
    fn key_create_makes_a_key_for_the_days_asked_or_with_no_expiry() {
        let s = setup();
        let tenant = s.create("acme");
        let week = key_with(&s, &tenant, KeyLife::Days(7)).expect("made");
        let never = key_with(&s, &tenant, KeyLife::Never).expect("made");
        let listed = s.run(Command::KeyList { tenant: None }).expect("listed");
        assert_eq!(
            rfc3339(&week[0]["expires_at"]) - rfc3339(&listed[0]["created_at"]),
            time::Duration::days(7),
            "seven days"
        );
        assert!(
            never[0]["expires_at"].is_null(),
            "no expiry is printed as null"
        );
        assert!(listed[1]["expires_at"].is_null(), "and listed as null");
    }

    #[test]
    fn key_create_refuses_days_outside_1_to_365() {
        let s = setup();
        let tenant = s.create("acme");
        for days in [0, 366] {
            assert!(
                matches!(
                    key_with(&s, &tenant, KeyLife::Days(days)),
                    Err(CommandError::Tenant(TenantError::BadExpiry))
                ),
                "{days} days"
            );
        }
        let listed = s.run(Command::KeyList { tenant: None }).expect("listed");
        assert_eq!(listed.len(), 0, "no key was made");
    }

    #[test]
    fn key_create_refuses_no_expiry_where_the_configuration_says_so() {
        let mut s = setup();
        let tenant = s.create("acme");
        s.config.keys_no_expiry = false;
        assert!(
            matches!(
                key_with(&s, &tenant, KeyLife::Never),
                Err(CommandError::Tenant(TenantError::BadExpiry))
            ),
            "a tenant key"
        );
        let operator = s.run(Command::KeyCreate {
            owner: Owner::Operator,
            label: "ops".to_owned(),
            life: KeyLife::Never,
        });
        assert!(
            matches!(operator, Err(CommandError::Tenant(TenantError::BadExpiry))),
            "an operator key"
        );
        assert!(
            key_with(&s, &tenant, KeyLife::Default).is_ok(),
            "a key with an expiry is still made"
        );
    }

    #[test]
    fn an_operator_key_may_have_no_expiry_by_default() {
        let s = setup();
        let made = s
            .run(Command::KeyCreate {
                owner: Owner::Operator,
                label: "ops".to_owned(),
                life: KeyLife::Never,
            })
            .expect("made");
        assert!(made[0]["expires_at"].is_null(), "no expiry");
    }

    fn tenant_error(result: Result<Vec<Value>, CommandError>) -> TenantError {
        match result {
            Err(CommandError::Tenant(e)) => e,
            other => panic!("not a tenant error: {other:?}"),
        }
    }

    #[test]
    fn tenant_create_prints_the_new_id_and_list_shows_it() {
        let s = setup();
        let out = s
            .run(Command::TenantCreate {
                name: "acme".to_owned(),
                retention: Some("18m".to_owned()),
            })
            .expect("made");
        assert_eq!(out.len(), 1);
        let id = out[0]["id"].as_str().expect("an id").to_owned();
        assert!(TenantId::parse(&id).is_some(), "{id}");
        assert_eq!(out[0], json!({ "id": id }));
        let beta = s.create("beta");
        assert_eq!(
            s.run(Command::TenantList).expect("listed"),
            [
                json!({ "id": id, "name": "acme", "retention": "18m", "metadata": "none" }),
                json!({ "id": beta, "name": "beta", "retention": null, "metadata": "none" }),
            ]
        );
    }

    #[test]
    fn a_bad_period_is_refused_before_anything_is_made() {
        let s = setup();
        let error = s
            .run(Command::TenantCreate {
                name: "acme".to_owned(),
                retention: Some("7w".to_owned()),
            })
            .expect_err("refused");
        assert!(
            matches!(&error, CommandError::BadPeriod(p) if p == "7w"),
            "{error:?}"
        );
        assert!(error.to_string().contains("7w"), "{error}");
        assert_eq!(
            s.run(Command::TenantList).expect("listed"),
            Vec::<Value>::new()
        );
        let id = s.create("acme");
        let error = s
            .run(Command::TenantSetRetention {
                id,
                retention: "forever".to_owned(),
            })
            .expect_err("refused");
        assert!(matches!(&error, CommandError::BadPeriod(p) if p == "forever"));
    }

    #[test]
    fn set_retention_takes_a_period_or_default() {
        let s = setup();
        let id = s.create("acme");
        let set = |retention: &str| {
            s.run(Command::TenantSetRetention {
                id: id.clone(),
                retention: retention.to_owned(),
            })
            .expect("set")
        };
        assert_eq!(set("90d"), [json!({ "id": id, "retention": "90d" })]);
        assert_eq!(
            s.run(Command::TenantList).expect("listed")[0]["retention"],
            "90d"
        );
        assert_eq!(set("default"), [json!({ "id": id, "retention": null })]);
        assert_eq!(
            s.run(Command::TenantList).expect("listed")[0]["retention"],
            Value::Null
        );
    }

    #[test]
    fn an_unknown_or_malformed_tenant_is_not_found() {
        let s = setup();
        for id in [TenantId::generate().to_string(), "not-an-id".to_owned()] {
            assert_eq!(
                tenant_error(s.run(Command::TenantSetRetention {
                    id: id.clone(),
                    retention: "7y".to_owned()
                })),
                TenantError::NotFound
            );
            assert_eq!(
                tenant_error(s.run(Command::TenantRemove {
                    id: id.clone(),
                    force: true,
                    override_holds: false
                })),
                TenantError::NotFound
            );
            assert_eq!(
                tenant_error(s.run(Command::KeyCreate {
                    owner: Owner::Tenant {
                        id: id.clone(),
                        scopes: vec!["read".to_owned()]
                    },
                    label: "l".to_owned(),
                    life: KeyLife::Default,
                })),
                TenantError::NotFound
            );
            assert_eq!(
                tenant_error(s.run(Command::KeyList {
                    tenant: Some(id.clone())
                })),
                TenantError::NotFound
            );
            assert_eq!(
                tenant_error(s.run(Command::AuditExport {
                    log: Log::Tenant(id.clone()),
                    since: None
                })),
                TenantError::NotFound
            );
        }
    }

    /// Without --force a tenant holding vCons stays; --force does not pass
    /// a legal hold; --override-holds does.
    #[test]
    fn remove_goes_as_far_as_its_flags_allow() {
        let s = setup();
        let empty = s.create("empty");
        let full = s.create("full");
        {
            let tenants = s.tenants();
            let store = tenants
                .store(TenantId::parse(&full).expect("id"))
                .expect("store");
            let caller = Caller { key: KeyId([1; 8]) };
            let (found, body) = vcon(UUID, "kept");
            store.create(&caller, &found, &body).expect("stored");
            store.set_hold(&caller, found.uuid, true).expect("held");
        }
        let remove = |id: &str, force, override_holds| {
            s.run(Command::TenantRemove {
                id: id.to_owned(),
                force,
                override_holds,
            })
        };
        assert_eq!(
            remove(&empty, false, false).expect("removed"),
            Vec::<Value>::new()
        );
        assert_eq!(
            tenant_error(remove(&full, false, false)),
            TenantError::HoldsVcons
        );
        assert_eq!(
            tenant_error(remove(&full, true, false)),
            TenantError::UnderHold
        );
        assert_eq!(
            remove(&full, true, true).expect("removed"),
            Vec::<Value>::new()
        );
        assert_eq!(
            s.run(Command::TenantList).expect("listed"),
            Vec::<Value>::new()
        );
        assert_eq!(s.tenant_dirs(), Vec::<String>::new());
    }

    #[test]
    fn keys_are_made_listed_and_revoked() {
        let s = setup();
        let tenant = s.create("acme");
        let made = s
            .run(Command::KeyCreate {
                owner: Owner::Tenant {
                    id: tenant.clone(),
                    scopes: vec!["write".to_owned(), "read".to_owned()],
                },
                label: "producer".to_owned(),
                life: KeyLife::Default,
            })
            .expect("made");
        let id = made[0]["id"].as_str().expect("id").to_owned();
        let token = made[0]["token"].as_str().expect("token").to_owned();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].as_object().expect("object").len(), 3);
        assert!(token.starts_with(&format!("vcs_{id}_")), "{token}");
        let operator = s
            .run(Command::KeyCreate {
                owner: Owner::Operator,
                label: "ops".to_owned(),
                life: KeyLife::Default,
            })
            .expect("made")[0]["id"]
            .as_str()
            .expect("id")
            .to_owned();

        let listed = s.run(Command::KeyList { tenant: None }).expect("listed");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["id"], id.as_str());
        assert_eq!(listed[0]["tenant"], tenant.as_str());
        assert_eq!(listed[0]["scopes"], json!(["write", "read"]));
        assert_eq!(listed[0]["label"], "producer");
        assert_eq!(listed[0]["revoked_at"], Value::Null);
        assert!(
            listed[0]["created_at"]
                .as_str()
                .is_some_and(|t| t.ends_with('Z'))
        );
        assert!(
            listed.iter().all(|k| k.get("token").is_none()),
            "never a token"
        );
        assert_eq!(listed[1]["id"], operator.as_str());
        assert_eq!(listed[1]["tenant"], Value::Null);
        assert_eq!(listed[1]["scopes"], json!(["admin"]));

        let own = s
            .run(Command::KeyList {
                tenant: Some(tenant.clone()),
            })
            .expect("listed");
        assert_eq!(own, listed[..1]);

        assert_eq!(
            s.run(Command::KeyRevoke { id: id.clone() })
                .expect("revoked"),
            Vec::<Value>::new()
        );
        let listed = s.run(Command::KeyList { tenant: None }).expect("listed");
        assert!(listed[0]["revoked_at"].as_str().is_some(), "{listed:?}");
        assert_eq!(listed[1]["revoked_at"], Value::Null);
    }

    #[test]
    fn bad_scopes_and_unknown_keys_are_refused() {
        let s = setup();
        let tenant = s.create("acme");
        for scopes in [
            vec!["superuser"],
            vec!["read", "superuser"],
            vec!["read", "read"],
            vec!["admin"],
            vec![],
        ] {
            let error = s.run(Command::KeyCreate {
                owner: Owner::Tenant {
                    id: tenant.clone(),
                    scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
                },
                label: "l".to_owned(),
                life: KeyLife::Default,
            });
            assert_eq!(tenant_error(error), TenantError::BadScopes, "{scopes:?}");
        }
        assert_eq!(
            s.run(Command::KeyList { tenant: None }).expect("listed"),
            Vec::<Value>::new()
        );
        for id in ["0123456789abcdef".to_owned(), "nope".to_owned()] {
            assert!(
                matches!(
                    s.run(Command::KeyRevoke { id }),
                    Err(CommandError::UnknownKey)
                ),
                "unknown"
            );
        }
    }

    #[test]
    fn audit_export_writes_either_log_from_a_sequence_number() {
        let s = setup();
        let tenant = s.create("acme");
        s.run(Command::TenantList).expect("listed");
        s.run(Command::KeyCreate {
            owner: Owner::Tenant {
                id: tenant.clone(),
                scopes: vec!["read".to_owned()],
            },
            label: "l".to_owned(),
            life: KeyLife::Default,
        })
        .expect("made");

        let admin = s
            .run(Command::AuditExport {
                log: Log::Admin,
                since: None,
            })
            .expect("exported");
        let actions: Vec<&str> = admin
            .iter()
            .map(|r| r["action"].as_str().expect("action"))
            .collect();
        assert_eq!(actions, ["tenant_create", "tenant_list", "key_create"]);
        assert!(admin.iter().all(|r| r["actor"] == "cli"));
        let since = admin[0]["seq"].as_i64().expect("seq");
        let later = s
            .run(Command::AuditExport {
                log: Log::Admin,
                since: Some(since),
            })
            .expect("exported");
        assert_eq!(later, admin[1..]);

        let own = s
            .run(Command::AuditExport {
                log: Log::Tenant(tenant),
                since: None,
            })
            .expect("exported");
        let actions: Vec<&str> = own
            .iter()
            .map(|r| r["action"].as_str().expect("action"))
            .collect();
        assert_eq!(actions, ["key_create"]);
    }

    #[test]
    fn rekey_tenant_rotates_the_key_and_prints_nothing() {
        let s = setup();
        let id = s.create("acme");
        let tenant = TenantId::parse(&id).expect("id");
        let before = s.tenants().tenant_key_id_for_test(tenant);
        assert_eq!(
            s.run(Command::RekeyTenant { id: id.clone() })
                .expect("rekeyed"),
            Vec::<Value>::new()
        );
        assert_ne!(s.tenants().tenant_key_id_for_test(tenant), before);
        assert_eq!(
            tenant_error(s.run(Command::RekeyTenant {
                id: TenantId::generate().to_string()
            })),
            TenantError::NotFound
        );
        let server = datadir::lock(&s.config.data_dir, Sharing::Serve).expect("a server");
        assert!(matches!(
            s.run(Command::RekeyTenant { id }),
            Err(CommandError::DataDir(LockError::InUse(Sharing::Alone)))
        ));
        drop(server);
    }

    #[test]
    fn rekey_master_moves_the_data_to_the_new_master_key() {
        let mut s = setup();
        s.create("acme");
        let new_file = s.config.master_key.with_file_name("new.key");
        std::fs::write(&new_file, [0x5c; 32]).expect("new key");
        chmod(&new_file, 0o600);
        assert!(matches!(
            s.run(Command::RekeyMaster {
                new_master: s.config.master_key.clone()
            }),
            Err(CommandError::SameMasterKey)
        ));
        let server = datadir::lock(&s.config.data_dir, Sharing::Serve).expect("a server");
        assert!(matches!(
            s.run(Command::RekeyMaster {
                new_master: new_file.clone()
            }),
            Err(CommandError::DataDir(LockError::InUse(Sharing::Alone)))
        ));
        drop(server);
        let out = s
            .run(Command::RekeyMaster {
                new_master: new_file.clone(),
            })
            .expect("rotated");
        let new_id = Key::load_master(&new_file).expect("key").id().to_string();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["master_key_id"], new_id.as_str());
        assert!(
            out[0]["next"]
                .as_str()
                .is_some_and(|n| n.contains("destroy"))
        );
        assert!(matches!(
            s.run(Command::TenantList),
            Err(CommandError::WrongMasterKey)
        ));
        s.config.master_key = new_file;
        assert_eq!(s.run(Command::TenantList).expect("listed").len(), 1);
    }

    mod check {
        use time::macros::datetime;

        use super::*;
        use crate::ids::VersionId;
        use crate::store::{Examination, StoreError};

        const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);
        const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

        fn sound() -> Examination {
            Examination {
                versions: 3,
                packed: 0,
                damaged: vec![],
                leftovers: vec!["a.tmp".to_owned()],
                due: 0,
                oldest_due_since: None,
                purges_since_key: 4,
                oldest_purge_since_key: Some(datetime!(2026-10-01 00:00 UTC)),
            }
        }

        fn tenant(store: Result<Examination, Unexamined>, stopped: bool) -> TenantExamination {
            TenantExamination {
                id: TenantId([1; 16]),
                name: "acme".to_owned(),
                rotation_stopped: stopped,
                store,
            }
        }

        #[test]
        fn leftovers_and_work_waiting_for_rotation_are_not_problems() {
            assert_eq!(
                problems(&tenant(Ok(sound()), false), HOUR, NOW, true),
                Vec::<String>::new()
            );
        }

        #[test]
        fn each_problem_is_named() {
            let damaged = Examination {
                damaged: vec![VersionId([2; 16]), VersionId([3; 16])],
                ..sound()
            };
            assert_eq!(
                problems(&tenant(Ok(damaged), false), HOUR, NOW, true),
                [
                    "version 02020202020202020202020202020202 is missing or does not open",
                    "version 03030303030303030303030303030303 is missing or does not open",
                ],
                "one line per damaged version, in its order"
            );
            assert_eq!(
                problems(&tenant(Ok(sound()), true), HOUR, NOW, true),
                ["a key rotation was stopped part way; run rekey tenant again"],
                "a stopped rotation is named with the command that finishes it"
            );
            assert_eq!(
                problems(
                    &tenant(
                        Err(Unexamined::Open(OpenFailure::Refused(TenantError::Io))),
                        false
                    ),
                    HOUR,
                    NOW,
                    true,
                ),
                ["its catalog cannot be opened: Io"],
                "a refused catalog names the refusal, not a key"
            );
        }

        /// Only a catalog its key does not open is a key problem; one the
        /// process may not open is named with its path and the system's
        /// error.
        #[test]
        fn a_key_problem_and_an_access_problem_are_told_apart() {
            let key = problems(
                &tenant(Err(Unexamined::Open(OpenFailure::WrongKey)), false),
                HOUR,
                NOW,
                true,
            );
            assert_eq!(key, ["its catalog does not open under its tenant key"]);
            let access = OpenFailure::Access {
                path: std::path::PathBuf::from("/srv/data/tenants/01/catalog.sqlite"),
                error: std::io::Error::from_raw_os_error(libc::EACCES),
            };
            assert_eq!(
                problems(
                    &tenant(Err(Unexamined::Open(access)), false),
                    HOUR,
                    NOW,
                    true
                ),
                [
                    "its catalog cannot be opened: /srv/data/tenants/01/catalog.sqlite: Permission denied (os error 13)"
                ]
            );
        }

        /// Packed versions with no `[storage.parquet]` in the
        /// configuration are a problem, named with the setting; with the
        /// table, or with nothing packed, they are not.
        #[test]
        fn packed_versions_without_storage_parquet_are_a_problem() {
            let packed = Examination {
                packed: 2,
                ..sound()
            };
            assert_eq!(
                problems(&tenant(Ok(packed.clone()), false), HOUR, NOW, false),
                [
                    "2 version(s) are packed into Parquet files, and the configuration has no [storage.parquet]: serve refuses to start until the table is put back"
                ],
                "without the table"
            );
            assert_eq!(
                problems(&tenant(Ok(packed), false), HOUR, NOW, true),
                Vec::<String>::new(),
                "with the table"
            );
            assert_eq!(
                problems(&tenant(Ok(sound()), false), HOUR, NOW, false),
                Vec::<String>::new(),
                "nothing packed"
            );
        }

        /// A Store that opened and whose objects the storage did not read
        /// is named as that, with the error's kind and the system's error
        /// number when there is one, and never as a catalog that does not
        /// open; any other failed examination says so with its error.
        #[test]
        fn objects_that_cannot_be_read_are_told_apart_from_a_catalog() {
            let unread = |kind, os_error| {
                problems(
                    &tenant(
                        Err(Unexamined::Examine(ExamineError::Objects {
                            kind,
                            os_error,
                        })),
                        false,
                    ),
                    HOUR,
                    NOW,
                    true,
                )
            };
            assert_eq!(
                unread(std::io::ErrorKind::PermissionDenied, Some(libc::EACCES)),
                ["its objects cannot be read: permission denied (os error 13)"],
                "a file the process may not read"
            );
            assert_eq!(
                unread(std::io::ErrorKind::TimedOut, None),
                ["its objects cannot be read: timed out"],
                "a service's failure has a kind and no system error number"
            );
            assert_eq!(
                problems(
                    &tenant(
                        Err(Unexamined::Examine(ExamineError::Failed(StoreError::Io))),
                        false
                    ),
                    HOUR,
                    NOW,
                    true,
                ),
                ["it could not be examined: Io"],
                "any other failure, with its error"
            );
        }

        #[test]
        fn a_sweep_is_stalled_only_once_a_version_waited_longer_than_the_interval() {
            let waited = |since| Examination {
                due: 1,
                oldest_due_since: Some(since),
                ..sound()
            };
            let ok = waited(datetime!(2026-10-25 11:00 UTC));
            assert_eq!(
                problems(&tenant(Ok(ok), false), HOUR, NOW, true),
                Vec::<String>::new()
            );
            let stalled = waited(datetime!(2026-10-25 10:59:59 UTC));
            assert_eq!(
                problems(&tenant(Ok(stalled), false), HOUR, NOW, true),
                [
                    "1 version(s) due for purge, the oldest longer than purge_interval: the sweep is not keeping up"
                ],
                "one second past purge_interval is a stalled sweep"
            );
        }

        #[test]
        fn check_reports_each_tenant_and_a_summary_and_exits_1_on_a_problem() {
            let s = setup();
            let id = s.create("acme");
            let out = s.run(Command::Check).expect("nothing wrong");
            assert_eq!(out.len(), 2);
            assert_eq!(out[0]["tenant"], id.as_str());
            assert_eq!(out[0]["problems"], json!([]));
            assert_eq!(out[1]["tenants"], 1);
            assert_eq!(out[1]["problems"], 0);

            let tenant = TenantId::parse(&id).expect("id");
            let version = {
                let tenants = s.tenants();
                let store = tenants.store(tenant).expect("store");
                let (found, body) = vcon(UUID, "kept");
                let Ok(crate::store::Created::New(v)) =
                    store.create(&Caller { key: KeyId([1; 8]) }, &found, &body)
                else {
                    panic!("stored")
                };
                v
            };
            let objects = s.config.data_dir.join("tenants").join(&id).join("objects");
            std::fs::remove_file(objects.join(version.to_string())).expect("remove");
            std::fs::write(objects.join(format!("{version}.tmp")), b"x").expect("leftover");
            let mut printed = Vec::new();
            let found = run(&s.config, &Command::Check, &mut printed);
            assert!(
                matches!(found, Err(CommandError::CheckFound(1))),
                "{found:?}"
            );
            let lines: Vec<Value> = String::from_utf8(printed)
                .expect("text")
                .lines()
                .map(|l| serde_json::from_str(l).expect("json"))
                .collect();
            assert_eq!(lines[0]["damaged"], json!([version.to_string()]));
            assert_eq!(lines[0]["leftovers"], json!([format!("{version}.tmp")]));
            assert_eq!(lines[1]["problems"], 1);
            assert!(
                objects.join(format!("{version}.tmp")).exists(),
                "check reports leftovers; it does not remove them"
            );
            let server = datadir::lock(&s.config.data_dir, Sharing::Serve).expect("a server");
            assert!(matches!(
                s.run(Command::Check),
                Err(CommandError::DataDir(LockError::InUse(Sharing::Alone)))
            ));
            drop(server);
        }
    }

    /// The commands that run alone are refused while a server holds the
    /// data directory; the others run beside it.
    #[test]
    fn beside_a_server_only_the_beside_commands_run() {
        let s = setup();
        let tenant = s.create("acme");
        let server = datadir::lock(&s.config.data_dir, Sharing::Serve).expect("a server");
        for command in [
            Command::TenantCreate {
                name: "beta".to_owned(),
                retention: None,
            },
            Command::TenantRemove {
                id: tenant.clone(),
                force: true,
                override_holds: false,
            },
        ] {
            assert!(
                matches!(
                    s.run(command.clone()),
                    Err(CommandError::DataDir(LockError::InUse(Sharing::Alone)))
                ),
                "{command:?}"
            );
        }
        s.run(Command::TenantList).expect("beside");
        s.run(Command::TenantSetRetention {
            id: tenant.clone(),
            retention: "1y".to_owned(),
        })
        .expect("beside");
        s.run(Command::KeyList { tenant: None }).expect("beside");
        let made = s
            .run(Command::KeyCreate {
                owner: Owner::Operator,
                label: "l".to_owned(),
                life: KeyLife::Default,
            })
            .expect("beside");
        s.run(Command::KeyRevoke {
            id: made[0]["id"].as_str().expect("an id").to_owned(),
        })
        .expect("beside");
        s.run(Command::AuditExport {
            log: Log::Admin,
            since: None,
        })
        .expect("beside");
        s.run(Command::Backup {
            to: s._dir.path().join("backup"),
        })
        .expect("beside");
        drop(server);
        assert_eq!(s.run(Command::TenantList).expect("listed").len(), 1);
    }

    /// A command beside a server opens without recovery, so a create the
    /// server has in progress stays; one that runs alone recovers.
    #[test]
    fn only_a_command_that_runs_alone_recovers() {
        let s = setup();
        s.create("acme");
        let in_progress = TenantId::generate().to_string();
        std::fs::create_dir(s.config.data_dir.join("tenants").join(&in_progress))
            .expect("a directory with no row");
        s.run(Command::TenantList).expect("listed");
        s.run(Command::KeyList { tenant: None }).expect("listed");
        assert!(
            s.tenant_dirs().contains(&in_progress),
            "left by beside commands"
        );
        s.create("beta");
        assert!(
            !s.tenant_dirs().contains(&in_progress),
            "removed by an alone command"
        );
    }

    #[test]
    fn a_master_key_other_users_may_read_stops_every_command() {
        let s = setup();
        s.create("acme");
        chmod(&s.config.master_key, 0o644);
        let error = s.run(Command::TenantList).expect_err("refused");
        let path = s.config.master_key.display();
        assert_eq!(
            error.to_string(),
            format!(
                "the master key cannot be read: master_key {path}: other users may read or \
                 change the file (mode 0644); run `chmod 600 {path}`, or `chmod 640 {path}` for \
                 a group that only reads it"
            ),
            "the command stops with the rule's text, naming the key, the file and the fix"
        );
    }

    #[test]
    fn rekey_master_refuses_a_new_key_other_users_may_read_under_its_option() {
        let s = setup();
        s.create("acme");
        let new_file = s.config.master_key.with_file_name("new.key");
        std::fs::write(&new_file, [0x5c; 32]).expect("new key");
        chmod(&new_file, 0o644);
        let error = s
            .run(Command::RekeyMaster {
                new_master: new_file.clone(),
            })
            .expect_err("refused");
        assert!(
            error
                .to_string()
                .contains(&format!("--new-master {}: other users", new_file.display())),
            "{error}"
        );
    }

    #[cfg(feature = "s3")]
    #[test]
    fn an_export_secret_other_users_may_read_is_refused_under_export_s3() {
        let s = setup();
        let secret = s.config.master_key.with_file_name("export.secret");
        std::fs::write(&secret, "k3y-0123\n").expect("secret");
        chmod(&secret, 0o644);
        let mut config = s.config.clone();
        config.export = crate::config::parse(&format!(
            "listen = \"127.0.0.1:1\"\ndata_dir = \"/d\"\nmaster_key = \"/m\"\n\
             [tls]\ncert = \"c\"\nkey = \"k\"\n\
             [export.s3]\nendpoint = \"http://127.0.0.1:9\"\nregion = \"eu-west-3\"\n\
             access_key_id = \"id\"\nsecret_access_key_file = {secret:?}\nallow_http = true\n"
        ))
        .expect("a configuration")
        .export;
        let error = s3_target(&config, "vcon", "").err().expect("refused");
        let text = error.to_string();
        assert!(
            text.contains(&format!(
                "export.s3.secret_access_key_file {}: other users",
                secret.display()
            )),
            "{text}"
        );
        assert!(!text.contains("k3y"), "{text}");
    }

    /// A user or membership command that fails in the registry says what
    /// every other command says for the same failure.
    #[test]
    fn a_user_commands_registry_failure_is_the_registrys_error() {
        for (failure, words) in [
            (
                TenantError::AuditFailed,
                "the audit record could not be written",
            ),
            (
                TenantError::WrongKey,
                "is not the one server.sqlite was made under",
            ),
        ] {
            let error = CommandError::from(UserError::Registry(failure));
            assert!(
                matches!(error, CommandError::Tenant(e) if e == failure),
                "{failure:?} is carried as the registry's error: {error:?}"
            );
            assert!(error.to_string().contains(words), "{error}");
        }
        assert!(
            matches!(
                CommandError::from(UserError::NoUser),
                CommandError::User(UserError::NoUser)
            ),
            "a refusal stays a user error"
        );
    }

    #[test]
    fn a_wrong_master_key_is_named() {
        let s = setup();
        s.create("acme");
        std::fs::write(&s.config.master_key, [0x5b; 32]).expect("another key");
        let error = s.run(Command::TenantList).expect_err("refused");
        assert!(matches!(error, CommandError::WrongMasterKey), "{error:?}");
        assert!(error.to_string().contains("master key"), "{error}");
        std::fs::write(&s.config.master_key, [0x5a; 31]).expect("short key");
        let error = s.run(Command::TenantList).expect_err("refused");
        assert!(matches!(error, CommandError::MasterKey(_)), "{error:?}");
    }

    #[test]
    fn backup_prints_what_it_copied_and_check_finds_the_copy_whole() {
        let s = setup();
        s.create("acme");
        s.create("globex");
        let to = s._dir.path().join("backup");
        let out = s.run(Command::Backup { to: to.clone() }).expect("backs up");
        assert_eq!(
            out,
            [serde_json::json!({ "tenants": 2, "versions": 0, "tries": 1 })]
        );
        let copy = Setup {
            _dir: tempfile::tempdir().expect("temp dir"),
            config: Config {
                data_dir: to,
                ..s.config.clone()
            },
        };
        let checked = copy.run(Command::Check).expect("check finds no problem");
        assert_eq!(checked.last().expect("a summary")["problems"], 0);
        let names: Vec<_> = copy
            .run(Command::TenantList)
            .expect("lists")
            .iter()
            .map(|t| t["name"].clone())
            .collect();
        assert_eq!(names, ["acme", "globex"]);
    }

    #[test]
    fn a_backup_path_already_there_is_refused() {
        let s = setup();
        let to = s._dir.path().join("backup");
        std::fs::create_dir(&to).expect("dir");
        let error = s.run(Command::Backup { to }).expect_err("refused");
        assert!(
            matches!(error, CommandError::Backup(BackupError::Exists)),
            "{error:?}"
        );
    }

    /// Words the message of `error` must hold to say what went wrong. The
    /// matches have no catch-all arm, so a new error fails to compile here
    /// until its words are given.
    fn says(error: &CommandError) -> &'static str {
        match error {
            CommandError::DataDir(_) => "the data directory is in use",
            CommandError::MasterKey(_) => "the master key cannot be read",
            // The step, the file and the error under it, all three.
            CommandError::Open(_) => "opening /srv/data/server.sqlite failed: disk I/O error",
            CommandError::WrongMasterKey => "is not the one server.sqlite was made under",
            CommandError::BadPeriod(_) => "\"7w\" is not a period",
            CommandError::UnknownKey => "no key has this id",
            CommandError::User(e) => match e {
                UserError::CliOnly => "command line",
                UserError::BadLabel => "a user label is",
                UserError::BadIdentity => "an issuer is an https:// URL",
                UserError::IdentityTaken => "an active user has this issuer and subject",
                UserError::NoUser => "no user has this id",
                UserError::Deactivated => "the user is deactivated",
                UserError::NoTenant => "no tenant has this id",
                UserError::Closing => "being removed",
                UserError::BadRoles => "reader, producer, records, legal, auditor and tenant-admin",
                UserError::OverCap => "outside the tenant's role cap",
                UserError::BadCap => "a role cap is one or more of",
                UserError::NoMembership => "the user has no such membership",
                UserError::Registry(_) => "could not be read or written",
            },
            CommandError::Output(_) => "the output could not be written",
            CommandError::SameMasterKey => "already in use",
            CommandError::CheckFound(_) => "check found 2 problem(s)",
            CommandError::Storage(_) => "the storage cannot be used: bucket \"vcon\"",
            CommandError::BadLevel(_) => "\"most\" is not a metadata level",
            CommandError::BadTime(_) => "\"yesterday\" is not an RFC 3339 time",
            CommandError::BadTarget(_) => "\"s3://\" is not a directory or an s3://bucket/prefix",
            CommandError::NoS3 => "[export.s3] or [storage.s3]",
            CommandError::Export(_) => "another export wrote",
            CommandError::Move { .. } => "running storage move-objects again moves the rest",
            CommandError::Backup(e) => match e {
                BackupError::Exists => "already exists",
                BackupError::KeptChanging => "try again",
                BackupError::Io => "could not be written",
                BackupError::AuditFailed => "audit record",
            },
            CommandError::Tenant(e) => match e {
                TenantError::NotFound => "no tenant",
                TenantError::NameTaken => "another tenant has this name",
                TenantError::BadName => "a tenant name is",
                TenantError::HoldsVcons => "--force",
                TenantError::UnderHold => "--override-holds",
                TenantError::LimitReached => "rekey master",
                TenantError::AuditFailed => "audit record",
                TenantError::Closing => "being removed",
                TenantError::CliOnly => "command line",
                TenantError::BadScopes => "scopes",
                TenantError::BadExpiry => "1 to 365 days",
                TenantError::BadSetting(_) => "setting",
                // Display folds a registry made under another master key
                // into WrongMasterKey's message.
                TenantError::WrongKey => "is not the one server.sqlite was made under",
                TenantError::Io => "could not be read or written",
            },
        }
    }

    /// With the objects kept in the data directory there is no bucket to
    /// move them to: the command is refused, naming the setting.
    #[test]
    fn storage_move_objects_needs_a_bucket() {
        let s = setup();
        s.create("acme");
        let error = s.run(Command::StorageMoveObjects).expect_err("refused");
        assert!(
            error.to_string().contains("storage.objects = \"s3\""),
            "names the setting: {error}"
        );
    }

    /// Against the test service: a data directory's objects move to the
    /// bucket, byte for byte; the vCon then reads back from the bucket, no
    /// object file is left, and a second run moves nothing.
    #[cfg(feature = "backend-tests")]
    #[test]
    fn storage_move_objects_moves_each_tenants_objects_to_the_bucket() {
        use crate::backend::Objects;
        let s = setup();
        let id = s.create("acme");
        store_one(&s, &id);
        let tenant = TenantId::parse(&id).expect("an id");
        let local = s.config.data_dir.join("tenants").join(&id).join("objects");
        let files = || std::fs::read_dir(&local).expect("listed").count();
        assert_eq!(files(), 1, "one object file");
        let (bucket, mut settings) = crate::backend::s3::remote::bucket(false);
        let secret = crate::backend::s3::remote::secret_file();
        settings.secret_access_key_file = Some(secret.1.clone());
        let mut config = s.config.clone();
        config.storage = crate::config::Storage {
            objects: crate::config::ObjectStore::S3(settings),
            parquet: None,
        };
        let moving = |config: &Config| -> Vec<Value> {
            let mut out = Vec::new();
            run(config, &Command::StorageMoveObjects, &mut out).expect("moved");
            String::from_utf8(out)
                .expect("text")
                .lines()
                .map(|l| serde_json::from_str(l).expect("a JSON line"))
                .collect()
        };
        assert_eq!(
            moving(&config),
            [
                json!({"tenant": id, "moved": 1, "missing": 0}),
                json!({"moved": 1})
            ],
            "one object moved, and nothing missing"
        );
        assert_eq!(files(), 0, "no object file is left");
        assert_eq!(
            bucket.tenant(tenant).list().expect("listed").len(),
            1,
            "the object is in the bucket"
        );
        let tenants = Tenants::open_beside_in(
            &config.data_dir,
            Key::load_master(&config.master_key).expect("master"),
            Arc::new(OsFs),
            Arc::new(SystemClock),
            crate::backend::ObjectPlace::S3(bucket.clone()),
        )
        .expect("opens");
        let (_, body) = vcon(UUID, "exported");
        assert_eq!(
            tenants
                .store(tenant)
                .expect("its store")
                .get(&Caller { key: KeyId([7; 8]) }, UUID.parse().expect("uuid"))
                .expect("reads")
                .body,
            body,
            "the vCon reads back from the bucket"
        );
        drop(tenants);
        assert_eq!(
            moving(&config),
            [
                json!({"tenant": id, "moved": 0, "missing": 0}),
                json!({"moved": 0})
            ],
            "a second run moves nothing"
        );
    }

    /// One vCon stored in the tenant `id`.
    fn store_one(s: &Setup, id: &str) {
        let store = s
            .tenants()
            .store(TenantId::parse(id).expect("an id"))
            .expect("the store");
        let (found, body) = vcon(UUID, "exported");
        store
            .create(&Caller { key: KeyId([7; 8]) }, &found, &body)
            .expect("stored");
    }

    #[test]
    fn tenant_set_metadata_prints_the_level_and_list_shows_it() {
        let s = setup();
        let id = s.create("acme");
        let out = s
            .run(Command::TenantSetMetadata {
                id: id.clone(),
                metadata: "counts".to_owned(),
            })
            .expect("set");
        assert_eq!(
            out,
            [json!({ "id": id, "metadata": "counts" })],
            "the command prints the tenant and its level"
        );
        assert_eq!(
            s.run(Command::TenantList).expect("listed")[0]["metadata"],
            "counts",
            "the list shows the level"
        );
        assert!(
            matches!(
                s.run(Command::TenantSetMetadata {
                    id: id.clone(),
                    metadata: "most".to_owned(),
                }),
                Err(CommandError::BadLevel(text)) if text == "most"
            ),
            "a level that is not one is refused, naming it"
        );
        assert_eq!(
            tenant_error(s.run(Command::TenantSetMetadata {
                id: TenantId::generate().to_string(),
                metadata: "full".to_owned(),
            })),
            TenantError::NotFound,
            "a tenant id no tenant has is not found"
        );
    }

    /// The export runs beside a server, at the tenant's level, and prints
    /// one line; it is refused beside a command that runs alone.
    #[test]
    fn export_parquet_runs_beside_a_server_and_prints_what_it_wrote() {
        let s = setup();
        let id = s.create("acme");
        store_one(&s, &id);
        s.run(Command::TenantSetMetadata {
            id: id.clone(),
            metadata: "counts".to_owned(),
        })
        .expect("set");
        let to = s._dir.path().join("export");
        let export = Command::ExportParquet {
            tenant: id.clone(),
            to: to.display().to_string(),
            since: Some("2000-01-01T00:00:00Z".to_owned()),
            until: None,
        };
        let server = datadir::lock(&s.config.data_dir, Sharing::Serve).expect("a server");
        let out = s.run(export.clone()).expect("exports beside a server");
        drop(server);
        assert_eq!(out.len(), 1, "{out:?}");
        let line = &out[0];
        let manifest = format!("tenant={id}/manifest/00000000000000000001.json");
        assert_eq!(line["tenant"], id, "the line names the tenant");
        assert_eq!(
            line["metadata"], "counts",
            "the export is at the tenant's level"
        );
        assert_eq!(
            line["to"],
            to.display().to_string(),
            "the line names the target"
        );
        assert_eq!(line["manifest"], manifest, "the line names the manifest");
        assert_eq!(line["seq"], 1, "the first export to the target");
        assert_eq!(line["files"], 1, "one store date, one file");
        assert_eq!(line["rows"], 1, "the one vCon stored");
        assert_eq!(line["skipped"], 0, "nothing was purged during the run");
        assert_eq!(line["unread"], 0, "every body was read");
        let m: Value =
            serde_json::from_slice(&std::fs::read(to.join(&manifest)).expect("the manifest"))
                .expect("JSON");
        assert_eq!(m["bytes"], line["bytes"], "the bytes the manifest names");
        assert_eq!(
            m["since"], "2000-01-01T00:00:00Z",
            "the manifest records --since"
        );
        let alone = datadir::lock(&s.config.data_dir, Sharing::Alone).expect("alone");
        assert!(
            matches!(
                s.run(export),
                Err(CommandError::DataDir(LockError::InUse(Sharing::Beside)))
            ),
            "an export does not run beside a command that runs alone"
        );
        drop(alone);
    }

    #[test]
    fn export_parquet_refuses_a_bad_time_target_or_tenant() {
        let s = setup();
        let id = s.create("acme");
        let export = |to: &str, since: Option<&str>, until: Option<&str>| Command::ExportParquet {
            tenant: id.clone(),
            to: to.to_owned(),
            since: since.map(str::to_owned),
            until: until.map(str::to_owned),
        };
        let dir = s._dir.path().join("out").display().to_string();
        assert!(
            matches!(
                s.run(export(&dir, Some("yesterday"), None)),
                Err(CommandError::BadTime(t)) if t == "yesterday"
            ),
            "a word that is not an RFC 3339 time is refused, naming it"
        );
        assert!(
            matches!(
                s.run(export(&dir, None, Some("2026-13-01T00:00:00Z"))),
                Err(CommandError::BadTime(_))
            ),
            "a month 13 is not a time"
        );
        assert!(
            matches!(
                s.run(export("s3://", None, None)),
                Err(CommandError::BadTarget(t)) if t == "s3://"
            ),
            "an s3:// URL with no bucket is refused, naming it"
        );
        assert!(
            matches!(
                s.run(export("s3://reports/vcon", None, None)),
                Err(CommandError::NoS3)
            ),
            "no S3 service is configured"
        );
        assert_eq!(
            tenant_error(s.run(Command::ExportParquet {
                tenant: TenantId::generate().to_string(),
                to: dir.clone(),
                since: None,
                until: None,
            })),
            TenantError::NotFound,
            "an unknown tenant is not found"
        );
        assert!(
            !std::path::Path::new(&dir).exists(),
            "a refused export writes nothing"
        );
    }

    #[test]
    fn each_error_says_what_went_wrong() {
        let tenant = [
            TenantError::NotFound,
            TenantError::NameTaken,
            TenantError::BadName,
            TenantError::HoldsVcons,
            TenantError::UnderHold,
            TenantError::LimitReached,
            TenantError::AuditFailed,
            TenantError::Closing,
            TenantError::CliOnly,
            TenantError::BadScopes,
            TenantError::BadExpiry,
            TenantError::BadSetting(crate::settings::SettingError::UnknownName),
            TenantError::WrongKey,
            TenantError::Io,
        ]
        .map(CommandError::Tenant);
        let backup = [
            BackupError::Exists,
            BackupError::KeptChanging,
            BackupError::Io,
            BackupError::AuditFailed,
        ]
        .map(CommandError::Backup);
        let rest = [
            CommandError::DataDir(LockError::InUse(Sharing::Alone)),
            CommandError::MasterKey(crate::envelope::KeyError::WrongSize),
            CommandError::Open(RegistryError::failed(
                tenants::OpenStep::OpenCatalog,
                std::path::PathBuf::from("/srv/data/server.sqlite"),
                io::Error::other("disk I/O error"),
            )),
            CommandError::WrongMasterKey,
            CommandError::BadPeriod("7w".to_owned()),
            CommandError::UnknownKey,
            CommandError::Output(io::Error::other("closed")),
            CommandError::SameMasterKey,
            CommandError::CheckFound(2),
            CommandError::Storage(StorageError(
                "bucket \"vcon\" cannot be reached or read".to_owned(),
            )),
            CommandError::BadLevel("most".to_owned()),
            CommandError::BadTime("yesterday".to_owned()),
            CommandError::BadTarget("s3://".to_owned()),
            CommandError::NoS3,
            CommandError::Export(crate::export::ExportError::Raced("m".to_owned())),
            CommandError::Move {
                tenant: TenantId([1; 16]),
                why: "the bucket could not be written".to_owned(),
            },
        ];
        let user = [
            UserError::CliOnly,
            UserError::BadLabel,
            UserError::BadIdentity,
            UserError::IdentityTaken,
            UserError::NoUser,
            UserError::Deactivated,
            UserError::NoTenant,
            UserError::Closing,
            UserError::BadRoles,
            UserError::NoMembership,
            UserError::Registry(TenantError::Io),
        ]
        .map(CommandError::User);
        for error in tenant.into_iter().chain(backup).chain(rest).chain(user) {
            let text = error.to_string();
            assert!(text.contains(says(&error)), "{error:?}: {text}");
        }
    }
}

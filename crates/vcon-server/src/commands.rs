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
use crate::clock::{Clock, SystemClock};
use crate::config::Config;
use crate::datadir::{self, LockError, Sharing, Work};
use crate::envelope::{Key, KeyError};
use crate::fs::OsFs;
use crate::ids::{KeyId, TenantId};
use crate::retention::Retention;
use crate::tenants::{
    self, BackupError, OpenFailure, Removal, TenantError, TenantExamination, Tenants,
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
    KeyCreate {
        owner: Owner,
        label: String,
    },
    KeyList {
        tenant: Option<String>,
    },
    KeyRevoke {
        id: String,
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
            Command::KeyCreate { .. } => Work::KeyCreate,
            Command::KeyList { .. } => Work::KeyList,
            Command::KeyRevoke { .. } => Work::KeyRevoke,
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
    /// No key has this id.
    UnknownKey,
    /// The output could not be written.
    Output(io::Error),
    /// The new master key is the one already in use.
    SameMasterKey,
    /// `check` found this many problems; its report says which.
    CheckFound(usize),
    Backup(BackupError),
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CommandError::DataDir(e) => write!(f, "{e}"),
            CommandError::MasterKey(e) => write!(f, "the master key cannot be read: {e:?}"),
            CommandError::WrongMasterKey | CommandError::Tenant(TenantError::WrongKey) => {
                f.write_str("the master key is not the one server.sqlite was made under")
            }
            CommandError::BadPeriod(text) => write!(
                f,
                "{text:?} is not a period such as 7y, 18m or 90d (or, for set-retention, default)"
            ),
            CommandError::UnknownKey => f.write_str("no key has this id"),
            CommandError::Output(e) => write!(f, "the output could not be written: {e}"),
            CommandError::SameMasterKey => {
                f.write_str("the new master key is the one already in use")
            }
            CommandError::CheckFound(n) => write!(f, "check found {n} problem(s); see its report"),
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
                    "a tenant key's scopes are one or more of write, read, delete, legal and audit, each named once"
                }
                TenantError::LimitReached => {
                    "the master key has wrapped all the tenant keys it may; run rekey master"
                }
                TenantError::AuditFailed => "the audit record could not be written",
                TenantError::Closing => "the tenant is being removed",
                TenantError::CliOnly => "only the command line may do this",
                TenantError::WrongKey => unreachable!("matched above"),
                TenantError::Io => "the data could not be read or written",
            }),
        }
    }
}

/// Runs `command` against the data directory `config` names, writing its
/// answer to `out`.
pub fn run(config: &Config, command: &Command, out: &mut dyn Write) -> Result<(), CommandError> {
    let sharing = command.work().sharing();
    let _lock = datadir::lock(&config.data_dir, sharing).map_err(CommandError::DataDir)?;
    let master = Key::load_master(&config.master_key).map_err(CommandError::MasterKey)?;
    // A master rotation renames over server.sqlite, so nothing may hold it
    // open: it runs before the registry is opened.
    if let Command::RekeyMaster { new_master } = command {
        return rekey_master(config, &master, new_master, out);
    }
    // A command beside a running server must not run start-up recovery: it
    // would take the server's work in progress for a crash's leftovers. Nor
    // does `check`, which reports what recovery would remove.
    let open = if sharing == Sharing::Beside || matches!(command, Command::Check) {
        Tenants::open_beside
    } else {
        Tenants::open
    };
    let tenants = open(
        &config.data_dir,
        master,
        Arc::new(OsFs),
        Arc::new(SystemClock),
    )
    .map_err(|e| match e {
        TenantError::WrongKey => CommandError::WrongMasterKey,
        other => CommandError::Tenant(other),
    })?;
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
        Command::KeyCreate { owner, label } => {
            let (tenant, scopes) = match owner {
                Owner::Tenant { id, scopes } => (
                    Some(tenant_id(id)?),
                    Scopes::named(scopes).ok_or(CommandError::Tenant(TenantError::BadScopes))?,
                ),
                Owner::Operator => (None, Scopes::of(&[Scope::Admin])),
            };
            let made = tenants.create_key(who, tenant, scopes, label)?;
            line(
                out,
                &serde_json::json!({ "id": made.id.to_string(), "token": made.token }),
            )
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
        Command::RekeyTenant { id } => Ok(tenants.rekey_tenant(who, tenant_id(id)?)?),
        Command::RekeyMaster { .. } => unreachable!("rotated before the registry is opened"),
        Command::Check => check(config, &tenants, out),
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

/// Rotates the master key and says what the operator does next.
fn rekey_master(
    config: &Config,
    master: &Key,
    new_master: &std::path::Path,
    out: &mut dyn Write,
) -> Result<(), CommandError> {
    let new = Key::load_master(new_master).map_err(CommandError::MasterKey)?;
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
fn check(config: &Config, tenants: &Tenants, out: &mut dyn Write) -> Result<(), CommandError> {
    let now = SystemClock.now();
    let found = tenants.examine(config.retention_default, now)?;
    let time = |at: Option<OffsetDateTime>| at.and_then(|at| at.format(&Rfc3339).ok());
    let mut total = 0;
    for tenant in &found.tenants {
        let named = problems(tenant, config.purge_interval, now);
        total += named.len();
        let store = tenant.store.as_ref().ok();
        line(
            out,
            &serde_json::json!({
                "tenant": tenant.id.to_string(),
                "name": tenant.name,
                "versions": store.map(|s| s.versions),
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
/// a shorter wait is the next sweep's work. Leftover files and work waiting
/// for rotation are reported but are not problems.
pub fn problems(
    tenant: &TenantExamination,
    purge_interval: std::time::Duration,
    now: OffsetDateTime,
) -> Vec<String> {
    let mut found = Vec::new();
    if tenant.rotation_stopped {
        found.push("a key rotation was stopped part way; run rekey tenant again".to_owned());
    }
    match &tenant.store {
        Err(OpenFailure::WrongKey) => {
            found.push("its catalog does not open under its tenant key".to_owned());
        }
        Err(OpenFailure::Access { path, error }) => {
            found.push(format!(
                "its catalog cannot be opened: {}: {error}",
                path.display()
            ));
        }
        Err(OpenFailure::Refused(e)) => {
            found.push(format!("its catalog cannot be opened: {e:?}"));
        }
        Ok(store) => {
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

    fn setup() -> Setup {
        let dir = tempfile::tempdir().expect("temp dir");
        let data = dir.path().join("data");
        std::fs::create_dir(&data).expect("data dir");
        let master = dir.path().join("master.key");
        std::fs::write(&master, [0x5a; 32]).expect("master key file");
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
                json!({ "id": id, "name": "acme", "retention": "18m" }),
                json!({ "id": beta, "name": "beta", "retention": null }),
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
                    label: "l".to_owned()
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
            })
            .expect("made");
        let id = made[0]["id"].as_str().expect("id").to_owned();
        let token = made[0]["token"].as_str().expect("token").to_owned();
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].as_object().expect("object").len(), 2);
        assert!(token.starts_with(&format!("vcs_{id}_")), "{token}");
        let operator = s
            .run(Command::KeyCreate {
                owner: Owner::Operator,
                label: "ops".to_owned(),
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
        use crate::store::Examination;

        const NOW: OffsetDateTime = datetime!(2026-10-25 12:00 UTC);
        const HOUR: std::time::Duration = std::time::Duration::from_secs(3600);

        fn sound() -> Examination {
            Examination {
                versions: 3,
                damaged: vec![],
                leftovers: vec!["a.tmp".to_owned()],
                due: 0,
                oldest_due_since: None,
                purges_since_key: 4,
                oldest_purge_since_key: Some(datetime!(2026-10-01 00:00 UTC)),
            }
        }

        fn tenant(store: Result<Examination, OpenFailure>, stopped: bool) -> TenantExamination {
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
                problems(&tenant(Ok(sound()), false), HOUR, NOW),
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
                problems(&tenant(Ok(damaged), false), HOUR, NOW),
                [
                    "version 02020202020202020202020202020202 is missing or does not open",
                    "version 03030303030303030303030303030303 is missing or does not open",
                ],
                "one line per damaged version, in its order"
            );
            assert_eq!(
                problems(&tenant(Ok(sound()), true), HOUR, NOW),
                ["a key rotation was stopped part way; run rekey tenant again"],
                "a stopped rotation is named with the command that finishes it"
            );
            assert_eq!(
                problems(
                    &tenant(Err(OpenFailure::Refused(TenantError::Io)), false),
                    HOUR,
                    NOW,
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
            let key = problems(&tenant(Err(OpenFailure::WrongKey), false), HOUR, NOW);
            assert_eq!(key, ["its catalog does not open under its tenant key"]);
            let access = OpenFailure::Access {
                path: std::path::PathBuf::from("/srv/data/tenants/01/catalog.sqlite"),
                error: std::io::Error::from_raw_os_error(libc::EACCES),
            };
            assert_eq!(
                problems(&tenant(Err(access), false), HOUR, NOW),
                [
                    "its catalog cannot be opened: /srv/data/tenants/01/catalog.sqlite: Permission denied (os error 13)"
                ]
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
                problems(&tenant(Ok(ok), false), HOUR, NOW),
                Vec::<String>::new()
            );
            let stalled = waited(datetime!(2026-10-25 10:59:59 UTC));
            assert_eq!(
                problems(&tenant(Ok(stalled), false), HOUR, NOW),
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
            CommandError::WrongMasterKey => "is not the one server.sqlite was made under",
            CommandError::BadPeriod(_) => "\"7w\" is not a period",
            CommandError::UnknownKey => "no key has this id",
            CommandError::Output(_) => "the output could not be written",
            CommandError::SameMasterKey => "already in use",
            CommandError::CheckFound(_) => "check found 2 problem(s)",
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
                // Display folds a registry made under another master key
                // into WrongMasterKey's message.
                TenantError::WrongKey => "is not the one server.sqlite was made under",
                TenantError::Io => "could not be read or written",
            },
        }
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
            CommandError::WrongMasterKey,
            CommandError::BadPeriod("7w".to_owned()),
            CommandError::UnknownKey,
            CommandError::Output(io::Error::other("closed")),
            CommandError::SameMasterKey,
            CommandError::CheckFound(2),
        ];
        for error in tenant.into_iter().chain(backup).chain(rest) {
            let text = error.to_string();
            assert!(text.contains(says(&error)), "{error:?}: {text}");
        }
    }
}

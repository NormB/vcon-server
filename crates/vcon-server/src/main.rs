//! The vcon-server command: parses the command line and runs the
//! subcommand from the vcon_server library.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use vcon_server::commands::{self, Log, Owner, Place};
use vcon_server::{config, envelope, serve};

/// A vCon server: stores IETF vCons, encrypted at rest.
#[derive(Parser)]
#[command(name = "vcon-server", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Starts the server.
    Serve {
        /// The configuration file.
        #[arg(long)]
        config: PathBuf,
    },
    /// Makes, lists, changes and removes tenants.
    #[command(subcommand)]
    Tenant(TenantCommand),
    /// Makes, lists and revokes keys.
    #[command(subcommand)]
    Key(KeyCommand),
    /// Makes, lists and deactivates users. Runs on the host only.
    #[command(subcommand)]
    User(UserCommand),
    /// Grants and revokes a user's roles in a tenant or as an operator. Runs on
    /// the host only.
    #[command(subcommand)]
    Membership(MembershipCommand),
    /// Reads the audit logs.
    #[command(subcommand)]
    Audit(AuditCommand),
    /// Rotates keys. Runs only while no server does.
    #[command(subcommand)]
    Rekey(RekeyCommand),
    /// Speaks MCP on stdin and stdout for an AI agent, forwarding each
    /// request to a running server with the one key its client file names.
    Mcp {
        /// The client file: the server's URL, the key file and the CA.
        #[arg(long)]
        config: PathBuf,
    },
    /// Shows a running server's vCons in the terminal, read over its HTTP
    /// API with the one key its client file names. In every build but one
    /// made without the default features.
    Tui {
        /// The client file: the server's URL, the key file and the CA.
        #[arg(long)]
        config: PathBuf,
    },
    /// Opens every stored version and reports damage, leftover files,
    /// versions past due and work waiting for key rotation; exits 1 when
    /// anything needs attention. Runs only while no server does.
    Check {
        #[command(flatten)]
        config: ConfigArg,
    },
    /// Writes a consistent copy of the data directory to a new directory
    /// while the server runs: each catalog with SQLite's online backup, and
    /// every object file a copied catalog names. Restore it as a data
    /// directory, under the same master key.
    Backup {
        #[command(flatten)]
        config: ConfigArg,
        /// The directory to write; it must not exist.
        dir: PathBuf,
    },
    /// Writes a copy of a tenant's data in another format, beside a running
    /// server.
    #[command(subcommand)]
    Export(ExportCommand),
    /// Moves the data to the storage the configuration names.
    #[command(subcommand)]
    Storage(StorageCommand),
    /// Makes or brings up to date the schema of the PostgreSQL catalog the
    /// configuration names, as a role that may act as vcon_owner. Runs only
    /// while no server does; a server never migrates at start.
    Migrate {
        #[command(flatten)]
        config: ConfigArg,
    },
}

#[derive(Subcommand)]
enum StorageCommand {
    /// Moves every tenant's sealed objects from the data directory to the
    /// bucket the configuration's [storage] table names: each is written,
    /// read back and compared before its file is removed. Runs only while
    /// no server does; run it again to finish a run that stopped.
    MoveObjects {
        #[command(flatten)]
        config: ConfigArg,
    },
}

#[derive(Subcommand)]
enum ExportCommand {
    /// Writes a tenant's kept versions as Parquet files, one directory per
    /// store date, and a manifest naming them: each version's sealed body
    /// byte for byte, and the clear columns of the tenant's metadata level.
    Parquet {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's id.
        #[arg(long)]
        tenant: String,
        /// A directory, or s3://bucket/prefix.
        #[arg(long)]
        to: String,
        /// Only versions stored at or after this RFC 3339 time.
        #[arg(long)]
        since: Option<String>,
        /// Only versions stored before this RFC 3339 time.
        #[arg(long)]
        until: Option<String>,
    },
}

#[derive(Subcommand)]
enum RekeyCommand {
    /// Rotates a tenant's key: every data key is rewrapped and the tenant's
    /// catalog re-encrypted under a new tenant key. A run that stops is
    /// finished by running it again.
    Tenant {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's id.
        id: String,
    },
    /// Rotates the master key: every tenant key is rewrapped under the new
    /// one, in a copy of server.sqlite that replaces it only once complete.
    Master {
        #[command(flatten)]
        config: ConfigArg,
        /// The file holding the new 32-byte master key.
        #[arg(long)]
        new_master: PathBuf,
    },
}

/// The configuration file every command reads.
#[derive(Args)]
struct ConfigArg {
    /// The configuration file.
    #[arg(long)]
    config: PathBuf,
}

#[derive(Subcommand)]
enum TenantCommand {
    /// Makes a tenant and prints its id. Runs only while no server does.
    Create {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's name: 1 to 128 characters.
        name: String,
        /// Its retention period, such as 7y, 18m or 90d; the server
        /// default if left out.
        #[arg(long)]
        retention: Option<String>,
    },
    /// Lists the tenants.
    List {
        #[command(flatten)]
        config: ConfigArg,
    },
    /// Sets a tenant's retention period.
    SetRetention {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's id.
        id: String,
        /// A period such as 7y, 18m or 90d, or `default` for the server's.
        retention: String,
    },
    /// Sets which clear columns a tenant's Parquet files carry.
    SetMetadata {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's id.
        id: String,
        /// none (the default), counts, or full, which holds personal data.
        metadata: String,
    },
    /// Removes a tenant. Runs only while no server does.
    Remove {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant's id.
        id: String,
        /// Remove its vCons too.
        #[arg(long)]
        force: bool,
        /// Remove its vCons even under a legal hold.
        #[arg(long, requires = "force")]
        override_holds: bool,
    },
}

#[derive(Subcommand)]
enum KeyCommand {
    /// Makes a key and prints its id and token, shown this once.
    Create {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant the key belongs to.
        #[arg(
            long,
            required_unless_present = "operator",
            conflicts_with = "operator"
        )]
        tenant: Option<String>,
        /// A scope the tenant key holds: write, read, delete, legal or audit.
        /// Repeat it for each scope.
        #[arg(long = "scope", requires = "tenant", conflicts_with = "operator")]
        scopes: Vec<String>,
        /// Makes an operator key, holding the admin scope.
        #[arg(long)]
        operator: bool,
        /// Who holds the key.
        #[arg(long)]
        label: String,
    },
    /// Lists keys, never their tokens.
    List {
        #[command(flatten)]
        config: ConfigArg,
        /// Only this tenant's keys.
        #[arg(long)]
        tenant: Option<String>,
    },
    /// Revokes a key from its next request on.
    Revoke {
        #[command(flatten)]
        config: ConfigArg,
        /// The key's id.
        id: String,
    },
}

#[derive(Subcommand)]
enum UserCommand {
    /// Makes a user, a person who signs in, and prints its id.
    Add {
        #[command(flatten)]
        config: ConfigArg,
        /// What the user is shown as.
        label: String,
        /// The issuer of the identity provider that knows the user. With
        /// neither this nor --subject, the user is a local account.
        #[arg(long, requires = "subject")]
        issuer: Option<String>,
        /// The subject the identity provider gives the user.
        #[arg(long, requires = "issuer")]
        subject: Option<String>,
    },
    /// Lists users, each with its memberships.
    List {
        #[command(flatten)]
        config: ConfigArg,
    },
    /// Deactivates a user and removes its memberships.
    Deactivate {
        #[command(flatten)]
        config: ConfigArg,
        /// The user's id.
        id: String,
    },
}

#[derive(Args)]
struct PlaceArgs {
    /// The tenant the membership is in.
    #[arg(
        long,
        required_unless_present = "operator",
        conflicts_with = "operator"
    )]
    tenant: Option<String>,
    /// The operator membership, which holds the operator role.
    #[arg(long)]
    operator: bool,
}

#[derive(Subcommand)]
enum MembershipCommand {
    /// Gives a user roles in a tenant, or the operator role, and prints
    /// the roles the membership then holds.
    Grant {
        #[command(flatten)]
        config: ConfigArg,
        /// The user's id.
        user: String,
        #[command(flatten)]
        place: PlaceArgs,
        /// A role in the tenant: reader, producer, records, legal, auditor
        /// or tenant-admin. Repeat it for each role.
        #[arg(
            long = "role",
            required_unless_present = "operator",
            conflicts_with = "operator"
        )]
        roles: Vec<String>,
    },
    /// Takes roles from a user's membership and prints the roles it still
    /// holds. With no --role, the membership is removed.
    Revoke {
        #[command(flatten)]
        config: ConfigArg,
        /// The user's id.
        user: String,
        #[command(flatten)]
        place: PlaceArgs,
        /// A role to take. Repeat it for each role.
        #[arg(long = "role", conflicts_with = "operator")]
        roles: Vec<String>,
    },
}

impl PlaceArgs {
    fn place(self, roles: Vec<String>) -> Place {
        match self.tenant {
            Some(id) => Place::Tenant { id, roles },
            None => Place::Operator,
        }
    }
}

fn user(command: UserCommand) -> (ConfigArg, commands::Command) {
    match command {
        UserCommand::Add {
            config,
            label,
            issuer,
            subject,
        } => (
            config,
            commands::Command::UserAdd {
                label,
                identity: issuer.zip(subject),
            },
        ),
        UserCommand::List { config } => (config, commands::Command::UserList),
        UserCommand::Deactivate { config, id } => {
            (config, commands::Command::UserDeactivate { id })
        }
    }
}

fn membership(command: MembershipCommand) -> (ConfigArg, commands::Command) {
    match command {
        MembershipCommand::Grant {
            config,
            user,
            place,
            roles,
        } => (
            config,
            commands::Command::MembershipGrant {
                user,
                place: place.place(roles),
            },
        ),
        MembershipCommand::Revoke {
            config,
            user,
            place,
            roles,
        } => (
            config,
            commands::Command::MembershipRevoke {
                user,
                place: place.place(roles),
            },
        ),
    }
}

#[derive(Subcommand)]
enum AuditCommand {
    /// Writes an audit log as JSON lines, oldest first.
    Export {
        #[command(flatten)]
        config: ConfigArg,
        /// The tenant whose log to write.
        #[arg(long, required_unless_present = "admin", conflicts_with = "admin")]
        tenant: Option<String>,
        /// Write the admin log.
        #[arg(long)]
        admin: bool,
        /// Only the records after this sequence number.
        #[arg(long)]
        since: Option<i64>,
    },
}

fn main() -> ExitCode {
    // Parsing touches no secret, and `--help` must work anywhere.
    let cli = Cli::parse();
    // Then, before any key, token or body is read: no core dumps, and all
    // memory locked, so none can reach a dump or swap.
    if let Err(e) = envelope::disable_core_dumps() {
        return fail(format!("core dumps cannot be turned off: {e}"));
    }
    if let Err(e) = envelope::lock_all_memory() {
        return fail(format!(
            "memory cannot be locked: {e}; allow it, for example LimitMEMLOCK=infinity"
        ));
    }
    let (config, command) = match cli.command {
        Command::Serve { config } => return serve(&config),
        Command::Migrate { config } => return migrate(&config.config),
        Command::Mcp { config } => return mcp(&config),
        Command::Tui { config } => return tui(&config),
        Command::Tenant(c) => tenant(c),
        Command::Key(c) => key(c),
        Command::User(c) => user(c),
        Command::Membership(c) => membership(c),
        Command::Rekey(RekeyCommand::Tenant { config, id }) => {
            (config, commands::Command::RekeyTenant { id })
        }
        Command::Rekey(RekeyCommand::Master { config, new_master }) => {
            (config, commands::Command::RekeyMaster { new_master })
        }
        Command::Check { config } => (config, commands::Command::Check),
        Command::Storage(StorageCommand::MoveObjects { config }) => {
            (config, commands::Command::StorageMoveObjects)
        }
        Command::Backup { config, dir } => (config, commands::Command::Backup { to: dir }),
        Command::Export(ExportCommand::Parquet {
            config,
            tenant,
            to,
            since,
            until,
        }) => (
            config,
            commands::Command::ExportParquet {
                tenant,
                to,
                since,
                until,
            },
        ),
        Command::Audit(AuditCommand::Export {
            config,
            tenant,
            admin: _,
            since,
        }) => (
            config,
            commands::Command::AuditExport {
                log: tenant.map_or(Log::Admin, Log::Tenant),
                since,
            },
        ),
    };
    let config = match config::load(&config.config) {
        Ok(config) => config,
        Err(e) => return fail(format!("configuration: {e}")),
    };
    match commands::run(&config, &command, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn tenant(command: TenantCommand) -> (ConfigArg, commands::Command) {
    match command {
        TenantCommand::Create {
            config,
            name,
            retention,
        } => (config, commands::Command::TenantCreate { name, retention }),
        TenantCommand::List { config } => (config, commands::Command::TenantList),
        TenantCommand::SetRetention {
            config,
            id,
            retention,
        } => (
            config,
            commands::Command::TenantSetRetention { id, retention },
        ),
        TenantCommand::SetMetadata {
            config,
            id,
            metadata,
        } => (
            config,
            commands::Command::TenantSetMetadata { id, metadata },
        ),
        TenantCommand::Remove {
            config,
            id,
            force,
            override_holds,
        } => (
            config,
            commands::Command::TenantRemove {
                id,
                force,
                override_holds,
            },
        ),
    }
}

fn key(command: KeyCommand) -> (ConfigArg, commands::Command) {
    match command {
        KeyCommand::Create {
            config,
            tenant,
            scopes,
            operator: _,
            label,
        } => {
            let owner = match tenant {
                Some(id) => Owner::Tenant { id, scopes },
                None => Owner::Operator,
            };
            (config, commands::Command::KeyCreate { owner, label })
        }
        KeyCommand::List { config, tenant } => (config, commands::Command::KeyList { tenant }),
        KeyCommand::Revoke { config, id } => (config, commands::Command::KeyRevoke { id }),
    }
}

/// `vcon-server migrate`: reads the configuration and applies the
/// migrations, needing no data directory or master key.
fn migrate(config: &Path) -> ExitCode {
    let config = match config::load(config) {
        Ok(config) => config,
        Err(e) => return fail(format!("configuration: {e}")),
    };
    match commands::migrate(&config, &mut std::io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("vcon-server: {message}");
    ExitCode::FAILURE
}

fn serve(config: &Path) -> ExitCode {
    match run_serve(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failed) => failed,
    }
}

/// `vcon-server serve` until it stops, or the exit for why it stopped.
fn run_serve(config: &Path) -> Result<(), ExitCode> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let config = config::load(config).map_err(|e| fail(format!("configuration: {e}")))?;
    runtime()?.block_on(async {
        // Taken before the server starts, so a signal that arrives at any
        // moment from here on is an orderly stop.
        let stop =
            stop_signal().map_err(|e| fail(format!("the stop signals cannot be taken: {e}")))?;
        serve::run(&config, stop).await.map_err(fail)
    })
}

/// The async runtime `serve` and `mcp` run on, or the exit for a runtime
/// that cannot start.
fn runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| fail(format!("the runtime cannot start: {e}")))
}

fn mcp(config: &Path) -> ExitCode {
    match run_mcp(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(failed) => failed,
    }
}

/// `vcon-server mcp` until stdin ends, or the exit for why it stopped.
fn run_mcp(config: &Path) -> Result<(), ExitCode> {
    let config = vcon_server::mcp::stdio::client_config(config)
        .map_err(|e| fail(format!("client file: {e}")))?;
    runtime()?
        .block_on(vcon_server::mcp::stdio::stdio(&config))
        .map_err(fail)
}

/// `vcon-server tui` until the user quits.
#[cfg(feature = "tui")]
fn tui(config: &Path) -> ExitCode {
    let config = match vcon_server::mcp::stdio::client_config(config) {
        Ok(config) => config,
        Err(e) => return fail(format!("client file: {e}")),
    };
    match vcon_server::tui::run(&config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(why) => fail(why),
    }
}

/// `vcon-server tui` in a build without the terminal UI: says how to get
/// one, and fails.
#[cfg(not(feature = "tui"))]
fn tui(_config: &Path) -> ExitCode {
    fail("this build has no terminal UI: build vcon-server with the cargo feature tui")
}

/// Takes SIGINT and SIGTERM from this call on, and gives a future that
/// completes when either has arrived, including one that arrived before
/// the future was first polled. Called inside the runtime.
///
/// # Errors
///
/// The system's error when a signal cannot be taken. No test reaches it:
/// the system refuses only a signal that may not be caught, and these two
/// may.
fn stop_signal() -> std::io::Result<impl Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = terminate.recv() => {}
        }
    })
}

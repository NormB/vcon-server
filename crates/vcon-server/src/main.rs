use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use vcon_server::commands::{self, Log, Owner};
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
    /// Reads the audit logs.
    #[command(subcommand)]
    Audit(AuditCommand),
    /// Rotates keys. Runs only while no server does.
    #[command(subcommand)]
    Rekey(RekeyCommand),
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
        /// A scope the tenant key holds: write, read, delete or legal.
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
        Command::Tenant(c) => tenant(c),
        Command::Key(c) => key(c),
        Command::Rekey(RekeyCommand::Tenant { config, id }) => {
            (config, commands::Command::RekeyTenant { id })
        }
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

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("vcon-server: {message}");
    ExitCode::FAILURE
}

fn serve(config: &Path) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();
    let config = match config::load(config) {
        Ok(config) => config,
        Err(e) => return fail(format!("configuration: {e}")),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => return fail(format!("the runtime cannot start: {e}")),
    };
    match runtime.block_on(serve::run(&config, stop_signal())) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fail(e),
    }
}

/// Completes on SIGINT or SIGTERM.
async fn stop_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let (Ok(mut interrupt), Ok(mut terminate)) = (
        signal(SignalKind::interrupt()),
        signal(SignalKind::terminate()),
    ) else {
        return;
    };
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
}

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
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
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Serve { config } => serve(&config),
    }
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("vcon-server: {message}");
    ExitCode::FAILURE
}

fn serve(config: &Path) -> ExitCode {
    // Before anything else: no core dumps, and all memory locked, so no
    // key or body can reach a dump or swap.
    if let Err(e) = envelope::disable_core_dumps() {
        return fail(format!("core dumps cannot be turned off: {e}"));
    }
    if let Err(e) = envelope::lock_all_memory() {
        return fail(format!(
            "memory cannot be locked: {e}; allow it, for example LimitMEMLOCK=infinity"
        ));
    }
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

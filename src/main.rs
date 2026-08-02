use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

/// CLI for a local markdown notebook.
///
/// Every command targets one note. With no target option that note is today's
/// daily note, created from the daily template if it does not exist yet.
#[derive(Parser)]
#[command(name = "kladde", version, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Work with kladde's configuration.
    #[command(subcommand)]
    Config(ConfigCommand),
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the path of the config file.
    Path,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Config(ConfigCommand::Path) => config_path(),
    }
}

fn config_path() -> ExitCode {
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    );
    if let Some(path) = file {
        println!("{}", path.display());
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "kladde: cannot locate the config directory: neither XDG_CONFIG_HOME nor HOME is set"
        );
        ExitCode::FAILURE
    }
}

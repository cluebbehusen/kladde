use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};

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
    /// Open the config file in your editor.
    ///
    /// The editor is the `editor` config key if set, otherwise the VISUAL or
    /// EDITOR environment variable.
    Open,
    /// Print a setting's value.
    ///
    /// Exits with status 1 and no output when the setting is unset.
    Get {
        /// Setting to print.
        key: ConfigKey,
    },
    /// Change a setting.
    Set {
        /// Setting to change.
        key: ConfigKey,
        /// New value.
        value: String,
    },
    /// Remove a setting.
    Unset {
        /// Setting to remove.
        key: ConfigKey,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum ConfigKey {
    /// Notebook used when a command is not given an explicit notebook.
    DefaultNotebook,
    /// Command that opens files, for example `vim` or `code --wait`.
    Editor,
}

impl ConfigKey {
    fn name(self) -> &'static str {
        match self {
            Self::DefaultNotebook => kladde::config::DEFAULT_NOTEBOOK,
            Self::Editor => kladde::config::EDITOR,
        }
    }
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Config(command) => config(command),
    }
}

fn config(command: ConfigCommand) -> ExitCode {
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    );
    let Some(file) = file else {
        return fail("cannot locate the config directory: neither XDG_CONFIG_HOME nor HOME is set");
    };
    match command {
        ConfigCommand::Path => {
            println!("{}", file.display());
            ExitCode::SUCCESS
        }
        ConfigCommand::Open => open(&file),
        ConfigCommand::Get { key } => get(&file, key),
        ConfigCommand::Set { key, value } => set(&file, key, &value),
        ConfigCommand::Unset { key } => finish(kladde::config::unset(&file, key.name())),
    }
}

fn get(file: &Path, key: ConfigKey) -> ExitCode {
    let config = match kladde::config::load(file) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let value = match key {
        ConfigKey::DefaultNotebook => config
            .default_notebook
            .map(|notebook| notebook.display().to_string()),
        ConfigKey::Editor => config.editor,
    };
    if let Some(value) = value {
        println!("{value}");
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn set(file: &Path, key: ConfigKey, value: &str) -> ExitCode {
    match key {
        ConfigKey::DefaultNotebook => match std::path::absolute(value) {
            Ok(notebook) => finish(kladde::config::set_default_notebook(file, &notebook)),
            Err(error) => fail(format!("invalid path \"{value}\": {error}")),
        },
        ConfigKey::Editor => finish(kladde::config::set_editor(file, value)),
    }
}

fn open(file: &Path) -> ExitCode {
    let configured = match kladde::config::load(file) {
        Ok(config) => config.editor,
        Err(error) => {
            eprintln!("kladde: ignoring invalid config: {error}");
            None
        }
    };
    let command = configured
        .or_else(|| editor_env("VISUAL"))
        .or_else(|| editor_env("EDITOR"));
    if let Err(error) = kladde::config::ensure_dir(file) {
        return fail(error);
    }
    match kladde::editor::open(file, command.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

/// An editor command from the environment; a blank value counts as unset.
fn editor_env(name: &str) -> Option<String> {
    env::var(name)
        .ok()
        .filter(|value| value.split_whitespace().next().is_some())
}

fn finish(result: Result<(), kladde::config::Error>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn fail(message: impl Display) -> ExitCode {
    eprintln!("kladde: {message}");
    ExitCode::FAILURE
}

use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Print the absolute path of a note.
    ///
    /// The note does not need to exist; the printed path is where it lives or
    /// would be created inside the notebook.
    Path {
        /// The note, as a relative path inside the notebook.
        target: PathBuf,
        #[command(flatten)]
        notebook: NotebookArg,
    },
}

/// The notebook selection shared by every command that reads or writes notes.
#[derive(Args)]
struct NotebookArg {
    /// Notebook to use, overriding the configured default.
    #[arg(long, value_name = "DIR")]
    notebook: Option<PathBuf>,
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

const NO_CONFIG_DIR: &str =
    "cannot locate the config directory: neither XDG_CONFIG_HOME nor HOME is set";

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Config(command) => config(command),
        Command::Path { target, notebook } => note_path(&target, notebook.notebook),
    }
}

fn note_path(target: &Path, flag: Option<PathBuf>) -> ExitCode {
    let root = match notebook_root(flag) {
        Ok(root) => root,
        Err(message) => return fail(message),
    };
    let notebook = match kladde::notebook::Notebook::open(&root) {
        Ok(notebook) => notebook,
        Err(error) => return fail(error),
    };
    match notebook.note(target) {
        Ok(note) => {
            print_path(note.as_path());
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

/// The notebook a command operates on: the `--notebook` value as given, or
/// the configured default. An explicit `--notebook` skips the config file
/// entirely, so an override keeps working while the config is broken.
fn notebook_root(flag: Option<PathBuf>) -> Result<PathBuf, String> {
    if let Some(root) = flag {
        return Ok(root);
    }
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or_else(|| NO_CONFIG_DIR.to_owned())?;
    let config = kladde::config::load(&file).map_err(|error| error.to_string())?;
    config.default_notebook.ok_or_else(|| {
        "no notebook: pass --notebook or set the default-notebook config key".to_owned()
    })
}

fn config(command: ConfigCommand) -> ExitCode {
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    );
    let Some(file) = file else {
        return fail(NO_CONFIG_DIR);
    };
    match command {
        ConfigCommand::Path => {
            print_path(&file);
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
    match key {
        ConfigKey::DefaultNotebook => {
            if let Some(notebook) = config.default_notebook {
                print_path(&notebook);
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::Editor => {
            if let Some(editor) = config.editor {
                println!("{editor}");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
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

/// Prints a path followed by a newline. On Unix the raw bytes are written,
/// so a non-Unicode path survives; `Display` would replace its invalid
/// bytes and print a path that does not exist.
#[cfg(unix)]
fn print_path(path: &Path) {
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    let mut stdout = std::io::stdout();
    stdout
        .write_all(path.as_os_str().as_bytes())
        .and_then(|()| stdout.write_all(b"\n"))
        .expect("stdout writes");
}

#[cfg(windows)]
fn print_path(path: &Path) {
    println!("{}", path.display());
}

fn fail(message: impl Display) -> ExitCode {
    eprintln!("kladde: {message}");
    ExitCode::FAILURE
}

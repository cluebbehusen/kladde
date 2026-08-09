use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use unicase::UniCase;

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
    /// would be created inside the notebook. With no target, the note is
    /// today's daily note.
    Path {
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Append text to the end of a note.
    ///
    /// The text is appended verbatim as its own line, so a bullet is
    /// whatever you type. The note is created if it does not exist, parent
    /// folders included, except that a note targeted by name must already
    /// exist. With no target, the note is today's daily note. Writes take
    /// the notebook's lock and replace the note atomically, so concurrent
    /// appends never lose an entry.
    Append {
        /// Text to append, verbatim. Text spelled exactly like an option
        /// of this command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        text: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Work with a note's properties.
    ///
    /// Properties are the note's YAML frontmatter: `key: value` lines
    /// between `---` fences at the very start of the note. A value is
    /// text or a list of texts. Edits rewrite only the named property
    /// and take the notebook's lock, like every kladde write.
    #[command(subcommand)]
    Frontmatter(FrontmatterCommand),
}

#[derive(Subcommand)]
enum FrontmatterCommand {
    /// Print a property's value.
    ///
    /// A text value prints on one line, a list value one item per line.
    /// Exits with status 1 and no output when the note or the property
    /// does not exist. With no target, the note is today's daily note.
    Get {
        /// Property to read.
        key: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Set a property to a text value.
    ///
    /// Replaces the property's value whatever shape it held, and creates
    /// the note, its frontmatter block, and the property as needed.
    Set {
        /// Property to change.
        key: String,
        /// New value. A value spelled exactly like an option of this
        /// command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        value: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Remove a property.
    ///
    /// Removing the last property removes the frontmatter block too. A
    /// property that does not exist is already removed, which is
    /// success.
    Unset {
        /// Property to remove.
        key: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Add an item to a list property.
    ///
    /// Creates the note, its frontmatter block, and the property as
    /// needed. An item already in the list is already added, which is
    /// success.
    Add {
        /// List property to extend.
        key: String,
        /// Item to add. An item spelled exactly like an option of this
        /// command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        item: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Remove an item from a list property.
    ///
    /// An item or property that does not exist is already removed, which
    /// is success; removing the last item leaves an empty list.
    Remove {
        /// List property to shorten.
        key: String,
        /// Item to remove. An item spelled exactly like an option of
        /// this command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        item: String,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
}

/// The note a command operates on. The three forms are mutually exclusive;
/// none of them means today's daily note.
#[derive(Args)]
#[group(multiple = false)]
struct TargetArgs {
    /// The note, as a relative path inside the notebook.
    target: Option<PathBuf>,
    /// The note, by name, resolved the way a wikilink is.
    #[arg(long)]
    name: Option<String>,
    /// A daily note: today, yesterday, tomorrow, or YYYY-MM-DD.
    #[arg(long, value_name = "WHEN")]
    date: Option<String>,
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
    /// Folder inside the notebook that holds daily notes.
    DailyFolder,
    /// Date format for daily note file names, for example `%Y-%m-%d`.
    DailyDateFormat,
    /// Whether writes stamp created/updated properties: true or false.
    Stamp,
    /// Property name for the created stamp.
    StampCreatedKey,
    /// Property name for the updated stamp.
    StampUpdatedKey,
    /// Timestamp format for stamp values, for example `%Y-%m-%dT%H:%M:%S`.
    StampFormat,
    /// Comma-separated notebook-relative paths that are never stamped.
    StampExclude,
}

impl ConfigKey {
    fn name(self) -> &'static str {
        match self {
            Self::DefaultNotebook => kladde::config::DEFAULT_NOTEBOOK,
            Self::Editor => kladde::config::EDITOR,
            Self::DailyFolder => kladde::config::DAILY_FOLDER,
            Self::DailyDateFormat => kladde::config::DAILY_DATE_FORMAT,
            Self::Stamp => kladde::config::STAMP,
            Self::StampCreatedKey => kladde::config::STAMP_CREATED_KEY,
            Self::StampUpdatedKey => kladde::config::STAMP_UPDATED_KEY,
            Self::StampFormat => kladde::config::STAMP_FORMAT,
            Self::StampExclude => kladde::config::STAMP_EXCLUDE,
        }
    }
}

const NO_CONFIG_DIR: &str =
    "cannot locate the config directory: neither XDG_CONFIG_HOME nor HOME is set";

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Config(command) => config(command),
        Command::Path { target, notebook } => {
            dispatch(target, notebook.notebook, None, |note, _guard| {
                print_path(note.as_path());
                ExitCode::SUCCESS
            })
        }
        Command::Append {
            text,
            target,
            notebook,
        } => append(&text, target, notebook.notebook),
        Command::Frontmatter(command) => frontmatter(command),
    }
}

const NO_NOTEBOOK: &str = "no notebook: pass --notebook or set the default-notebook config key";

const NO_STATE_DIR: &str =
    "cannot locate the state directory: neither XDG_STATE_HOME nor HOME is set";

/// Resolves the note a command targets and runs `act` on it. With
/// `locks`, the notebook's lock is taken before the note is resolved and
/// `act` receives a guard: resolution racing another writer's atomic
/// replace can transiently misread the filesystem, so writers resolve
/// inside the critical section.
fn dispatch(
    target: TargetArgs,
    flag: Option<PathBuf>,
    locks: Option<&Path>,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>) -> ExitCode,
) -> ExitCode {
    if let Some(relative) = target.target {
        return with_notebook(flag, locks, |notebook| notebook.note(&relative), act);
    }
    if let Some(name) = target.name {
        return with_notebook(flag, locks, |notebook| notebook.find(&name), act);
    }
    daily_note(target.date, flag, locks, act)
}

/// The directory for lock files, from the environment.
fn locks() -> Result<PathBuf, &'static str> {
    kladde::write::lock_dir(
        env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or(NO_STATE_DIR)
}

fn append(text: &str, target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let config = match loaded_config(flag.is_some()) {
        Ok(config) => config,
        Err(message) => return fail(message),
    };
    let stamping = stamping(config);
    dispatch(target, flag, Some(&locks), |note, guard| {
        let guard = guard.expect("write dispatch locks the notebook");
        let timestamp = stamping.timestamp(note);
        match guard.append(text, stamping.stamp(timestamp.as_deref()).as_ref()) {
            Ok(()) => {
                print_path(note.as_path());
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        }
    })
}

/// The stamping inputs a write resolves once: whether stamping is on, the
/// stamp key names, the timestamp format, and the excluded paths.
struct Stamping {
    enabled: bool,
    created_key: String,
    updated_key: String,
    format: kladde::day::StampFormat,
    excludes: Vec<PathBuf>,
}

/// Resolves stamping from the config; the clock is read later, per
/// write, because the library stays clock-free.
fn stamping(config: kladde::config::Config) -> Stamping {
    Stamping {
        enabled: config.stamp.unwrap_or(true),
        created_key: config
            .stamp_created_key
            .unwrap_or_else(|| kladde::frontmatter::DEFAULT_CREATED_KEY.to_owned()),
        updated_key: config
            .stamp_updated_key
            .unwrap_or_else(|| kladde::frontmatter::DEFAULT_UPDATED_KEY.to_owned()),
        format: config.stamp_format.unwrap_or_default(),
        excludes: config.stamp_exclude.unwrap_or_default(),
    }
}

impl Stamping {
    /// The timestamp to stamp `note` with, rendered from the clock now:
    /// callers ask while already holding the notebook's lock, so stamps
    /// record the serialized write order and updated never runs
    /// backwards. `None` when stamping is off or the note sits under an
    /// excluded path.
    fn timestamp(&self, note: &Note) -> Option<String> {
        let excluded = self.excludes.iter().any(|entry| excluded(note, entry));
        (self.enabled && !excluded).then(|| self.format.render(jiff::Zoned::now().datetime()))
    }

    /// The stamp carrying `timestamp`, or `None` when there is none.
    fn stamp<'a>(&'a self, timestamp: Option<&'a str>) -> Option<kladde::frontmatter::Stamp<'a>> {
        timestamp.map(|timestamp| {
            kladde::frontmatter::Stamp::new(&self.created_key, &self.updated_key, timestamp)
                .expect("stamp settings are validated by config")
        })
    }
}

/// Whether `note` sits under the excluded `entry`, matched the way notes
/// are identified: by spelling with case folded, and by what the entry
/// resolves to inside the notebook. Folding uses the same equivalence as
/// name lookup, so an entry matches a case-aliased spelling even before
/// its folder first exists; on a filesystem that does not fold, an entry
/// can at worst over-match, and the cost is a note left unstamped.
/// Resolution covers linked and normalized spellings once the folder
/// exists. Unicode-normalization aliases before the folder exists are
/// the accepted gap, shared with name lookup: closing it would take
/// normalization tables for an alias only a foreign toolchain can
/// produce, and the first write heals on the next.
fn excluded(note: &Note, entry: &Path) -> bool {
    if folded_starts_with(note.relative(), entry) {
        return true;
    }
    resolved_exclude(note.root(), entry)
        .is_some_and(|resolved| folded_starts_with(note.relative(), &resolved))
}

/// The entry's place inside the notebook once links and filesystem
/// spellings resolve: the deepest existing ancestor is canonicalized the
/// way note paths are, and the not-yet-created remainder rides along as
/// spelled. `None` when nothing of the entry exists yet, or when it
/// resolves outside the notebook.
fn resolved_exclude(root: &Path, entry: &Path) -> Option<PathBuf> {
    let components: Vec<_> = entry.components().collect();
    for split in (1..=components.len()).rev() {
        let prefix: PathBuf = components[..split].iter().collect();
        let Ok(target) = std::fs::canonicalize(root.join(&prefix)) else {
            continue;
        };
        let suffix: PathBuf = components[split..].iter().collect();
        return Some(target.strip_prefix(root).ok()?.join(suffix));
    }
    None
}

/// Component-wise prefix match with case folded.
fn folded_starts_with(path: &Path, prefix: &Path) -> bool {
    let mut components = path.components();
    prefix.components().all(|wanted| {
        components.next().is_some_and(|component| {
            UniCase::new(component.as_os_str().to_string_lossy())
                == UniCase::new(wanted.as_os_str().to_string_lossy())
        })
    })
}

fn frontmatter(command: FrontmatterCommand) -> ExitCode {
    match command {
        FrontmatterCommand::Get {
            key,
            target,
            notebook,
        } => frontmatter_get(&key, target, notebook.notebook),
        FrontmatterCommand::Set {
            key,
            value,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, |current| {
            kladde::frontmatter::set(current, &key, &value)
        }),
        FrontmatterCommand::Unset {
            key,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, |current| {
            kladde::frontmatter::unset(current, &key)
        }),
        FrontmatterCommand::Add {
            key,
            item,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, |current| {
            kladde::frontmatter::add(current, &key, &item)
        }),
        FrontmatterCommand::Remove {
            key,
            item,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, |current| {
            kladde::frontmatter::remove(current, &key, &item)
        }),
    }
}

/// Prints a property's value without taking the notebook's lock: the
/// atomic replace means a reader never sees a half-written note, and a
/// read must not create or wait for anything.
fn frontmatter_get(key: &str, target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    dispatch(target, flag, None, |note, _guard| {
        let path = note.as_path();
        let contents = match std::fs::read_to_string(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return fail(format!("cannot read {}: {error}", path.display())),
        };
        match kladde::frontmatter::get(&contents, key) {
            Ok(Some(kladde::frontmatter::Value::Scalar(value))) => {
                println!("{value}");
                ExitCode::SUCCESS
            }
            Ok(Some(kladde::frontmatter::Value::List(items))) => {
                for item in items {
                    println!("{item}");
                }
                ExitCode::SUCCESS
            }
            Ok(None) => ExitCode::FAILURE,
            Err(error) => fail(error),
        }
    })
}

/// Runs a frontmatter edit under the notebook's lock: read, transform,
/// stamp, atomically replace. An edit that changes nothing skips the
/// write and the stamp, so an idempotent edit cannot create a note or
/// bump its updated stamp.
fn frontmatter_edit(
    target: TargetArgs,
    flag: Option<PathBuf>,
    edit: impl FnOnce(&str) -> Result<String, kladde::frontmatter::Error>,
) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let config = match loaded_config(flag.is_some()) {
        Ok(config) => config,
        Err(message) => return fail(message),
    };
    let stamping = stamping(config);
    dispatch(target, flag, Some(&locks), |note, guard| {
        let guard = guard.expect("write dispatch locks the notebook");
        let current = match guard.current() {
            Ok(current) => current,
            Err(error) => return fail(error),
        };
        let new = match edit(&current) {
            Ok(new) => new,
            Err(error) => return fail(error),
        };
        if new != current {
            let had_block = kladde::frontmatter::has_block(&current);
            let timestamp = stamping.timestamp(note);
            let stamped = match stamping.stamp(timestamp.as_deref()) {
                Some(stamp) => kladde::frontmatter::stamped(&new, had_block, &stamp),
                None => new,
            };
            if let Err(error) = guard.replace(&stamped) {
                return fail(error);
            }
        }
        print_path(note.as_path());
        ExitCode::SUCCESS
    })
}

/// Resolves the notebook root, opens it, takes its lock when `locks`
/// asks for one, and runs `act` on the note `resolve` picks inside it.
fn with_notebook(
    flag: Option<PathBuf>,
    locks: Option<&Path>,
    resolve: impl FnOnce(&kladde::notebook::Notebook) -> Result<Note, kladde::notebook::Error>,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>) -> ExitCode,
) -> ExitCode {
    let root = match notebook_root(flag) {
        Ok(root) => root,
        Err(message) => return fail(message),
    };
    let notebook = match kladde::notebook::Notebook::open(&root) {
        Ok(notebook) => notebook,
        Err(error) => return fail(error),
    };
    let lock = match locked(locks, notebook.root()) {
        Ok(lock) => lock,
        Err(error) => return fail(error),
    };
    let note = match resolve(&notebook) {
        Ok(note) => note,
        Err(error) => return fail(error),
    };
    let guard = lock.as_ref().map(|lock| lock.guard(&note));
    act(&note, guard.as_ref())
}

/// The notebook's lock, when a write asked for one.
fn locked(
    locks: Option<&Path>,
    root: &Path,
) -> Result<Option<kladde::write::Lock>, kladde::write::Error> {
    match locks {
        Some(lock_dir) => Ok(Some(kladde::write::Lock::acquire(lock_dir, root)?)),
        None => Ok(None),
    }
}

type Note = kladde::notebook::NotePath;

/// A daily note needs the config even when `--notebook` is given, because
/// the daily keys have no flag override, and printing a wrong path with a
/// zero exit would be worse than failing.
fn daily_note(
    date: Option<String>,
    flag: Option<PathBuf>,
    locks: Option<&Path>,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>) -> ExitCode,
) -> ExitCode {
    let config = match loaded_config(flag.is_some()) {
        Ok(config) => config,
        Err(message) => return fail(message),
    };
    let today = jiff::Zoned::now().date();
    let day = match date {
        Some(value) => match kladde::day::parse(&value, today) {
            Ok(day) => day,
            Err(error) => return fail(error),
        },
        None => today,
    };
    let Some(root) = flag.or(config.default_notebook) else {
        return fail(NO_NOTEBOOK);
    };
    let format = config.daily_date_format.unwrap_or_default();
    with_notebook(
        Some(root),
        locks,
        |notebook| notebook.daily(day, config.daily_folder.as_deref(), &format),
        act,
    )
}

/// The config a command draws optional keys from: the daily note keys and
/// the stamp keys have no flag override, so even an explicit `--notebook`
/// reads the file, and a broken config fails the command rather than
/// silently dropping those keys. A missing file is an empty config; an
/// unlocatable config directory is one too when `--notebook` pins the
/// notebook, and an error otherwise, since the default notebook could
/// only come from config.
fn loaded_config(explicit_notebook: bool) -> Result<kladde::config::Config, String> {
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    );
    match file {
        Some(file) => kladde::config::load(&file).map_err(|error| error.to_string()),
        None if explicit_notebook => Ok(kladde::config::Config::default()),
        None => Err(NO_CONFIG_DIR.to_owned()),
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
    config
        .default_notebook
        .ok_or_else(|| NO_NOTEBOOK.to_owned())
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
        ConfigKey::DailyFolder => {
            if let Some(folder) = config.daily_folder {
                print_path(&folder);
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::DailyDateFormat => {
            if let Some(format) = config.daily_date_format {
                println!("{}", format.as_str());
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::Stamp => {
            if let Some(stamp) = config.stamp {
                println!("{stamp}");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::StampCreatedKey => {
            if let Some(key) = config.stamp_created_key {
                println!("{key}");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::StampUpdatedKey => {
            if let Some(key) = config.stamp_updated_key {
                println!("{key}");
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::StampFormat => {
            if let Some(format) = config.stamp_format {
                println!("{}", format.as_str());
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        ConfigKey::StampExclude => {
            if let Some(entries) = config.stamp_exclude {
                // Entries print with `/` on every platform, the spelling
                // the config file stores, not the platform separator.
                let joined = entries
                    .iter()
                    .map(|entry| {
                        entry
                            .components()
                            .map(|component| component.as_os_str().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("/")
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                println!("{joined}");
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
        ConfigKey::DailyFolder => finish(kladde::config::set_daily_folder(file, value)),
        ConfigKey::DailyDateFormat => finish(kladde::config::set_daily_date_format(file, value)),
        ConfigKey::Stamp => finish(kladde::config::set_stamp(file, value)),
        ConfigKey::StampCreatedKey => finish(kladde::config::set_stamp_created_key(file, value)),
        ConfigKey::StampUpdatedKey => finish(kladde::config::set_stamp_updated_key(file, value)),
        ConfigKey::StampFormat => finish(kladde::config::set_stamp_format(file, value)),
        ConfigKey::StampExclude => finish(kladde::config::set_stamp_exclude(file, value)),
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

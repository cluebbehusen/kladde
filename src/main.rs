use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use unicase::UniCase;

/// CLI for a local markdown notebook.
///
/// Commands that take a target work on one note; with no target, that note is
/// today's daily note. A write that creates a daily note seeds it from the
/// daily template when one is configured.
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
    /// Print a note's contents.
    ///
    /// The contents print exactly as stored. A missing note is an error,
    /// so a missing note and an empty one can be told apart. With no
    /// target, the note is today's daily note.
    Read {
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// List every note in the notebook.
    ///
    /// Prints notebook-relative paths, one per line, sorted. Folders and
    /// files whose names start with a dot are skipped, and links are
    /// neither followed nor listed.
    List {
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Find notes by searching their contents.
    ///
    /// The match is case-insensitive; matching notes print as
    /// notebook-relative paths, one per line, sorted. Exits with status 1
    /// and no output when nothing matches.
    Search {
        /// Text to find. Text spelled exactly like an option of this
        /// command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        query: String,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Open a note in an editor.
    ///
    /// The editor is the `editor` config key if set, otherwise the VISUAL
    /// or EDITOR environment variable; a value naming no program counts
    /// as unset. The note does not need to exist: nothing is created, and
    /// the editor decides what a missing file means. With no target, the
    /// note is today's daily note.
    Open {
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Create a note.
    ///
    /// The note is created empty, parent folders included; a daily note
    /// starts from the daily template when one is configured. A note
    /// that already exists is left untouched, which is success. With no
    /// target, the note is today's daily note.
    New {
        #[command(flatten)]
        target: NewTargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Append text to a note.
    ///
    /// The text is appended verbatim as its own line, so a bullet is
    /// whatever you type. It lands at the end of the note, or inside it
    /// with `--under` and `--under-bullet`: at the end of a heading's
    /// section, or nested at the end of a bullet's thread. The note is
    /// created if it does not exist, parent folders included, except
    /// that a note targeted by name must already exist, and a placed
    /// append fails rather than create a note its target cannot match.
    /// With no target, the note is today's daily note. Writes take the
    /// notebook's lock and replace the note atomically, so concurrent
    /// appends never lose an entry.
    Append {
        /// Text to append, verbatim. Text spelled exactly like an option
        /// of this command needs a `--` separator first.
        #[arg(allow_hyphen_values = true)]
        text: String,
        /// Heading whose section receives the text, a prefix of the
        /// heading as written, its `#` marks excluded. Exactly one
        /// heading must match. Repeat to descend: each later heading is
        /// found inside the previous one's section.
        #[arg(long, value_name = "HEADING", allow_hyphen_values = true)]
        under: Vec<String>,
        /// Bullet the text is nested under, a prefix of the bullet's
        /// first line past its marker. Exactly one bullet must match.
        /// Repeat to descend a thread; with `--under`, the first bullet
        /// is found inside that section.
        #[arg(long, value_name = "BULLET", allow_hyphen_values = true)]
        under_bullet: Vec<String>,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Remove a bullet from a note.
    ///
    /// The bullet is matched by `--match`, a prefix of its first line
    /// past its marker, inside the scope `--under` and `--under-bullet`
    /// name the way an appended entry's place is. Exactly one bullet
    /// must match, and a bullet holding anything beyond its first line,
    /// child bullets or nested blocks, is refused rather than taken
    /// with it. Nothing is created: a missing note has no bullet to
    /// match. With no target, the note is today's daily note. Writes
    /// take the notebook's lock and replace the note atomically.
    Remove {
        /// Bullet to remove, a prefix of its first line past its
        /// marker. Exactly one bullet must match.
        #[arg(long = "match", value_name = "BULLET", allow_hyphen_values = true)]
        matched: String,
        /// Heading whose section holds the bullet, a prefix of the
        /// heading as written, its `#` marks excluded. Exactly one
        /// heading must match. Repeat to descend: each later heading is
        /// found inside the previous one's section.
        #[arg(long, value_name = "HEADING", allow_hyphen_values = true)]
        under: Vec<String>,
        /// Bullet whose thread holds the removed bullet, a prefix of
        /// its first line past its marker. Exactly one bullet must
        /// match. Repeat to descend a thread; with `--under`, the first
        /// bullet is found inside that section.
        #[arg(long, value_name = "BULLET", allow_hyphen_values = true)]
        under_bullet: Vec<String>,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Check a task in a note.
    ///
    /// A task is a bullet whose text starts with `[ ]` or `[x]`. The
    /// task is matched by `--match`, a prefix of its text past the box,
    /// so the same match works before and after checking, inside the
    /// scope `--under` and `--under-bullet` name. Exactly one task must
    /// match. A task that is already checked is left alone, which is
    /// success. Nothing is created: a missing note has no task to
    /// match. With no target, the note is today's daily note. Writes
    /// take the notebook's lock and replace the note atomically.
    Check {
        /// Task to check, a prefix of its text past the box. Exactly
        /// one task must match.
        #[arg(long = "match", value_name = "TASK", allow_hyphen_values = true)]
        matched: String,
        /// Heading whose section holds the task, a prefix of the
        /// heading as written, its `#` marks excluded. Exactly one
        /// heading must match. Repeat to descend: each later heading is
        /// found inside the previous one's section.
        #[arg(long, value_name = "HEADING", allow_hyphen_values = true)]
        under: Vec<String>,
        /// Bullet whose thread holds the task, a prefix of its first
        /// line past its marker. Exactly one bullet must match. Repeat
        /// to descend a thread; with `--under`, the first bullet is
        /// found inside that section.
        #[arg(long, value_name = "BULLET", allow_hyphen_values = true)]
        under_bullet: Vec<String>,
        #[command(flatten)]
        target: TargetArgs,
        #[command(flatten)]
        notebook: NotebookArg,
    },
    /// Uncheck a task in a note.
    ///
    /// A task is a bullet whose text starts with `[ ]` or `[x]`. The
    /// task is matched by `--match`, a prefix of its text past the box,
    /// so the same match works before and after checking, inside the
    /// scope `--under` and `--under-bullet` name. Exactly one task must
    /// match. A task that is already unchecked is left alone, which is
    /// success. Nothing is created: a missing note has no task to
    /// match. With no target, the note is today's daily note. Writes
    /// take the notebook's lock and replace the note atomically.
    Uncheck {
        /// Task to uncheck, a prefix of its text past the box. Exactly
        /// one task must match.
        #[arg(long = "match", value_name = "TASK", allow_hyphen_values = true)]
        matched: String,
        /// Heading whose section holds the task, a prefix of the
        /// heading as written, its `#` marks excluded. Exactly one
        /// heading must match. Repeat to descend: each later heading is
        /// found inside the previous one's section.
        #[arg(long, value_name = "HEADING", allow_hyphen_values = true)]
        under: Vec<String>,
        /// Bullet whose thread holds the task, a prefix of its first
        /// line past its marker. Exactly one bullet must match. Repeat
        /// to descend a thread; with `--under`, the first bullet is
        /// found inside that section.
        #[arg(long, value_name = "BULLET", allow_hyphen_values = true)]
        under_bullet: Vec<String>,
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

/// The note `new` creates. The two forms are mutually exclusive; a name
/// lookup means an existing note, so `new` takes none. None of them
/// means today's daily note.
#[derive(Args)]
#[group(multiple = false)]
struct NewTargetArgs {
    /// The note, as a relative path inside the notebook.
    target: Option<PathBuf>,
    /// A daily note: today, yesterday, tomorrow, or YYYY-MM-DD.
    #[arg(long, value_name = "WHEN")]
    date: Option<String>,
}

impl From<NewTargetArgs> for TargetArgs {
    fn from(target: NewTargetArgs) -> Self {
        Self {
            target: target.target,
            name: None,
            date: target.date,
        }
    }
}

/// The notebook selection shared by every command that reads or writes notes.
#[derive(Args)]
struct NotebookArg {
    /// Notebook to use, overriding the configured default.
    #[arg(long, value_name = "DIR")]
    notebook: Option<PathBuf>,
}

/// The notebook selection shared by the config subcommands: with it, a
/// subcommand targets that notebook's own config file instead of the
/// base config.
#[derive(Args)]
struct ConfigNotebookArg {
    /// Notebook whose config to target, instead of the base config.
    #[arg(long, value_name = "DIR")]
    notebook: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// Print the path of the config file.
    Path {
        #[command(flatten)]
        notebook: ConfigNotebookArg,
    },
    /// Open the config file in your editor.
    ///
    /// The editor is the `editor` config key if set, otherwise the VISUAL or
    /// EDITOR environment variable.
    Open {
        #[command(flatten)]
        notebook: ConfigNotebookArg,
    },
    /// Print a setting's value.
    ///
    /// Exits with status 1 and no output when the setting is unset.
    Get {
        /// Setting to print.
        key: ConfigKey,
        #[command(flatten)]
        notebook: ConfigNotebookArg,
    },
    /// Change a setting.
    Set {
        /// Setting to change.
        key: ConfigKey,
        /// New value.
        value: String,
        #[command(flatten)]
        notebook: ConfigNotebookArg,
    },
    /// Remove a setting.
    Unset {
        /// Setting to remove.
        key: ConfigKey,
        #[command(flatten)]
        notebook: ConfigNotebookArg,
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
    /// Notebook-relative path of the note that seeds a new daily note.
    DailyTemplate,
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
    /// Indent unit for entries nested under a childless bullet: tab or
    /// spaces.
    BulletIndent,
}

impl ConfigKey {
    fn name(self) -> &'static str {
        match self {
            Self::DefaultNotebook => kladde::config::DEFAULT_NOTEBOOK,
            Self::Editor => kladde::config::EDITOR,
            Self::DailyFolder => kladde::config::DAILY_FOLDER,
            Self::DailyDateFormat => kladde::config::DAILY_DATE_FORMAT,
            Self::DailyTemplate => kladde::config::DAILY_TEMPLATE,
            Self::Stamp => kladde::config::STAMP,
            Self::StampCreatedKey => kladde::config::STAMP_CREATED_KEY,
            Self::StampUpdatedKey => kladde::config::STAMP_UPDATED_KEY,
            Self::StampFormat => kladde::config::STAMP_FORMAT,
            Self::StampExclude => kladde::config::STAMP_EXCLUDE,
            Self::BulletIndent => kladde::config::BULLET_INDENT,
        }
    }
}

const NO_CONFIG_DIR: &str =
    "cannot locate the config directory: neither XDG_CONFIG_HOME nor HOME is set";

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Config(command) => config(command),
        Command::Path { target, notebook } => dispatch(
            target,
            notebook.notebook,
            None,
            false,
            |note, _guard, _seed| {
                print_path(note.as_path());
                ExitCode::SUCCESS
            },
        ),
        Command::Read { target, notebook } => read(target, notebook.notebook),
        Command::List { notebook } => list(notebook.notebook),
        Command::Search { query, notebook } => search(&query, notebook.notebook),
        Command::Open { target, notebook } => open_note(target, notebook.notebook),
        Command::New { target, notebook } => new_note(target.into(), notebook.notebook),
        Command::Append {
            text,
            under,
            under_bullet,
            target,
            notebook,
        } => append(&text, under, under_bullet, target, notebook.notebook),
        Command::Remove {
            matched,
            under,
            under_bullet,
            target,
            notebook,
        } => remove_bullet(&matched, under, under_bullet, target, notebook.notebook),
        Command::Check {
            matched,
            under,
            under_bullet,
            target,
            notebook,
        } => toggle_task(
            &matched,
            under,
            under_bullet,
            target,
            notebook.notebook,
            true,
        ),
        Command::Uncheck {
            matched,
            under,
            under_bullet,
            target,
            notebook,
        } => toggle_task(
            &matched,
            under,
            under_bullet,
            target,
            notebook.notebook,
            false,
        ),
        Command::Frontmatter(command) => frontmatter(command),
    }
}

const NO_NOTEBOOK: &str = "no notebook: pass --notebook or set the default-notebook config key";

const NO_STATE_DIR: &str =
    "cannot locate the state directory: neither XDG_STATE_HOME nor HOME is set";

/// The contents a write seeds a missing daily note with: `None` when no
/// template applies, `Some(Err(_))` when a template is configured but
/// cannot seed — an error that surfaces only when a write actually needs
/// the seed, so a broken template never blocks a write to an existing
/// note.
type Seed = Option<Result<String, String>>;

/// Resolves the note a command targets and runs `act` on it. With
/// `locks`, the notebook's lock is taken before the note is resolved and
/// `act` receives a guard: resolution racing another writer's atomic
/// replace can transiently misread the filesystem, so writers resolve
/// inside the critical section. Only daily resolution carries a seed,
/// and only for a `seeded` command: an edit that never creates a note
/// must not read and render the template under the lock, and an
/// explicit target never seeds, even one spelling a daily note's path.
fn dispatch(
    target: TargetArgs,
    flag: Option<PathBuf>,
    locks: Option<&Path>,
    seeded: bool,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>, Seed) -> ExitCode,
) -> ExitCode {
    if let Some(relative) = target.target {
        return with_notebook(
            flag,
            locks,
            |notebook| Ok((notebook.note(&relative)?, None)),
            act,
        );
    }
    if let Some(name) = target.name {
        return with_notebook(
            flag,
            locks,
            |notebook| Ok((notebook.find(&name)?, None)),
            act,
        );
    }
    daily_note(target.date, flag, locks, seeded, act)
}

/// The directory for lock files, from the environment.
fn locks() -> Result<PathBuf, &'static str> {
    kladde::write::lock_dir(
        env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or(NO_STATE_DIR)
}

fn append(
    text: &str,
    under: Vec<String>,
    under_bullet: Vec<String>,
    target: TargetArgs,
    flag: Option<PathBuf>,
) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let placement = kladde::structure::Placement {
        headings: under,
        bullets: under_bullet,
        indent: config.bullet_indent.unwrap_or_default(),
    };
    let stamping = stamping(config);
    dispatch(target, root, Some(&locks), true, |note, guard, seed| {
        let guard = guard.expect("write dispatch locks the notebook");
        let seed = match seeding(&seed) {
            Ok(seed) => seed,
            Err(message) => return fail(message),
        };
        let timestamp = stamping.timestamp(note);
        match guard.append(
            text,
            seed,
            &placement,
            stamping.stamp(timestamp.as_deref()).as_ref(),
        ) {
            Ok(()) => {
                print_path(note.as_path());
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        }
    })
}

fn remove_bullet(
    matched: &str,
    under: Vec<String>,
    under_bullet: Vec<String>,
    target: TargetArgs,
    flag: Option<PathBuf>,
) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let scope = kladde::structure::Placement {
        headings: under,
        bullets: under_bullet,
        indent: kladde::structure::Indent::default(),
    };
    let stamping = stamping(config);
    dispatch(target, root, Some(&locks), false, |note, guard, _seed| {
        // The seed stays unused: a removal edits what exists, so a
        // missing daily note must not materialize its template first.
        let guard = guard.expect("write dispatch locks the notebook");
        let timestamp = stamping.timestamp(note);
        match guard.remove(
            matched,
            &scope,
            stamping.stamp(timestamp.as_deref()).as_ref(),
        ) {
            Ok(()) => {
                print_path(note.as_path());
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        }
    })
}

fn toggle_task(
    matched: &str,
    under: Vec<String>,
    under_bullet: Vec<String>,
    target: TargetArgs,
    flag: Option<PathBuf>,
    checked: bool,
) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let scope = kladde::structure::Placement {
        headings: under,
        bullets: under_bullet,
        indent: kladde::structure::Indent::default(),
    };
    let stamping = stamping(config);
    dispatch(target, root, Some(&locks), false, |note, guard, _seed| {
        // The seed stays unused: a flip edits what exists, so a
        // missing daily note must not materialize its template first.
        let guard = guard.expect("write dispatch locks the notebook");
        let timestamp = stamping.timestamp(note);
        match guard.toggle(
            matched,
            &scope,
            checked,
            stamping.stamp(timestamp.as_deref()).as_ref(),
        ) {
            Ok(()) => {
                print_path(note.as_path());
                ExitCode::SUCCESS
            }
            Err(error) => fail(error),
        }
    })
}

/// Prints a note's contents without taking the notebook's lock: the
/// atomic replace means a reader never sees a half-written note, and a
/// read must not create or wait for anything.
fn read(target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    dispatch(target, flag, None, false, |note, _guard, _seed| {
        let path = note.as_path();
        match std::fs::read_to_string(path) {
            Ok(contents) => {
                print_contents(&contents);
                ExitCode::SUCCESS
            }
            Err(error) => fail(format!("cannot read {}: {error}", path.display())),
        }
    })
}

/// Lists every note as a notebook-relative path, without taking the
/// notebook's lock.
fn list(flag: Option<PathBuf>) -> ExitCode {
    let (_, notes) = match listed(flag) {
        Ok(listed) => listed,
        Err(message) => return fail(message),
    };
    for note in notes {
        print_path(&note);
    }
    ExitCode::SUCCESS
}

/// The notebook's notes, ready for one-path-per-line output: a name
/// holding a line break would let one note print as several, so the
/// listing commands refuse it loudly instead — wherever it sits in the
/// notebook, matched or not, because a forged-looking listing is worse
/// than a failed one.
fn listed(flag: Option<PathBuf>) -> Result<(kladde::notebook::Notebook, Vec<PathBuf>), String> {
    let notebook = opened(flag)?;
    // Stringified eagerly, through a fn reference rather than a closure:
    // only Unix can make the walk fail under test, and a closure that
    // never runs on Windows is a function its coverage counts as missed.
    let outcome = notebook.notes();
    let message = outcome.as_ref().err().map(ToString::to_string);
    let message = message.unwrap_or_default();
    let notes = outcome.ok().ok_or(message)?;
    for note in &notes {
        printable(note)?;
    }
    Ok((notebook, notes))
}

/// Checks a note name against the one-path-per-line output contract.
/// The lossy conversion keeps line-break bytes, so a non-Unicode name
/// cannot smuggle one past the check. The message is built eagerly:
/// only Unix can create a failing name, so a lazy closure would never
/// run on Windows and its lines would fail the coverage gate there.
fn printable(note: &Path) -> Result<(), String> {
    let name = note.to_string_lossy();
    let message = format!(
        "note name \"{}\" contains a line break",
        name.escape_debug()
    );
    let clean = !name.contains(['\n', '\r']);
    clean.then_some(()).ok_or(message)
}

/// Finds the notes containing `query`, without taking the notebook's
/// lock. Both sides are folded with Unicode default case folding — the
/// folding name lookup uses — so STRASSE finds Straße; spellings that
/// differ by normalization stay distinct, as they do everywhere in
/// kladde.
fn search(query: &str, flag: Option<PathBuf>) -> ExitCode {
    if query.is_empty() {
        return fail("nothing to search for: the query is empty");
    }
    let (notebook, notes) = match listed(flag) {
        Ok(listed) => listed,
        Err(message) => return fail(message),
    };
    let needle = caseless::default_case_fold_str(query);
    // Matches are held back until every note has been read: a read
    // failure midway must not leave a plausible partial result on
    // stdout, particularly when the failure exit code is also the
    // no-match one.
    let mut matches = Vec::new();
    for note in notes {
        let path = notebook.root().join(&note);
        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) => return fail(format!("cannot read {}: {error}", path.display())),
        };
        if caseless::default_case_fold_str(&contents).contains(&needle) {
            matches.push(note);
        }
    }
    if matches.is_empty() {
        return ExitCode::FAILURE;
    }
    for note in &matches {
        print_path(note);
    }
    ExitCode::SUCCESS
}

/// Opens a note in the editor, without taking the notebook's lock and
/// without creating anything. The editor chain matches `config open`,
/// but the config load does not: a broken config is an error here, since
/// resolving a daily note needs the daily keys, while `config open`
/// stays lenient so a broken config file can be opened and fixed.
fn open_note(target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let command = config
        .editor
        .or_else(|| editor_env("VISUAL"))
        .or_else(|| editor_env("EDITOR"));
    dispatch(
        target,
        root,
        None,
        false,
        |note, _guard, _seed| match kladde::editor::open(note.as_path(), command.as_deref()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => fail(error),
        },
    )
}

/// Creates a note under the notebook's lock, seeded and stamped like any
/// other creating write; an existing note is a success left untouched.
fn new_note(target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let stamping = stamping(config);
    dispatch(target, root, Some(&locks), true, |note, guard, seed| {
        let guard = guard.expect("write dispatch locks the notebook");
        let seed = match seeding(&seed) {
            Ok(seed) => seed,
            Err(message) => return fail(message),
        };
        let timestamp = stamping.timestamp(note);
        match guard.create(seed, stamping.stamp(timestamp.as_deref()).as_ref()) {
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
        } => frontmatter_edit(target, notebook.notebook, true, |current| {
            kladde::frontmatter::set(current, &key, &value)
        }),
        FrontmatterCommand::Unset {
            key,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, false, |current| {
            kladde::frontmatter::unset(current, &key)
        }),
        FrontmatterCommand::Add {
            key,
            item,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, true, |current| {
            kladde::frontmatter::add(current, &key, &item)
        }),
        FrontmatterCommand::Remove {
            key,
            item,
            target,
            notebook,
        } => frontmatter_edit(target, notebook.notebook, false, |current| {
            kladde::frontmatter::remove(current, &key, &item)
        }),
    }
}

/// Prints a property's value without taking the notebook's lock: the
/// atomic replace means a reader never sees a half-written note, and a
/// read must not create or wait for anything.
fn frontmatter_get(key: &str, target: TargetArgs, flag: Option<PathBuf>) -> ExitCode {
    dispatch(target, flag, None, false, |note, _guard, _seed| {
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
/// bump its updated stamp — except that an edit that `creates` (set and
/// add, whose outcome is a readable property) must leave the note
/// existing: when a template seed already satisfies it, the seed is
/// written as a creation rather than skipped.
fn frontmatter_edit(
    target: TargetArgs,
    flag: Option<PathBuf>,
    creates: bool,
    edit: impl FnOnce(&str) -> Result<String, kladde::frontmatter::Error>,
) -> ExitCode {
    let locks = match locks() {
        Ok(locks) => locks,
        Err(message) => return fail(message),
    };
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
        Err(message) => return fail(message),
    };
    let stamping = stamping(config);
    dispatch(target, root, Some(&locks), true, |note, guard, seed| {
        let guard = guard.expect("write dispatch locks the notebook");
        let seed = match seeding(&seed) {
            Ok(seed) => seed,
            Err(message) => return fail(message),
        };
        let (current, existed) = match guard.current(seed) {
            Ok(current) => current,
            Err(error) => return fail(error),
        };
        let new = match edit(&current) {
            Ok(new) => new,
            Err(error) => return fail(error),
        };
        if new != current || (creates && !existed) {
            let had_block = existed && kladde::frontmatter::has_block(&current);
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

/// Resolves the notebook root and opens the notebook there.
fn opened(flag: Option<PathBuf>) -> Result<kladde::notebook::Notebook, String> {
    let root = notebook_root(flag)?;
    kladde::notebook::Notebook::open(&root).map_err(|error| error.to_string())
}

/// Resolves the notebook root, opens it, takes its lock when `locks`
/// asks for one, and runs `act` on the note (and seed) `resolve` picks
/// inside it. Resolution runs after the lock is taken, so a seed read
/// from a template file is serialized like the note itself.
fn with_notebook(
    flag: Option<PathBuf>,
    locks: Option<&Path>,
    resolve: impl FnOnce(&kladde::notebook::Notebook) -> Result<(Note, Seed), kladde::notebook::Error>,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>, Seed) -> ExitCode,
) -> ExitCode {
    let notebook = match opened(flag) {
        Ok(notebook) => notebook,
        Err(message) => return fail(message),
    };
    let lock = match locked(locks, notebook.root()) {
        Ok(lock) => lock,
        Err(error) => return fail(error),
    };
    let (note, seed) = match resolve(&notebook) {
        Ok(resolved) => resolved,
        Err(error) => return fail(error),
    };
    let guard = lock.as_ref().map(|lock| lock.guard(&note));
    act(&note, guard.as_ref(), seed)
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
    seeded: bool,
    act: impl FnOnce(&Note, Option<&kladde::write::Guard>, Seed) -> ExitCode,
) -> ExitCode {
    let (config, root) = match layered_config(flag) {
        Ok(loaded) => loaded,
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
    let Some(root) = root else {
        return fail(NO_NOTEBOOK);
    };
    let format = config.daily_date_format.unwrap_or_default();
    with_notebook(
        Some(root),
        locks,
        |notebook| {
            let note = notebook.daily(day, config.daily_folder.as_deref(), &format)?;
            let seed = daily_seed(
                notebook,
                config.daily_template.as_deref(),
                locks.is_some() && seeded,
                day,
                &note,
            );
            Ok((note, seed))
        },
        act,
    )
}

/// The seed for a missing daily note: the configured template's
/// contents, rendered for the note's date. `None` without a configured
/// template; `None` unless a creating write asks, since only such a
/// write can need a seed, so reads and edits that never create leave
/// the template file untouched; and `None` when the note already
/// exists, so the template is read only by the write that will use it.
/// The existence probe runs under the write lock, like the
/// write it feeds. A template that cannot be resolved, read, or
/// rendered becomes the error the write surfaces.
fn daily_seed(
    notebook: &kladde::notebook::Notebook,
    template: Option<&Path>,
    writing: bool,
    date: jiff::civil::Date,
    note: &Note,
) -> Seed {
    let template = template.filter(|_| writing && !note.as_path().exists())?;
    let resolved = match notebook.note(template) {
        Ok(resolved) => resolved,
        Err(error) => return Some(Err(error.to_string())),
    };
    let text = match std::fs::read_to_string(resolved.as_path()) {
        Ok(text) => text,
        Err(error) => {
            return Some(Err(format!(
                "cannot read template {}: {error}",
                resolved.as_path().display()
            )));
        }
    };
    let time = jiff::Zoned::now().time();
    Some(
        kladde::template::rendered(&text, date, time, &title(note))
            .map_err(|error| error.to_string()),
    )
}

/// The note's title, the value `{{title}}` renders to: the file name
/// without its `.md` extension.
fn title(note: &Note) -> String {
    note.as_path()
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Resolves the seed for a write: a seed exists only when the note was
/// missing under this lock, so a broken template fails exactly the
/// write that would have needed it.
fn seeding(seed: &Seed) -> Result<Option<&str>, &str> {
    match seed {
        None => Ok(None),
        Some(Ok(rendered)) => Ok(Some(rendered)),
        Some(Err(message)) => Err(message),
    }
}

/// The base config file, strictly loaded: a broken config fails the
/// command rather than silently dropping keys. A missing file is an
/// empty config; an unlocatable config directory is one too when
/// `--notebook` pins the notebook, and an error otherwise, since the
/// default notebook could only come from config.
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

/// The config a command draws optional keys from: the base config with
/// the notebook's own `.kladde.toml` layered over it. The daily note
/// keys and the stamp keys have no flag override, so even an explicit
/// `--notebook` reads both files, and a broken one fails the command
/// rather than silently dropping keys. Also returns the notebook root
/// the layering used, `None` when nothing names one; the command then
/// fails wherever it needs a notebook.
fn layered_config(
    flag: Option<PathBuf>,
) -> Result<(kladde::config::Config, Option<PathBuf>), String> {
    let config = loaded_config(flag.is_some())?;
    let Some(root) = flag.or_else(|| config.default_notebook.clone()) else {
        return Ok((config, None));
    };
    let notebook = kladde::notebook::Notebook::open(&root).map_err(|error| error.to_string())?;
    let file = notebook.config_file().map_err(|error| error.to_string())?;
    let overlay = kladde::config::load_notebook(&file).map_err(|error| error.to_string())?;
    Ok((config.layered(overlay), Some(root)))
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
    let flag = match &command {
        ConfigCommand::Path { notebook }
        | ConfigCommand::Open { notebook }
        | ConfigCommand::Get { notebook, .. }
        | ConfigCommand::Set { notebook, .. }
        | ConfigCommand::Unset { notebook, .. } => notebook.notebook.clone(),
    };
    let (file, in_notebook) = match config_target(flag) {
        Ok(target) => target,
        Err(message) => return fail(message),
    };
    match command {
        ConfigCommand::Path { .. } => {
            print_path(&file);
            ExitCode::SUCCESS
        }
        ConfigCommand::Open { .. } => open(&file, in_notebook),
        ConfigCommand::Get { key, .. } => get(&file, key, in_notebook),
        ConfigCommand::Set { key, value, .. } => set(&file, key, &value, in_notebook),
        ConfigCommand::Unset { key, .. } => finish(kladde::config::unset(&file, key.name())),
    }
}

/// The config file a config subcommand targets, and whether it is a
/// notebook's own: with `--notebook`, that notebook's config file (the
/// notebook must exist); the base config file otherwise.
fn config_target(flag: Option<PathBuf>) -> Result<(PathBuf, bool), String> {
    if let Some(root) = flag {
        let notebook =
            kladde::notebook::Notebook::open(&root).map_err(|error| error.to_string())?;
        let file = notebook.config_file().map_err(|error| error.to_string())?;
        return Ok((file, true));
    }
    let file = kladde::config::file(
        env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
        env::var_os("HOME").map(PathBuf::from),
    )
    .ok_or_else(|| NO_CONFIG_DIR.to_owned())?;
    Ok((file, false))
}

/// The refusal for a machine-scoped key aimed at a notebook config,
/// `None` when the key is allowed there.
fn barred(key: ConfigKey, in_notebook: bool) -> Option<String> {
    (in_notebook && kladde::config::machine_scoped(key.name())).then(|| {
        format!(
            "`{}` is machine-scoped: a notebook config cannot hold it",
            key.name()
        )
    })
}

fn get(file: &Path, key: ConfigKey, in_notebook: bool) -> ExitCode {
    if let Some(message) = barred(key, in_notebook) {
        return fail(message);
    }
    let loaded = if in_notebook {
        kladde::config::load_notebook(file)
    } else {
        kladde::config::load(file)
    };
    let config = match loaded {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    match key {
        ConfigKey::DefaultNotebook => printed(config.default_notebook, |path| print_path(path)),
        ConfigKey::Editor => printed(config.editor, |editor| println!("{editor}")),
        ConfigKey::DailyFolder => printed(config.daily_folder, |folder| print_path(folder)),
        ConfigKey::DailyDateFormat => printed(config.daily_date_format, |format| {
            println!("{}", format.as_str());
        }),
        ConfigKey::DailyTemplate => printed(config.daily_template, |template| print_path(template)),
        ConfigKey::Stamp => printed(config.stamp, |stamp| println!("{stamp}")),
        ConfigKey::StampCreatedKey => printed(config.stamp_created_key, |key| println!("{key}")),
        ConfigKey::StampUpdatedKey => printed(config.stamp_updated_key, |key| println!("{key}")),
        ConfigKey::StampFormat => printed(config.stamp_format, |format| {
            println!("{}", format.as_str());
        }),
        ConfigKey::StampExclude => printed(config.stamp_exclude, |entries| {
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
        }),
        ConfigKey::BulletIndent => printed(config.bullet_indent, |indent| {
            println!("{}", indent.as_str());
        }),
    }
}

fn set(file: &Path, key: ConfigKey, value: &str, in_notebook: bool) -> ExitCode {
    if let Some(message) = barred(key, in_notebook) {
        return fail(message);
    }
    match key {
        ConfigKey::DefaultNotebook => match std::path::absolute(value) {
            Ok(notebook) => finish(kladde::config::set_default_notebook(file, &notebook)),
            Err(error) => fail(format!("invalid path \"{value}\": {error}")),
        },
        ConfigKey::Editor => finish(kladde::config::set_editor(file, value)),
        ConfigKey::DailyFolder => finish(kladde::config::set_daily_folder(file, value)),
        ConfigKey::DailyDateFormat => finish(kladde::config::set_daily_date_format(file, value)),
        ConfigKey::DailyTemplate => finish(kladde::config::set_daily_template(file, value)),
        ConfigKey::Stamp => finish(kladde::config::set_stamp(file, value)),
        ConfigKey::StampCreatedKey => finish(kladde::config::set_stamp_created_key(file, value)),
        ConfigKey::StampUpdatedKey => finish(kladde::config::set_stamp_updated_key(file, value)),
        ConfigKey::StampFormat => finish(kladde::config::set_stamp_format(file, value)),
        ConfigKey::StampExclude => finish(kladde::config::set_stamp_exclude(file, value)),
        ConfigKey::BulletIndent => finish(kladde::config::set_bullet_indent(file, value)),
    }
}

/// Prints a config value when it is set; an unset key prints nothing and
/// fails, so scripts can tell set from unset.
fn printed<T>(value: Option<T>, print: impl FnOnce(&T)) -> ExitCode {
    match value {
        Some(value) => {
            print(&value);
            ExitCode::SUCCESS
        }
        None => ExitCode::FAILURE,
    }
}

/// Opens `file` in the editor. The editor comes from the base config,
/// which a notebook config cannot override; the load stays lenient so
/// a broken config can be opened and fixed.
fn open(file: &Path, in_notebook: bool) -> ExitCode {
    let source = if in_notebook {
        kladde::config::file(
            env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            env::var_os("HOME").map(PathBuf::from),
        )
    } else {
        Some(file.to_path_buf())
    };
    let configured = source.and_then(|source| match kladde::config::load(&source) {
        Ok(config) => config.editor,
        Err(error) => {
            eprintln!("kladde: ignoring invalid config: {error}");
            None
        }
    });
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

/// An editor command from the environment. A value that parses to no
/// program — blank, or quoted emptiness like `''` — counts as unset, so
/// the chain falls through to the next candidate; a value that cannot
/// be parsed at all is kept, to fail loudly rather than be skipped
/// silently.
fn editor_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| {
        shell_words::split(value).map_or(true, |words| {
            words.first().is_some_and(|program| !program.is_empty())
        })
    })
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
    use std::os::unix::ffi::OsStrExt;
    let mut bytes = path.as_os_str().as_bytes().to_vec();
    bytes.push(b'\n');
    write_stdout(&bytes);
}

#[cfg(windows)]
fn print_path(path: &Path) {
    write_stdout(format!("{}\n", path.display()).as_bytes());
}

/// Prints text exactly as given.
fn print_contents(contents: &str) {
    write_stdout(contents.as_bytes());
}

/// Writes to stdout, flushed, so a final line without a newline still
/// leaves the buffer. A broken pipe means the reader stopped listening —
/// `kladde read note.md | head` — which is the reader's call: the
/// process ends quietly as a success. Any other stdout failure has no
/// better report channel than the panic.
#[cfg(unix)]
fn write_stdout(bytes: &[u8]) {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let result = stdout.write_all(bytes).and_then(|()| stdout.flush());
    let broken = matches!(&result, Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe);
    if broken {
        std::process::exit(0);
    }
    result.expect("stdout writes");
}

/// The Windows twin keeps the plain panic: a probe there cannot even
/// observe a broken pipe — a write with the pipe's read end closed
/// succeeds — so a quiet-exit path would be untestable dead code.
#[cfg(windows)]
fn write_stdout(bytes: &[u8]) {
    use std::io::Write;
    let mut stdout = std::io::stdout();
    let result = stdout.write_all(bytes).and_then(|()| stdout.flush());
    result.expect("stdout writes");
}

fn fail(message: impl Display) -> ExitCode {
    eprintln!("kladde: {message}");
    ExitCode::FAILURE
}

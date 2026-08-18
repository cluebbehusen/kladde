//! Locating, loading, and editing kladde's configuration.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use toml_edit::DocumentMut;

use crate::day;
use crate::structure;

/// Config key naming the notebook used when a command is not given an
/// explicit notebook.
pub const DEFAULT_NOTEBOOK: &str = "default-notebook";

/// Config key naming the command that opens files in an editor.
pub const EDITOR: &str = "editor";

/// Config key naming the folder, inside the notebook, that holds daily
/// notes.
pub const DAILY_FOLDER: &str = "daily-folder";

/// Config key naming the strftime format for daily note file names.
pub const DAILY_DATE_FORMAT: &str = "daily-date-format";

/// Config key naming the note, inside the notebook, that seeds a daily
/// note kladde creates.
pub const DAILY_TEMPLATE: &str = "daily-template";

/// Config key switching created/updated stamping on or off.
pub const STAMP: &str = "stamp";

/// Config key naming the property that records when kladde created a
/// note's frontmatter block.
pub const STAMP_CREATED_KEY: &str = "stamp-created-key";

/// Config key naming the property that records when kladde last wrote
/// the note.
pub const STAMP_UPDATED_KEY: &str = "stamp-updated-key";

/// Config key naming the strftime format for stamp values.
pub const STAMP_FORMAT: &str = "stamp-format";

/// Config key listing notebook-relative paths whose notes are never
/// stamped.
pub const STAMP_EXCLUDE: &str = "stamp-exclude";

/// Config key choosing the indent unit for entries nested under a
/// childless bullet.
pub const BULLET_INDENT: &str = "bullet-indent";

/// Settings read from the config file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Notebook used when a command is not given an explicit notebook.
    pub default_notebook: Option<PathBuf>,
    /// Command that opens files in an editor, parsed with shell quoting
    /// rules when run.
    pub editor: Option<String>,
    /// Folder inside the notebook that holds daily notes; unset means the
    /// notebook root.
    pub daily_folder: Option<PathBuf>,
    /// strftime format for daily note file names; unset means
    /// [`day::DEFAULT_FORMAT`].
    pub daily_date_format: Option<day::Format>,
    /// Notebook-relative path of the note that seeds a daily note kladde
    /// creates; unset means daily notes start empty.
    pub daily_template: Option<PathBuf>,
    /// Whether writes stamp created/updated properties; unset means true.
    pub stamp: Option<bool>,
    /// Property name for the created stamp; unset means `created`.
    pub stamp_created_key: Option<String>,
    /// Property name for the updated stamp; unset means `updated`.
    pub stamp_updated_key: Option<String>,
    /// strftime format for stamp values; unset means
    /// [`day::DEFAULT_STAMP_FORMAT`].
    pub stamp_format: Option<day::StampFormat>,
    /// Notebook-relative paths whose notes are never stamped; unset means
    /// none.
    pub stamp_exclude: Option<Vec<PathBuf>>,
    /// Indent unit for entries nested under a childless bullet; unset
    /// means [`structure::Indent::Tab`].
    pub bullet_indent: Option<structure::Indent>,
}

impl Config {
    /// This config with `notebook`'s notebook-scoped keys layered over it:
    /// a key set in both takes the notebook's value whole, so list values
    /// replace rather than merge. The machine-scoped keys always keep this
    /// config's values; [`load_notebook`] cannot produce them, so only a
    /// hand-built `Config` could hold any.
    #[must_use]
    pub fn layered(self, notebook: Config) -> Config {
        Config {
            default_notebook: self.default_notebook,
            editor: self.editor,
            daily_folder: notebook.daily_folder.or(self.daily_folder),
            daily_date_format: notebook.daily_date_format.or(self.daily_date_format),
            daily_template: notebook.daily_template.or(self.daily_template),
            stamp: notebook.stamp.or(self.stamp),
            stamp_created_key: notebook.stamp_created_key.or(self.stamp_created_key),
            stamp_updated_key: notebook.stamp_updated_key.or(self.stamp_updated_key),
            stamp_format: notebook.stamp_format.or(self.stamp_format),
            stamp_exclude: notebook.stamp_exclude.or(self.stamp_exclude),
            bullet_indent: notebook.bullet_indent.or(self.bullet_indent),
        }
    }
}

/// Failure while reading, validating, or writing the config file.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot read {}: {cause}", path.display())]
    Read {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("invalid TOML in {}: {cause}", path.display())]
    Parse {
        path: PathBuf,
        cause: toml_edit::TomlError,
    },
    #[error("unknown key `{key}` in {}", path.display())]
    UnknownKey { path: PathBuf, key: String },
    #[error("the notebook config {} sets `{key}`, which is machine-scoped", path.display())]
    MachineKey { path: PathBuf, key: String },
    #[error("in notebook config {}: {cause}", path.display())]
    NotebookValue { path: PathBuf, cause: Box<Error> },
    #[error("`{key}` must be a string")]
    NotAString { key: &'static str },
    #[error("`{key}` must be an absolute path, got \"{value}\"")]
    NotAbsolute { key: &'static str, value: String },
    #[error("`editor` must contain a command")]
    EmptyEditor,
    #[error("`editor`: {cause}")]
    UnparsableEditor { cause: shell_words::ParseError },
    #[error("`{key}` must be a relative path, got \"{value}\"")]
    NotRelative { key: &'static str, value: String },
    #[error("`daily-folder` must not be empty")]
    EmptyDailyFolder,
    #[error("`daily-template` must not be empty")]
    EmptyDailyTemplate,
    #[error("`daily-date-format`: {cause}")]
    InvalidDateFormat { cause: day::Error },
    #[error("`{key}` must be true or false")]
    NotABoolean { key: &'static str },
    #[error("`{key}`: {cause}")]
    InvalidStampKey {
        key: &'static str,
        cause: crate::frontmatter::Error,
    },
    #[error("`stamp-format`: {cause}")]
    InvalidStampFormat { cause: day::Error },
    #[error("`{key}` must be an array of strings")]
    NotAnArray { key: &'static str },
    #[error("`stamp-exclude` entries must be relative paths, got \"{value}\"")]
    RootedExclude { value: String },
    #[error("`stamp-exclude` entries must not be empty")]
    EmptyExclude,
    #[error("`stamp-exclude` entries must name a place inside the notebook, got \"{value}\"")]
    EscapingExclude { value: String },
    #[error("`bullet-indent` must be \"tab\" or \"spaces\"")]
    InvalidBulletIndent,
    #[error("not a directory: {}", path.display())]
    NotADirectory { path: PathBuf },
    #[error("cannot create {}: {cause}", path.display())]
    CreateDir {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("cannot write {}: {cause}", path.display())]
    Write {
        path: PathBuf,
        cause: std::io::Error,
    },
}

/// File name of a notebook's own config, at the notebook root. The dot
/// prefix keeps it invisible to note listing and name lookup.
pub const NOTEBOOK_FILE: &str = ".kladde.toml";

/// Whether `key` may only be set in the base config, never in a notebook
/// config: a notebook that syncs between machines is data, so it must not
/// choose the machine's notebook or supply commands kladde executes.
#[must_use]
pub fn machine_scoped(key: &str) -> bool {
    matches!(key, DEFAULT_NOTEBOOK | EDITOR)
}

/// Directory holding kladde's configuration, following the XDG base directory
/// convention on every platform: `$XDG_CONFIG_HOME/kladde`, falling back to
/// `$HOME/.config/kladde`. As the XDG specification requires, a relative path
/// (which includes an empty one) counts as unset.
///
/// Returns `None` when neither variable provides an absolute base.
#[must_use]
pub fn dir(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg_config_home
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .map(|config_base| config_base.join("kladde"))
}

/// Path of the config file inside [`dir`].
#[must_use]
pub fn file(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    dir(xdg_config_home, home).map(|config_dir| config_dir.join("config.toml"))
}

/// Reads and validates the config file. A missing file is an empty config,
/// not an error.
///
/// # Errors
///
/// Returns an error when the file cannot be read, is not valid TOML, contains
/// an unknown key, or holds a value of the wrong shape: `default-notebook`
/// must be an absolute path, `editor` must be a shell-parsable command,
/// `daily-folder` must be a non-empty relative path, and
/// `daily-date-format` must render a date.
pub fn load(file: &Path) -> Result<Config, Error> {
    parsed(file, false)
}

/// Reads and validates a notebook's own config file, with the same keys,
/// shapes, and missing-file behavior as [`load`], except that
/// machine-scoped keys are rejected.
///
/// # Errors
///
/// Returns an error in every case [`load`] does, and when the file sets a
/// machine-scoped key. A value error is wrapped with the file's path,
/// because by the time a notebook config loads there are two files a bad
/// value could live in.
pub fn load_notebook(file: &Path) -> Result<Config, Error> {
    parsed(file, true).map_err(|error| match error {
        error @ (Error::Read { .. }
        | Error::Parse { .. }
        | Error::UnknownKey { .. }
        | Error::MachineKey { .. }) => error,
        cause => Error::NotebookValue {
            path: file.to_owned(),
            cause: Box::new(cause),
        },
    })
}

/// The shared reader behind [`load`] and [`load_notebook`]: one set of
/// per-key parse arms, with the machine-scoped rejection ahead of them
/// when `notebook` is set.
fn parsed(file: &Path, notebook: bool) -> Result<Config, Error> {
    let document = read_document(file)?;
    let mut config = Config::default();
    for (key, item) in document.iter() {
        match key {
            key if notebook && machine_scoped(key) => {
                return Err(Error::MachineKey {
                    path: file.to_owned(),
                    key: key.to_owned(),
                });
            }
            DEFAULT_NOTEBOOK => {
                let value = item.as_str().ok_or(Error::NotAString {
                    key: DEFAULT_NOTEBOOK,
                })?;
                let path = PathBuf::from(value);
                if path.is_relative() {
                    return Err(Error::NotAbsolute {
                        key: DEFAULT_NOTEBOOK,
                        value: value.to_owned(),
                    });
                }
                config.default_notebook = Some(path);
            }
            EDITOR => {
                let value = item.as_str().ok_or(Error::NotAString { key: EDITOR })?;
                validated_editor(value)?;
                config.editor = Some(value.to_owned());
            }
            DAILY_FOLDER => {
                let value = item
                    .as_str()
                    .ok_or(Error::NotAString { key: DAILY_FOLDER })?;
                config.daily_folder = Some(validated_daily_folder(value)?);
            }
            DAILY_DATE_FORMAT => {
                let value = item.as_str().ok_or(Error::NotAString {
                    key: DAILY_DATE_FORMAT,
                })?;
                let format =
                    day::Format::new(value).map_err(|cause| Error::InvalidDateFormat { cause })?;
                config.daily_date_format = Some(format);
            }
            DAILY_TEMPLATE => {
                let value = item.as_str().ok_or(Error::NotAString {
                    key: DAILY_TEMPLATE,
                })?;
                config.daily_template = Some(validated_daily_template(value)?);
            }
            STAMP => {
                config.stamp = Some(item.as_bool().ok_or(Error::NotABoolean { key: STAMP })?);
            }
            STAMP_CREATED_KEY => {
                let value = item.as_str().ok_or(Error::NotAString {
                    key: STAMP_CREATED_KEY,
                })?;
                config.stamp_created_key = Some(validated_stamp_key(STAMP_CREATED_KEY, value)?);
            }
            STAMP_UPDATED_KEY => {
                let value = item.as_str().ok_or(Error::NotAString {
                    key: STAMP_UPDATED_KEY,
                })?;
                config.stamp_updated_key = Some(validated_stamp_key(STAMP_UPDATED_KEY, value)?);
            }
            STAMP_FORMAT => {
                let value = item
                    .as_str()
                    .ok_or(Error::NotAString { key: STAMP_FORMAT })?;
                let format = day::StampFormat::new(value)
                    .map_err(|cause| Error::InvalidStampFormat { cause })?;
                config.stamp_format = Some(format);
            }
            STAMP_EXCLUDE => {
                let array = item
                    .as_array()
                    .ok_or(Error::NotAnArray { key: STAMP_EXCLUDE })?;
                let mut entries = Vec::new();
                for element in array {
                    let value = element
                        .as_str()
                        .ok_or(Error::NotAnArray { key: STAMP_EXCLUDE })?;
                    entries.push(validated_exclude(value)?);
                }
                config.stamp_exclude = Some(entries);
            }
            BULLET_INDENT => {
                let value = item
                    .as_str()
                    .ok_or(Error::NotAString { key: BULLET_INDENT })?;
                config.bullet_indent = Some(validated_bullet_indent(value)?);
            }
            unknown => {
                return Err(Error::UnknownKey {
                    path: file.to_owned(),
                    key: unknown.to_owned(),
                });
            }
        }
    }
    Ok(config)
}

/// Stores `notebook` under the `default-notebook` key, creating the config
/// file and its directory if needed and preserving the rest of the file,
/// comments included. The caller passes an absolute path; `notebook` must be
/// an existing directory.
///
/// # Errors
///
/// Returns an error when `notebook` is not a directory, or when the config
/// file cannot be read, parsed, or written back.
///
/// # Panics
///
/// Panics when `notebook` is not valid Unicode, which TOML cannot store. The
/// binary can only produce such a path from a non-Unicode working directory.
pub fn set_default_notebook(file: &Path, notebook: &Path) -> Result<(), Error> {
    if !notebook.is_dir() {
        return Err(Error::NotADirectory {
            path: notebook.to_owned(),
        });
    }
    let value = notebook
        .to_str()
        .expect("notebook paths are valid Unicode; TOML cannot store other bytes");
    let mut document = read_document(file)?;
    document[DEFAULT_NOTEBOOK] = toml_edit::value(value);
    save(file, &document)
}

/// Stores `editor` under the `editor` key, creating the config file and its
/// directory if needed and preserving the rest of the file, comments
/// included.
///
/// # Errors
///
/// Returns an error when `editor` contains no command or does not parse
/// under shell quoting rules, or when the config file cannot be read,
/// parsed, or written back.
pub fn set_editor(file: &Path, editor: &str) -> Result<(), Error> {
    validated_editor(editor)?;
    let mut document = read_document(file)?;
    document[EDITOR] = toml_edit::value(editor);
    save(file, &document)
}

/// Stores `folder` under the `daily-folder` key, creating the config file
/// and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `folder` is empty or rooted, or when the config
/// file cannot be read, parsed, or written back.
pub fn set_daily_folder(file: &Path, folder: &str) -> Result<(), Error> {
    validated_daily_folder(folder)?;
    let mut document = read_document(file)?;
    document[DAILY_FOLDER] = toml_edit::value(folder);
    save(file, &document)
}

/// Stores `format` under the `daily-date-format` key, creating the config
/// file and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `format` cannot render a date or renders an empty
/// file name, or when the config file cannot be read, parsed, or written
/// back.
pub fn set_daily_date_format(file: &Path, format: &str) -> Result<(), Error> {
    day::Format::new(format).map_err(|cause| Error::InvalidDateFormat { cause })?;
    let mut document = read_document(file)?;
    document[DAILY_DATE_FORMAT] = toml_edit::value(format);
    save(file, &document)
}

/// Stores `template` under the `daily-template` key, creating the config
/// file and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `template` is empty or rooted, or when the
/// config file cannot be read, parsed, or written back.
pub fn set_daily_template(file: &Path, template: &str) -> Result<(), Error> {
    validated_daily_template(template)?;
    let mut document = read_document(file)?;
    document[DAILY_TEMPLATE] = toml_edit::value(template);
    save(file, &document)
}

/// Stores `value` under the `stamp` key, creating the config file and its
/// directory if needed and preserving the rest of the file, comments
/// included.
///
/// # Errors
///
/// Returns an error when `value` is not `true` or `false`, or when the
/// config file cannot be read, parsed, or written back.
pub fn set_stamp(file: &Path, value: &str) -> Result<(), Error> {
    let stamp = match value {
        "true" => true,
        "false" => false,
        _ => return Err(Error::NotABoolean { key: STAMP }),
    };
    let mut document = read_document(file)?;
    document[STAMP] = toml_edit::value(stamp);
    save(file, &document)
}

/// Stores `value` under the `stamp-created-key` key, creating the config
/// file and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `value` cannot name a property, or when the
/// config file cannot be read, parsed, or written back.
pub fn set_stamp_created_key(file: &Path, value: &str) -> Result<(), Error> {
    validated_stamp_key(STAMP_CREATED_KEY, value)?;
    let mut document = read_document(file)?;
    document[STAMP_CREATED_KEY] = toml_edit::value(value);
    save(file, &document)
}

/// Stores `value` under the `stamp-updated-key` key, creating the config
/// file and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `value` cannot name a property, or when the
/// config file cannot be read, parsed, or written back.
pub fn set_stamp_updated_key(file: &Path, value: &str) -> Result<(), Error> {
    validated_stamp_key(STAMP_UPDATED_KEY, value)?;
    let mut document = read_document(file)?;
    document[STAMP_UPDATED_KEY] = toml_edit::value(value);
    save(file, &document)
}

/// Stores `value` under the `stamp-format` key, creating the config file
/// and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `value` cannot render a datetime to a non-empty
/// single line, or when the config file cannot be read, parsed, or
/// written back.
pub fn set_stamp_format(file: &Path, value: &str) -> Result<(), Error> {
    day::StampFormat::new(value).map_err(|cause| Error::InvalidStampFormat { cause })?;
    let mut document = read_document(file)?;
    document[STAMP_FORMAT] = toml_edit::value(value);
    save(file, &document)
}

/// Stores the comma-separated `value` under the `stamp-exclude` key as a
/// TOML array, creating the config file and its directory if needed and
/// preserving the rest of the file, comments included. Whitespace around
/// each entry is dropped; an entry containing a comma can only be written
/// by editing the file directly.
///
/// # Errors
///
/// Returns an error when an entry is empty or rooted, or when the config
/// file cannot be read, parsed, or written back.
pub fn set_stamp_exclude(file: &Path, value: &str) -> Result<(), Error> {
    let mut array = toml_edit::Array::new();
    for entry in value.split(',') {
        let entry = entry.trim();
        validated_exclude(entry)?;
        array.push(entry);
    }
    let mut document = read_document(file)?;
    document[STAMP_EXCLUDE] = toml_edit::value(array);
    save(file, &document)
}

/// Stores `value` under the `bullet-indent` key, creating the config file
/// and its directory if needed and preserving the rest of the file,
/// comments included.
///
/// # Errors
///
/// Returns an error when `value` is not `tab` or `spaces`, or when the
/// config file cannot be read, parsed, or written back.
pub fn set_bullet_indent(file: &Path, value: &str) -> Result<(), Error> {
    validated_bullet_indent(value)?;
    let mut document = read_document(file)?;
    document[BULLET_INDENT] = toml_edit::value(value);
    save(file, &document)
}

/// An `editor` value must parse under the shell quoting rules
/// [`crate::editor::open`] applies, and its first word — the program — must
/// not be empty, which is also how a blank value is rejected.
fn validated_editor(value: &str) -> Result<(), Error> {
    let words = shell_words::split(value).map_err(|cause| Error::UnparsableEditor { cause })?;
    match words.first() {
        Some(program) if !program.is_empty() => Ok(()),
        _ => Err(Error::EmptyEditor),
    }
}

/// A `bullet-indent` value names one of the two indent units.
fn validated_bullet_indent(value: &str) -> Result<structure::Indent, Error> {
    match value {
        "tab" => Ok(structure::Indent::Tab),
        "spaces" => Ok(structure::Indent::Spaces),
        _ => Err(Error::InvalidBulletIndent),
    }
}

/// A stamp key must be writable as a property by the frontmatter module,
/// which shares its key rules with every property edit.
fn validated_stamp_key(key: &'static str, value: &str) -> Result<String, Error> {
    crate::frontmatter::validated_key(value)
        .map_err(|cause| Error::InvalidStampKey { key, cause })?;
    Ok(value.to_owned())
}

/// A `stamp-exclude` entry must name a place inside the notebook, so it
/// has to be non-empty and relative, like `daily-folder`. Entries are
/// normalized to plain components, because exclusion matches a note's
/// notebook-relative path component by component: a `./` would never
/// match anything, and a `..` could only name a place outside.
fn validated_exclude(value: &str) -> Result<PathBuf, Error> {
    if value.is_empty() {
        return Err(Error::EmptyExclude);
    }
    let path = PathBuf::from(value);
    if path.has_root() {
        return Err(Error::RootedExclude {
            value: value.to_owned(),
        });
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            _ => {
                return Err(Error::EscapingExclude {
                    value: value.to_owned(),
                });
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Err(Error::EscapingExclude {
            value: value.to_owned(),
        });
    }
    Ok(normalized)
}

/// A `daily-folder` value must name a place inside the notebook, so it has
/// to be non-empty and relative; containment proper is enforced when the
/// folder is resolved against a notebook.
fn validated_daily_folder(value: &str) -> Result<PathBuf, Error> {
    if value.is_empty() {
        return Err(Error::EmptyDailyFolder);
    }
    let folder = PathBuf::from(value);
    if folder.has_root() {
        return Err(Error::NotRelative {
            key: DAILY_FOLDER,
            value: value.to_owned(),
        });
    }
    Ok(folder)
}

/// A `daily-template` value must name a note inside the notebook, so it
/// has to be non-empty and relative; containment proper is enforced when
/// the template is resolved against a notebook.
fn validated_daily_template(value: &str) -> Result<PathBuf, Error> {
    if value.is_empty() {
        return Err(Error::EmptyDailyTemplate);
    }
    let template = PathBuf::from(value);
    if template.has_root() {
        return Err(Error::NotRelative {
            key: DAILY_TEMPLATE,
            value: value.to_owned(),
        });
    }
    Ok(template)
}

/// Removes `key` from the config file, preserving the rest of the file,
/// comments included. Removing an absent key is a success that leaves the
/// file untouched and never creates one.
///
/// # Errors
///
/// Returns an error when the config file cannot be read, parsed, or written
/// back.
pub fn unset(file: &Path, key: &str) -> Result<(), Error> {
    let mut document = read_document(file)?;
    if document.as_table_mut().remove(key).is_none() {
        return Ok(());
    }
    save(file, &document)
}

/// Creates the directory holding `file`, so that a following write (by
/// kladde or by an editor) can succeed.
///
/// # Errors
///
/// Returns an error when the directory cannot be created.
pub fn ensure_dir(file: &Path) -> Result<(), Error> {
    file.parent().map_or(Ok(()), |config_dir| {
        fs::create_dir_all(config_dir).map_err(|cause| Error::CreateDir {
            path: config_dir.to_owned(),
            cause,
        })
    })
}

fn read_document(file: &Path) -> Result<DocumentMut, Error> {
    let contents = match fs::read_to_string(file) {
        Ok(contents) => contents,
        Err(cause) if cause.kind() == ErrorKind::NotFound => return Ok(DocumentMut::new()),
        Err(cause) => {
            return Err(Error::Read {
                path: file.to_owned(),
                cause,
            });
        }
    };
    contents.parse().map_err(|cause| Error::Parse {
        path: file.to_owned(),
        cause,
    })
}

/// Counts this process's saves, so each gets its own temp file name.
static SAVES: AtomicU64 = AtomicU64::new(0);

/// The path a config write lands at: the canonical location when the
/// entry resolves, otherwise the end of the dangling link chain, so a
/// link installed ahead of the file it manages gets that file created.
/// A chain still unresolved after the last hop is refused; renaming
/// onto a link would replace it.
fn write_target(file: &Path) -> Result<PathBuf, Error> {
    let mut path = file.to_owned();
    for _ in 0..8 {
        if let Ok(canonical) = fs::canonicalize(&path) {
            return Ok(canonical);
        }
        let Ok(target) = fs::read_link(&path) else {
            return Ok(path);
        };
        let parent = path.parent().unwrap_or(&path).to_path_buf();
        path = parent.join(target);
    }
    Err(Error::Write {
        path: file.to_owned(),
        cause: std::io::Error::other("too many levels of links"),
    })
}

/// Discards the temp file this save created, best effort. On Windows a
/// readonly temp would survive a plain remove, so the attribute is
/// cleared first; only here, on a temp the exclusive create proved
/// ours, never on a pre-existing entry.
#[cfg(unix)]
fn discard(temp: &Path) {
    let _ = fs::remove_file(temp);
}

#[cfg(windows)]
fn discard(temp: &Path) {
    if let Ok(metadata) = fs::metadata(temp) {
        let mut permissions = metadata.permissions();
        permissions.set_readonly(false);
        let _ = fs::set_permissions(temp, permissions);
    }
    let _ = fs::remove_file(temp);
}

/// Writes by temp-file-and-rename, like every note write: the entry is
/// replaced, never written through, so a crash cannot tear the file and
/// a hard link cannot carry the write to another inode. The target is
/// resolved through [`write_target`], so a symlinked base config keeps
/// its link. The temp is created exclusively under a hashed per-save
/// name; process ids can collide across containers or hosts, and
/// concurrent saves stay last-writer-wins. Permissions carry over; the
/// temp is removed on failure, best effort.
fn save(file: &Path, document: &DocumentMut) -> Result<(), Error> {
    let file = write_target(file)?;
    ensure_dir(&file)?;
    let temp = file.with_file_name(format!(
        ".{}.{}-{}{}",
        crate::write::hashed(&file),
        std::process::id(),
        SAVES.fetch_add(1, Ordering::Relaxed),
        crate::write::TEMP_SUFFIX
    ));
    let permissions = fs::metadata(&file)
        .ok()
        .map(|metadata| metadata.permissions());
    // A stale leftover (pid reuse) is removed without touching its
    // attributes; if it resists, the exclusive create fails.
    let _ = fs::remove_file(&temp);
    File::options()
        .write(true)
        .create_new(true)
        .open(&temp)
        .and_then(|mut created| {
            if let Some(permissions) = permissions {
                created.set_permissions(permissions)?;
            }
            created.write_all(document.to_string().as_bytes())?;
            created.sync_all()
        })
        .and_then(|()| fs::rename(&temp, &file))
        .map_err(|cause| {
            discard(&temp);
            Error::Write {
                path: file.clone(),
                cause,
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// An absolute path on every platform, so the same tests hold on Windows,
    /// where `/xdg` is not absolute. `#[cfg]` rather than `cfg!` so the other
    /// platform's branch is not compiled in as an uncoverable line.
    #[cfg(not(windows))]
    fn abs(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[cfg(windows)]
    fn abs(value: &str) -> PathBuf {
        PathBuf::from(format!("C:{}", value.replace('/', "\\")))
    }

    fn temp() -> TempDir {
        TempDir::new().expect("temp dir creates")
    }

    fn set_readonly(path: &Path, readonly: bool) {
        let mut permissions = fs::metadata(path).expect("metadata reads").permissions();
        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "restores a test file"
        )]
        permissions.set_readonly(readonly);
        fs::set_permissions(path, permissions).expect("permissions apply");
    }

    #[test]
    fn dir_prefers_xdg_config_home() {
        assert_eq!(
            dir(Some(abs("/xdg")), Some(abs("/home/me"))),
            Some(abs("/xdg/kladde"))
        );
    }

    #[test]
    fn dir_treats_empty_xdg_config_home_as_unset() {
        assert_eq!(
            dir(Some(PathBuf::new()), Some(abs("/home/me"))),
            Some(abs("/home/me/.config/kladde"))
        );
    }

    #[test]
    fn dir_treats_relative_xdg_config_home_as_unset() {
        assert_eq!(
            dir(
                Some(PathBuf::from("relative/config")),
                Some(abs("/home/me"))
            ),
            Some(abs("/home/me/.config/kladde"))
        );
    }

    #[test]
    fn dir_falls_back_to_home() {
        assert_eq!(
            dir(None, Some(abs("/home/me"))),
            Some(abs("/home/me/.config/kladde"))
        );
    }

    #[test]
    fn dir_needs_an_absolute_base() {
        assert_eq!(dir(None, None), None);
        assert_eq!(dir(Some(PathBuf::new()), None), None);
        assert_eq!(dir(None, Some(PathBuf::from("relative"))), None);
    }

    #[test]
    fn file_is_config_toml_inside_dir() {
        assert_eq!(
            file(Some(abs("/xdg")), None),
            Some(abs("/xdg/kladde/config.toml"))
        );
    }

    #[test]
    fn load_returns_empty_config_when_file_missing() {
        let base = temp();
        let config = load(&base.path().join("config.toml")).expect("missing file loads");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_reads_both_keys() {
        let base = temp();
        let file = base.path().join("config.toml");
        let notebook = abs("/notes");
        fs::write(
            &file,
            format!(
                "default-notebook = '{}'\neditor = 'vim'\n",
                notebook.display()
            ),
        )
        .expect("fixture writes");
        let config = load(&file).expect("fixture loads");
        assert_eq!(config.default_notebook, Some(notebook));
        assert_eq!(config.editor, Some("vim".to_owned()));
    }

    #[test]
    fn load_reports_unreadable_file() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::create_dir(&file).expect("obstacle creates");
        let error = load(&file).expect_err("directory does not read");
        assert!(error.to_string().contains("cannot read"));
    }

    #[test]
    fn load_reports_invalid_toml() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "editor = [oops\n").expect("fixture writes");
        let error = load(&file).expect_err("garbage does not parse");
        assert!(error.to_string().contains("invalid TOML"));
    }

    #[test]
    fn load_rejects_unknown_key() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "unknown = 1\n").expect("fixture writes");
        let error = load(&file).expect_err("unknown key fails");
        assert!(error.to_string().contains("unknown key `unknown`"));
    }

    #[test]
    fn load_rejects_non_string_default_notebook() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "default-notebook = 3\n").expect("fixture writes");
        let error = load(&file).expect_err("number fails");
        assert!(
            error
                .to_string()
                .contains("`default-notebook` must be a string")
        );
    }

    #[test]
    fn load_rejects_relative_default_notebook() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "default-notebook = 'notes'\n").expect("fixture writes");
        let error = load(&file).expect_err("relative path fails");
        assert!(error.to_string().contains("must be an absolute path"));
    }

    #[test]
    fn load_rejects_non_string_editor() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "editor = 3\n").expect("fixture writes");
        let error = load(&file).expect_err("number fails");
        assert!(error.to_string().contains("`editor` must be a string"));
    }

    #[test]
    fn load_rejects_blank_editor() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "editor = ' '\n").expect("fixture writes");
        let error = load(&file).expect_err("blank editor fails");
        assert!(
            error
                .to_string()
                .contains("`editor` must contain a command")
        );
    }

    #[test]
    fn load_rejects_unparsable_editor() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "editor = \"'unclosed\"\n").expect("fixture writes");
        let error = load(&file).expect_err("unclosed quote fails");
        assert!(
            error.to_string().contains("`editor`:"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn load_rejects_quoted_empty_editor() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "editor = \"'' --wait\"\n").expect("fixture writes");
        let error = load(&file).expect_err("empty program fails");
        assert!(
            error
                .to_string()
                .contains("`editor` must contain a command")
        );
    }

    #[test]
    fn set_default_notebook_creates_file_and_directories() {
        let base = temp();
        let notebook = temp();
        let file = base.path().join("nested").join("config.toml");
        set_default_notebook(&file, notebook.path()).expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(config.default_notebook, Some(notebook.path().to_owned()));
    }

    #[test]
    fn set_default_notebook_preserves_comments_and_other_keys() {
        let base = temp();
        let notebook = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "# mine\neditor = 'vim'\n").expect("fixture writes");
        set_default_notebook(&file, notebook.path()).expect("set succeeds");
        let contents = fs::read_to_string(&file).expect("written config reads");
        assert!(contents.contains("# mine"));
        assert!(contents.contains("editor = 'vim'"));
        let config = load(&file).expect("written config loads");
        assert_eq!(config.default_notebook, Some(notebook.path().to_owned()));
    }

    #[test]
    fn set_default_notebook_rejects_missing_directory() {
        let base = temp();
        let file = base.path().join("config.toml");
        let missing = base.path().join("nope");
        let error = set_default_notebook(&file, &missing).expect_err("missing dir fails");
        assert!(error.to_string().contains("not a directory"));
        let plain_file = base.path().join("occupied");
        fs::write(&plain_file, "").expect("obstacle writes");
        let error = set_default_notebook(&file, &plain_file).expect_err("plain file fails");
        assert!(error.to_string().contains("not a directory"));
    }

    /// Unix allows renaming over a readonly file, like every note
    /// write; the permissions carry over.
    #[cfg(unix)]
    #[test]
    fn set_default_notebook_replaces_a_readonly_file() {
        let base = temp();
        let notebook = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "").expect("fixture writes");
        set_readonly(&file, true);
        set_default_notebook(&file, notebook.path()).expect("readonly entry is replaceable");
        let config = load(&file).expect("written config loads");
        assert_eq!(config.default_notebook, Some(notebook.path().to_owned()));
        assert!(
            fs::metadata(&file)
                .expect("metadata reads")
                .permissions()
                .readonly()
        );
        set_readonly(&file, false);
    }

    /// Windows refuses to rename over a readonly file, so the write
    /// fails loudly there.
    #[cfg(windows)]
    #[test]
    fn set_default_notebook_reports_unwritable_file() {
        let base = temp();
        let notebook = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "").expect("fixture writes");
        set_readonly(&file, true);
        let error = set_default_notebook(&file, notebook.path()).expect_err("readonly fails");
        assert!(error.to_string().contains("cannot write"));
        set_readonly(&file, false);
        assert_eq!(temp_files(base.path()), 0);
    }

    #[cfg(unix)]
    #[test]
    fn set_default_notebook_reports_an_unwritable_directory() {
        let base = temp();
        let notebook = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "").expect("fixture writes");
        set_readonly(base.path(), true);
        let error = set_default_notebook(&file, notebook.path()).expect_err("readonly dir fails");
        assert!(error.to_string().contains("cannot write"));
        set_readonly(base.path(), false);
        assert_eq!(temp_files(base.path()), 0);
    }

    /// The number of kladde temp files sitting in `dir`.
    fn temp_files(dir: &Path) -> usize {
        fs::read_dir(dir)
            .expect("dir reads")
            .filter(|entry| {
                entry
                    .as_ref()
                    .expect("entry reads")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".kladde-tmp")
            })
            .count()
    }

    #[test]
    fn set_leaves_no_temp_file() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_editor(&file, "vim").expect("set succeeds");
        assert_eq!(temp_files(base.path()), 0);
    }

    /// A link installed ahead of the file it manages keeps its link;
    /// the write creates the missing target, absolute or link-relative.
    #[cfg(unix)]
    #[test]
    fn set_creates_the_target_of_a_dangling_link() {
        let base = temp();
        let managed = temp();
        let absolute = managed.path().join("machine.toml");
        let file = base.path().join("config.toml");
        std::os::unix::fs::symlink(&absolute, &file).expect("symlink creates");
        set_editor(&file, "vim").expect("set succeeds");
        assert!(
            fs::symlink_metadata(&file)
                .expect("metadata reads")
                .is_symlink()
        );
        assert!(
            fs::read_to_string(&absolute)
                .expect("target reads")
                .contains("editor")
        );
        let relative = base.path().join("other.toml");
        std::os::unix::fs::symlink("managed/other.toml", &relative).expect("symlink creates");
        set_editor(&relative, "vim").expect("set succeeds");
        assert!(base.path().join("managed").join("other.toml").exists());
    }

    /// A junction: created while its target exists, so chains can be
    /// built link by link and left dangling by removing the one real
    /// directory at the end.
    #[cfg(windows)]
    fn junction(link: &Path, target: &Path) {
        let cmd = format!(
            "{}\\System32\\cmd.exe",
            std::env::var("SYSTEMROOT").expect("SYSTEMROOT is set on Windows")
        );
        let status = std::process::Command::new(cmd)
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .status()
            .expect("mklink runs");
        assert!(status.success());
    }

    /// The Windows twin of the dangling-link walk, through a junction
    /// whose target directory is gone.
    #[cfg(windows)]
    #[test]
    fn set_creates_the_target_of_a_dangling_junction() {
        let base = temp();
        let managed = temp();
        let target = managed.path().join("gone");
        fs::create_dir(&target).expect("fixture dir creates");
        let file = base.path().join("config.toml");
        junction(&file, &target);
        fs::remove_dir(&target).expect("target removes");
        set_editor(&file, "vim").expect("set succeeds");
        assert!(
            fs::read_to_string(&target)
                .expect("target reads")
                .contains("editor")
        );
    }

    /// Two links installed ahead of the managed file both survive, and
    /// the write creates the file at the chain's end.
    #[cfg(unix)]
    #[test]
    fn set_follows_a_dangling_link_chain() {
        let base = temp();
        let managed = temp();
        let file = base.path().join("config.toml");
        let middle = managed.path().join("middle.toml");
        let target = managed.path().join("final.toml");
        std::os::unix::fs::symlink(&target, &middle).expect("symlink creates");
        std::os::unix::fs::symlink(&middle, &file).expect("symlink creates");
        set_editor(&file, "vim").expect("set succeeds");
        assert!(
            fs::symlink_metadata(&middle)
                .expect("metadata reads")
                .is_symlink()
        );
        assert!(target.exists());
    }

    /// A dangling chain deeper than any real layout is refused; the
    /// rename would replace the link the walk stopped at. A cycle
    /// fails earlier, at the read, with the operating system's error.
    #[cfg(unix)]
    #[test]
    fn set_reports_a_link_chain_too_deep() {
        let base = temp();
        let mut prev = base.path().join("gone.toml");
        for index in 0..8 {
            let link = base.path().join(format!("l{index}.toml"));
            std::os::unix::fs::symlink(&prev, &link).expect("symlink creates");
            prev = link;
        }
        let file = base.path().join("config.toml");
        std::os::unix::fs::symlink(&prev, &file).expect("symlink creates");
        let error = set_editor(&file, "vim").expect_err("deep chain fails");
        assert!(
            error.to_string().contains("too many levels of links"),
            "{error}"
        );
    }

    /// The Windows twin of the cycle refusal: a junction chain deeper
    /// than any real layout.
    #[cfg(windows)]
    #[test]
    fn set_reports_a_junction_chain_too_deep() {
        let base = temp();
        let real = base.path().join("real");
        fs::create_dir(&real).expect("fixture dir creates");
        let mut prev = real.clone();
        for index in 0..8 {
            let link = base.path().join(format!("j{index}"));
            junction(&link, &prev);
            prev = link;
        }
        let file = base.path().join("config.toml");
        junction(&file, &prev);
        fs::remove_dir(&real).expect("target removes");
        let error = set_editor(&file, "vim").expect_err("deep chain fails");
        assert!(
            error.to_string().contains("too many levels of links"),
            "{error}"
        );
    }

    /// A symlinked base config is followed: the link survives and the
    /// managed target receives the write.
    #[cfg(unix)]
    #[test]
    fn set_follows_a_symlinked_config() {
        let base = temp();
        let managed = temp();
        let target = managed.path().join("managed.toml");
        fs::write(&target, "editor = 'vim'\n").expect("fixture writes");
        let file = base.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &file).expect("symlink creates");
        set_daily_folder(&file, "Journal").expect("set succeeds");
        assert!(
            fs::symlink_metadata(&file)
                .expect("metadata reads")
                .is_symlink()
        );
        let contents = fs::read_to_string(&target).expect("target reads");
        assert!(contents.contains("editor = 'vim'"), "{contents}");
        assert!(
            contents.contains("daily-folder = \"Journal\""),
            "{contents}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn set_default_notebook_reports_uncreatable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let base = temp();
        let notebook = temp();
        let file = base.path().join("kladde").join("config.toml");
        fs::set_permissions(base.path(), fs::Permissions::from_mode(0o555))
            .expect("permissions apply");
        let error = set_default_notebook(&file, notebook.path()).expect_err("readonly base fails");
        assert!(error.to_string().contains("cannot create"));
        fs::set_permissions(base.path(), fs::Permissions::from_mode(0o755))
            .expect("permissions restore");
    }

    #[cfg(windows)]
    #[test]
    fn set_default_notebook_reports_uncreatable_directory() {
        let base = temp();
        let notebook = temp();
        fs::write(base.path().join("kladde"), "").expect("obstacle writes");
        let file = base.path().join("kladde").join("config.toml");
        let error = set_default_notebook(&file, notebook.path()).expect_err("obstacle fails");
        assert!(error.to_string().contains("cannot create"));
    }

    /// Only Linux filesystems allow creating a non-Unicode directory; APFS
    /// and NTFS reject or normalize the bytes this test needs.
    #[cfg(target_os = "linux")]
    #[test]
    #[should_panic(expected = "valid Unicode")]
    fn set_default_notebook_panics_on_non_unicode_path() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let base = temp();
        let notebook = base.path().join(OsString::from_vec(vec![0xFF]));
        fs::create_dir(&notebook).expect("non-Unicode dir creates");
        let file = base.path().join("config.toml");
        let _ = set_default_notebook(&file, &notebook);
    }

    #[test]
    fn load_reads_daily_keys() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(
            &file,
            "daily-folder = 'Daily Notes'\ndaily-date-format = '%Y-%m-%d'\n",
        )
        .expect("fixture writes");
        let config = load(&file).expect("fixture loads");
        assert_eq!(config.daily_folder, Some(PathBuf::from("Daily Notes")));
        assert_eq!(
            config.daily_date_format,
            Some(day::Format::new("%Y-%m-%d").expect("valid format"))
        );
    }

    #[test]
    fn load_rejects_non_string_daily_keys() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-folder = 3\n").expect("fixture writes");
        let error = load(&file).expect_err("number fails");
        assert!(
            error
                .to_string()
                .contains("`daily-folder` must be a string")
        );
        fs::write(&file, "daily-date-format = 3\n").expect("fixture writes");
        let error = load(&file).expect_err("number fails");
        assert!(
            error
                .to_string()
                .contains("`daily-date-format` must be a string")
        );
    }

    #[test]
    fn load_rejects_rooted_daily_folder() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-folder = '/daily'\n").expect("fixture writes");
        let error = load(&file).expect_err("rooted folder fails");
        assert!(
            error
                .to_string()
                .contains("`daily-folder` must be a relative path")
        );
    }

    #[test]
    fn load_rejects_empty_daily_folder() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-folder = ''\n").expect("fixture writes");
        let error = load(&file).expect_err("empty folder fails");
        assert!(
            error
                .to_string()
                .contains("`daily-folder` must not be empty")
        );
    }

    #[test]
    fn load_reads_daily_template() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-template = 'templates/Daily.md'\n").expect("fixture writes");
        let config = load(&file).expect("fixture loads");
        assert_eq!(
            config.daily_template,
            Some(PathBuf::from("templates/Daily.md"))
        );
    }

    #[test]
    fn load_rejects_daily_template_of_the_wrong_shape() {
        let base = temp();
        let file = base.path().join("config.toml");
        let cases = [
            ("daily-template = 3\n", "`daily-template` must be a string"),
            (
                "daily-template = ''\n",
                "`daily-template` must not be empty",
            ),
            (
                "daily-template = '/rooted.md'\n",
                "`daily-template` must be a relative path",
            ),
        ];
        for (contents, fragment) in cases {
            fs::write(&file, contents).expect("fixture writes");
            let error = load(&file).expect_err("bad template fails");
            assert!(
                error.to_string().contains(fragment),
                "for {contents:?}: {error}"
            );
        }
    }

    #[test]
    fn set_daily_template_round_trips_and_validates() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_daily_template(&file, "templates/Daily.md").expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(
            config.daily_template,
            Some(PathBuf::from("templates/Daily.md"))
        );
        let missing = base.path().join("nested").join("config.toml");
        let error = set_daily_template(&missing, "").expect_err("empty template fails");
        assert!(
            error
                .to_string()
                .contains("`daily-template` must not be empty")
        );
        let error = set_daily_template(&missing, "/rooted.md").expect_err("rooted template fails");
        assert!(
            error
                .to_string()
                .contains("`daily-template` must be a relative path")
        );
        assert!(!missing.exists());
    }

    #[test]
    fn load_rejects_unknown_date_directive() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-date-format = '%Q'\n").expect("fixture writes");
        let error = load(&file).expect_err("unknown directive fails");
        assert!(error.to_string().contains("date format \"%Q\" is invalid"));
    }

    /// The probe is a date without a clock, so time-of-day directives are
    /// rejected too.
    #[test]
    fn load_rejects_time_directive_in_date_format() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-date-format = '%Y-%H'\n").expect("fixture writes");
        load(&file).expect_err("time directive fails");
    }

    #[test]
    fn load_rejects_empty_date_format() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "daily-date-format = ''\n").expect("fixture writes");
        let error = load(&file).expect_err("empty format fails");
        assert!(error.to_string().contains("renders an empty file name"));
    }

    #[test]
    fn set_daily_folder_round_trips() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_daily_folder(&file, "Daily Notes").expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(config.daily_folder, Some(PathBuf::from("Daily Notes")));
    }

    #[test]
    fn set_daily_folder_rejects_rooted_path() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_daily_folder(&file, "/daily").expect_err("rooted folder fails");
        assert!(!file.exists());
    }

    #[test]
    fn set_daily_date_format_round_trips() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_daily_date_format(&file, "%Y/%m/%d").expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(
            config.daily_date_format,
            Some(day::Format::new("%Y/%m/%d").expect("valid format"))
        );
    }

    #[test]
    fn set_daily_date_format_rejects_invalid_format() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_daily_date_format(&file, "%Q").expect_err("unknown directive fails");
        assert!(!file.exists());
    }

    #[test]
    fn set_daily_date_format_rejects_empty_format() {
        let base = temp();
        let file = base.path().join("config.toml");
        let error = set_daily_date_format(&file, "").expect_err("empty format fails");
        assert!(error.to_string().contains("renders an empty file name"));
        assert!(!file.exists());
    }

    #[test]
    fn set_editor_stores_command() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_editor(&file, "code --wait").expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(config.editor, Some("code --wait".to_owned()));
    }

    #[test]
    fn set_editor_rejects_blank_command() {
        let base = temp();
        let file = base.path().join("config.toml");
        let error = set_editor(&file, " \t").expect_err("blank editor fails");
        assert!(
            error
                .to_string()
                .contains("`editor` must contain a command")
        );
        assert!(!file.exists());
    }

    #[test]
    fn set_editor_stores_quoted_command() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_editor(&file, "'/opt/spaced out/editor' --wait").expect("set succeeds");
        let config = load(&file).expect("written config loads");
        assert_eq!(
            config.editor,
            Some("'/opt/spaced out/editor' --wait".to_owned())
        );
    }

    #[test]
    fn set_editor_rejects_unparsable_command() {
        let base = temp();
        let file = base.path().join("config.toml");
        let error = set_editor(&file, "'unclosed").expect_err("unclosed quote fails");
        assert!(
            error.to_string().contains("`editor`:"),
            "unexpected: {error}"
        );
        assert!(!file.exists());
    }

    /// A comment binds to the key below it, so removing a key takes its own
    /// comment along while comments on other keys survive.
    #[test]
    fn unset_removes_key_and_preserves_rest() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "default-notebook = '/old'\n# mine\neditor = 'vim'\n")
            .expect("fixture writes");
        unset(&file, DEFAULT_NOTEBOOK).expect("unset succeeds");
        let contents = fs::read_to_string(&file).expect("written config reads");
        assert!(contents.contains("# mine"));
        assert!(contents.contains("editor = 'vim'"));
        assert!(!contents.contains("default-notebook"));
    }

    #[test]
    fn unset_is_noop_when_key_absent() {
        let base = temp();
        let file = base.path().join("config.toml");
        let fixture = "editor   =   'vim'   # spacing preserved\n";
        fs::write(&file, fixture).expect("fixture writes");
        unset(&file, DEFAULT_NOTEBOOK).expect("unset succeeds");
        let contents = fs::read_to_string(&file).expect("config reads");
        assert_eq!(contents, fixture);
    }

    #[test]
    fn unset_is_noop_when_file_missing() {
        let base = temp();
        let file = base.path().join("config.toml");
        unset(&file, DEFAULT_NOTEBOOK).expect("unset succeeds");
        assert!(!file.exists());
    }

    #[test]
    fn ensure_dir_creates_missing_directories() {
        let base = temp();
        let file = base.path().join("a").join("b").join("config.toml");
        ensure_dir(&file).expect("ensure_dir succeeds");
        assert!(base.path().join("a").join("b").is_dir());
    }

    #[test]
    fn load_reads_the_stamp_keys() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(
            &file,
            "stamp = false\nstamp-created-key = 'made'\nstamp-updated-key = 'touched'\n\
             stamp-format = '%Y'\nstamp-exclude = ['templates', 'archive/2026']\n",
        )
        .expect("fixture writes");
        let config = load(&file).expect("config loads");
        assert_eq!(config.stamp, Some(false));
        assert_eq!(config.stamp_created_key, Some("made".to_owned()));
        assert_eq!(config.stamp_updated_key, Some("touched".to_owned()));
        assert_eq!(
            config.stamp_format,
            Some(day::StampFormat::new("%Y").expect("format validates"))
        );
        assert_eq!(
            config.stamp_exclude,
            Some(vec![
                PathBuf::from("templates"),
                PathBuf::from("archive/2026")
            ])
        );
    }

    #[test]
    fn load_rejects_stamp_values_of_the_wrong_shape() {
        let cases = [
            ("stamp = 'yes'\n", "`stamp` must be true or false"),
            ("stamp-created-key = 1\n", "must be a string"),
            ("stamp-updated-key = 1\n", "must be a string"),
            (
                "stamp-created-key = 'a:b'\n",
                "`stamp-created-key`: invalid property key",
            ),
            (
                "stamp-updated-key = 'a#b'\n",
                "`stamp-updated-key`: invalid property key",
            ),
            ("stamp-format = 1\n", "must be a string"),
            ("stamp-format = '%Q'\n", "`stamp-format`: timestamp format"),
            ("stamp-exclude = 'x'\n", "must be an array of strings"),
            ("stamp-exclude = [1]\n", "must be an array of strings"),
            (
                "stamp-exclude = ['/abs']\n",
                "entries must be relative paths",
            ),
            ("stamp-exclude = ['']\n", "entries must not be empty"),
            (
                "stamp-exclude = ['..']\n",
                "must name a place inside the notebook",
            ),
            (
                "stamp-exclude = ['a/../b']\n",
                "must name a place inside the notebook",
            ),
            (
                "stamp-exclude = ['.']\n",
                "must name a place inside the notebook",
            ),
        ];
        let base = temp();
        let file = base.path().join("config.toml");
        for (contents, fragment) in cases {
            fs::write(&file, contents).expect("fixture writes");
            let error = load(&file).expect_err("bad value fails");
            assert!(error.to_string().contains(fragment), "for {contents:?}");
        }
    }

    #[test]
    fn set_stamp_round_trips_and_rejects_other_words() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_stamp(&file, "false").expect("set succeeds");
        assert_eq!(load(&file).expect("config loads").stamp, Some(false));
        set_stamp(&file, "true").expect("set succeeds");
        assert_eq!(load(&file).expect("config loads").stamp, Some(true));
        let fresh = temp();
        let missing = fresh.path().join("config.toml");
        let error = set_stamp(&missing, "maybe").expect_err("bad value fails");
        assert!(error.to_string().contains("must be true or false"));
        assert!(!missing.exists());
    }

    #[test]
    fn set_stamp_keys_round_trip_and_validate() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_stamp_created_key(&file, "made").expect("set succeeds");
        set_stamp_updated_key(&file, "touched").expect("set succeeds");
        let config = load(&file).expect("config loads");
        assert_eq!(config.stamp_created_key, Some("made".to_owned()));
        assert_eq!(config.stamp_updated_key, Some("touched".to_owned()));
        let fresh = temp();
        let missing = fresh.path().join("config.toml");
        set_stamp_created_key(&missing, "a:b").expect_err("bad key fails");
        set_stamp_updated_key(&missing, "-a").expect_err("bad key fails");
        assert!(!missing.exists());
    }

    #[test]
    fn set_stamp_format_round_trips_and_validates() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_stamp_format(&file, "%Y-%m-%d %H:%M").expect("set succeeds");
        assert_eq!(
            load(&file).expect("config loads").stamp_format,
            Some(day::StampFormat::new("%Y-%m-%d %H:%M").expect("format validates"))
        );
        let fresh = temp();
        let missing = fresh.path().join("config.toml");
        set_stamp_format(&missing, "%Q").expect_err("bad format fails");
        assert!(!missing.exists());
    }

    #[test]
    fn load_reads_bullet_indent() {
        let base = temp();
        let file = base.path().join("config.toml");
        let cases = [
            ("bullet-indent = 'tab'\n", structure::Indent::Tab),
            ("bullet-indent = 'spaces'\n", structure::Indent::Spaces),
        ];
        for (contents, indent) in cases {
            fs::write(&file, contents).expect("fixture writes");
            assert_eq!(
                load(&file).expect("config loads").bullet_indent,
                Some(indent)
            );
        }
    }

    #[test]
    fn load_rejects_bullet_indent_values_of_the_wrong_shape() {
        let cases = [
            ("bullet-indent = 1\n", "`bullet-indent` must be a string"),
            (
                "bullet-indent = 'wide'\n",
                "`bullet-indent` must be \"tab\" or \"spaces\"",
            ),
        ];
        let base = temp();
        let file = base.path().join("config.toml");
        for (contents, fragment) in cases {
            fs::write(&file, contents).expect("fixture writes");
            let error = load(&file).expect_err("bad value fails");
            assert!(error.to_string().contains(fragment), "for {contents:?}");
        }
    }

    #[test]
    fn set_bullet_indent_round_trips_and_validates() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_bullet_indent(&file, "spaces").expect("set succeeds");
        assert_eq!(
            load(&file).expect("config loads").bullet_indent,
            Some(structure::Indent::Spaces)
        );
        set_bullet_indent(&file, "tab").expect("set succeeds");
        assert_eq!(
            load(&file).expect("config loads").bullet_indent,
            Some(structure::Indent::Tab)
        );
        let fresh = temp();
        let missing = fresh.path().join("config.toml");
        let error = set_bullet_indent(&missing, "wide").expect_err("bad value fails");
        assert!(error.to_string().contains("must be \"tab\" or \"spaces\""));
        assert!(!missing.exists());
    }

    #[test]
    fn set_stamp_exclude_splits_on_commas_and_trims() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_stamp_exclude(&file, " templates , archive/2026 ").expect("set succeeds");
        assert_eq!(
            load(&file).expect("config loads").stamp_exclude,
            Some(vec![
                PathBuf::from("templates"),
                PathBuf::from("archive/2026")
            ])
        );
    }

    /// A `./` spelling loads as the place it names, so exclusion
    /// matching, which compares components, sees it.
    #[test]
    fn stamp_exclude_normalizes_curdir_components() {
        let base = temp();
        let file = base.path().join("config.toml");
        fs::write(&file, "stamp-exclude = ['./templates']\n").expect("fixture writes");
        assert_eq!(
            load(&file).expect("config loads").stamp_exclude,
            Some(vec![PathBuf::from("templates")])
        );
    }

    #[test]
    fn set_stamp_exclude_validates_every_entry() {
        let base = temp();
        let file = base.path().join("config.toml");
        set_stamp_exclude(&file, "").expect_err("empty entry fails");
        set_stamp_exclude(&file, "a,,b").expect_err("empty entry fails");
        set_stamp_exclude(&file, "/abs").expect_err("rooted entry fails");
        set_stamp_exclude(&file, "..").expect_err("escaping entry fails");
        set_stamp_exclude(&file, ".").expect_err("escaping entry fails");
        assert!(!file.exists());
    }

    #[test]
    fn machine_scoped_names_only_the_machine_keys() {
        assert!(machine_scoped(DEFAULT_NOTEBOOK));
        assert!(machine_scoped(EDITOR));
        for key in [
            DAILY_FOLDER,
            DAILY_DATE_FORMAT,
            DAILY_TEMPLATE,
            STAMP,
            STAMP_CREATED_KEY,
            STAMP_UPDATED_KEY,
            STAMP_FORMAT,
            STAMP_EXCLUDE,
            BULLET_INDENT,
        ] {
            assert!(!machine_scoped(key), "{key} is notebook-scoped");
        }
    }

    #[test]
    fn load_notebook_reads_every_notebook_scoped_key() {
        let base = temp();
        let file = base.path().join(NOTEBOOK_FILE);
        fs::write(
            &file,
            "daily-folder = 'Journal'\n\
             daily-date-format = '%Y/%m/%d'\n\
             daily-template = 'templates/Daily.md'\n\
             stamp = false\n\
             stamp-created-key = 'made'\n\
             stamp-updated-key = 'touched'\n\
             stamp-format = '%Y-%m-%d'\n\
             stamp-exclude = ['templates']\n\
             bullet-indent = 'spaces'\n",
        )
        .expect("fixture writes");
        let config = load_notebook(&file).expect("fixture loads");
        assert_eq!(config.daily_folder, Some(PathBuf::from("Journal")));
        assert_eq!(
            config.daily_date_format,
            Some(day::Format::new("%Y/%m/%d").expect("format builds"))
        );
        assert_eq!(
            config.daily_template,
            Some(PathBuf::from("templates/Daily.md"))
        );
        assert_eq!(config.stamp, Some(false));
        assert_eq!(config.stamp_created_key, Some("made".to_owned()));
        assert_eq!(config.stamp_updated_key, Some("touched".to_owned()));
        assert_eq!(
            config.stamp_format,
            Some(day::StampFormat::new("%Y-%m-%d").expect("format builds"))
        );
        assert_eq!(config.stamp_exclude, Some(vec![PathBuf::from("templates")]));
        assert_eq!(config.bullet_indent, Some(structure::Indent::Spaces));
    }

    #[test]
    fn load_notebook_returns_empty_config_when_file_missing() {
        let base = temp();
        let config = load_notebook(&base.path().join(NOTEBOOK_FILE)).expect("missing file loads");
        assert_eq!(config, Config::default());
    }

    #[test]
    fn load_notebook_rejects_machine_scoped_keys() {
        let base = temp();
        let file = base.path().join(NOTEBOOK_FILE);
        for (contents, key) in [
            ("editor = 'vim'\n", "editor"),
            ("default-notebook = '/notes'\n", "default-notebook"),
        ] {
            fs::write(&file, contents).expect("fixture writes");
            let message = load_notebook(&file)
                .expect_err("machine key fails")
                .to_string();
            assert!(message.contains(".kladde.toml"), "{message}");
            assert!(
                message.contains(&format!("sets `{key}`, which is machine-scoped")),
                "{message}"
            );
        }
    }

    /// Errors that already name the file pass through unwrapped, so the
    /// path never appears twice in one message.
    #[test]
    fn load_notebook_passes_file_errors_through_unwrapped() {
        let base = temp();
        let file = base.path().join(NOTEBOOK_FILE);
        fs::write(&file, "editor = [oops\n").expect("fixture writes");
        let parse = load_notebook(&file).expect_err("garbage does not parse");
        assert!(parse.to_string().starts_with("invalid TOML in"));
        fs::write(&file, "unknown = 1\n").expect("fixture writes");
        let unknown = load_notebook(&file).expect_err("unknown key fails");
        assert!(unknown.to_string().starts_with("unknown key `unknown`"));
        fs::remove_file(&file).expect("fixture removes");
        fs::create_dir(&file).expect("obstacle creates");
        let read = load_notebook(&file).expect_err("directory does not read");
        assert!(read.to_string().starts_with("cannot read"));
    }

    #[test]
    fn load_notebook_wraps_a_value_error_with_the_path() {
        let base = temp();
        let file = base.path().join(NOTEBOOK_FILE);
        fs::write(&file, "daily-folder = ''\n").expect("fixture writes");
        let message = load_notebook(&file)
            .expect_err("empty folder fails")
            .to_string();
        assert!(message.starts_with("in notebook config"), "{message}");
        assert!(message.contains(".kladde.toml"), "{message}");
        assert!(
            message.contains("`daily-folder` must not be empty"),
            "{message}"
        );
    }

    /// A config with every key set, one side of the layering tests.
    fn base_config() -> Config {
        Config {
            default_notebook: Some(abs("/machine")),
            editor: Some("vim".to_owned()),
            daily_folder: Some(PathBuf::from("base")),
            daily_date_format: Some(day::Format::new("%Y").expect("format builds")),
            daily_template: Some(PathBuf::from("base.md")),
            stamp: Some(true),
            stamp_created_key: Some("base-created".to_owned()),
            stamp_updated_key: Some("base-updated".to_owned()),
            stamp_format: Some(day::StampFormat::new("%Y").expect("format builds")),
            stamp_exclude: Some(vec![PathBuf::from("base")]),
            bullet_indent: Some(structure::Indent::Tab),
        }
    }

    /// A config setting every notebook-scoped key, differing from
    /// [`base_config`] in each.
    fn notebook_config() -> Config {
        Config {
            default_notebook: None,
            editor: None,
            daily_folder: Some(PathBuf::from("nb")),
            daily_date_format: Some(day::Format::new("%d").expect("format builds")),
            daily_template: Some(PathBuf::from("nb.md")),
            stamp: Some(false),
            stamp_created_key: Some("nb-created".to_owned()),
            stamp_updated_key: Some("nb-updated".to_owned()),
            stamp_format: Some(day::StampFormat::new("%H").expect("format builds")),
            stamp_exclude: Some(vec![PathBuf::from("nb")]),
            bullet_indent: Some(structure::Indent::Spaces),
        }
    }

    #[test]
    fn layered_takes_the_notebook_value_for_keys_set_in_both() {
        let layered = base_config().layered(notebook_config());
        let expected = Config {
            default_notebook: Some(abs("/machine")),
            editor: Some("vim".to_owned()),
            ..notebook_config()
        };
        assert_eq!(layered, expected);
    }

    #[test]
    fn layered_falls_back_to_the_base_for_unset_keys() {
        assert_eq!(base_config().layered(Config::default()), base_config());
    }

    /// Only a hand-built notebook config can hold machine keys, but the
    /// contract stands: layering never takes them.
    #[test]
    fn layered_keeps_machine_keys_from_the_base() {
        let layered = Config::default().layered(base_config());
        let expected = Config {
            default_notebook: None,
            editor: None,
            ..base_config()
        };
        assert_eq!(layered, expected);
    }
}

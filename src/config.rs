//! Locating, loading, and editing kladde's configuration.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use toml_edit::DocumentMut;

use crate::day;

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

/// Settings read from the config file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Notebook used when a command is not given an explicit notebook.
    pub default_notebook: Option<PathBuf>,
    /// Command that opens files in an editor, split on whitespace when run.
    pub editor: Option<String>,
    /// Folder inside the notebook that holds daily notes; unset means the
    /// notebook root.
    pub daily_folder: Option<PathBuf>,
    /// strftime format for daily note file names; unset means
    /// [`day::DEFAULT_FORMAT`].
    pub daily_date_format: Option<day::Format>,
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
    #[error("`{key}` must be a string")]
    NotAString { key: &'static str },
    #[error("`{key}` must be an absolute path, got \"{value}\"")]
    NotAbsolute { key: &'static str, value: String },
    #[error("`editor` must contain a command")]
    EmptyEditor,
    #[error("`{key}` must be a relative path, got \"{value}\"")]
    NotRelative { key: &'static str, value: String },
    #[error("`daily-folder` must not be empty")]
    EmptyDailyFolder,
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
/// must be an absolute path, `editor` must contain a command, `daily-folder`
/// must be a non-empty relative path, and `daily-date-format` must render a
/// date.
pub fn load(file: &Path) -> Result<Config, Error> {
    let document = read_document(file)?;
    let mut config = Config::default();
    for (key, item) in document.iter() {
        match key {
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
                if value.split_whitespace().next().is_none() {
                    return Err(Error::EmptyEditor);
                }
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
/// Returns an error when `editor` contains no command, or when the config
/// file cannot be read, parsed, or written back.
pub fn set_editor(file: &Path, editor: &str) -> Result<(), Error> {
    if editor.split_whitespace().next().is_none() {
        return Err(Error::EmptyEditor);
    }
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

fn save(file: &Path, document: &DocumentMut) -> Result<(), Error> {
    ensure_dir(file)?;
    fs::write(file, document.to_string()).map_err(|cause| Error::Write {
        path: file.to_owned(),
        cause,
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
}

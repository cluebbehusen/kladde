//! Locating, loading, and editing kladde's configuration.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

/// Config key naming the notebook used when a command is not given an
/// explicit notebook.
pub const DEFAULT_NOTEBOOK: &str = "default-notebook";

/// Config key naming the command that opens files in an editor.
pub const EDITOR: &str = "editor";

/// Settings read from the config file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Config {
    /// Notebook used when a command is not given an explicit notebook.
    pub default_notebook: Option<PathBuf>,
    /// Command that opens files in an editor, split on whitespace when run.
    pub editor: Option<String>,
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
/// must be an absolute path and `editor` must contain a command.
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
}

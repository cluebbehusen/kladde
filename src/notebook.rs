//! Notebooks and the paths of notes inside them.

use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Failure while opening a notebook or targeting a note inside it.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("cannot open notebook {}: {cause}", path.display())]
    Open {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("not a directory: {}", path.display())]
    NotADirectory { path: PathBuf },
    #[error("note targets are relative paths inside the notebook, got \"{}\"", target.display())]
    NotRelative { target: PathBuf },
    #[error("note targets cannot leave the notebook, got \"{}\"", target.display())]
    InvalidComponent { target: PathBuf },
    #[error("note target is empty")]
    EmptyTarget,
    #[error("cannot resolve {}: {cause}", path.display())]
    Resolve {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("\"{}\" escapes the notebook", target.display())]
    Escape { target: PathBuf },
    #[error("not a file: {}", path.display())]
    NotAFile { path: PathBuf },
}

/// A notebook: a directory of markdown notes. The root is canonicalized on
/// open, so every [`NotePath`] it produces compares against the real
/// on-disk location.
#[derive(Debug)]
pub struct Notebook {
    root: PathBuf,
}

/// The absolute path of a note proven to lie inside its notebook: a file
/// that exists there or can be created there. The proof is point-in-time;
/// code that writes through a `NotePath` later must tolerate or re-verify
/// concurrent filesystem changes.
#[derive(Debug)]
pub struct NotePath {
    absolute: PathBuf,
}

impl NotePath {
    /// The note's absolute path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.absolute
    }
}

impl Notebook {
    /// Opens the notebook rooted at `root`, canonicalizing it (symlinks,
    /// case, relative paths resolved against the current directory).
    ///
    /// # Errors
    ///
    /// Returns an error when `root` cannot be canonicalized or is not a
    /// directory.
    pub fn open(root: &Path) -> Result<Self, Error> {
        let canonical = fs::canonicalize(root).map_err(|cause| Error::Open {
            path: root.to_owned(),
            cause,
        })?;
        if !canonical.is_dir() {
            return Err(Error::NotADirectory { path: canonical });
        }
        Ok(Self { root: canonical })
    }

    /// Resolves `target`, a relative path inside the notebook, to the note
    /// it names. The note does not have to exist: the deepest existing
    /// ancestor is canonicalized and checked for containment, and the rest
    /// must be creatable under it.
    ///
    /// # Errors
    ///
    /// Returns an error when `target` is rooted, empty, or steps outside
    /// the notebook (`..`, or a link that resolves outside), when a
    /// component or link cannot be verified, when the target is not a
    /// file, or when a non-directory sits where a parent directory is
    /// needed.
    pub fn note(&self, target: &Path) -> Result<NotePath, Error> {
        if target.has_root() {
            return Err(Error::NotRelative {
                target: target.to_owned(),
            });
        }
        let mut names = Vec::new();
        for component in target.components() {
            match component {
                Component::Normal(name) => names.push(name),
                Component::CurDir => {}
                _ => {
                    return Err(Error::InvalidComponent {
                        target: target.to_owned(),
                    });
                }
            }
        }
        if names.is_empty() {
            return Err(Error::EmptyTarget);
        }
        let mut existing = self.root.clone();
        let mut remainder = PathBuf::new();
        for name in names {
            if !remainder.as_os_str().is_empty() {
                remainder.push(name);
                continue;
            }
            let probe = existing.join(name);
            match probe.symlink_metadata() {
                Ok(_) => existing.push(name),
                // A missing entry starts the creatable remainder; so does a
                // file where a directory would be needed, which the
                // creatability check below turns into a clear error. Any
                // other failure means the component cannot be verified, and
                // an unverified component must not become part of the proof.
                Err(cause)
                    if matches!(cause.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) =>
                {
                    remainder.push(name);
                }
                Err(cause) => return Err(Error::Resolve { path: probe, cause }),
            }
        }
        let resolved = fs::canonicalize(&existing).map_err(|cause| Error::Resolve {
            path: existing.clone(),
            cause,
        })?;
        if !resolved.starts_with(&self.root) {
            return Err(Error::Escape {
                target: target.to_owned(),
            });
        }
        if remainder.as_os_str().is_empty() {
            if !resolved.is_file() {
                return Err(Error::NotAFile { path: resolved });
            }
            Ok(NotePath { absolute: resolved })
        } else if resolved.is_dir() {
            Ok(NotePath {
                absolute: resolved.join(remainder),
            })
        } else {
            Err(Error::NotADirectory { path: resolved })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp() -> TempDir {
        TempDir::new().expect("temp dir creates")
    }

    /// A directory link: symlink on Unix, junction on Windows (junctions
    /// need no elevation and may dangle, which the dangling tests rely on).
    #[cfg(unix)]
    fn link_dir(link: &Path, target: &Path) {
        std::os::unix::fs::symlink(target, link).expect("symlink creates");
    }

    #[cfg(windows)]
    fn link_dir(link: &Path, target: &Path) {
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

    fn notebook(root: &TempDir) -> Notebook {
        Notebook::open(root.path()).expect("notebook opens")
    }

    fn canonical(root: &TempDir) -> PathBuf {
        fs::canonicalize(root.path()).expect("root canonicalizes")
    }

    #[test]
    fn open_canonicalizes_the_root() {
        let root = temp();
        let note = notebook(&root)
            .note(Path::new("x.md"))
            .expect("target resolves");
        assert_eq!(note.as_path(), canonical(&root).join("x.md"));
    }

    #[test]
    fn open_rejects_missing_root() {
        let base = temp();
        let error = Notebook::open(&base.path().join("gone")).expect_err("missing root fails");
        assert!(error.to_string().contains("cannot open notebook"));
    }

    #[test]
    fn open_rejects_file_root() {
        let base = temp();
        let file = base.path().join("plain");
        fs::write(&file, "").expect("fixture writes");
        let error = Notebook::open(&file).expect_err("file root fails");
        assert!(error.to_string().contains("not a directory"));
    }

    #[test]
    fn note_resolves_existing_file() {
        let root = temp();
        fs::write(root.path().join("note.md"), "").expect("fixture writes");
        let note = notebook(&root)
            .note(Path::new("note.md"))
            .expect("target resolves");
        assert_eq!(note.as_path(), canonical(&root).join("note.md"));
    }

    #[test]
    fn note_resolves_missing_tail() {
        let root = temp();
        fs::create_dir(root.path().join("daily")).expect("fixture dir creates");
        let note = notebook(&root)
            .note(Path::new("daily/2026-08-03.md"))
            .expect("target resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("daily").join("2026-08-03.md")
        );
    }

    #[test]
    fn note_resolves_multi_level_missing_tail() {
        let root = temp();
        let note = notebook(&root)
            .note(Path::new("a/b/c.md"))
            .expect("target resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("a").join("b").join("c.md")
        );
    }

    #[test]
    fn note_normalizes_leading_curdir() {
        let root = temp();
        let note = notebook(&root)
            .note(Path::new("./x.md"))
            .expect("target resolves");
        assert_eq!(note.as_path(), canonical(&root).join("x.md"));
    }

    #[test]
    fn note_rejects_rooted_target() {
        let root = temp();
        let error = notebook(&root)
            .note(Path::new("/x.md"))
            .expect_err("rooted target fails");
        assert!(error.to_string().contains("relative paths"));
    }

    #[test]
    fn note_rejects_parent_traversal() {
        let root = temp();
        let notebook = notebook(&root);
        let error = notebook
            .note(Path::new("../x.md"))
            .expect_err("leading dotdot fails");
        assert!(error.to_string().contains("cannot leave the notebook"));
        let error = notebook
            .note(Path::new("a/../x.md"))
            .expect_err("interior dotdot fails");
        assert!(error.to_string().contains("cannot leave the notebook"));
    }

    #[test]
    fn note_rejects_empty_target() {
        let root = temp();
        let notebook = notebook(&root);
        let error = notebook.note(Path::new("")).expect_err("empty fails");
        assert!(error.to_string().contains("note target is empty"));
        let error = notebook.note(Path::new(".")).expect_err("curdir fails");
        assert!(error.to_string().contains("note target is empty"));
    }

    #[test]
    fn note_rejects_directory_target() {
        let root = temp();
        fs::create_dir(root.path().join("folder")).expect("fixture dir creates");
        let error = notebook(&root)
            .note(Path::new("folder"))
            .expect_err("directory target fails");
        assert!(error.to_string().contains("not a file"));
    }

    #[test]
    fn note_rejects_target_under_file() {
        let root = temp();
        fs::write(root.path().join("note.md"), "").expect("fixture writes");
        let error = notebook(&root)
            .note(Path::new("note.md/nested.md"))
            .expect_err("target under a file fails");
        assert!(error.to_string().contains("not a directory"));
    }

    #[test]
    fn note_rejects_escaping_link_target() {
        let root = temp();
        let outside = temp();
        link_dir(&root.path().join("escape"), outside.path());
        let error = notebook(&root)
            .note(Path::new("escape"))
            .expect_err("escaping link fails");
        assert!(error.to_string().contains("escapes the notebook"));
    }

    #[test]
    fn note_rejects_escaping_link_with_missing_tail() {
        let root = temp();
        let outside = temp();
        link_dir(&root.path().join("escape"), outside.path());
        let error = notebook(&root)
            .note(Path::new("escape/new.md"))
            .expect_err("tail through escaping link fails");
        assert!(error.to_string().contains("escapes the notebook"));
    }

    #[test]
    fn note_reports_dangling_link() {
        let root = temp();
        link_dir(&root.path().join("dangling"), &root.path().join("gone"));
        let error = notebook(&root)
            .note(Path::new("dangling/new.md"))
            .expect_err("dangling link fails");
        assert!(error.to_string().contains("cannot resolve"));
    }

    /// Every relevant filesystem caps name components at 255, so an
    /// overlong component makes the existence probe fail with something
    /// other than "not found" on all three platforms.
    #[test]
    fn note_reports_unprobeable_component() {
        let root = temp();
        let overlong = "a".repeat(300);
        let error = notebook(&root)
            .note(Path::new(&overlong))
            .expect_err("overlong component fails");
        assert!(error.to_string().contains("cannot resolve"));
    }

    #[cfg(unix)]
    #[test]
    fn note_reports_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp();
        let locked = root.path().join("locked");
        fs::create_dir(&locked).expect("fixture dir creates");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("permissions apply");
        let error = notebook(&root)
            .note(Path::new("locked/new.md"))
            .expect_err("unreadable directory fails");
        assert!(error.to_string().contains("cannot resolve"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
            .expect("permissions restore");
    }

    #[test]
    fn note_follows_link_inside_the_notebook() {
        let root = temp();
        fs::create_dir(root.path().join("real")).expect("fixture dir creates");
        link_dir(&root.path().join("alias"), &root.path().join("real"));
        let note = notebook(&root)
            .note(Path::new("alias/x.md"))
            .expect("inside link resolves");
        assert_eq!(note.as_path(), canonical(&root).join("real").join("x.md"));
    }
}

//! Notebooks and the paths of notes inside them.

use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

use jiff::civil::Date;
use unicase::UniCase;

use crate::day;

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
    #[error("\"{}\" names the notebook config, not a note", target.display())]
    ReservedTarget { target: PathBuf },
    #[error("\"{}\" names a kladde temporary file, not a note", target.display())]
    TempTarget { target: PathBuf },
    #[error("not a file: {}", path.display())]
    NotAFile { path: PathBuf },
    #[error("the notebook config {} is not a regular file", path.display())]
    ConfigNotRegular { path: PathBuf },
    #[error("no note named \"{name}\"")]
    NoSuchName { name: String },
    #[error("multiple notes named \"{name}\": {}", list(matches))]
    AmbiguousName { name: String, matches: Vec<PathBuf> },
}

fn resolve_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |cause| Error::Resolve {
        path: path.to_owned(),
        cause,
    }
}

/// Probes `path` for resolution: its metadata when an entry exists,
/// `None` when it is creatably absent (missing, or below a file where a
/// directory would be needed). Any other failure means the entry cannot
/// be verified, and an unverified entry joins no proof.
fn probed(path: &Path) -> Result<Option<fs::Metadata>, Error> {
    match path.symlink_metadata() {
        Ok(metadata) => Ok(Some(metadata)),
        Err(cause) if matches!(cause.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {
            Ok(None)
        }
        Err(cause) => Err(Error::Resolve {
            path: path.to_owned(),
            cause,
        }),
    }
}

fn list(paths: &[PathBuf]) -> String {
    let entries: Vec<String> = paths
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    entries.join(", ")
}

/// A notebook: a directory of markdown notes. The root is canonicalized on
/// open, so every [`NotePath`] it produces compares against the real
/// on-disk location.
#[derive(Debug)]
pub struct Notebook {
    root: PathBuf,
}

/// The absolute path of a note proven to lie inside its notebook. The
/// proof is structural and point-in-time: containment is verified against
/// the real filesystem, and no existing entry obstructs creating what is
/// missing. It is not a promise that the filesystem will accept a create
/// (name limits, permissions, space); those failures surface at write
/// time. Code that writes through a `NotePath` must tolerate or re-verify
/// concurrent filesystem changes.
#[derive(Debug)]
pub struct NotePath {
    absolute: PathBuf,
    root: PathBuf,
}

impl NotePath {
    /// The note's absolute path.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.absolute
    }

    /// The folder that holds the note.
    ///
    /// # Panics
    ///
    /// Panics when the path has no parent, which construction rules out: a
    /// note always lies inside its notebook's root.
    pub(crate) fn folder(&self) -> &Path {
        self.absolute
            .parent()
            .expect("a note lies inside its notebook")
    }

    /// The canonical root of the notebook the note was resolved in.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The note's path relative to its notebook root, which is what
    /// notebook-relative settings like `stamp-exclude` match against.
    ///
    /// # Panics
    ///
    /// Panics when the path does not start with the root, which
    /// construction rules out: a note always lies inside its notebook's
    /// root.
    #[must_use]
    pub fn relative(&self) -> &Path {
        self.absolute
            .strip_prefix(&self.root)
            .expect("a note lies inside its notebook")
    }
}

impl Notebook {
    /// The notebook's canonical root, the identity its lock is keyed on.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

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
    /// ancestor is canonicalized and checked for containment, and no
    /// existing entry may obstruct the rest.
    ///
    /// # Errors
    ///
    /// Returns an error when `target` is rooted, empty, or steps outside
    /// the notebook (`..`, or a link that resolves outside), when a
    /// component or link cannot be verified, when the target is not a
    /// file, when a non-directory sits where a parent directory is
    /// needed, when the target names or resolves at or under the
    /// notebook's own config file, which is not a note, or when its file
    /// name carries kladde's temporary-file suffix, which is not a note
    /// name.
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
        let spelled = reserved_name(names[0]);
        let spelled_temp = names.last().copied().is_some_and(temp_suffixed);
        let mut existing = self.root.clone();
        let mut remainder = PathBuf::new();
        for name in names {
            if !remainder.as_os_str().is_empty() {
                remainder.push(name);
                continue;
            }
            let probe = existing.join(name);
            match probed(&probe)? {
                Some(_) => existing.push(name),
                None => remainder.push(name),
            }
        }
        let resolved = fs::canonicalize(&existing).map_err(resolve_error(&existing))?;
        if !resolved.starts_with(&self.root) {
            return Err(Error::Escape {
                target: target.to_owned(),
            });
        }
        let absolute = if remainder.as_os_str().is_empty() {
            if !resolved.is_file() {
                return Err(Error::NotAFile { path: resolved });
            }
            resolved
        } else if resolved.is_dir() {
            resolved.join(remainder)
        } else {
            return Err(Error::NotADirectory { path: resolved });
        };
        if spelled || reserved(&absolute, &self.root) || config_aliased(&absolute, &self.root) {
            return Err(Error::ReservedTarget {
                target: target.to_owned(),
            });
        }
        if spelled_temp || absolute.file_name().is_some_and(temp_suffixed) {
            return Err(Error::TempTarget {
                target: target.to_owned(),
            });
        }
        Ok(NotePath {
            absolute,
            root: self.root.clone(),
        })
    }

    /// The notebook's own config file at the root; missing loads as an
    /// empty config. An existing entry must be a regular file: a link is
    /// never followed, and a pipe would block a read forever.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry exists but is not a regular file,
    /// or when it cannot be verified at all. The verdict is
    /// point-in-time, like every resolution proof: a caller racing its
    /// own filesystem must tolerate staleness.
    pub fn config_file(&self) -> Result<PathBuf, Error> {
        let path = self.root.join(crate::config::NOTEBOOK_FILE);
        if probed(&path)?.is_some_and(|metadata| !metadata.is_file()) {
            return Err(Error::ConfigNotRegular { path });
        }
        Ok(path)
    }

    /// The daily note for `date`: the date rendered through `format` plus
    /// `.md`, inside `folder` (or the notebook root when `folder` is
    /// `None`), whether or not the note exists yet.
    ///
    /// # Errors
    ///
    /// Returns an error when the resulting target leaves the notebook or is
    /// obstructed, exactly as [`Notebook::note`] reports.
    pub fn daily(
        &self,
        date: Date,
        folder: Option<&Path>,
        format: &day::Format,
    ) -> Result<NotePath, Error> {
        let filename = format!("{}.md", format.render(date));
        let target =
            folder.map_or_else(|| PathBuf::from(&filename), |folder| folder.join(&filename));
        self.note(&target)
    }

    /// The unique note whose file name, without its `.md` extension,
    /// matches `name` case-insensitively: resolution the way a wikilink
    /// resolves, anywhere in the notebook, with ambiguity as an error and
    /// never a guess. A trailing `.md` on `name` is ignored. Folders and
    /// files whose names start with a dot are skipped, and links are
    /// neither followed nor matched.
    ///
    /// # Errors
    ///
    /// Returns an error when `name` is empty, when no note or several
    /// notes match, when a folder cannot be read while searching, or
    /// when the matched note is the notebook config under another name.
    pub fn find(&self, name: &str) -> Result<NotePath, Error> {
        let stem = name.strip_suffix(".md").unwrap_or(name);
        if stem.is_empty() {
            return Err(Error::EmptyTarget);
        }
        let mut matches: Vec<PathBuf> = self
            .notes()?
            .into_iter()
            .filter(|path| has_stem(path, stem))
            .collect();
        if matches.len() > 1 {
            return Err(Error::AmbiguousName {
                name: stem.to_owned(),
                matches,
            });
        }
        match matches.pop() {
            Some(found) => {
                let absolute = self.root.join(&found);
                if config_aliased(&absolute, &self.root) {
                    return Err(Error::ReservedTarget { target: found });
                }
                Ok(NotePath {
                    absolute,
                    root: self.root.clone(),
                })
            }
            None => Err(Error::NoSuchName {
                name: stem.to_owned(),
            }),
        }
    }

    /// The notebook-relative paths of every note, sorted. Folders and
    /// files whose names start with a dot are skipped, and links are
    /// neither followed nor listed.
    ///
    /// # Errors
    ///
    /// Returns an error when a folder cannot be read while walking.
    pub fn notes(&self) -> Result<Vec<PathBuf>, Error> {
        let mut notes = Vec::new();
        walk(&self.root, Path::new(""), &mut notes)?;
        notes.sort();
        Ok(notes)
    }
}

/// Collects the notebook-relative paths of `.md` files under `dir`.
/// Recurses only into real directories, so links are skipped, and skips
/// dot-prefixed entries.
fn walk(dir: &Path, rel: &Path, notes: &mut Vec<PathBuf>) -> Result<(), Error> {
    for entry in fs::read_dir(dir).map_err(resolve_error(dir))? {
        let entry = entry.map_err(resolve_error(dir))?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let file_type = entry.file_type().map_err(resolve_error(&entry.path()))?;
        let entry_rel = rel.join(&name);
        if file_type.is_dir() {
            walk(&entry.path(), &entry_rel, notes)?;
        } else if file_type.is_file() && entry_rel.extension() == Some(OsStr::new("md")) {
            notes.push(entry_rel);
        }
    }
    Ok(())
}

/// Whether a single name is the notebook config's, compared case-folded
/// so a spelling that aliases it on a case-insensitive filesystem is
/// caught before it exists. One expression, so a non-Unicode name simply
/// fails the match.
fn reserved_name(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| UniCase::new(name) == UniCase::new(crate::config::NOTEBOOK_FILE))
}

/// Whether a file name carries kladde's temporary-file suffix,
/// lowercased so a case-insensitive alias counts. Writes may clear
/// stale temp entries, so a note must never occupy one.
fn temp_suffixed(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| name.to_lowercase().ends_with(crate::write::TEMP_SUFFIX))
}

/// Whether the resolved note is the notebook's config file by identity,
/// which is all a hard link shares with it. The comparison opens both
/// files, so it runs only when the config stats as a regular file; a
/// pipe's open would block forever. A failed check reads as "not the
/// config".
fn config_aliased(absolute: &Path, root: &Path) -> bool {
    let config = root.join(crate::config::NOTEBOOK_FILE);
    config
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.is_file())
        && same_file::is_same_file(absolute, &config).unwrap_or(false)
}

/// Whether the resolved path lies at or under the notebook's own config
/// file: the first component of the notebook-relative path carries the
/// reserved name. The config file must be a regular file or absent, so
/// nothing can legitimately live below that name either.
fn reserved(absolute: &Path, root: &Path) -> bool {
    absolute
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.components().next())
        .is_some_and(|component| reserved_name(component.as_os_str()))
}

/// Whether the path's stem matches `wanted` under Unicode case folding.
/// Folded into one expression so a non-Unicode stem (only constructible on
/// Linux) simply fails the match instead of needing its own branch.
fn has_stem(path: &Path, wanted: &str) -> bool {
    path.file_stem()
        .and_then(OsStr::to_str)
        .is_some_and(|stem| UniCase::new(stem) == UniCase::new(wanted))
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

    fn fmt(value: &str) -> day::Format {
        day::Format::new(value).expect("valid format")
    }

    #[test]
    fn open_canonicalizes_the_root() {
        let root = temp();
        let opened = notebook(&root);
        assert_eq!(opened.root(), canonical(&root));
        let note = opened.note(Path::new("x.md")).expect("target resolves");
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
    fn find_resolves_a_unique_name_anywhere() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        fs::create_dir(root.path().join("sub")).expect("fixture dir creates");
        fs::write(root.path().join("sub").join("b.md"), "").expect("fixture writes");
        let note = notebook(&root).find("b").expect("name resolves");
        assert_eq!(note.as_path(), canonical(&root).join("sub").join("b.md"));
    }

    /// Full case folding, not just lowercasing: STRASSE folds to the same
    /// string as Straße.
    #[test]
    fn find_matches_by_case_folding() {
        let root = temp();
        fs::write(root.path().join("Stra\u{00df}e.md"), "").expect("fixture writes");
        let note = notebook(&root).find("STRASSE").expect("name resolves");
        assert_eq!(note.as_path(), canonical(&root).join("Stra\u{00df}e.md"));
    }

    #[test]
    fn find_is_case_insensitive() {
        let root = temp();
        fs::write(root.path().join("Pricing Questions.md"), "").expect("fixture writes");
        let note = notebook(&root)
            .find("pricing questions")
            .expect("name resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("Pricing Questions.md")
        );
    }

    #[test]
    fn find_strips_one_md_suffix() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        let note = notebook(&root).find("a.md").expect("name resolves");
        assert_eq!(note.as_path(), canonical(&root).join("a.md"));
    }

    /// The suffix strip is exact, so an uppercase `.MD` is part of the
    /// name, which then matches nothing.
    #[test]
    fn find_does_not_strip_uppercase_md() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        notebook(&root)
            .find("a.MD")
            .expect_err("uppercase suffix fails");
    }

    #[test]
    fn find_reports_a_missing_name() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        let error = notebook(&root)
            .find("nope")
            .expect_err("missing name fails");
        assert!(error.to_string().contains("no note named \"nope\""));
    }

    #[test]
    fn find_reports_an_ambiguous_name() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        fs::create_dir(root.path().join("sub")).expect("fixture dir creates");
        fs::write(root.path().join("sub").join("a.md"), "").expect("fixture writes");
        let error = notebook(&root).find("a").expect_err("ambiguous name fails");
        let expected = format!(
            "multiple notes named \"a\": a.md, {}",
            Path::new("sub").join("a.md").display()
        );
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn find_skips_hidden_entries() {
        let root = temp();
        fs::create_dir(root.path().join(".hidden")).expect("fixture dir creates");
        fs::write(root.path().join(".hidden").join("a.md"), "").expect("fixture writes");
        fs::write(root.path().join(".a.md"), "").expect("fixture writes");
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        let note = notebook(&root).find("a").expect("name resolves uniquely");
        assert_eq!(note.as_path(), canonical(&root).join("a.md"));
        notebook(&root)
            .find(".a")
            .expect_err("hidden file is not found");
    }

    #[test]
    fn find_does_not_follow_links() {
        let root = temp();
        let outside = temp();
        fs::write(outside.path().join("target.md"), "").expect("fixture writes");
        link_dir(&root.path().join("linked"), outside.path());
        notebook(&root)
            .find("target")
            .expect_err("linked note is not found");
    }

    #[cfg(unix)]
    #[test]
    fn find_skips_link_files() {
        let root = temp();
        let outside = temp();
        fs::write(outside.path().join("real.md"), "").expect("fixture writes");
        std::os::unix::fs::symlink(outside.path().join("real.md"), root.path().join("ghost.md"))
            .expect("symlink creates");
        notebook(&root)
            .find("ghost")
            .expect_err("link file is not found");
    }

    #[cfg(unix)]
    #[test]
    fn find_reports_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp();
        let locked = root.path().join("locked");
        fs::create_dir(&locked).expect("fixture dir creates");
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("permissions apply");
        let error = notebook(&root)
            .find("a")
            .expect_err("unreadable folder fails");
        assert!(error.to_string().contains("cannot resolve"));
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755))
            .expect("permissions restore");
    }

    #[test]
    fn find_rejects_an_empty_name() {
        let root = temp();
        let notebook = notebook(&root);
        let error = notebook.find("").expect_err("empty name fails");
        assert!(error.to_string().contains("note target is empty"));
        let error = notebook.find(".md").expect_err("bare suffix fails");
        assert!(error.to_string().contains("note target is empty"));
    }

    #[test]
    fn find_does_not_match_a_directory() {
        let root = temp();
        fs::create_dir(root.path().join("x.md")).expect("fixture dir creates");
        notebook(&root)
            .find("x")
            .expect_err("directory is not a note");
    }

    #[test]
    fn notes_lists_every_note_sorted() {
        let root = temp();
        fs::write(root.path().join("b.md"), "").expect("fixture writes");
        fs::create_dir(root.path().join("sub")).expect("fixture dir creates");
        fs::write(root.path().join("sub").join("a.md"), "").expect("fixture writes");
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        let notes = notebook(&root).notes().expect("notebook lists");
        assert_eq!(
            notes,
            vec![
                PathBuf::from("a.md"),
                PathBuf::from("b.md"),
                Path::new("sub").join("a.md"),
            ]
        );
    }

    #[test]
    fn notes_skips_hidden_entries_links_and_other_files() {
        let root = temp();
        fs::write(root.path().join("a.md"), "").expect("fixture writes");
        fs::write(root.path().join(".hidden.md"), "").expect("fixture writes");
        fs::write(root.path().join("plain.txt"), "").expect("fixture writes");
        fs::create_dir(root.path().join(".git")).expect("fixture dir creates");
        fs::write(root.path().join(".git").join("c.md"), "").expect("fixture writes");
        let outside = temp();
        fs::write(outside.path().join("d.md"), "").expect("fixture writes");
        link_dir(&root.path().join("linked"), outside.path());
        let notes = notebook(&root).notes().expect("notebook lists");
        assert_eq!(notes, vec![PathBuf::from("a.md")]);
    }

    #[test]
    fn notes_lists_an_empty_notebook_as_empty() {
        let root = temp();
        let notes = notebook(&root).notes().expect("notebook lists");
        assert!(notes.is_empty());
    }

    #[test]
    fn daily_resolves_at_the_root() {
        let root = temp();
        let note = notebook(&root)
            .daily(jiff::civil::date(2026, 8, 4), None, &fmt("%Y-%m-%d"))
            .expect("daily resolves");
        assert_eq!(note.as_path(), canonical(&root).join("2026-08-04.md"));
    }

    #[test]
    fn daily_resolves_in_a_folder() {
        let root = temp();
        let note = notebook(&root)
            .daily(
                jiff::civil::date(2026, 8, 4),
                Some(Path::new("Daily Notes")),
                &fmt("%Y-%m-%d"),
            )
            .expect("daily resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("Daily Notes").join("2026-08-04.md")
        );
    }

    #[test]
    fn daily_nests_format_directories() {
        let root = temp();
        let note = notebook(&root)
            .daily(jiff::civil::date(2026, 8, 4), None, &fmt("%Y/%m/%d"))
            .expect("daily resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("2026").join("08").join("04.md")
        );
    }

    #[test]
    fn daily_stays_inside_the_notebook() {
        let root = temp();
        let error = notebook(&root)
            .daily(
                jiff::civil::date(2026, 8, 4),
                Some(Path::new("..")),
                &fmt("%Y-%m-%d"),
            )
            .expect_err("escaping folder fails");
        assert!(error.to_string().contains("cannot leave the notebook"));
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

    #[test]
    fn note_rejects_the_notebook_config() {
        let root = temp();
        let error = notebook(&root)
            .note(Path::new(".kladde.toml"))
            .expect_err("reserved name fails");
        assert_eq!(
            error.to_string(),
            "\".kladde.toml\" names the notebook config, not a note"
        );
        fs::write(root.path().join(".kladde.toml"), "stamp = false\n").expect("fixture writes");
        notebook(&root)
            .note(Path::new(".kladde.toml"))
            .expect_err("the existing file is reserved too");
    }

    /// The comparison folds case, so a spelling that would alias the
    /// config file on a case-insensitive filesystem is rejected on every
    /// platform, before anything exists.
    #[test]
    fn note_rejects_a_case_alias_of_the_notebook_config() {
        let root = temp();
        let error = notebook(&root)
            .note(Path::new(".KLADDE.TOML"))
            .expect_err("case alias fails");
        assert!(
            error.to_string().contains("names the notebook config"),
            "{error}"
        );
    }

    /// Only the root-level name is reserved; deeper down it is an
    /// ordinary dot file.
    #[test]
    fn note_resolves_a_nested_kladde_toml() {
        let root = temp();
        let note = notebook(&root)
            .note(Path::new("sub/.kladde.toml"))
            .expect("nested dot file resolves");
        assert_eq!(
            note.as_path(),
            canonical(&root).join("sub").join(".kladde.toml")
        );
    }

    /// A link elsewhere in the notebook resolving to the config file is
    /// caught by the same comparison, since resolution canonicalizes.
    #[cfg(unix)]
    #[test]
    fn note_rejects_a_link_alias_of_the_notebook_config() {
        let root = temp();
        fs::write(root.path().join(".kladde.toml"), "").expect("fixture writes");
        std::os::unix::fs::symlink(
            root.path().join(".kladde.toml"),
            root.path().join("alias.md"),
        )
        .expect("symlink creates");
        let error = notebook(&root)
            .note(Path::new("alias.md"))
            .expect_err("link alias fails");
        assert!(
            error.to_string().contains("names the notebook config"),
            "{error}"
        );
    }

    /// Temp names are kladde's implementation namespace at any depth, in
    /// any case; a name merely containing the suffix mid-name is fine.
    #[test]
    fn note_rejects_a_temporary_file_name() {
        let root = temp();
        let opened = notebook(&root);
        for target in [
            ".kladde.toml.kladde-tmp",
            "x.KLADDE-TMP",
            "sub/.abc123.kladde-tmp",
        ] {
            let error = opened.note(Path::new(target)).expect_err("temp name fails");
            assert_eq!(
                error.to_string(),
                format!("\"{target}\" names a kladde temporary file, not a note")
            );
        }
        opened
            .note(Path::new("kladde-tmp.md"))
            .expect("a name without the dotted suffix resolves");
        opened
            .note(Path::new("x.kladde-tmp.md"))
            .expect("a name containing the suffix mid-name resolves");
    }

    /// A link resolving to a temp name is caught like the spelled form.
    #[cfg(unix)]
    #[test]
    fn note_rejects_a_link_alias_of_a_temporary_file() {
        let root = temp();
        fs::write(root.path().join(".x.kladde-tmp"), "").expect("fixture writes");
        std::os::unix::fs::symlink(
            root.path().join(".x.kladde-tmp"),
            root.path().join("alias.md"),
        )
        .expect("symlink creates");
        let error = notebook(&root)
            .note(Path::new("alias.md"))
            .expect_err("link alias fails");
        assert!(
            error.to_string().contains("names a kladde temporary file"),
            "{error}"
        );
    }

    #[test]
    fn config_file_is_the_root_kladde_toml() {
        let root = temp();
        let opened = notebook(&root);
        let path = opened.config_file().expect("a missing entry is fine");
        assert_eq!(path, canonical(&root).join(".kladde.toml"));
        fs::write(&path, "stamp = false\n").expect("fixture writes");
        assert_eq!(opened.config_file().expect("a regular file is fine"), path);
    }

    #[test]
    fn config_file_rejects_a_link() {
        let root = temp();
        let target = temp();
        link_dir(&root.path().join(".kladde.toml"), target.path());
        let error = notebook(&root).config_file().expect_err("link fails");
        assert!(
            error.to_string().contains("is not a regular file"),
            "{error}"
        );
    }

    #[test]
    fn config_file_rejects_a_directory() {
        let root = temp();
        fs::create_dir(root.path().join(".kladde.toml")).expect("fixture dir creates");
        let error = notebook(&root).config_file().expect_err("directory fails");
        assert!(
            error.to_string().contains("is not a regular file"),
            "{error}"
        );
    }

    /// A pipe would block the config read forever waiting for a writer,
    /// so it is rejected before anything reads it.
    #[cfg(unix)]
    #[test]
    fn config_file_rejects_a_fifo() {
        let root = temp();
        let status = std::process::Command::new("mkfifo")
            .arg(root.path().join(".kladde.toml"))
            .status()
            .expect("mkfifo runs");
        assert!(status.success());
        let error = notebook(&root).config_file().expect_err("fifo fails");
        assert!(
            error.to_string().contains("is not a regular file"),
            "{error}"
        );
    }

    /// The reserved name covers everything under it too; nothing can
    /// legitimately live below the config file.
    #[test]
    fn note_rejects_targets_under_the_notebook_config() {
        let root = temp();
        for target in [".kladde.toml/x.md", ".KLADDE.TOML/sub/x.md"] {
            let error = notebook(&root)
                .note(Path::new(target))
                .expect_err("nested target fails");
            assert!(
                error.to_string().contains("names the notebook config"),
                "{error}"
            );
        }
    }

    /// The spelling is reserved before resolution, so a link at the
    /// reserved name cannot make the name readable as the note it points
    /// to.
    #[cfg(unix)]
    #[test]
    fn note_rejects_the_reserved_spelling_of_an_inside_link() {
        let root = temp();
        fs::write(root.path().join("real.md"), "").expect("fixture writes");
        std::os::unix::fs::symlink(
            root.path().join("real.md"),
            root.path().join(".kladde.toml"),
        )
        .expect("symlink creates");
        let error = notebook(&root)
            .note(Path::new(".kladde.toml"))
            .expect_err("reserved spelling fails");
        assert!(
            error.to_string().contains("names the notebook config"),
            "{error}"
        );
    }

    /// An unverifiable entry is an error, never treated as absent.
    #[cfg(unix)]
    #[test]
    fn config_file_reports_an_unverifiable_entry() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp();
        let opened = notebook(&root);
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o000))
            .expect("permissions apply");
        let error = opened.config_file().expect_err("unverifiable entry fails");
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o755))
            .expect("permissions restore");
        assert!(error.to_string().contains("cannot resolve"), "{error}");
    }

    /// A hard link to the config carries no reserved spelling and
    /// canonicalization keeps the alias name, so identity settles it.
    #[test]
    fn note_rejects_a_hard_link_alias_of_the_notebook_config() {
        let root = temp();
        fs::write(root.path().join(".kladde.toml"), "stamp = false\n").expect("fixture writes");
        fs::hard_link(
            root.path().join(".kladde.toml"),
            root.path().join("alias.md"),
        )
        .expect("hard link creates");
        let error = notebook(&root)
            .note(Path::new("alias.md"))
            .expect_err("hard link alias fails");
        assert!(
            error.to_string().contains("names the notebook config"),
            "{error}"
        );
    }

    /// The identity check opens the config, and opening a pipe blocks
    /// forever, so an irregular config entry skips the check: explicit
    /// note resolution never touches it.
    #[cfg(unix)]
    #[test]
    fn note_resolution_ignores_a_fifo_notebook_config() {
        let root = temp();
        fs::write(root.path().join("x.md"), "").expect("fixture writes");
        let status = std::process::Command::new("mkfifo")
            .arg(root.path().join(".kladde.toml"))
            .status()
            .expect("mkfifo runs");
        assert!(status.success());
        let opened = notebook(&root);
        opened
            .note(Path::new("x.md"))
            .expect("resolution returns promptly");
        opened.find("x").expect("lookup returns promptly");
    }

    #[test]
    fn find_rejects_a_hard_link_alias_of_the_notebook_config() {
        let root = temp();
        fs::write(root.path().join(".kladde.toml"), "stamp = false\n").expect("fixture writes");
        fs::hard_link(
            root.path().join(".kladde.toml"),
            root.path().join("alias.md"),
        )
        .expect("hard link creates");
        let error = notebook(&root)
            .find("alias")
            .expect_err("hard link alias fails");
        assert!(
            error.to_string().contains("names the notebook config"),
            "{error}"
        );
    }
}

//! Locked, atomic mutation of notes.
//!
//! Every write follows the same shape: take the notebook's exclusive
//! lock, read the note, transform it in memory, and atomically replace
//! the file. Readers that do not take the lock (editors, sync tools) can
//! never observe a half-written note, and writers that do can never lose
//! an entry.
//!
//! The write path defends against content planted in the notebook: a
//! link at the temporary path is deleted, never followed, and
//! containment is re-verified under the lock. It does not defend against
//! a local process actively racing the filesystem operations; on the
//! single-user machines kladde targets, such a process already has every
//! power kladde has.

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use crate::frontmatter;
use crate::notebook::NotePath;

/// Failure while appending to or rewriting a note.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("nothing to append: the text is empty")]
    EmptyText,
    #[error("cannot create lock directory {}: {cause}", path.display())]
    LockDir {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("cannot lock {}: {cause}", path.display())]
    Lock {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("cannot read {}: {cause}", path.display())]
    Read {
        path: PathBuf,
        cause: std::io::Error,
    },
    #[error("{} escaped the notebook", path.display())]
    Escaped { path: PathBuf },
    #[error("cannot write {}: {cause}", path.display())]
    Write {
        path: PathBuf,
        cause: std::io::Error,
    },
}

fn lock_dir_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |cause| Error::LockDir {
        path: path.to_owned(),
        cause,
    }
}

fn lock_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |cause| Error::Lock {
        path: path.to_owned(),
        cause,
    }
}

fn write_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |cause| Error::Write {
        path: path.to_owned(),
        cause,
    }
}

/// Directory holding kladde's lock files, following the XDG base directory
/// convention on every platform: `$XDG_STATE_HOME/kladde/locks`, falling
/// back to `$HOME/.local/state/kladde/locks`. As the XDG specification
/// requires, a relative path (which includes an empty one) counts as unset.
///
/// Lock files live outside the notebook on purpose: a notebook may be a
/// synced folder, where extra files would sync to other machines and file
/// locks are unreliable.
///
/// Returns `None` when neither variable provides an absolute base.
#[must_use]
pub fn lock_dir(xdg_state_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    xdg_state_home
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|path| path.join(".local").join("state"))
        })
        .map(|state_base| state_base.join("kladde").join("locks"))
}

/// An FNV-1a hash of a path, as sixteen hex digits. FNV-1a is implemented
/// here so the result never depends on the standard library's hasher,
/// which may change between releases; two kladde versions running at
/// once must agree on lock file names or they would lock different files
/// and lose entries. Non-Unicode paths are hashed through their lossy
/// form. A collision (hash or lossy) merely makes two notebooks share a
/// lock file: spurious serialization, never lost mutual exclusion.
fn hashed(path: &Path) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for byte in path.to_string_lossy().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{hash:016x}")
}

/// The lock file name for a notebook, from its canonical root.
fn lock_name(root: &Path) -> String {
    format!("{}.lock", hashed(root))
}

/// The temporary file name for a note, from its notebook-relative path:
/// hashing keeps the name within filesystem limits however long the
/// note's name is, and a collision merely makes two notes share a
/// temporary name, which the notebook lock already serializes.
fn temp_name(relative: &Path) -> String {
    format!(".{}.kladde-tmp", hashed(relative))
}

/// An exclusive lock on a notebook, held until the guard is dropped.
///
/// The lock file is separate from the notebook's contents, because the
/// atomic replace swaps a note's inode: a lock on the note itself would
/// let two writers hold "the" lock on two different inodes and lose an
/// entry. One lock covers the whole notebook rather than one note:
/// filesystems disagree on which spellings name the same file (case
/// folding, Unicode normalization), so a portable per-note lock identity
/// cannot be derived from a path that may not exist yet, while the
/// canonical notebook root is a single spelling however the root was
/// reached. A writer holds the lock for one read-transform-write,
/// milliseconds, so notebook-wide granularity costs nothing. Notebooks
/// whose roots overlap do not share a lock; a notebook is expected to be
/// targeted at one root. Lock files are never deleted: they are tiny,
/// reusable, and deleting one would race a concurrent acquirer.
///
/// Acquisition blocks without a timeout: the operating system releases
/// the lock if the holder dies.
#[derive(Debug)]
pub struct Guard<'a> {
    note: &'a NotePath,
    file: File,
}

impl<'a> Guard<'a> {
    /// Takes the exclusive lock of `note`'s notebook, creating `lock_dir`
    /// and the lock file as needed, and blocking until any other holder
    /// releases.
    ///
    /// # Errors
    ///
    /// Returns an error when the lock directory cannot be created or the
    /// lock file cannot be opened or locked.
    pub fn acquire(lock_dir: &Path, note: &'a NotePath) -> Result<Self, Error> {
        fs::create_dir_all(lock_dir).map_err(lock_dir_error(lock_dir))?;
        let path = lock_dir.join(lock_name(note.root()));
        // Read and write access, not append: Windows cannot lock an
        // append-only handle. Never truncate: other processes may hold
        // the file locked.
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(lock_error(&path))?;
        file.lock().map_err(lock_error(&path))?;
        Ok(Self { note, file })
    }

    /// Reads the note's current contents; a missing note reads as empty,
    /// so create-if-missing needs no separate step.
    ///
    /// # Errors
    ///
    /// Returns an error when the note exists but cannot be read as UTF-8.
    pub fn current(&self) -> Result<String, Error> {
        let path = self.note.as_path();
        match fs::read_to_string(path) {
            Ok(contents) => Ok(contents),
            Err(cause) if cause.kind() == ErrorKind::NotFound => Ok(String::new()),
            Err(cause) => Err(Error::Read {
                path: path.to_owned(),
                cause,
            }),
        }
    }

    /// Atomically replaces the note with `new`, creating parent folders
    /// as needed. An existing note keeps its permissions across the
    /// replacement, and both the note and its folder are synced, so a
    /// reported write survives a power cut.
    ///
    /// The new contents are written to a dot-prefixed temporary file in
    /// the note's own folder: the same filesystem, so the final rename is
    /// atomic, and a name that note lookup skips, so a half-written file
    /// can never resolve as a note. The name is fixed rather than random
    /// because the lock rules out concurrent writers; whatever a crash
    /// left at that name is deleted, and the file is then created anew
    /// with nothing followed, so a planted link cannot redirect the write.
    ///
    /// # Errors
    ///
    /// Returns an error when the note's folder no longer lies inside the
    /// notebook, or when a folder or the temporary file cannot be
    /// written.
    pub fn replace(&self, new: &str) -> Result<(), Error> {
        let path = self.note.as_path();
        let folder = self.note.folder();
        fs::create_dir_all(folder).map_err(write_error(folder))?;
        // A NotePath's containment proof is point-in-time and the lock
        // wait is unbounded, so re-verify it before writing: the folder
        // about to receive the note must still lie inside the notebook.
        let resolved = fs::canonicalize(folder).map_err(write_error(folder))?;
        let escaped = Error::Escaped {
            path: resolved.clone(),
        };
        resolved
            .starts_with(self.note.root())
            .then_some(())
            .ok_or(escaped)?;
        let temp = folder.join(temp_name(self.note.relative()));
        let permissions = fs::metadata(path)
            .ok()
            .map(|metadata| metadata.permissions());
        let _ = fs::remove_file(&temp);
        File::options()
            .write(true)
            .create_new(true)
            .open(&temp)
            .and_then(|file| write_note(file, permissions, new.as_bytes()))
            .map_err(write_error(&temp))?;
        fs::rename(&temp, path).map_err(write_error(path))?;
        sync_folder(folder).map_err(write_error(folder))?;
        Ok(())
    }

    /// Appends `text` to the end of the note as its own line, creating
    /// the note (parent folders included) if it does not exist. The text
    /// is appended verbatim, so a bullet is whatever the caller types;
    /// trailing newlines on `text` are dropped and exactly one
    /// terminating newline is written, with a separating newline first
    /// when the note does not already end in one. Appending at the end
    /// of the file never disturbs frontmatter, which ends where the body
    /// begins.
    ///
    /// With a `stamp`, the note is stamped as [`frontmatter::stamped`]
    /// describes, in the same atomic write. The caller renders the
    /// stamp's timestamp while already holding this guard, so stamps
    /// record the serialized write order.
    ///
    /// # Errors
    ///
    /// Returns an error when `text` is empty or nothing but newlines, or
    /// when reading or writing fails as [`Self::current`] and
    /// [`Self::replace`] report.
    pub fn append(&self, text: &str, stamp: Option<&frontmatter::Stamp<'_>>) -> Result<(), Error> {
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            return Err(Error::EmptyText);
        }
        let current = self.current()?;
        let mut new = appended(&current, text);
        if let Some(stamp) = stamp {
            let had_block = frontmatter::has_block(&current);
            new = frontmatter::stamped(&new, had_block, stamp);
        }
        self.replace(&new)
    }
}

/// Fills the temporary file: permissions first, so a private note's
/// contents never sit in a world-readable file, then the contents, synced
/// to disk before the handle closes.
fn write_note(
    mut file: File,
    permissions: Option<fs::Permissions>,
    bytes: &[u8],
) -> std::io::Result<()> {
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)?;
    }
    file.write_all(bytes)?;
    file.sync_all()
}

/// Forces the folder's entries to disk, making a just-renamed note
/// durable: the rename rewrote the folder's own data, and until that
/// reaches disk a power cut can revert a reported append even though the
/// note's contents are synced.
#[cfg(unix)]
fn sync_folder(folder: &Path) -> std::io::Result<()> {
    File::open(folder)?.sync_all()
}

/// Windows refuses to open a directory unless the handle asks for backup
/// semantics, and the flush behind `sync_all` needs write access; with
/// both, the same flush works.
#[cfg(windows)]
fn sync_folder(folder: &Path) -> std::io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    File::options()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(folder)?
        .sync_all()
}

impl Drop for Guard<'_> {
    /// Releases the lock. Closing the file would release it too, but
    /// possibly not promptly on every platform; an explicit unlock is.
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn appended(current: &str, text: &str) -> String {
    let mut new = String::with_capacity(current.len() + text.len() + 2);
    new.push_str(current);
    if !current.is_empty() && !current.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(text);
    new.push('\n');
    new
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notebook::Notebook;
    use std::fs::TryLockError;
    use tempfile::TempDir;

    fn temp() -> TempDir {
        TempDir::new().expect("temp dir creates")
    }

    fn note(root: &TempDir, target: &str) -> NotePath {
        Notebook::open(root.path())
            .expect("notebook opens")
            .note(Path::new(target))
            .expect("target resolves")
    }

    #[cfg(not(windows))]
    fn abs(value: &str) -> PathBuf {
        PathBuf::from(value)
    }

    #[cfg(windows)]
    fn abs(value: &str) -> PathBuf {
        PathBuf::from(format!("C:{}", value.replace('/', "\\")))
    }

    #[test]
    fn lock_dir_prefers_xdg_state_home() {
        assert_eq!(
            lock_dir(Some(abs("/xdg")), Some(abs("/home/me"))),
            Some(abs("/xdg/kladde/locks"))
        );
    }

    #[test]
    fn lock_dir_treats_relative_bases_as_unset() {
        assert_eq!(
            lock_dir(Some(PathBuf::new()), Some(abs("/home/me"))),
            Some(abs("/home/me/.local/state/kladde/locks"))
        );
        assert_eq!(
            lock_dir(Some(PathBuf::from("relative")), Some(abs("/home/me"))),
            Some(abs("/home/me/.local/state/kladde/locks"))
        );
    }

    #[test]
    fn lock_dir_falls_back_to_home() {
        assert_eq!(
            lock_dir(None, Some(abs("/home/me"))),
            Some(abs("/home/me/.local/state/kladde/locks"))
        );
    }

    #[test]
    fn lock_dir_needs_an_absolute_base() {
        assert_eq!(lock_dir(None, None), None);
        assert_eq!(lock_dir(None, Some(PathBuf::from("relative"))), None);
    }

    /// Pins the hash so a change to the function surfaces here: two kladde
    /// versions hashing differently would lock different files.
    #[test]
    fn lock_name_is_stable() {
        assert_eq!(lock_name(Path::new("kladde")), "92d4ee658d96b024.lock");
    }

    #[test]
    fn lock_name_distinguishes_paths() {
        assert_ne!(lock_name(Path::new("a.md")), lock_name(Path::new("b.md")));
    }

    /// Pins the temporary name too: the integration suite cannot call
    /// this function and hardcodes this hash for a note named `x.md`.
    #[test]
    fn temp_name_is_stable() {
        assert_eq!(temp_name(Path::new("x.md")), ".72d8d45320ac7f62.kladde-tmp");
    }

    #[test]
    fn guard_excludes_other_handles_until_dropped() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let guard = Guard::acquire(locks.path(), &target).expect("lock acquires");
        let lock_file = locks.path().join(lock_name(target.root()));
        let contender = File::open(&lock_file).expect("lock file opens");
        assert!(matches!(
            contender.try_lock(),
            Err(TryLockError::WouldBlock)
        ));
        drop(guard);
        contender.try_lock().expect("released lock acquires");
    }

    #[test]
    fn current_reads_a_missing_note_as_empty() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let guard = Guard::acquire(locks.path(), &target).expect("lock acquires");
        assert_eq!(guard.current().expect("read succeeds"), "");
    }

    #[test]
    fn current_reads_the_note() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "contents\n").expect("fixture writes");
        let target = note(&root, "x.md");
        let guard = Guard::acquire(locks.path(), &target).expect("lock acquires");
        assert_eq!(guard.current().expect("read succeeds"), "contents\n");
    }

    #[test]
    fn replace_swaps_the_whole_note() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "old\n").expect("fixture writes");
        let target = note(&root, "x.md");
        let guard = Guard::acquire(locks.path(), &target).expect("lock acquires");
        guard.replace("new\n").expect("replace succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "new\n");
    }

    #[test]
    fn append_stamps_when_given_a_stamp() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let stamp = frontmatter::Stamp::new("created", "updated", "T").expect("stamp validates");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- first", Some(&stamp))
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "---\ncreated: T\nupdated: T\n---\n- first\n");
    }

    #[test]
    fn append_creates_a_missing_note_with_parents() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "a/b/c.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- first", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- first\n");
    }

    #[test]
    fn append_appends_after_a_trailing_newline() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "start\n").expect("fixture writes");
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- next", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "start\n- next\n");
    }

    #[test]
    fn append_inserts_a_missing_separator() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "no newline").expect("fixture writes");
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- next", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "no newline\n- next\n");
    }

    #[test]
    fn append_adds_no_separator_to_an_empty_note() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "").expect("fixture writes");
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- only", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- only\n");
    }

    #[test]
    fn append_keeps_interior_newlines() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- parent\n\t- child", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- parent\n\t- child\n");
    }

    #[test]
    fn append_strips_trailing_newlines_from_the_text() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry\r\n\n", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- entry\n");
    }

    #[test]
    fn append_rejects_empty_text() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let error = Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("", None)
            .expect_err("empty text fails");
        assert!(error.to_string().contains("nothing to append"));
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("\n\n", None)
            .expect_err("newline-only text fails");
        assert!(!target.as_path().exists());
    }

    #[test]
    fn append_rejects_a_note_that_is_not_utf8() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), [0xff, 0xfe, 0xfd]).expect("fixture writes");
        let target = note(&root, "x.md");
        let error = Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect_err("bad encoding fails");
        assert!(error.to_string().contains("cannot read"));
    }

    #[test]
    fn append_consumes_a_stale_temp_file() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let stale = root.path().join(temp_name(Path::new("x.md")));
        fs::write(&stale, "crash leftovers").expect("fixture writes");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect("append succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- entry\n");
        assert!(!stale.exists());
    }

    #[test]
    fn append_reports_an_obstructed_temp_path() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        fs::create_dir(root.path().join(temp_name(Path::new("x.md"))))
            .expect("fixture dir creates");
        let error = Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect_err("obstructed temp fails");
        assert!(error.to_string().contains("cannot write"));
    }

    /// A link planted at the temporary path must never be followed: the
    /// write would land outside the notebook. It is deleted like any
    /// other stale entry and the append proceeds.
    #[cfg(unix)]
    #[test]
    fn append_does_not_follow_a_planted_temp_link() {
        let root = temp();
        let locks = temp();
        let outside = temp();
        let precious = outside.path().join("precious");
        fs::write(&precious, "untouched").expect("fixture writes");
        let target = note(&root, "x.md");
        let planted = root.path().join(temp_name(Path::new("x.md")));
        std::os::unix::fs::symlink(&precious, &planted).expect("symlink creates");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect("append succeeds");
        assert_eq!(
            fs::read_to_string(&precious).expect("outside file reads"),
            "untouched"
        );
        let note_type = fs::symlink_metadata(target.as_path())
            .expect("note metadata reads")
            .file_type();
        assert!(note_type.is_file());
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "- entry\n");
    }

    /// A directory link: symlink on Unix, junction on Windows.
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

    /// The containment proof is point-in-time: a folder swapped for an
    /// escaping link after resolution is caught again under the lock.
    #[test]
    fn append_rejects_a_folder_that_left_the_notebook() {
        let root = temp();
        let locks = temp();
        let outside = temp();
        let target = note(&root, "a/b.md");
        link_dir(&root.path().join("a"), outside.path());
        let error = Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect_err("escaped folder fails");
        assert!(error.to_string().contains("escaped the notebook"));
        assert!(!outside.path().join("b.md").exists());
    }

    #[test]
    fn append_reports_an_obstructed_lock_dir() {
        let root = temp();
        let base = temp();
        let target = note(&root, "x.md");
        let obstacle = base.path().join("locks");
        fs::write(&obstacle, "").expect("fixture writes");
        let error = Guard::acquire(&obstacle, &target).expect_err("lock dir obstruction fails");
        assert!(error.to_string().contains("cannot create lock directory"));
    }

    #[test]
    fn append_reports_an_obstructed_lock_file() {
        let root = temp();
        let locks = temp();
        let target = note(&root, "x.md");
        let obstacle = locks.path().join(lock_name(target.root()));
        fs::create_dir(&obstacle).expect("fixture dir creates");
        let error = Guard::acquire(locks.path(), &target).expect_err("lock obstruction fails");
        assert!(error.to_string().contains("cannot lock"));
    }

    /// One lock file covers the notebook: appends to two different notes
    /// share it.
    #[test]
    fn appends_share_one_notebook_lock_file() {
        let root = temp();
        let locks = temp();
        Guard::acquire(locks.path(), &note(&root, "x.md"))
            .expect("lock acquires")
            .append("- one", None)
            .expect("append succeeds");
        Guard::acquire(locks.path(), &note(&root, "y.md"))
            .expect("lock acquires")
            .append("- two", None)
            .expect("append succeeds");
        let entries = fs::read_dir(locks.path()).expect("lock dir reads");
        assert_eq!(entries.count(), 1);
    }

    #[test]
    fn current_and_replace_compose_under_the_lock() {
        let root = temp();
        let locks = temp();
        fs::write(root.path().join("x.md"), "before").expect("fixture writes");
        let target = note(&root, "x.md");
        let guard = Guard::acquire(locks.path(), &target).expect("lock acquires");
        let current = guard.current().expect("read succeeds");
        guard
            .replace(&format!("{current} after"))
            .expect("replace succeeds");
        let contents = fs::read_to_string(target.as_path()).expect("note reads");
        assert_eq!(contents, "before after");
    }

    #[cfg(unix)]
    #[test]
    fn append_preserves_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let root = temp();
        let locks = temp();
        let path = root.path().join("x.md");
        fs::write(&path, "secret\n").expect("fixture writes");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("permissions apply");
        let target = note(&root, "x.md");
        Guard::acquire(locks.path(), &target)
            .expect("lock acquires")
            .append("- entry", None)
            .expect("append succeeds");
        let mode = fs::metadata(&path)
            .expect("metadata reads")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

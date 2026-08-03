//! Opening files in the user's editor.

use std::path::Path;
use std::process::{Command, ExitStatus};

/// Failure while resolving or running the editor.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "no editor configured: set the `editor` config key or the VISUAL or EDITOR environment variable"
    )]
    NoEditor,
    #[error("cannot run editor `{program}`: {cause}")]
    Spawn {
        program: String,
        cause: std::io::Error,
    },
    #[error("editor `{program}` failed: {status}")]
    Failed { program: String, status: ExitStatus },
}

/// Opens `target` in the editor described by `command` and waits for it to
/// exit. The command is split on whitespace: the first token is the program,
/// the rest are its leading arguments, and `target` is appended.
///
/// # Errors
///
/// Returns an error when `command` is `None` or blank, when the editor
/// cannot be started, or when it exits unsuccessfully.
pub fn open(target: &Path, command: Option<&str>) -> Result<(), Error> {
    let mut tokens = command.unwrap_or_default().split_whitespace();
    let program = tokens.next().ok_or(Error::NoEditor)?;
    let status = Command::new(program)
        .args(tokens)
        .arg(target)
        .status()
        .map_err(|cause| Error::Spawn {
            program: program.to_owned(),
            cause,
        })?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed {
            program: program.to_owned(),
            status,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Editors that exist on the platform's test machines. Unit tests keep
    /// the parent environment, so `PATH` resolution works here; the
    /// integration suite covers the `PATH`-free case with absolute paths.
    /// The creating editor proves the target argument arrived by creating
    /// the file it is given.
    #[cfg(not(windows))]
    fn creating_editor() -> &'static str {
        "touch"
    }

    #[cfg(windows)]
    fn creating_editor() -> &'static str {
        "cmd /C copy NUL"
    }

    /// An editor that exits unsuccessfully: `false` ignores its argument;
    /// `type` fails on the missing target the test passes.
    #[cfg(not(windows))]
    fn failing_editor() -> &'static str {
        "false"
    }

    #[cfg(windows)]
    fn failing_editor() -> &'static str {
        "cmd /C type"
    }

    #[test]
    fn open_requires_a_command() {
        let base = TempDir::new().expect("temp dir creates");
        let target = base.path().join("file.txt");
        let error = open(&target, None).expect_err("no editor fails");
        assert!(error.to_string().contains("no editor configured"));
        let error = open(&target, Some(" \t")).expect_err("blank editor fails");
        assert!(error.to_string().contains("no editor configured"));
    }

    #[test]
    fn open_runs_the_editor_on_the_target() {
        let base = TempDir::new().expect("temp dir creates");
        let target = base.path().join("file.txt");
        open(&target, Some(creating_editor())).expect("editor succeeds");
        assert!(target.exists());
    }

    #[test]
    fn open_reports_spawn_failure() {
        let base = TempDir::new().expect("temp dir creates");
        let missing = base.path().join("no-such-editor");
        let error = open(
            &base.path().join("file.txt"),
            Some(&missing.to_string_lossy()),
        )
        .expect_err("missing editor fails");
        assert!(error.to_string().contains("cannot run editor"));
    }

    #[test]
    fn open_reports_editor_failure() {
        let base = TempDir::new().expect("temp dir creates");
        let target = base.path().join("missing.txt");
        let error = open(&target, Some(failing_editor())).expect_err("failing editor fails");
        assert!(error.to_string().contains("failed"));
    }
}

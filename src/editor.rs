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
    #[error("cannot parse editor command: {cause}")]
    Unparsable { cause: shell_words::ParseError },
    #[error("cannot run editor `{program}`: {cause}")]
    Spawn {
        program: String,
        cause: std::io::Error,
    },
    #[error("editor `{program}` failed: {status}")]
    Failed { program: String, status: ExitStatus },
}

/// Opens `target` in the editor described by `command` and waits for it to
/// exit. The command is parsed with shell quoting rules: unquoted words
/// split on whitespace and quotes hold a word together, so a program path
/// containing spaces is written quoted. The first word is the program, the
/// rest are its leading arguments, and `target` is appended.
///
/// # Errors
///
/// Returns an error when `command` is `None`, blank, or parses to an
/// empty program word, when it cannot be parsed, when the editor cannot
/// be started, or when it exits unsuccessfully.
pub fn open(target: &Path, command: Option<&str>) -> Result<(), Error> {
    let words = shell_words::split(command.unwrap_or_default())
        .map_err(|cause| Error::Unparsable { cause })?;
    let mut words = words.into_iter();
    let program = words
        .next()
        .filter(|program| !program.is_empty())
        .ok_or(Error::NoEditor)?;
    let status = match Command::new(&program).args(words).arg(target).status() {
        Ok(status) => status,
        Err(cause) => return Err(Error::Spawn { program, cause }),
    };
    if status.success() {
        Ok(())
    } else {
        Err(Error::Failed { program, status })
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

    /// Places a real creating editor at a path containing spaces; the
    /// file it is given appears. Unix links rather than copies, because
    /// macOS kills a copied system binary. The Windows copy is `cmd.exe`,
    /// which needs its creating arguments appended after the program.
    #[cfg(not(windows))]
    fn copied_editor(dir: &Path) -> std::path::PathBuf {
        let program = dir.join("my editor");
        std::os::unix::fs::symlink("/usr/bin/touch", &program).expect("editor links");
        program
    }

    #[cfg(not(windows))]
    fn editor_arguments() -> &'static str {
        ""
    }

    #[cfg(windows)]
    fn copied_editor(dir: &Path) -> std::path::PathBuf {
        let system_root = std::env::var("SYSTEMROOT").expect("SYSTEMROOT is set");
        let source = std::path::Path::new(&system_root)
            .join("System32")
            .join("cmd.exe");
        let program = dir.join("my editor.exe");
        std::fs::copy(source, &program).expect("editor copies");
        program
    }

    #[cfg(windows)]
    fn editor_arguments() -> &'static str {
        " /C copy /Y NUL"
    }

    #[test]
    fn open_runs_a_quoted_program_path_with_spaces() {
        let base = TempDir::new().expect("temp dir creates");
        let dir = base.path().join("space dir");
        std::fs::create_dir(&dir).expect("dir creates");
        let program = copied_editor(&dir);
        let command = format!("'{}'{}", program.display(), editor_arguments());
        let target = base.path().join("made.txt");
        open(&target, Some(&command)).expect("quoted editor succeeds");
        assert!(target.exists());
    }

    /// A command whose program word is empty — `''` and friends — names
    /// no editor at all, the same answer a blank command gets.
    #[test]
    fn open_treats_an_empty_program_as_no_editor() {
        let base = TempDir::new().expect("temp dir creates");
        let target = base.path().join("file.txt");
        let error = open(&target, Some("'' --wait")).expect_err("empty program fails");
        assert!(error.to_string().contains("no editor configured"));
    }

    #[test]
    fn open_reports_an_unparsable_command() {
        let base = TempDir::new().expect("temp dir creates");
        let target = base.path().join("file.txt");
        let error = open(&target, Some("'unclosed")).expect_err("unclosed quote fails");
        assert!(
            error.to_string().contains("cannot parse editor command"),
            "unexpected: {error}"
        );
    }

    /// Pins the shell-words behavior the editor contract is built on:
    /// quotes of either kind hold a word together and keep backslashes
    /// literal, an unquoted backslash escapes the character after it (so
    /// a Windows program path must be quoted), blank input parses to no
    /// words at all, and an unclosed quote refuses to parse.
    #[test]
    fn open_relies_on_these_shell_words_rules() {
        let split = |command| shell_words::split(command).expect("parses");
        assert_eq!(
            split(r#""C:\Program Files\Sublime Text\subl.exe" -w"#),
            vec![r"C:\Program Files\Sublime Text\subl.exe", "-w"]
        );
        assert_eq!(split(r"'C:\Tools\np.exe'"), vec![r"C:\Tools\np.exe"]);
        assert_eq!(split(r"C:\Tools\np.exe"), vec!["C:Toolsnp.exe"]);
        assert_eq!(split(" \t "), Vec::<String>::new());
        assert_eq!(split("'' x"), vec!["", "x"]);
        assert!(shell_words::split("'unclosed").is_err());
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

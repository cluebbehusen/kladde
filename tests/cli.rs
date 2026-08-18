use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::TempDir;

/// A `kladde` command with a hermetic environment.
///
/// `LLVM_PROFILE_FILE` survives the clearing so that a spawned binary built
/// under `cargo llvm-cov` still writes its coverage data; `SYSTEMROOT`
/// survives because Windows processes fail to start without it.
fn kladde() -> Command {
    let mut command = Command::cargo_bin("kladde").expect("binary builds");
    command.env_clear();
    for key in ["LLVM_PROFILE_FILE", "SYSTEMROOT"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
}

/// A `kladde` command whose config lives under the given `XDG_CONFIG_HOME`.
fn kladde_in(xdg: &Path) -> Command {
    let mut command = kladde();
    command.env("XDG_CONFIG_HOME", xdg);
    command
}

/// A `kladde` command whose lock files live under the given `XDG_STATE_HOME`.
fn kladde_state(state: &Path) -> Command {
    let mut command = kladde();
    command.env("XDG_STATE_HOME", state);
    command
}

/// A `kladde` command with lock files under `state` and a config under
/// `xdg` turning stamping off, so a write's bytes stay predictable. The
/// stamping tests pin stamping itself.
fn kladde_unstamped(state: &Path, xdg: &Path) -> Command {
    write_config(xdg, "stamp = false\n");
    let mut command = kladde_state(state);
    command.env("XDG_CONFIG_HOME", xdg);
    command
}

/// A command against a config pointing at the notebook's daily template,
/// unstamped so seeded writes stay byte-predictable.
fn kladde_templated(state: &Path, xdg: &Path) -> Command {
    write_config(
        xdg,
        "stamp = false\ndaily-template = 'templates/Daily.md'\n",
    );
    let mut command = kladde_state(state);
    command.env("XDG_CONFIG_HOME", xdg);
    command
}

/// Writes the daily template the templated config points at.
fn write_template(nb: &TempDir, contents: &str) {
    let folder = nb.path().join("templates");
    fs::create_dir_all(&folder).expect("fixture dir creates");
    fs::write(folder.join("Daily.md"), contents).expect("fixture writes");
}

/// The dated note the templated tests write.
fn dated_contents(nb: &TempDir) -> String {
    fs::read_to_string(nb.path().join("2026-01-05.md")).expect("note reads")
}

fn temp() -> TempDir {
    TempDir::new().expect("temp dir creates")
}

fn config_file(xdg: &Path) -> PathBuf {
    xdg.join("kladde").join("config.toml")
}

fn write_config(xdg: &Path, contents: &str) {
    let config_dir = xdg.join("kladde");
    fs::create_dir_all(&config_dir).expect("config dir creates");
    fs::write(config_dir.join("config.toml"), contents).expect("config file writes");
}

/// Writes a notebook's own config at its root.
fn write_notebook_config(nb: &TempDir, contents: &str) {
    fs::write(nb.path().join(".kladde.toml"), contents).expect("notebook config writes");
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

/// Editors reachable without `PATH`, which the hermetic environment clears.
/// The creating editor proves the target argument arrived by creating the
/// file it is given; the success editor ignores its argument and exits 0;
/// the failing editor exits unsuccessfully. The Windows program paths are
/// double-quoted — backslashes before letters stay literal inside shell
/// double quotes — leaving the values embeddable in single-quoted TOML.
#[cfg(not(windows))]
fn creating_editor() -> String {
    "/usr/bin/touch".to_owned()
}

#[cfg(windows)]
fn creating_editor() -> String {
    format!("\"{}\\System32\\cmd.exe\" /C copy /Y NUL", systemroot())
}

#[cfg(not(windows))]
fn success_editor() -> String {
    "/usr/bin/true".to_owned()
}

#[cfg(windows)]
fn success_editor() -> String {
    format!("\"{}\\System32\\cmd.exe\" /C rem", systemroot())
}

/// `false` ignores its argument; `type` fails on the missing config file the
/// test leaves absent.
#[cfg(not(windows))]
fn failing_editor() -> String {
    "/usr/bin/false".to_owned()
}

#[cfg(windows)]
fn failing_editor() -> String {
    format!("\"{}\\System32\\cmd.exe\" /C type", systemroot())
}

/// A creating editor placed at a path containing a space, quoted the way
/// the shell-words contract requires. Unix links rather than copies,
/// because macOS kills a copied system binary.
#[cfg(not(windows))]
fn spaced_editor(dir: &Path) -> String {
    let program = dir.join("my editor");
    std::os::unix::fs::symlink("/usr/bin/touch", &program).expect("editor links");
    format!("'{}'", program.display())
}

#[cfg(windows)]
fn spaced_editor(dir: &Path) -> String {
    let source = format!("{}\\System32\\cmd.exe", systemroot());
    let program = dir.join("my editor.exe");
    fs::copy(source, &program).expect("editor copies");
    format!("\"{}\" /C copy /Y NUL", program.display())
}

#[cfg(windows)]
fn systemroot() -> String {
    std::env::var("SYSTEMROOT").expect("SYSTEMROOT is set on Windows")
}

/// A directory link: symlink on Unix, junction on Windows (junctions need
/// no elevation and may dangle, which the dangling test relies on).
#[cfg(unix)]
fn link_dir(link: &Path, target: &Path) {
    std::os::unix::fs::symlink(target, link).expect("symlink creates");
}

#[cfg(windows)]
fn link_dir(link: &Path, target: &Path) {
    let cmd = format!("{}\\System32\\cmd.exe", systemroot());
    let status = std::process::Command::new(cmd)
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .status()
        .expect("mklink runs");
    assert!(status.success());
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).expect("path canonicalizes")
}

#[test]
fn bare_invocation_shows_usage_and_fails() {
    kladde().assert().code(2).stderr(contains("Usage: kladde"));
}

#[test]
fn version_prints_name_and_version() {
    kladde()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("kladde {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn long_help_succeeds() {
    kladde()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("markdown note"));
}

#[test]
fn config_path_honors_xdg_config_home() {
    // temp_dir is absolute on every platform; a literal "/tmp/xdg" is not
    // absolute on Windows and would be ignored per the XDG spec.
    let xdg = std::env::temp_dir().join("kladde-xdg");
    let expected = xdg.join("kladde").join("config.toml");
    kladde()
        .env("XDG_CONFIG_HOME", &xdg)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(format!("{}\n", expected.display()));
}

#[test]
fn config_path_falls_back_to_home() {
    let home = std::env::temp_dir().join("kladde-home");
    let expected = home.join(".config").join("kladde").join("config.toml");
    kladde()
        .env("HOME", &home)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(format!("{}\n", expected.display()));
}

#[test]
fn config_path_fails_without_any_base() {
    kladde()
        .args(["config", "path"])
        .assert()
        .failure()
        .stderr(contains("kladde: cannot locate the config directory"));
}

#[test]
fn config_get_prints_configured_default_notebook() {
    let xdg = temp();
    let notebook = temp();
    write_config(
        xdg.path(),
        &format!("default-notebook = '{}'\n", notebook.path().display()),
    );
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .success()
        .stdout(format!("{}\n", notebook.path().display()));
}

#[test]
fn config_get_prints_configured_editor() {
    let xdg = temp();
    write_config(xdg.path(), "editor = 'vim'\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .success()
        .stdout("vim\n");
}

#[test]
fn config_get_exits_one_when_unset() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn config_get_reports_invalid_toml() {
    let xdg = temp();
    write_config(xdg.path(), "editor = [oops\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
}

#[test]
fn config_get_reports_unknown_key() {
    let xdg = temp();
    write_config(xdg.path(), "unknown = 1\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("unknown key `unknown`"));
}

#[test]
fn config_get_reports_non_string_default_notebook() {
    let xdg = temp();
    write_config(xdg.path(), "default-notebook = 3\n");
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .code(1)
        .stderr(contains("`default-notebook` must be a string"));
}

#[test]
fn config_get_reports_relative_default_notebook() {
    let xdg = temp();
    write_config(xdg.path(), "default-notebook = 'notes'\n");
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .code(1)
        .stderr(contains("must be an absolute path"));
}

#[test]
fn config_get_reports_non_string_editor() {
    let xdg = temp();
    write_config(xdg.path(), "editor = 3\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("`editor` must be a string"));
}

#[test]
fn config_get_reports_unreadable_file() {
    let xdg = temp();
    fs::create_dir_all(config_file(xdg.path())).expect("obstacle creates");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

#[test]
fn config_get_reports_blank_editor() {
    let xdg = temp();
    write_config(xdg.path(), "editor = ' '\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("`editor` must contain a command"));
}

#[test]
fn config_set_default_notebook_round_trips() {
    let xdg = temp();
    let notebook = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(notebook.path())
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .success()
        .stdout(format!("{}\n", notebook.path().display()));
}

#[test]
fn config_set_absolutizes_relative_path() {
    let xdg = temp();
    let cwd = temp();
    fs::create_dir(cwd.path().join("notes")).expect("notebook dir creates");
    kladde_in(xdg.path())
        .current_dir(cwd.path())
        .args(["config", "set", "default-notebook", "notes"])
        .assert()
        .success();
    let assert = kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let stored = PathBuf::from(stdout.trim());
    assert!(stored.is_absolute());
    assert!(stored.ends_with("notes"));
}

#[test]
fn config_set_rejects_empty_value() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook", ""])
        .assert()
        .code(1)
        .stderr(contains("invalid path"));
}

#[test]
fn config_set_rejects_missing_directory() {
    let xdg = temp();
    let missing = xdg.path().join("nope");
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(&missing)
        .assert()
        .code(1)
        .stderr(contains("not a directory"));
}

/// Unix allows renaming over a readonly file, like every note write.
#[cfg(unix)]
#[test]
fn config_set_replaces_a_readonly_file() {
    let xdg = temp();
    let notebook = temp();
    write_config(xdg.path(), "");
    set_readonly(&config_file(xdg.path()), true);
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(notebook.path())
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .success();
    set_readonly(&config_file(xdg.path()), false);
}

#[cfg(windows)]
#[test]
fn config_set_reports_unwritable_file() {
    let xdg = temp();
    let notebook = temp();
    write_config(xdg.path(), "");
    set_readonly(&config_file(xdg.path()), true);
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(notebook.path())
        .assert()
        .code(1)
        .stderr(contains("cannot write"));
    set_readonly(&config_file(xdg.path()), false);
    let leftovers = fs::read_dir(xdg.path().join("kladde"))
        .expect("dir reads")
        .filter(|entry| {
            entry
                .as_ref()
                .expect("entry reads")
                .file_name()
                .to_string_lossy()
                .ends_with(".kladde-tmp")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[cfg(unix)]
#[test]
fn config_set_reports_an_unwritable_directory() {
    let xdg = temp();
    write_config(xdg.path(), "");
    let config_dir = xdg.path().join("kladde");
    set_readonly(&config_dir, true);
    kladde_in(xdg.path())
        .args(["config", "set", "editor", "vim"])
        .assert()
        .code(1)
        .stderr(contains("cannot write"));
    set_readonly(&config_dir, false);
}

#[cfg(unix)]
#[test]
fn config_set_reports_uncreatable_directory() {
    let xdg = temp();
    let notebook = temp();
    set_mode(xdg.path(), 0o555);
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(notebook.path())
        .assert()
        .code(1)
        .stderr(contains("cannot create"));
    set_mode(xdg.path(), 0o755);
}

#[cfg(windows)]
#[test]
fn config_set_reports_uncreatable_directory() {
    let xdg = temp();
    let notebook = temp();
    fs::write(xdg.path().join("kladde"), "").expect("obstacle writes");
    kladde_in(xdg.path())
        .args(["config", "set", "default-notebook"])
        .arg(notebook.path())
        .assert()
        .code(1)
        .stderr(contains("cannot create"));
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("permissions apply");
}

#[test]
fn config_set_editor_round_trips() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "editor", "code --wait"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .success()
        .stdout("code --wait\n");
}

#[test]
fn config_set_editor_rejects_blank_command() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "editor", " "])
        .assert()
        .code(1)
        .stderr(contains("`editor` must contain a command"));
}

#[test]
fn config_set_editor_rejects_an_unclosed_quote() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "editor", "'unclosed"])
        .assert()
        .code(1)
        .stderr(contains("`editor`:"));
    assert!(!config_file(xdg.path()).exists());
}

#[test]
fn config_rejects_an_unparsable_editor() {
    let xdg = temp();
    write_config(xdg.path(), "editor = \"'unclosed\"\n");
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stderr(contains("`editor`:"));
}

#[test]
fn config_unset_removes_key() {
    let xdg = temp();
    let notebook = temp();
    write_config(
        xdg.path(),
        &format!(
            "default-notebook = '{}'\neditor = 'vim'\n",
            notebook.path().display()
        ),
    );
    kladde_in(xdg.path())
        .args(["config", "unset", "default-notebook"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "default-notebook"])
        .assert()
        .code(1);
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .success()
        .stdout("vim\n");
}

#[test]
fn config_unset_succeeds_without_file() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "unset", "editor"])
        .assert()
        .success();
    assert!(!config_file(xdg.path()).exists());
}

#[test]
fn config_open_runs_configured_editor() {
    let xdg = temp();
    write_config(xdg.path(), &format!("editor = '{}'\n", success_editor()));
    kladde_in(xdg.path())
        .args(["config", "open"])
        .assert()
        .success();
}

#[test]
fn config_open_creates_dir_and_passes_config_path() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("VISUAL", creating_editor())
        .args(["config", "open"])
        .assert()
        .success();
    assert!(config_file(xdg.path()).exists());
}

#[test]
fn config_open_prefers_config_editor() {
    let xdg = temp();
    write_config(xdg.path(), &format!("editor = '{}'\n", success_editor()));
    kladde_in(xdg.path())
        .env("VISUAL", xdg.path().join("no-such-editor"))
        .args(["config", "open"])
        .assert()
        .success();
}

#[test]
fn config_open_falls_back_to_editor_env() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("EDITOR", creating_editor())
        .args(["config", "open"])
        .assert()
        .success();
    assert!(config_file(xdg.path()).exists());
}

#[test]
fn config_open_skips_blank_visual() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("VISUAL", "   ")
        .env("EDITOR", creating_editor())
        .args(["config", "open"])
        .assert()
        .success();
    assert!(config_file(xdg.path()).exists());
}

#[test]
fn config_open_errors_without_editor() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("no editor configured"));
}

#[test]
fn config_open_reports_spawn_failure() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("VISUAL", xdg.path().join("no-such-editor"))
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("cannot run editor"));
}

#[test]
fn config_open_reports_editor_failure() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("VISUAL", failing_editor())
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("failed"));
}

#[test]
fn config_open_runs_a_quoted_editor_with_spaces() {
    let xdg = temp();
    let dir = xdg.path().join("space dir");
    fs::create_dir(&dir).expect("dir creates");
    kladde_in(xdg.path())
        .env("VISUAL", spaced_editor(&dir))
        .args(["config", "open"])
        .assert()
        .success();
    assert!(config_file(xdg.path()).exists());
}

#[test]
fn config_open_reports_an_unparsable_visual() {
    let xdg = temp();
    kladde_in(xdg.path())
        .env("VISUAL", "'unclosed")
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("cannot parse editor command"));
}

#[test]
fn config_open_warns_on_invalid_config() {
    let xdg = temp();
    write_config(xdg.path(), "editor = [oops\n");
    kladde_in(xdg.path())
        .env("VISUAL", success_editor())
        .args(["config", "open"])
        .assert()
        .success()
        .stderr(contains("ignoring invalid config"));
}

#[cfg(unix)]
#[test]
fn config_open_reports_uncreatable_directory() {
    let xdg = temp();
    set_mode(xdg.path(), 0o555);
    kladde_in(xdg.path())
        .env("VISUAL", success_editor())
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("cannot create"));
    set_mode(xdg.path(), 0o755);
}

#[cfg(windows)]
#[test]
fn config_open_reports_uncreatable_directory() {
    let xdg = temp();
    fs::write(xdg.path().join("kladde"), "").expect("obstacle writes");
    kladde_in(xdg.path())
        .env("VISUAL", success_editor())
        .args(["config", "open"])
        .assert()
        .code(1)
        .stderr(contains("cannot create"));
}

#[test]
fn path_resolves_deep_missing_target() {
    let nb = temp();
    kladde()
        .args(["path", "a/b/c.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("a")
                .join("b")
                .join("c.md")
                .display()
        ));
}

#[test]
fn path_normalizes_leading_curdir() {
    let nb = temp();
    kladde()
        .args(["path", "./x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!("{}\n", canonical(nb.path()).join("x.md").display()));
}

#[test]
fn path_falls_back_to_default_notebook() {
    let xdg = temp();
    let nb = temp();
    write_config(
        xdg.path(),
        &format!("default-notebook = '{}'\n", nb.path().display()),
    );
    kladde_in(xdg.path())
        .args(["path", "x.md"])
        .assert()
        .success()
        .stdout(format!("{}\n", canonical(nb.path()).join("x.md").display()));
}

#[test]
fn path_flag_beats_default_notebook() {
    let xdg = temp();
    let configured = temp();
    let flagged = temp();
    write_config(
        xdg.path(),
        &format!("default-notebook = '{}'\n", configured.path().display()),
    );
    kladde_in(xdg.path())
        .args(["path", "x.md", "--notebook"])
        .arg(flagged.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(flagged.path()).join("x.md").display()
        ));
}

#[test]
fn path_resolves_relative_notebook_flag() {
    let base = temp();
    fs::create_dir(base.path().join("nb")).expect("notebook dir creates");
    kladde()
        .current_dir(base.path())
        .args(["path", "x.md", "--notebook", "nb"])
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(&base.path().join("nb")).join("x.md").display()
        ));
}

#[test]
fn path_errors_without_any_notebook() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["path", "x.md"])
        .assert()
        .code(1)
        .stderr(contains("no notebook: pass --notebook"));
}

#[test]
fn path_errors_without_config_base() {
    kladde()
        .args(["path", "x.md"])
        .assert()
        .code(1)
        .stderr(contains("cannot locate the config directory"));
}

#[test]
fn path_reports_missing_notebook() {
    let base = temp();
    kladde()
        .args(["path", "x.md", "--notebook"])
        .arg(base.path().join("gone"))
        .assert()
        .code(1)
        .stderr(contains("cannot open notebook"));
}

#[test]
fn path_reports_file_notebook() {
    let base = temp();
    let file = base.path().join("plain");
    fs::write(&file, "").expect("fixture writes");
    kladde()
        .args(["path", "x.md", "--notebook"])
        .arg(&file)
        .assert()
        .code(1)
        .stderr(contains("not a directory"));
}

#[test]
fn path_reports_broken_config() {
    let xdg = temp();
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .args(["path", "x.md"])
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
}

#[test]
fn path_ignores_broken_config_with_flag() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .args(["path", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!("{}\n", canonical(nb.path()).join("x.md").display()));
}

#[test]
fn path_rejects_rooted_target() {
    let nb = temp();
    kladde()
        .args(["path", "/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("relative paths"));
}

#[test]
fn path_rejects_parent_traversal() {
    let nb = temp();
    kladde()
        .args(["path", "../x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot leave the notebook"));
}

#[test]
fn path_rejects_empty_target() {
    let nb = temp();
    kladde()
        .args(["path", ".", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("note target is empty"));
}

#[test]
fn path_rejects_directory_target() {
    let nb = temp();
    fs::create_dir(nb.path().join("folder")).expect("fixture dir creates");
    kladde()
        .args(["path", "folder", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("not a file"));
}

#[test]
fn path_rejects_target_under_file() {
    let nb = temp();
    fs::write(nb.path().join("note.md"), "").expect("fixture writes");
    kladde()
        .args(["path", "note.md/nested.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("not a directory"));
}

#[test]
fn path_rejects_escaping_link() {
    let nb = temp();
    let outside = temp();
    link_dir(&nb.path().join("escape"), outside.path());
    kladde()
        .args(["path", "escape/new.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("escapes the notebook"));
}

#[test]
fn path_reports_dangling_link() {
    let nb = temp();
    link_dir(&nb.path().join("dangling"), &nb.path().join("gone"));
    kladde()
        .args(["path", "dangling/new.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
}

#[test]
fn path_resolves_existing_note() {
    let nb = temp();
    fs::write(nb.path().join("note.md"), "").expect("fixture writes");
    kladde()
        .args(["path", "note.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("note.md").display()
        ));
}

#[test]
fn path_reports_unprobeable_component() {
    let nb = temp();
    let overlong = "a".repeat(300);
    kladde()
        .args(["path", &overlong, "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
}

#[cfg(unix)]
#[test]
fn path_reports_unreadable_directory() {
    let nb = temp();
    let locked = nb.path().join("locked");
    fs::create_dir(&locked).expect("fixture dir creates");
    set_mode(&locked, 0o000);
    kladde()
        .args(["path", "locked/new.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
    set_mode(&locked, 0o755);
}

#[test]
fn config_get_editor_exits_one_when_unset() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "get", "editor"])
        .assert()
        .code(1)
        .stdout("");
}

/// Only Linux allows non-Unicode names to exist, and only there does the
/// lookup succeed; APFS refuses such names at lookup time.
#[cfg(target_os = "linux")]
#[test]
fn path_prints_non_unicode_target_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let nb = temp();
    let target = OsString::from_vec(b"b\xFF.md".to_vec());
    let assert = kladde()
        .arg("path")
        .arg(&target)
        .arg("--notebook")
        .arg(nb.path())
        .assert()
        .success();
    let mut expected = canonical(nb.path())
        .join(&target)
        .as_os_str()
        .as_bytes()
        .to_vec();
    expected.push(b'\n');
    assert_eq!(assert.get_output().stdout, expected);
}

#[test]
fn read_prints_the_note_verbatim() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "# T\n\nbody\n").expect("fixture writes");
    kladde()
        .args(["read", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("# T\n\nbody\n");
}

#[test]
fn read_prints_a_note_without_trailing_newline() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "no newline").expect("fixture writes");
    kladde()
        .args(["read", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("no newline");
}

/// A missing note is an error, unlike the empty note it reads the same
/// as elsewhere: an empty note prints nothing and succeeds.
#[test]
fn read_tells_an_empty_note_from_a_missing_one() {
    let nb = temp();
    fs::write(nb.path().join("empty.md"), "").expect("fixture writes");
    kladde()
        .args(["read", "empty.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("");
    kladde()
        .args(["read", "missing.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

#[test]
fn read_reports_an_unreadable_note() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), b"\xFF\xFE").expect("fixture writes");
    kladde()
        .args(["read", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

/// A reader that stops early — the `| head` shape — closes the pipe
/// while the note is still going through it; the broken pipe is the
/// reader's call, so the process ends quietly as a success. The note is
/// larger than the pipe buffer, so the write is mid-stream when the
/// reader disappears. Unix only: a Windows write succeeds even with the
/// pipe's read end closed, so the scenario cannot be produced there.
#[cfg(unix)]
#[test]
fn read_exits_cleanly_when_the_reader_stops_early() {
    let nb = temp();
    fs::write(nb.path().join("big.md"), "x".repeat(4 * 1024 * 1024)).expect("fixture writes");
    let mut command = std::process::Command::new(assert_cmd::cargo::cargo_bin("kladde"));
    command.env_clear();
    for key in ["LLVM_PROFILE_FILE", "SYSTEMROOT"] {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    let mut child = command
        .args(["read", "big.md", "--notebook"])
        .arg(nb.path())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("binary spawns");
    drop(child.stdout.take());
    let status = child.wait().expect("child waits");
    assert!(status.success(), "{status}");
}

#[test]
fn read_resolves_a_name() {
    let nb = temp();
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("Topic.md"), "found\n").expect("fixture writes");
    kladde()
        .args(["read", "--name", "topic", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("found\n");
}

#[test]
fn read_resolves_a_date() {
    let nb = temp();
    fs::write(nb.path().join("2026-01-05.md"), "that day\n").expect("fixture writes");
    kladde()
        .args(["read", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("that day\n");
}

#[test]
fn read_reports_an_ambiguous_name() {
    let nb = temp();
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("a.md"), "").expect("fixture writes");
    fs::write(nb.path().join("sub").join("a.md"), "").expect("fixture writes");
    kladde()
        .args(["read", "--name", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("multiple notes named"));
}

#[test]
fn list_prints_sorted_relative_paths() {
    let nb = temp();
    fs::write(nb.path().join("b.md"), "").expect("fixture writes");
    fs::write(nb.path().join("a.md"), "").expect("fixture writes");
    fs::write(nb.path().join(".hidden.md"), "").expect("fixture writes");
    fs::write(nb.path().join("plain.txt"), "").expect("fixture writes");
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("c.md"), "").expect("fixture writes");
    kladde()
        .args(["list", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "a.md\nb.md\n{}\n",
            Path::new("sub").join("c.md").display()
        ));
}

#[test]
fn list_succeeds_on_an_empty_notebook() {
    let nb = temp();
    kladde()
        .args(["list", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("");
}

#[test]
fn list_errors_without_any_notebook() {
    kladde()
        .arg("list")
        .assert()
        .code(1)
        .stderr(contains("cannot locate the config directory"));
}

/// Only Unix lets a file name hold a line break; such a name would let
/// one note print as several, so the listing refuses loudly with
/// nothing on stdout.
#[cfg(unix)]
#[test]
fn list_refuses_a_note_name_with_a_line_break() {
    let nb = temp();
    fs::write(nb.path().join("a.md"), "").expect("fixture writes");
    fs::write(nb.path().join("evil\nx.md"), "").expect("fixture writes");
    kladde()
        .args(["list", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("contains a line break"));
}

/// Search shares the listing refusal wherever the name sits, matched or
/// not: a forged-looking result is worse than a failed one.
#[cfg(unix)]
#[test]
fn search_refuses_a_note_name_with_a_line_break() {
    let nb = temp();
    fs::write(nb.path().join("a.md"), "alpha match\n").expect("fixture writes");
    fs::write(nb.path().join("evil\nx.md"), "gamma\n").expect("fixture writes");
    kladde()
        .args(["search", "alpha", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("contains a line break"));
}

#[cfg(unix)]
#[test]
fn list_reports_an_unreadable_folder() {
    let nb = temp();
    let sub = nb.path().join("sub");
    fs::create_dir(&sub).expect("fixture dir creates");
    set_mode(&sub, 0o000);
    kladde()
        .args(["list", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
    set_mode(&sub, 0o755);
}

#[test]
fn search_prints_matching_notes() {
    let nb = temp();
    fs::write(nb.path().join("b.md"), "the beta row\n").expect("fixture writes");
    fs::write(nb.path().join("a.md"), "Alpha and BETA\n").expect("fixture writes");
    fs::write(nb.path().join("c.md"), "gamma\n").expect("fixture writes");
    kladde()
        .args(["search", "beta", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("a.md\nb.md\n");
}

#[test]
fn search_is_case_insensitive_both_ways() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "MiXeD CaSe\n").expect("fixture writes");
    kladde()
        .args(["search", "mixed case", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\n");
    kladde()
        .args(["search", "MIXED CASE", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\n");
}

/// Search folds like name lookup: full Unicode case folding, so
/// spellings that differ by letter count still match in both
/// directions.
#[test]
fn search_folds_case_like_name_lookup() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "die Stra\u{df}e bei Nacht\n").expect("fixture writes");
    fs::write(nb.path().join("y.md"), "LOUD STRASSE HERE\n").expect("fixture writes");
    kladde()
        .args(["search", "STRASSE", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\ny.md\n");
    kladde()
        .args(["search", "stra\u{df}e", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\ny.md\n");
}

#[test]
fn search_exits_one_without_matches() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "alpha\n").expect("fixture writes");
    kladde()
        .args(["search", "zeta", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn search_rejects_an_empty_query() {
    let nb = temp();
    kladde()
        .args(["search", "", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("the query is empty"));
}

#[test]
fn search_accepts_hyphen_queries() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "uses --notebook and -q\n").expect("fixture writes");
    kladde()
        .args(["search", "-q", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\n");
    kladde()
        .args(["search", "--notebook"])
        .arg(nb.path())
        .args(["--", "--notebook"])
        .assert()
        .success()
        .stdout("x.md\n");
}

/// A read failure fails the whole search with nothing on stdout, even
/// when an earlier note already matched: a partial result would look
/// complete, and the failure exit code is also the no-match one.
#[test]
fn search_reports_an_unreadable_note() {
    let nb = temp();
    fs::write(nb.path().join("a.md"), "alpha match\n").expect("fixture writes");
    fs::write(nb.path().join("bad.md"), b"\xFF\xFE").expect("fixture writes");
    kladde()
        .args(["search", "alpha", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("")
        .stderr(contains("cannot read"));
}

#[test]
fn search_errors_without_any_notebook() {
    kladde()
        .args(["search", "alpha"])
        .assert()
        .code(1)
        .stderr(contains("cannot locate the config directory"));
}

#[cfg(unix)]
#[test]
fn search_reports_an_unreadable_folder() {
    let nb = temp();
    let sub = nb.path().join("sub");
    fs::create_dir(&sub).expect("fixture dir creates");
    set_mode(&sub, 0o000);
    kladde()
        .args(["search", "alpha", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
    set_mode(&sub, 0o755);
}

#[test]
fn open_runs_the_configured_editor_on_the_note() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), &format!("editor = '{}'\n", creating_editor()));
    kladde_in(xdg.path())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("x.md").exists());
}

/// The success editor ignores its argument, so the missing note stays
/// missing: `open` itself never creates anything.
#[test]
fn open_never_creates_the_note() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), &format!("editor = '{}'\n", success_editor()));
    kladde_in(xdg.path())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn open_falls_back_to_visual() {
    let nb = temp();
    kladde()
        .env("VISUAL", creating_editor())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("x.md").exists());
}

#[test]
fn open_falls_back_to_editor_env() {
    let nb = temp();
    kladde()
        .env("EDITOR", creating_editor())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("x.md").exists());
}

#[test]
fn open_skips_blank_visual() {
    let nb = temp();
    kladde()
        .env("VISUAL", "   ")
        .env("EDITOR", creating_editor())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("x.md").exists());
}

/// A VISUAL that parses to no program is as unset as a blank one: the
/// chain falls through instead of trying to spawn an empty editor.
#[test]
fn open_skips_a_quoted_empty_visual() {
    let nb = temp();
    kladde()
        .env("VISUAL", "''")
        .env("EDITOR", creating_editor())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("x.md").exists());
}

#[test]
fn open_resolves_a_name() {
    let nb = temp();
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("Topic.md"), "found\n").expect("fixture writes");
    kladde()
        .env("VISUAL", success_editor())
        .args(["open", "--name", "topic", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
}

#[test]
fn open_resolves_a_daily_note() {
    let nb = temp();
    kladde()
        .env("VISUAL", creating_editor())
        .args(["open", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("2026-01-05.md").exists());
}

#[test]
fn open_prefers_config_editor() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), &format!("editor = '{}'\n", success_editor()));
    kladde_in(xdg.path())
        .env("VISUAL", nb.path().join("no-such-editor"))
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
}

#[test]
fn open_errors_without_editor() {
    let nb = temp();
    kladde()
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("no editor configured"));
}

#[test]
fn open_reports_editor_failure() {
    let nb = temp();
    kladde()
        .env("VISUAL", failing_editor())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("failed"));
}

/// `open` needs the config for the daily keys and the editor, so a
/// broken config is an error here, while `path` keeps working with an
/// explicit `--notebook`.
#[test]
fn open_reports_broken_config_where_path_succeeds() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), "editor = [oops\n");
    kladde_in(xdg.path())
        .args(["open", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
    kladde_in(xdg.path())
        .args(["path", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
}

fn utc_today() -> jiff::civil::Date {
    jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date()
}

#[test]
fn config_get_prints_daily_keys() {
    let xdg = temp();
    write_config(
        xdg.path(),
        "daily-folder = 'Journal'\ndaily-date-format = '%Y/%m/%d'\n",
    );
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .success()
        .stdout("Journal\n");
    kladde_in(xdg.path())
        .args(["config", "get", "daily-date-format"])
        .assert()
        .success()
        .stdout("%Y/%m/%d\n");
}

#[test]
fn config_get_daily_keys_exit_one_when_unset() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .code(1)
        .stdout("");
    kladde_in(xdg.path())
        .args(["config", "get", "daily-date-format"])
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn config_get_reports_rooted_daily_folder() {
    let xdg = temp();
    write_config(xdg.path(), "daily-folder = '/daily'\n");
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .code(1)
        .stderr(contains("`daily-folder` must be a relative path"));
}

#[test]
fn config_get_reports_invalid_date_format() {
    let xdg = temp();
    write_config(xdg.path(), "daily-date-format = '%Q'\n");
    kladde_in(xdg.path())
        .args(["config", "get", "daily-date-format"])
        .assert()
        .code(1)
        .stderr(contains("date format \"%Q\" is invalid"));
}

#[test]
fn config_set_daily_keys_round_trip() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "Daily Notes"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-date-format", "%Y-%m-%d"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .success()
        .stdout("Daily Notes\n");
    kladde_in(xdg.path())
        .args(["config", "get", "daily-date-format"])
        .assert()
        .success()
        .stdout("%Y-%m-%d\n");
}

#[test]
fn config_set_daily_folder_rejects_rooted_path() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "/daily"])
        .assert()
        .code(1)
        .stderr(contains("must be a relative path"));
}

#[test]
fn config_set_daily_folder_rejects_empty_value() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", ""])
        .assert()
        .code(1)
        .stderr(contains("`daily-folder` must not be empty"));
}

#[test]
fn config_set_daily_date_format_rejects_invalid_format() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-date-format", "%Q"])
        .assert()
        .code(1)
        .stderr(contains("is invalid"));
}

#[test]
fn config_set_daily_date_format_rejects_empty_value() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-date-format", ""])
        .assert()
        .code(1)
        .stderr(contains("renders an empty file name"));
}

#[test]
fn config_set_daily_date_format_rejects_a_line_break() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-date-format", "%Y%n"])
        .assert()
        .code(1)
        .stderr(contains("renders a line break into a file name"));
}

#[test]
fn config_unset_daily_keys() {
    let xdg = temp();
    write_config(
        xdg.path(),
        "daily-folder = 'Journal'\ndaily-date-format = '%Y/%m/%d'\n",
    );
    kladde_in(xdg.path())
        .args(["config", "unset", "daily-folder"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "unset", "daily-date-format"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .code(1);
}

#[test]
fn config_daily_template_round_trips() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-template"])
        .assert()
        .code(1)
        .stdout("");
    kladde_in(xdg.path())
        .args(["config", "set", "daily-template", "templates/Daily.md"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-template"])
        .assert()
        .success()
        .stdout("templates/Daily.md\n");
    kladde_in(xdg.path())
        .args(["config", "unset", "daily-template"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "daily-template"])
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn config_rejects_a_bad_daily_template() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "daily-template", ""])
        .assert()
        .code(1)
        .stderr(contains("`daily-template` must not be empty"));
    kladde_in(xdg.path())
        .args(["config", "set", "daily-template", "/rooted.md"])
        .assert()
        .code(1)
        .stderr(contains("`daily-template` must be a relative path"));
    assert!(!config_file(xdg.path()).exists());
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
        write_config(xdg.path(), contents);
        kladde_in(xdg.path())
            .args(["config", "get", "daily-template"])
            .assert()
            .code(1)
            .stderr(contains(fragment));
    }
}

#[test]
fn append_seeds_a_missing_daily_from_the_template() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n\n## Log\n");
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "# 2026-01-05\n\n## Log\n- entry\n");
}

/// The whole point of seeding: the first append of the day can land
/// under a heading that only exists in the template.
#[test]
fn append_under_places_into_the_daily_template() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n\n## Log\n\n## Other\n\nx\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "append",
            "- entry",
            "--under",
            "Log",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        dated_contents(&nb),
        "# 2026-01-05\n\n## Log\n- entry\n\n## Other\n\nx\n"
    );
}

/// Every variable shape runs through the CLI; the clock-dependent parts
/// are asserted by shape, since only unit tests can pick the time.
#[test]
fn append_renders_every_template_variable() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(
        &nb,
        "t={{title}}\nd={{date}}\nc={{date:dddd, MMMM D, YYYY [at] h:mm A}}\ne={{date:YY MMM M ddd}}\nf={{time:H hh m ss s a}}\nu={{tags}}\nw={{time}}\n",
    );
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let note = dated_contents(&nb);
    assert!(note.contains("t=2026-01-05\n"), "in {note}");
    assert!(note.contains("d=2026-01-05\n"), "in {note}");
    assert!(note.contains("e=26 Jan 1 Mon\n"), "in {note}");
    assert!(note.contains("u={{tags}}\n"), "in {note}");
    let clock = note
        .lines()
        .find_map(|line| line.strip_prefix("c="))
        .expect("clock line renders");
    assert!(clock.starts_with("Monday, January 5, 2026 at "), "{clock}");
    assert!(clock.ends_with('M'), "{clock}");
    let parts: Vec<&str> = note
        .lines()
        .find_map(|line| line.strip_prefix("f="))
        .expect("token line renders")
        .split(' ')
        .collect();
    assert_eq!(parts.len(), 6, "in {note}");
    assert!(
        parts[..5]
            .iter()
            .all(|part| part.chars().all(|character| character.is_ascii_digit())),
        "in {note}"
    );
    assert_eq!(parts[1].len(), 2, "in {note}");
    assert_eq!(parts[3].len(), 2, "in {note}");
    assert!(parts[5] == "am" || parts[5] == "pm", "in {note}");
    let time = note
        .lines()
        .find_map(|line| line.strip_prefix("w="))
        .expect("time line renders");
    assert_eq!(time.len(), 5, "{time}");
    assert!(
        time.chars()
            .enumerate()
            .all(|(place, character)| if place == 2 {
                character == ':'
            } else {
                character.is_ascii_digit()
            }),
        "{time}"
    );
    assert!(note.ends_with("- entry\n"), "in {note}");
}

#[test]
fn append_with_a_broken_template_fails_to_create_a_daily() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read template"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

#[test]
fn append_with_a_broken_template_still_appends_to_an_existing_daily() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    fs::write(nb.path().join("2026-01-05.md"), "old\n").expect("fixture writes");
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "old\n- entry\n");
}

#[test]
fn append_reports_a_template_outside_the_notebook() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_config(
        xdg.path(),
        "stamp = false\ndaily-template = '../escape.md'\n",
    );
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot leave the notebook"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

#[test]
fn append_reports_a_template_with_an_unsupported_token() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "{{date:Q}}\n");
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("unsupported token"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

#[test]
fn append_reports_a_template_with_an_unclosed_bracket() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "{{date:[oops}}\n");
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("unclosed '['"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// A template carrying frontmatter still stamps as a creation: the
/// template's own properties survive and created/updated are added.
#[test]
fn append_stamps_a_seeded_daily_as_a_creation() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "---\ntags: daily\n---\nBody\n");
    write_config(xdg.path(), "daily-template = 'templates/Daily.md'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let note = dated_contents(&nb);
    assert!(note.contains("tags: daily\n"), "in {note}");
    assert!(note.contains("\ncreated: "), "in {note}");
    assert!(note.contains("\nupdated: "), "in {note}");
    assert!(note.ends_with("Body\n- entry\n"), "in {note}");
}

#[test]
fn frontmatter_set_seeds_a_missing_templated_daily() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "set",
            "topic",
            "seeds",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        dated_contents(&nb),
        "---\ntopic: seeds\n---\n# 2026-01-05\n"
    );
}

/// A set the template already satisfies still creates the note: the
/// command's outcome is a readable property, which needs the note to
/// exist, so the seed is written rather than skipped.
#[test]
fn frontmatter_set_satisfied_by_the_template_still_creates() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "---\ntopic: seeds\n---\nBody\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "set",
            "topic",
            "seeds",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "---\ntopic: seeds\n---\nBody\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "get",
            "topic",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("seeds\n");
}

/// The add twin of the satisfied-set case: an item the template's list
/// already holds still creates the note.
#[test]
fn frontmatter_add_satisfied_by_the_template_still_creates() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "---\ntags:\n  - daily\n---\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "add",
            "tags",
            "daily",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "---\ntags:\n  - daily\n---\n");
}

#[test]
fn frontmatter_set_with_a_broken_template_fails_to_create_a_daily() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "set",
            "topic",
            "seeds",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read template"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// An idempotent edit cannot create a note, seeded or not: removing a
/// property the template does not hold changes nothing, so nothing is
/// written.
#[test]
fn frontmatter_unset_on_a_missing_templated_daily_creates_nothing() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n");
    kladde_templated(state.path(), xdg.path())
        .args([
            "frontmatter",
            "unset",
            "topic",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// Seeding belongs to daily resolution: the same note addressed by its
/// explicit path starts empty, template or not.
#[test]
fn append_to_an_explicit_path_does_not_seed() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n");
    kladde_templated(state.path(), xdg.path())
        .args(["append", "- entry", "2026-01-05.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "- entry\n");
}

#[test]
fn new_creates_a_daily_from_the_template() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# {{title}}\n\n## Log\n");
    let assert = kladde_templated(state.path(), xdg.path())
        .args(["new", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "# 2026-01-05\n\n## Log\n");
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    assert!(stdout.ends_with("2026-01-05.md\n"), "{stdout}");
}

#[test]
fn new_creates_an_empty_note_with_stamps() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["new", "sub/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("sub").join("x.md")).expect("note reads");
    assert!(contents.starts_with("---\ncreated: "), "{contents}");
    assert!(contents.contains("\nupdated: "), "{contents}");
}

#[test]
fn new_unstamped_creates_a_bare_note() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    kladde_unstamped(state.path(), xdg.path())
        .args(["new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "");
}

#[test]
fn new_succeeds_on_an_existing_note_untouched() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), "keep\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "keep\n");
}

#[test]
fn new_rejects_a_name_target() {
    let nb = temp();
    kladde()
        .args(["new", "--name", "x", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(2)
        .stderr(contains("unexpected argument"));
}

#[test]
fn new_errors_without_a_state_directory() {
    let nb = temp();
    kladde()
        .args(["new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot locate the state directory"));
}

#[test]
fn new_reports_a_broken_config() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_config(xdg.path(), "editor = [oops\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
}

#[test]
fn new_reports_an_obstructed_temp_path() {
    let nb = temp();
    let state = temp();
    fs::create_dir(nb.path().join(TEMP_X)).expect("fixture dir creates");
    kladde_state(state.path())
        .args(["new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot write"));
}

#[test]
fn new_with_a_broken_template_creates_nothing() {
    let (nb, state, xdg) = (temp(), temp(), temp());
    kladde_templated(state.path(), xdg.path())
        .args(["new", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read template"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// The create-if-missing race with a template: whoever wins the lock
/// seeds, everyone else appends into the seeded note, and the template
/// head appears exactly once.
#[test]
fn append_concurrent_writers_seed_one_template() {
    const WRITERS: usize = 8;
    let (nb, state, xdg) = (temp(), temp(), temp());
    write_template(&nb, "# Day\n\n## Log\n");
    write_config(
        xdg.path(),
        "stamp = false\ndaily-template = 'templates/Daily.md'\n",
    );
    let barrier = std::sync::Barrier::new(WRITERS);
    std::thread::scope(|scope| {
        for writer in 0..WRITERS {
            let (barrier, nb, state, xdg) = (&barrier, nb.path(), state.path(), xdg.path());
            scope.spawn(move || {
                barrier.wait();
                kladde_state(state)
                    .env("XDG_CONFIG_HOME", xdg)
                    .args([
                        "append",
                        &format!("- w{writer}"),
                        "--under",
                        "Log",
                        "--date",
                        "2026-01-05",
                        "--notebook",
                    ])
                    .arg(nb)
                    .assert()
                    .success();
            });
        }
    });
    let contents = dated_contents(&nb);
    assert_eq!(
        contents.lines().filter(|line| *line == "# Day").count(),
        1,
        "in {contents}"
    );
    assert_eq!(
        contents.lines().filter(|line| *line == "## Log").count(),
        1,
        "in {contents}"
    );
    let entries: std::collections::HashSet<&str> = contents
        .lines()
        .filter(|line| line.starts_with("- w"))
        .collect();
    assert_eq!(entries.len(), WRITERS, "in {contents}");
    for writer in 0..WRITERS {
        assert!(entries.contains(format!("- w{writer}").as_str()));
    }
}

#[test]
fn path_defaults_to_todays_daily_note() {
    let nb = temp();
    let before = utc_today();
    let assert = kladde()
        .env("TZ", "UTC0")
        .args(["path", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let after = utc_today();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
    let expected = |day: jiff::civil::Date| {
        format!(
            "{}\n",
            canonical(nb.path()).join(format!("{day}.md")).display()
        )
    };
    assert!(stdout == expected(before) || stdout == expected(after));
}

#[test]
fn path_resolves_date_keywords() {
    use jiff::ToSpan;
    let nb = temp();
    let expected = |day: jiff::civil::Date| {
        format!(
            "{}\n",
            canonical(nb.path()).join(format!("{day}.md")).display()
        )
    };
    for (keyword, offset) in [("today", 0), ("yesterday", -1), ("tomorrow", 1)] {
        let before = utc_today();
        let assert = kladde()
            .env("TZ", "UTC0")
            .args(["path", "--date", keyword, "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        let after = utc_today();
        let stdout =
            String::from_utf8(assert.get_output().stdout.clone()).expect("stdout is UTF-8");
        let low = expected(before.saturating_add(offset.days()));
        let high = expected(after.saturating_add(offset.days()));
        assert!(stdout == low || stdout == high, "keyword {keyword}");
    }
}

#[test]
fn path_resolves_dated_daily_note() {
    let nb = temp();
    kladde()
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("2026-01-05.md").display()
        ));
}

#[test]
fn path_reports_invalid_date() {
    let nb = temp();
    kladde()
        .args(["path", "--date", "someday", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid date \"someday\""));
}

#[test]
fn path_daily_uses_configured_folder_and_default_notebook() {
    let xdg = temp();
    let nb = temp();
    write_config(
        xdg.path(),
        &format!(
            "default-notebook = '{}'\ndaily-folder = 'Journal'\n",
            nb.path().display()
        ),
    );
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05"])
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("Journal")
                .join("2026-01-05.md")
                .display()
        ));
}

#[test]
fn path_daily_uses_configured_format() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), "daily-date-format = '%Y/%m/%d'\n");
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("2026")
                .join("01")
                .join("05.md")
                .display()
        ));
}

#[test]
fn path_daily_folder_stays_inside_the_notebook() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), "daily-folder = '..'\n");
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot leave the notebook"));
}

#[test]
fn path_daily_reports_broken_config_despite_flag() {
    let xdg = temp();
    let nb = temp();
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
}

#[test]
fn path_daily_works_without_config_base_given_flag() {
    let nb = temp();
    kladde()
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
}

#[test]
fn path_daily_fails_without_config_base_or_flag() {
    kladde()
        .arg("path")
        .assert()
        .code(1)
        .stderr(contains("cannot locate the config directory"));
}

#[test]
fn path_daily_fails_without_any_notebook() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05"])
        .assert()
        .code(1)
        .stderr(contains("no notebook: pass --notebook"));
}

#[test]
fn path_resolves_name() {
    let nb = temp();
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("b.md"), "").expect("fixture writes");
    fs::create_dir(nb.path().join(".hidden")).expect("fixture dir creates");
    fs::write(nb.path().join(".hidden").join("b.md"), "").expect("fixture writes");
    kladde()
        .args(["path", "--name", "b", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("sub").join("b.md").display()
        ));
}

#[test]
fn path_reports_ambiguous_name() {
    let nb = temp();
    fs::write(nb.path().join("a.md"), "").expect("fixture writes");
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("a.md"), "").expect("fixture writes");
    kladde()
        .args(["path", "--name", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("multiple notes named \"a\""));
}

#[test]
fn path_reports_missing_name() {
    let nb = temp();
    kladde()
        .args(["path", "--name", "nope", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("no note named \"nope\""));
}

#[test]
fn path_rejects_empty_name() {
    let nb = temp();
    kladde()
        .args(["path", "--name", "", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("note target is empty"));
}

#[test]
fn path_name_ignores_broken_config_with_flag() {
    let xdg = temp();
    let nb = temp();
    fs::write(nb.path().join("a.md"), "").expect("fixture writes");
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .args(["path", "--name", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
}

#[cfg(unix)]
#[test]
fn path_name_reports_unreadable_directory() {
    let nb = temp();
    let locked = nb.path().join("locked");
    fs::create_dir(&locked).expect("fixture dir creates");
    set_mode(&locked, 0o000);
    kladde()
        .args(["path", "--name", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
    set_mode(&locked, 0o755);
}

#[test]
fn path_rejects_conflicting_targets() {
    let nb = temp();
    kladde()
        .args(["path", "x.md", "--name", "y", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(2)
        .stderr(contains("cannot be used with"));
}

#[test]
fn append_creates_a_missing_note() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- first", "notes/new.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("notes").join("new.md").display()
        ));
    let contents = fs::read_to_string(nb.path().join("notes").join("new.md")).expect("note reads");
    assert_eq!(contents, "- first\n");
}

#[test]
fn append_appends_to_an_existing_note() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "start\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- next", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "start\n- next\n");
}

#[test]
fn append_inserts_a_missing_separator() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "no newline").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- next", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "no newline\n- next\n");
}

#[test]
fn append_strips_trailing_newlines_from_the_text() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- entry\n\n", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "- entry\n");
}

#[test]
fn append_keeps_interior_newlines() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- parent\n\t- child", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "- parent\n\t- child\n");
}

#[test]
fn append_accepts_flags_before_the_text() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .arg("append")
        .arg("--notebook")
        .arg(nb.path())
        .args(["- dashed", "x.md"])
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "- dashed\n");
}

/// After a `--`, even text spelled exactly like an option is a positional.
#[test]
fn append_accepts_option_shaped_text_after_a_separator() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .arg("append")
        .arg("--notebook")
        .arg(nb.path())
        .args(["--", "--name", "x.md"])
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "--name\n");
}

#[test]
fn append_rejects_empty_text() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["append", "", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("nothing to append"));
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn append_requires_text() {
    kladde()
        .arg("append")
        .assert()
        .code(2)
        .stderr(contains("Usage"));
}

#[test]
fn append_rejects_conflicting_targets() {
    let nb = temp();
    kladde()
        .args(["append", "- x", "x.md", "--name", "y", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(2)
        .stderr(contains("cannot be used with"));
}

#[test]
fn append_resolves_name() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::create_dir(nb.path().join("sub")).expect("fixture dir creates");
    fs::write(nb.path().join("sub").join("b.md"), "start\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- found", "--name", "b", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("sub").join("b.md").display()
        ));
    let contents = fs::read_to_string(nb.path().join("sub").join("b.md")).expect("note reads");
    assert_eq!(contents, "start\n- found\n");
}

/// A note targeted by name must already exist: there is no path to create.
#[test]
fn append_missing_name_creates_nothing() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["append", "- x", "--name", "nope", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("no note named \"nope\""));
    let entries = fs::read_dir(nb.path()).expect("notebook reads");
    assert_eq!(entries.count(), 0);
}

#[test]
fn append_daily_uses_configured_folder_and_format() {
    let xdg = temp();
    let nb = temp();
    let state = temp();
    write_config(
        xdg.path(),
        &format!(
            "default-notebook = '{}'\ndaily-folder = 'Journal'\ndaily-date-format = '%Y/%m/%d'\nstamp = false\n",
            nb.path().display()
        ),
    );
    kladde_in(xdg.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["append", "- daily", "--date", "2026-01-05"])
        .assert()
        .success();
    let note = nb
        .path()
        .join("Journal")
        .join("2026")
        .join("01")
        .join("05.md");
    let contents = fs::read_to_string(note).expect("note reads");
    assert_eq!(contents, "- daily\n");
}

#[test]
fn append_daily_works_without_config_base_given_flag() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["append", "- daily", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("2026-01-05.md")).expect("note reads");
    assert!(contents.starts_with("---\ncreated: 2"), "{contents:?}");
    assert!(contents.contains("\nupdated: 2"), "{contents:?}");
    assert!(contents.ends_with("---\n- daily\n"), "{contents:?}");
}

#[test]
fn append_daily_reports_broken_config_despite_flag() {
    let xdg = temp();
    let nb = temp();
    let state = temp();
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["append", "- x", "--date", "today", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
}

#[test]
fn append_fails_without_state_base() {
    let nb = temp();
    kladde()
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot locate the state directory"));
}

#[test]
fn append_falls_back_to_home_state() {
    let nb = temp();
    let home = temp();
    kladde()
        .env("HOME", home.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let locks = home
        .path()
        .join(".local")
        .join("state")
        .join("kladde")
        .join("locks");
    let entries = fs::read_dir(locks).expect("locks dir reads");
    assert_eq!(entries.count(), 1);
}

#[test]
fn append_rejects_a_note_that_is_not_utf8() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), [0xff, 0xfe, 0xfd]).expect("fixture writes");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

#[test]
fn append_reports_an_obstructed_lock_dir() {
    let nb = temp();
    let state = temp();
    fs::create_dir(state.path().join("kladde")).expect("fixture dir creates");
    fs::write(state.path().join("kladde").join("locks"), "").expect("fixture writes");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot create lock directory"));
}

#[test]
fn append_reports_an_obstructed_lock_file() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let locks = state.path().join("kladde").join("locks");
    let lock_file = fs::read_dir(&locks)
        .expect("locks dir reads")
        .next()
        .expect("a lock file exists")
        .expect("entry reads")
        .path();
    fs::remove_file(&lock_file).expect("lock file removes");
    fs::create_dir(&lock_file).expect("fixture dir creates");
    kladde_state(state.path())
        .args(["append", "- y", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot lock"));
}

/// The temporary file name for a note named `x.md`, pinned by the
/// `temp_name_is_stable` unit test.
const TEMP_X: &str = ".72d8d45320ac7f62.kladde-tmp";

#[test]
fn append_reports_an_obstructed_temp_path() {
    let nb = temp();
    let state = temp();
    fs::create_dir(nb.path().join(TEMP_X)).expect("fixture dir creates");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot write"));
}

/// A link planted at the temporary path is deleted, never followed: the
/// append succeeds without touching what the link pointed at.
#[cfg(unix)]
#[test]
fn append_ignores_a_planted_temp_link() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let outside = temp();
    let precious = outside.path().join("precious");
    fs::write(&precious, "untouched").expect("fixture writes");
    std::os::unix::fs::symlink(&precious, nb.path().join(TEMP_X)).expect("symlink creates");
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&precious).expect("outside file reads"),
        "untouched"
    );
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "- x\n");
}

/// Runs `append` with stamping off against `x.md` holding `contents`,
/// passing the placement `args`, and returns the assertion.
fn append_into(
    nb: &TempDir,
    state: &TempDir,
    xdg: &TempDir,
    contents: &str,
    entry: &str,
    args: &[&str],
) -> assert_cmd::assert::Assert {
    fs::write(nb.path().join("x.md"), contents).expect("fixture writes");
    let mut command = kladde_unstamped(state.path(), xdg.path());
    command
        .args(["append", entry, "x.md"])
        .args(args)
        .arg("--notebook")
        .arg(nb.path());
    command.assert()
}

fn x_contents(nb: &TempDir) -> String {
    fs::read_to_string(nb.path().join("x.md")).expect("note reads")
}

#[test]
fn append_under_lands_at_the_section_end() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\nalpha\n# B\nbeta\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\nalpha\n- e\n# B\nbeta\n");
}

#[test]
fn append_under_descends_headings() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n## kladde\nx\n## other\ny\n",
        "- e",
        &["--under", "A", "--under", "kladde"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n## kladde\nx\n- e\n## other\ny\n");
}

#[test]
fn append_under_reads_heading_shapes() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        ("Title\n=====\nx\n", "Tit", "Title\n=====\nx\n- e\n"),
        ("## Foo ##\nx\n", "Foo", "## Foo ##\nx\n- e\n"),
        ("#hash\n===\nx\n", "#hash", "#hash\n===\nx\n- e\n"),
        (
            "####### Foo\n===\nx\n",
            "####### Foo",
            "####### Foo\n===\nx\n- e\n",
        ),
        ("# C#\nx\n", "C#", "# C#\nx\n- e\n"),
        ("  # A\nx\n# B\n", "A", "  # A\nx\n- e\n# B\n"),
        ("# A\n# B\n", "A", "# A\n- e\n# B\n"),
        ("# A\nx\n\n\n# B\n", "A", "# A\nx\n- e\n\n\n# B\n"),
        ("# A\nbody", "A", "# A\nbody\n- e\n"),
        (
            "# A\n```rust\ncode\n```",
            "A",
            "# A\n```rust\ncode\n```\n- e\n",
        ),
        (
            "# A\n<!DOCTYPE html>\n# B\n",
            "A",
            "# A\n<!DOCTYPE html>\n- e\n# B\n",
        ),
        (
            "# A\n<pretend>\nraw\n\n# B\n",
            "A",
            "# A\n<pretend>\nraw\n\n- e\n# B\n",
        ),
        ("# A\n*em* text\n# B\n", "A", "# A\n*em* text\n- e\n# B\n"),
        (
            "# A\n<script>\na\n</script>\n\n# B\n",
            "A",
            "# A\n<script>\na\n</script>\n- e\n\n# B\n",
        ),
        (
            "---\r\nk: v\r\n---\r\n# A",
            "A",
            "---\r\nk: v\r\n---\r\n# A\r\n- e\r\n",
        ),
    ];
    for (contents, query, expected) in cases {
        append_into(&nb, &state, &xdg, contents, "- e", &["--under", query]).success();
        assert_eq!(x_contents(&nb), expected, "for {contents:?}");
    }
}

#[test]
fn append_under_bullet_uses_a_tab_by_default() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- parent\n- other\n",
        "- e",
        &["--under-bullet", "parent"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- parent\n\t- e\n- other\n");
}

#[test]
fn append_under_bullet_copies_an_existing_child_indent() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- parent\n  - child\n",
        "- e",
        &["--under-bullet", "parent"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- parent\n  - child\n  - e\n");
}

#[test]
fn append_under_bullet_honors_spaces_mode() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp = false\nbullet-indent = 'spaces'\n");
    fs::write(nb.path().join("x.md"), "1. a\n").expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- e", "x.md", "--under-bullet", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "1. a\n   - e\n");
}

#[test]
fn append_under_bullet_descends_a_thread() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- a\n\t- b\n\t\t- c\n- d\n",
        "- e",
        &["--under-bullet", "a", "--under-bullet", "b"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- a\n\t- b\n\t\t- c\n\t\t- e\n- d\n");
}

#[test]
fn append_under_bullet_reads_loose_and_unterminated_items() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- a\n\n- b\n",
        "- e",
        &["--under-bullet", "a"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- a\n\t- e\n\n- b\n");
    append_into(&nb, &state, &xdg, "- a", "- e", &["--under-bullet", "a"]).success();
    assert_eq!(x_contents(&nb), "- a\n\t- e\n");
}

/// The parser reads bare carriage returns inconsistently, so placed
/// appends refuse such notes outright; a plain append still works, as
/// it claims no structure. An entry's own bare CR is normalized before
/// it ever reaches the note.
#[test]
fn append_under_refuses_bare_carriage_returns() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        ("# A\rbody\r", &["--under", "A"][..]),
        ("- p\r  - c\r", &["--under-bullet", "p"][..]),
        ("# A\r\nbody\r", &["--under", "A"][..]),
    ];
    for (contents, args) in cases {
        append_into(&nb, &state, &xdg, contents, "- e", args)
            .code(1)
            .stderr(contains("bare carriage-return line endings"));
        assert_eq!(x_contents(&nb), contents, "for {contents:?}");
    }
    append_into(&nb, &state, &xdg, "# A\rbody\r", "- e", &[]).success();
    assert_eq!(x_contents(&nb), "# A\rbody\r\n- e\n");
}

/// A plain entry directly after a quotation would read as its lazy
/// continuation, so the quote is terminated with a blank line first.
#[test]
fn append_under_stays_out_of_quotations() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n> quoted\n# B\n",
        "plain",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n> quoted\n\nplain\n# B\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  > q\n- q2\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n  > q\n\n\t- e\n- q2\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n> early\n\nlate\n# B\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n> early\n\nlate\n- e\n# B\n");
}

/// A fence inside an item closes with an indent measured against the
/// item's content column; cut off by the note's end, it still counts as
/// closed and the entry follows it.
#[test]
fn append_under_bullet_follows_a_nested_closed_fence() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  ```\n  code\n  ```",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n  ```\n  code\n  ```\n\t- e\n");
}

#[test]
fn append_under_heading_and_bullet_together() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n- todo\n# B\n- todo\n",
        "- e",
        &["--under", "B", "--under-bullet", "todo"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n- todo\n# B\n- todo\n\t- e\n");
}

#[test]
fn append_under_indents_every_entry_line() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- a\n",
        "- e\n\nmore\r\nlast\rtail",
        &["--under-bullet", "a"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- a\n\t- e\n\n\tmore\n\tlast\n\ttail\n");
}

#[test]
fn append_under_keeps_a_crlf_note_crlf() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\r\nx\r\n# B\r\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\r\nx\r\n- e\r\n# B\r\n");
}

/// Pseudo-structure never matches: fenced and indented code are content,
/// quoted structure is not kladde's to extend, and a heading indented
/// into a list item is item content, not a section boundary.
#[test]
fn append_under_ignores_disguised_structure() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    // The paragraph before the indented code matters: four-space
    // content directly after a list item is item continuation, not code.
    let contents =
        "# A\n```\n# fake\n- fake\n```\n> # H\n> - q\n- item\n  # inner\n\npara\n\n    - code\n";
    let cases = [
        (&["--under", "fake"][..], "no heading matching \"fake\""),
        (
            &["--under-bullet", "fake"][..],
            "no bullet matching \"fake\"",
        ),
        (&["--under", "H"][..], "no heading matching \"H\""),
        (&["--under-bullet", "q"][..], "no bullet matching \"q\""),
        (&["--under", "inner"][..], "no heading matching \"inner\""),
        (
            &["--under-bullet", "code"][..],
            "no bullet matching \"code\"",
        ),
    ];
    for (args, fragment) in cases {
        append_into(&nb, &state, &xdg, contents, "- e", args)
            .code(1)
            .stderr(contains(fragment));
        assert_eq!(x_contents(&nb), contents, "for {args:?}");
    }
}

#[test]
fn append_under_reports_unmatched_and_invalid_targets() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        (
            "# Alpha\n# Alp\n",
            &["--under", "Alp"][..],
            "multiple headings matching \"Alp\"",
        ),
        (
            "- x\n\t- xy\n",
            &["--under-bullet", "x"][..],
            "multiple bullets matching \"x\"",
        ),
        ("#\nx\n", &["--under", "x"][..], "no heading matching \"x\""),
        (
            "# #\nx\n",
            &["--under", "#"][..],
            "no heading matching \"#\"",
        ),
        (
            "# A\n",
            &["--under", ""][..],
            "invalid heading \"\": it is empty",
        ),
        (
            "# A\n",
            &["--under-bullet", ""][..],
            "invalid bullet \"\": it is empty",
        ),
        (
            "# A\n",
            &["--under", "a\nb"][..],
            "invalid heading \"a\\nb\": it contains a line break",
        ),
        (
            "# A\n",
            &["--under-bullet", "a\rb"][..],
            "invalid bullet \"a\\rb\": it contains a line break",
        ),
        (
            "-\n  ```\n  code\n",
            &["--under-bullet", "x"][..],
            "no bullet matching \"x\"",
        ),
    ];
    for (contents, args, fragment) in cases {
        append_into(&nb, &state, &xdg, contents, "- e", args)
            .code(1)
            .stderr(contains(fragment));
        assert_eq!(x_contents(&nb), contents, "for {args:?}");
    }
}

/// A placement can never match in a missing note, so the append fails
/// rather than create a note holding only the entry.
#[test]
fn append_under_missing_note_creates_nothing() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- e", "missing.md", "--under", "A", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("no heading matching \"A\""));
    assert!(!nb.path().join("missing.md").exists());
}

#[test]
fn append_under_skips_frontmatter() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let contents = "---\ntitle: x\n---\n# A\nbody\n";
    append_into(&nb, &state, &xdg, contents, "- e", &["--under", "title"])
        .code(1)
        .stderr(contains("no heading matching \"title\""));
    append_into(&nb, &state, &xdg, contents, "- e", &["--under", "A"]).success();
    assert_eq!(x_contents(&nb), "---\ntitle: x\n---\n# A\nbody\n- e\n");
}

#[test]
fn append_under_stamps_the_placed_write() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    fs::write(nb.path().join("x.md"), "# A\nx\n").expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- e", "x.md", "--under", "A", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        x_contents(&nb),
        "---\ncreated: X\nupdated: X\n---\n# A\nx\n- e\n"
    );
}

/// A closing `#` run is heading syntax, not text, so `Foo #` names only
/// the heading whose text really continues past the hash.
#[test]
fn append_under_disambiguates_closing_hashes() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# Foo #\nx\n# Foo # bar\ny\n",
        "- e",
        &["--under", "Foo #"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# Foo #\nx\n# Foo # bar\ny\n- e\n");
}

/// The entry must reach the parent's content column or markdown does
/// not nest it; descending through the new child immediately proves the
/// written structure is real.
#[test]
fn append_under_tab_reaches_the_content_column() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "100. p\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "100. p\n\t\t- e\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "append",
            "- x",
            "x.md",
            "--under-bullet",
            "p",
            "--under-bullet",
            "e",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "100. p\n\t\t- e\n\t\t\t- x\n");
}

/// An entry near raw HTML lands after the blank that terminates the
/// block, never inside it; HTML cut off by the end of the note gets its
/// terminating blank written, and a self-closed block needs neither.
#[test]
fn append_under_respects_raw_html() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n<div>\nraw\n\n# B\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n<div>\nraw\n\n- e\n# B\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "append",
            "- x",
            "x.md",
            "--under",
            "A",
            "--under-bullet",
            "e",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "# A\n<div>\nraw\n\n- e\n\t- x\n# B\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n<div>\nraw",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n<div>\nraw\n\n- e\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n<script>\na\n</script>\n# B\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n<script>\na\n</script>\n- e\n# B\n");
}

/// An entry can never be smuggled into code or raw HTML: an unclosed
/// fence or marker-terminated HTML block refuses the placement outright.
#[test]
fn append_under_rejects_unclosed_code_and_html() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let fence = "unclosed code fence";
    let html = "unclosed raw HTML";
    let cases = [
        ("# A\n~~~\ncode\n", &["--under", "A"][..], fence),
        ("# A\n```\n", &["--under", "A"][..], fence),
        ("# A\n````\ncode\n```\n", &["--under", "A"][..], fence),
        ("# A\n~~~\ncode\n    ~~~\n", &["--under", "A"][..], fence),
        ("# A\n```\ncode\n```\t\n", &["--under", "A"][..], fence),
        ("- p\n  ```\n  code\n", &["--under-bullet", "p"][..], fence),
        (
            "- p\n  ```\n  code\n      ```\n",
            &["--under-bullet", "p"][..],
            fence,
        ),
        ("# A\n<script>\nraw\n", &["--under", "A"][..], html),
        (
            "# A\n<script>\nraw\n</style>\npara\n",
            &["--under", "A"][..],
            html,
        ),
        ("# A\n<script\u{c}>\nraw\n", &["--under", "A"][..], html),
        ("# A\n<script>\né", &["--under", "A"][..], html),
        ("# A\n<!-- note\n", &["--under", "A"][..], html),
        ("# A\n<?process\n", &["--under", "A"][..], html),
        ("# A\n<![CDATA[\n", &["--under", "A"][..], html),
        (
            "- p\n  <script>\n  raw\n- q\n",
            &["--under-bullet", "p"][..],
            html,
        ),
    ];
    for (contents, args, fragment) in cases {
        append_into(&nb, &state, &xdg, contents, "- e", args)
            .code(1)
            .stderr(contains(fragment));
        assert_eq!(x_contents(&nb), contents, "for {contents:?}");
    }
}

/// A blank-terminated HTML block cut off by a sibling item gets its
/// terminating blank written, so the entry stays out of the HTML and
/// can be descended through.
#[test]
fn append_under_closes_html_cut_by_a_sibling() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  <div>\n  raw\n- q\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n  <div>\n  raw\n\n\t- e\n- q\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "append",
            "- x",
            "x.md",
            "--under-bullet",
            "p",
            "--under-bullet",
            "e",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        x_contents(&nb),
        "- p\n  <div>\n  raw\n\n\t- e\n\t\t- x\n- q\n"
    );
}

/// Five or more spaces after a marker are one space of padding plus
/// indented code, so the content column snaps back to the marker and
/// the child still nests.
#[test]
fn append_under_respects_list_padding() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "-     p\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "-     p\n\t- e\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "append",
            "- x",
            "x.md",
            "--under-bullet",
            "p",
            "--under-bullet",
            "e",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "-     p\n\t- e\n\t\t- x\n");
}

/// The splice must leave surrounding structure meaning what it meant:
/// a blank line restores the boundary where one can, and the placement
/// is refused where none can.
#[test]
fn append_under_preserves_surrounding_structure() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n```\nc\n```\nB\n===\ny\n",
        "- e",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n```\nc\n```\n- e\n\nB\n===\ny\n");
    kladde_unstamped(state.path(), xdg.path())
        .args(["append", "- x", "x.md", "--under", "B", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n- item\n# B\n",
        "plain",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n- item\n\nplain\n# B\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  > q\n\npara\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n  > q\n\n\t- e\n\npara\n");
    let indented = "# A\np\n  # B\ny\n";
    append_into(&nb, &state, &xdg, indented, "- e", &["--under", "A"])
        .code(1)
        .stderr(contains("the entry would change the structure around it"));
    assert_eq!(x_contents(&nb), indented);
}

/// An entry carrying a heading must not reparent the sections after it,
/// and one that would complete a half-open frontmatter fence is refused
/// before the boundary can move.
#[test]
fn append_under_guards_surrounding_meaning() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let reparenting = "# A\n## B\nx\n## C\ny\n";
    append_into(
        &nb,
        &state,
        &xdg,
        reparenting,
        "# New",
        &["--under", "A", "--under", "B"],
    )
    .code(1)
    .stderr(contains("the entry would change the structure around it"));
    assert_eq!(x_contents(&nb), reparenting);
    append_into(
        &nb,
        &state,
        &xdg,
        reparenting,
        "### deep",
        &["--under", "A", "--under", "B"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n## B\nx\n### deep\n## C\ny\n");
    let half_open = "---\n# A\nx\n";
    append_into(&nb, &state, &xdg, half_open, "---", &["--under", "A"])
        .code(1)
        .stderr(contains("the entry would change the structure around it"));
    assert_eq!(x_contents(&nb), half_open);
}

/// Plain text under a parent with children becomes the parent's own
/// paragraph, and a multiline entry owns its block at its first line.
#[test]
fn append_under_anchors_entries_to_their_own_blocks() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- parent\n  - child\n",
        "plain",
        &["--under-bullet", "parent"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- parent\n  - child\n\n  plain\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\n- item\n# B\n",
        "new\n- later",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\n- item\n\nnew\n- later\n# B\n");
    append_into(&nb, &state, &xdg, "# A\nx\n# B\n", "---", &["--under", "A"]).success();
    assert_eq!(x_contents(&nb), "# A\nx\n\n---\n# B\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "# A\nx\n",
        "[ref]: /url",
        &["--under", "A"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# A\nx\n\n[ref]: /url\n");
}

#[test]
fn append_under_copies_the_last_direct_childs_indent() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n    - deep\n  - shallow\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n    - deep\n  - shallow\n  - e\n");
}

/// A sublist can open on its parent's marker line: the indent written
/// beside it is column-equivalent whitespace, never a copied marker,
/// and a descent chain can reach it.
#[test]
fn append_under_reaches_marker_line_children() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- - beta\n",
        "- e",
        &["--under-bullet", "- beta"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- - beta\n  - e\n");
    append_into(
        &nb,
        &state,
        &xdg,
        "- - beta\n",
        "- e",
        &["--under-bullet", "- beta", "--under-bullet", "beta"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- - beta\n\t- e\n");
}

/// An entry indented below an unclosed block's container ends the
/// container and the block with it; the placement is safe and the new
/// entry is immediately addressable.
#[test]
fn append_under_deindents_past_contained_blocks() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  - c\n    ```\n    code\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(x_contents(&nb), "- p\n  - c\n    ```\n    code\n  - e\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "append",
            "- x",
            "x.md",
            "--under-bullet",
            "p",
            "--under-bullet",
            "e",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        x_contents(&nb),
        "- p\n  - c\n    ```\n    code\n  - e\n  \t- x\n"
    );
    append_into(
        &nb,
        &state,
        &xdg,
        "- p\n  - c\n    <script>\n    raw\n",
        "- e",
        &["--under-bullet", "p"],
    )
    .success();
    assert_eq!(
        x_contents(&nb),
        "- p\n  - c\n    <script>\n    raw\n  - e\n"
    );
}

#[test]
fn append_under_accepts_hyphen_targets() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    append_into(
        &nb,
        &state,
        &xdg,
        "# -Head\n- -dash\n",
        "- e",
        &["--under", "-Head", "--under-bullet", "-dash"],
    )
    .success();
    assert_eq!(x_contents(&nb), "# -Head\n- -dash\n\t- e\n");
}

/// `path` reports where a note would live without creating it; only
/// `append` creates.
#[test]
fn path_creates_nothing() {
    let nb = temp();
    kladde()
        .args(["path", "missing.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(!nb.path().join("missing.md").exists());
}

/// Runs `frontmatter get k` against a note holding `contents` and
/// returns the assertion. Reads take no lock, so no state dir is set:
/// success doubles as proof that reads work without one.
fn get_k(nb: &TempDir, contents: &str) -> assert_cmd::assert::Assert {
    fs::write(nb.path().join("x.md"), contents).expect("fixture writes");
    kladde()
        .args(["frontmatter", "get", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
}

#[test]
fn frontmatter_get_reads_scalar_shapes() {
    let nb = temp();
    let cases = [
        ("---\nk: v\n---\n", "v\n"),
        ("\u{feff}---\nk: v\n---\n", "v\n"),
        ("---\r\nk: v\r\n---\r\nbody\r\n", "v\n"),
        ("---\nk:   v  \n---\n", "v\n"),
        ("---\nk:\n---\n", "\n"),
        ("---\nk: \"a \\\"b\\\" \\\\ c\"\n---\n", "a \"b\" \\ c\n"),
        ("---\nk: 'it''s'\n---\n", "it's\n"),
        ("---\nk: -1\n---\n", "-1\n"),
        ("---\nother: x\nk: v\n---\nbody\n", "v\n"),
        ("---\nk: v\n\"other\": kept\n---\n", "v\n"),
        ("---\n'k': v\n---\n", "v\n"),
        ("---\n\"k\":\n---\n", "\n"),
        ("---\nk: \"a\\/b\"\n---\n", "a/b\n"),
        ("---\nk: \"a\\ b\"\n---\n", "a b\n"),
        ("---\nk: \"a\\tb\"\n---\n", "a\tb\n"),
        ("---\nk: \"a\\_b\"\n---\n", "a\u{a0}b\n"),
        ("---\nk: \"\\x41\"\n---\n", "A\n"),
        ("---\nk: \"\\u0064\"\n---\n", "d\n"),
        ("---\nk: \"\\U0001F4DD\"\n---\n", "\u{1f4dd}\n"),
        ("---\nk:\tv\n---\n", "v\n"),
        ("---\n\"k\":\tv\n---\n", "v\n"),
        ("---\nk: \u{a0}v\n---\n", "\u{a0}v\n"),
        ("---\nk: old\n  # keep\n---\n", "old\n"),
        ("---\n  # note\nk: v\n---\n", "v\n"),
    ];
    for (contents, expected) in cases {
        get_k(&nb, contents).success().stdout(expected.to_owned());
    }
}

#[test]
fn frontmatter_get_reads_list_shapes() {
    let nb = temp();
    let cases = [
        ("---\nk: [a, \"b c\", 'd']\n---\n", "a\nb c\nd\n"),
        ("---\nk:\n  - a\n  - b c\n---\n", "a\nb c\n"),
        ("---\nk:\n- a\n- b\n---\n", "a\nb\n"),
        ("---\nk:\n  - a\n  -\n---\n", "a\n\n"),
        ("---\nk: []\n---\n", ""),
        ("---\nk:\n  - a\n\n# note\nother: x\n---\n", "a\n"),
    ];
    for (contents, expected) in cases {
        get_k(&nb, contents).success().stdout(expected.to_owned());
    }
}

#[test]
fn frontmatter_get_misses_quietly() {
    let nb = temp();
    let cases = [
        "body\n",
        "",
        "\n---\nk: v\n---\n",
        "--- \nk: v\n---\n",
        "---\nk: v\nbody\n",
        "---\nk: v\n...\n",
        "x\n---\nk: v\n---\n",
        "---\n---\nbody\n",
        "---\nk:value\n---\n",
        "---\n# comment\n: v\nother: x\n---\n",
        "---\n\"\": k\n---\n",
        "---\n\"k\" v\n---\n",
        "---\n\"k\" x: v\n---\n",
        "---\nk\u{a0}: v\n---\n",
    ];
    for contents in cases {
        get_k(&nb, contents).code(1).stdout("").stderr("");
    }
}

#[test]
fn frontmatter_get_misses_a_missing_note() {
    let nb = temp();
    kladde()
        .args(["frontmatter", "get", "k", "missing.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("")
        .stderr("");
    assert!(!nb.path().join("missing.md").exists());
}

#[test]
fn frontmatter_get_rejects_out_of_subset_values() {
    let nb = temp();
    let cases = [
        "---\nk:\n  sub: x\n---\n",
        "---\nk: v\n  continued\n---\n",
        "---\nk: |\n  text\n---\n",
        "---\nk: >\n  text\n---\n",
        "---\nk: |\n---\n",
        "---\nk: {a: b}\n---\n",
        "---\nk: v # note\n---\n",
        "---\nk: &anchor\n---\n",
        "---\nk: *alias\n---\n",
        "---\nk: \"a \\q\"\n---\n",
        "---\nk: \"a\n---\n",
        "---\nk: 'a\n---\n",
        "---\nk: \"a\" b\n---\n",
        "---\nk: 'a' b\n---\n",
        "---\nk: [[a], b]\n---\n",
        "---\nk: [a, {b: c}]\n---\n",
        "---\nk: [a,]\n---\n",
        "---\nk: [a, , b]\n---\n",
        "---\nk: [a\n---\n",
        "---\nk: [\"a\" b]\n---\n",
        "---\nk:\n  - a\n    - b\n---\n",
        "---\nk:\n  - a\n  # note\n  - b\n---\n",
        "---\nk:\n  - a\n\n  - b\n---\n",
        "---\nk:\n  - a # note\n---\n",
        "---\nk:\n  - name: Alice\n---\n",
        "---\nk:\n  - - a\n---\n",
        "---\nk:\n  - [a, b]\n---\n",
        "---\nk: [a: b]\n---\n",
        "---\nk: a: b\n---\n",
        "---\nk: a\tb\n---\n",
        "---\nk: a\u{7}b\n---\n",
        "---\nk: \"a\u{1b}[31mb\"\n---\n",
        "---\nk: 'a\u{7}b'\n---\n",
        "---\nk: [a\u{7}b]\n---\n",
        "---\nk: \"a\\0b\"\n---\n",
        "---\nk: \"a\\ab\"\n---\n",
        "---\nk: \"a\\bb\"\n---\n",
        "---\nk: \"a\\nb\"\n---\n",
        "---\nk: \"a\\vb\"\n---\n",
        "---\nk: \"a\\fb\"\n---\n",
        "---\nk: \"a\\rb\"\n---\n",
        "---\nk: \"a\\eb\"\n---\n",
        "---\nk: \"a\\Nb\"\n---\n",
        "---\nk: \"a\\Lb\"\n---\n",
        "---\nk: \"a\\Pb\"\n---\n",
        "---\nk: \"\\uD800\"\n---\n",
        "---\nk: a\u{fffe}b\n---\n",
        "---\nk: a\u{2028}b\n---\n",
        "---\nk: \"\\uFFFF\"\n---\n",
        "---\nk:\n\t- a\n---\n",
        "---\nk:\n  - a\n  # trailing\n---\n",
    ];
    for contents in cases {
        get_k(&nb, contents)
            .code(1)
            .stderr(contains("is not a text or list value"));
    }
}

/// YAML trims whitespace between a key and its colon, so kladde finds
/// the property under its trimmed name and rewrites it canonically.
#[test]
fn frontmatter_finds_a_key_padded_before_its_colon() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    get_k(&nb, "---\nk : draft\n---\n")
        .success()
        .stdout("draft\n");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "k", "done", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nk: done\n---\n"
    );
}

/// Lines the edit does not target stay byte-identical, whether the
/// parser recognizes them (a quoted key) or not (a stray dash line).
#[test]
fn frontmatter_set_leaves_unrecognized_neighbors_alone() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(
        nb.path().join("x.md"),
        "---\nk: old\n- stray\n\"other\": kept\n\u{a0}nb: kept\n---\nbody\n",
    )
    .expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "k", "new", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nk: new\n- stray\n\"other\": kept\n\u{a0}nb: kept\n---\nbody\n"
    );
}

/// A block rooted in anything but property lines is preserved whole:
/// appends land in the body unstamped, and property edits refuse.
#[test]
fn frontmatter_never_edits_foreign_blocks() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    fs::write(nb.path().join("q.md"), "---\n{foo: bar}\n---\nbody\n").expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "q.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("q.md")).expect("note reads"),
        "---\n{foo: bar}\n---\nbody\n- x\n"
    );
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "q.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    kladde_state(state.path())
        .args(["frontmatter", "add", "k", "a", "q.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    fs::write(nb.path().join("s.md"), "---\n[a, b]\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "s.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    fs::write(nb.path().join("c.md"), "---\n# c\n{foo: bar}\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "c.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    fs::write(nb.path().join("seq.md"), "---\n- a\n- b\n---\nbody\n").expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "seq.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("seq.md")).expect("note reads"),
        "---\n- a\n- b\n---\nbody\n- x\n"
    );
    fs::write(nb.path().join("sc.md"), "---\njust a scalar\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "sc.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    fs::write(nb.path().join("m.md"), "---\n- item\nk: v\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "unset", "k", "m.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    kladde_state(state.path())
        .args(["frontmatter", "remove", "k", "a", "m.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
    assert_eq!(
        fs::read_to_string(nb.path().join("m.md")).expect("note reads"),
        "---\n- item\nk: v\n---\n"
    );
    fs::write(
        nb.path().join("an.md"),
        "---\n&props {foo: bar}\n---\nbody\n",
    )
    .expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "an.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("an.md")).expect("note reads"),
        "---\n&props {foo: bar}\n---\nbody\n- x\n"
    );
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "an.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("single value, not properties"));
}

/// Setting one property never overwrites a different one whose key ends
/// in wide whitespace: the no-break space is part of the name.
#[test]
fn frontmatter_set_leaves_wide_whitespace_keys_alone() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "---\na\u{a0}: kept\n---\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "a", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\na\u{a0}: kept\na: v\n---\n"
    );
    fs::write(nb.path().join("y.md"), "---\nk: \u{a0}\n- raw\n---\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "k", "new", "y.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("y.md")).expect("note reads"),
        "---\nk: new\n- raw\n---\n"
    );
}

#[test]
fn frontmatter_rejects_control_characters_in_values() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "a\u{7}b", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot hold unprintable characters"));
    kladde_state(state.path())
        .args([
            "frontmatter",
            "set",
            "k",
            "a\u{2028}b",
            "x.md",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot hold unprintable characters"));
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn frontmatter_get_rejects_duplicates() {
    let nb = temp();
    get_k(&nb, "---\nk: a\nk: b\n---\n")
        .code(1)
        .stderr(contains("multiple properties named \"k\""));
}

#[test]
fn frontmatter_rejects_invalid_keys() {
    let nb = temp();
    let cases = [
        ("", "it is empty"),
        ("a:b", "it contains a colon"),
        ("a#b", "it contains a hash"),
        ("a\nb", "it contains a line break"),
        ("a\tb", "it contains an unprintable character"),
        ("a\u{fffe}b", "it contains an unprintable character"),
        ("a\u{2028}b", "it contains an unprintable character"),
        (" a", "it has leading or trailing whitespace"),
        ("-a", "it starts with a character YAML reserves"),
        ("'a", "it starts with a character YAML reserves"),
        ("@a", "it starts with a character YAML reserves"),
        ("[a", "it starts with a character YAML reserves"),
    ];
    for (key, reason) in cases {
        kladde()
            .args(["frontmatter", "get", "--notebook"])
            .arg(nb.path())
            .args(["--", key, "x.md"])
            .assert()
            .code(1)
            .stderr(contains(reason));
    }
}

#[test]
fn frontmatter_get_reports_an_unreadable_note() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), [0xff, 0xfe, 0xfd]).expect("fixture writes");
    kladde()
        .args(["frontmatter", "get", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

#[test]
fn frontmatter_set_creates_the_note_with_parents() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "k", "v", "a/b/c.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("a")
                .join("b")
                .join("c.md")
                .display()
        ));
    let contents =
        fs::read_to_string(nb.path().join("a").join("b").join("c.md")).expect("note reads");
    assert_eq!(contents, "---\nk: v\n---\n");
}

#[test]
fn frontmatter_set_edits_only_the_named_property() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        (
            "---\na: 'kept'\n# note\nk: old\nz:\n  - kept\n---\nbody\n",
            "---\na: 'kept'\n# note\nk: v\nz:\n  - kept\n---\nbody\n",
        ),
        ("---\nk:\n  - a\n---\n", "---\nk: v\n---\n"),
        ("---\nk:\n  sub: x\n---\n", "---\nk: v\n---\n"),
        ("---\na: x\n---\n", "---\na: x\nk: v\n---\n"),
        ("---\n---\nbody\n", "---\nk: v\n---\nbody\n"),
        ("body\n", "---\nk: v\n---\nbody\n"),
        ("---\nnot closed\n", "---\nk: v\n---\n---\nnot closed\n"),
        ("---\nk: |\n  old\n  # c\n---\n", "---\nk: v\n---\n"),
        ("---\nk: old\n  # keep\n---\n", "---\nk: v\n  # keep\n---\n"),
        ("---\n  # note\nk: old\n---\n", "---\n  # note\nk: v\n---\n"),
        ("\u{feff}---\na: x\n---\n", "\u{feff}---\na: x\nk: v\n---\n"),
        ("\u{feff}body\n", "\u{feff}---\nk: v\n---\nbody\n"),
        (
            "---\r\na: x\r\n---\r\nbody\r\n",
            "---\r\na: x\r\nk: v\r\n---\r\nbody\r\n",
        ),
    ];
    for (before, after) in cases {
        fs::write(nb.path().join("x.md"), before).expect("fixture writes");
        kladde_unstamped(state.path(), xdg.path())
            .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
        assert_eq!(contents, after, "for {before:?}");
    }
}

#[test]
fn frontmatter_set_quotes_only_when_needed() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        ("[[Link]]", "k: \"[[Link]]\""),
        ("", "k: \"\""),
        ("a\tb", "k: \"a\tb\""),
        ("true", "k: true"),
    ];
    for (value, line) in cases {
        fs::write(nb.path().join("x.md"), "").expect("fixture writes");
        kladde_unstamped(state.path(), xdg.path())
            .args(["frontmatter", "set", "k", value, "x.md", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
        assert_eq!(contents, format!("---\n{line}\n---\n"));
    }
}

#[test]
fn frontmatter_set_takes_hyphen_values() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "k", "-v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "---\nk: -v\n---\n");
}

#[test]
fn frontmatter_set_rejects_a_multiline_value() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "a\nb", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("property values are single lines"));
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "a\rb", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("property values are single lines"));
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn frontmatter_set_rejects_duplicates() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), "---\nk: a\nk: b\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("multiple properties named \"k\""));
}

#[test]
fn frontmatter_unset_removes_the_property() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "---\na: x\nk: v\n---\nbody\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "unset", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "---\na: x\n---\nbody\n");
}

#[test]
fn frontmatter_unset_removes_the_fences_with_the_last_property() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), "---\nk: v\n---\nbody\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "unset", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "body\n");
}

#[test]
fn frontmatter_unset_keeps_fences_holding_other_lines() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "---\n# note\nk: v\n---\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "unset", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "---\n# note\n---\n");
}

#[test]
fn frontmatter_unset_of_a_missing_property_changes_nothing() {
    let nb = temp();
    let state = temp();
    let text = "---\na: x\n---\n";
    fs::write(nb.path().join("x.md"), text).expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "unset", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        text
    );
}

#[test]
fn frontmatter_unset_of_a_missing_note_creates_nothing() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["frontmatter", "unset", "k", "missing.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(!nb.path().join("missing.md").exists());
}

#[test]
fn frontmatter_add_creates_and_extends_the_list() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "add", "tags", "rust", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "add", "tags", "b c", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "---\ntags:\n  - rust\n  - b c\n---\n");
}

#[test]
fn frontmatter_add_rewrites_inline_and_empty_lists_in_block_style() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let cases = [
        ("---\nk: [a]\n---\n", "---\nk:\n  - a\n  - b\n---\n"),
        ("---\nk: []\n---\n", "---\nk:\n  - b\n---\n"),
    ];
    for (before, after) in cases {
        fs::write(nb.path().join("x.md"), before).expect("fixture writes");
        kladde_unstamped(state.path(), xdg.path())
            .args(["frontmatter", "add", "k", "b", "x.md", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
        assert_eq!(contents, after, "for {before:?}");
    }
}

#[test]
fn frontmatter_add_of_a_present_item_changes_nothing() {
    let nb = temp();
    let state = temp();
    let text = "---\nk: [a]\n---\n";
    fs::write(nb.path().join("x.md"), text).expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "add", "k", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, text);
}

#[test]
fn frontmatter_add_rejects_non_lists() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), "---\nk: v\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "add", "k", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("property \"k\" is not a list"));
    fs::write(nb.path().join("x.md"), "---\nk:\n  sub: x\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "add", "k", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a text or list value"));
}

#[test]
fn frontmatter_remove_removes_items_down_to_an_empty_list() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::write(nb.path().join("x.md"), "---\nk:\n  - a\n  - b\n---\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "remove", "k", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nk:\n  - b\n---\n"
    );
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "remove", "k", "b", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nk: []\n---\n"
    );
}

#[test]
fn frontmatter_remove_of_an_absent_item_changes_nothing() {
    let nb = temp();
    let state = temp();
    let text = "---\nk:\n  - a\n---\nbody\n";
    fs::write(nb.path().join("x.md"), text).expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "remove", "k", "b", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        text
    );
    kladde_state(state.path())
        .args(["frontmatter", "remove", "other", "b", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        text
    );
}

#[test]
fn frontmatter_remove_of_a_missing_note_creates_nothing() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args([
            "frontmatter",
            "remove",
            "k",
            "a",
            "missing.md",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert!(!nb.path().join("missing.md").exists());
}

#[test]
fn frontmatter_remove_rejects_a_text_property() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), "---\nk: v\n---\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "remove", "k", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("property \"k\" is not a list"));
}

#[test]
fn frontmatter_set_requires_a_notebook() {
    let state = temp();
    let xdg = temp();
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["frontmatter", "set", "k", "v", "x.md"])
        .assert()
        .code(1)
        .stderr(contains("no notebook"));
}

#[test]
fn frontmatter_set_reports_an_unopenable_notebook() {
    let state = temp();
    let base = temp();
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(base.path().join("missing"))
        .assert()
        .code(1)
        .stderr(contains("cannot open notebook"));
}

#[test]
fn frontmatter_set_rejects_an_escaping_target() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "..", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot leave the notebook"));
}

#[test]
fn frontmatter_mutations_need_a_state_dir() {
    let nb = temp();
    kladde()
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot locate the state directory"));
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn frontmatter_set_reports_an_obstructed_lock_dir() {
    let nb = temp();
    let state = temp();
    fs::create_dir(state.path().join("kladde")).expect("fixture dir creates");
    fs::write(state.path().join("kladde").join("locks"), "").expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot create lock directory"));
}

#[test]
fn frontmatter_set_reports_an_unreadable_note() {
    let nb = temp();
    let state = temp();
    fs::write(nb.path().join("x.md"), [0xff, 0xfe, 0xfd]).expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

#[test]
fn frontmatter_set_reports_an_obstructed_temp_path() {
    let nb = temp();
    let state = temp();
    fs::create_dir(nb.path().join(TEMP_X)).expect("fixture dir creates");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot write"));
}

#[test]
fn frontmatter_targets_notes_by_name_and_date() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::create_dir(nb.path().join("deep")).expect("fixture dir creates");
    fs::write(nb.path().join("deep").join("named.md"), "").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "frontmatter",
            "set",
            "k",
            "v",
            "--name",
            "named",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    kladde()
        .args(["frontmatter", "get", "k", "--name", "named", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("v\n");
    kladde_unstamped(state.path(), xdg.path())
        .args([
            "frontmatter",
            "set",
            "k",
            "v",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("2026-01-05.md")).expect("note reads"),
        "---\nk: v\n---\n"
    );
}

/// Every verb reaches a note through the daily and name routes, not just
/// a relative path.
#[test]
fn frontmatter_verbs_reach_daily_and_named_notes() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    let steps: [(&[&str], &str); 4] = [
        (&["add", "k", "a"], "---\nk:\n  - a\n---\n"),
        (&["remove", "k", "a"], "---\nk: []\n---\n"),
        (&["set", "k", "v"], "---\nk: v\n---\n"),
        (&["unset", "k"], ""),
    ];
    for (verb, expected) in steps {
        kladde_unstamped(state.path(), xdg.path())
            .arg("frontmatter")
            .args(verb)
            .args(["--date", "2026-01-05", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        assert_eq!(
            fs::read_to_string(nb.path().join("2026-01-05.md")).expect("note reads"),
            expected
        );
    }
    kladde()
        .args([
            "frontmatter",
            "get",
            "k",
            "--date",
            "2026-01-05",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("");
    fs::write(nb.path().join("named.md"), "---\nk:\n  - a\n---\n").expect("fixture writes");
    for (verb, expected) in [
        (["add", "k", "b"].as_slice(), "---\nk:\n  - a\n  - b\n---\n"),
        (&["remove", "k", "a"], "---\nk:\n  - b\n---\n"),
        (&["unset", "k"], ""),
    ] {
        kladde_unstamped(state.path(), xdg.path())
            .arg("frontmatter")
            .args(verb)
            .args(["--name", "named", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        assert_eq!(
            fs::read_to_string(nb.path().join("named.md")).expect("note reads"),
            expected
        );
    }
}

#[test]
fn frontmatter_set_by_name_never_creates() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args([
            "frontmatter",
            "set",
            "k",
            "v",
            "--name",
            "nope",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("no note named \"nope\""));
    let entries = fs::read_dir(nb.path()).expect("notebook reads");
    assert_eq!(entries.count(), 0);
}

#[test]
fn config_stamp_keys_round_trip() {
    let xdg = temp();
    let cases = [
        ("stamp", "false", "false\n"),
        ("stamp-created-key", "made", "made\n"),
        ("stamp-updated-key", "touched", "touched\n"),
        ("stamp-format", "%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M\n"),
        (
            "stamp-exclude",
            "templates, archive/2026",
            "templates,archive/2026\n",
        ),
    ];
    for (key, value, printed) in cases {
        kladde_in(xdg.path())
            .args(["config", "get", key])
            .assert()
            .code(1)
            .stdout("");
        kladde_in(xdg.path())
            .args(["config", "set", key, value])
            .assert()
            .success();
        kladde_in(xdg.path())
            .args(["config", "get", key])
            .assert()
            .success()
            .stdout(printed.to_owned());
        kladde_in(xdg.path())
            .args(["config", "unset", key])
            .assert()
            .success();
        kladde_in(xdg.path())
            .args(["config", "get", key])
            .assert()
            .code(1)
            .stdout("");
    }
}

#[test]
fn config_set_rejects_bad_stamp_values() {
    let xdg = temp();
    let cases = [
        ("stamp", "maybe", "must be true or false"),
        ("stamp-created-key", "a:b", "it contains a colon"),
        ("stamp-updated-key", " a", "leading or trailing whitespace"),
        ("stamp-format", "%Q", "is invalid"),
        ("stamp-format", "", "renders nothing"),
        ("stamp-format", "%Y\n%m", "renders an unprintable character"),
        (
            "stamp-format",
            "a\u{7}b",
            "renders an unprintable character",
        ),
        (
            "stamp-format",
            "a\u{fffe}b",
            "renders an unprintable character",
        ),
        (
            "stamp-format",
            "a\u{2028}b",
            "renders an unprintable character",
        ),
        ("stamp-exclude", "", "must not be empty"),
        ("stamp-exclude", "/abs", "must be relative paths"),
        (
            "stamp-exclude",
            "..",
            "must name a place inside the notebook",
        ),
        (
            "stamp-exclude",
            "a/../b",
            "must name a place inside the notebook",
        ),
        (
            "stamp-exclude",
            ".",
            "must name a place inside the notebook",
        ),
    ];
    for (key, value, fragment) in cases {
        kladde_in(xdg.path())
            .args(["config", "set", key, value])
            .assert()
            .code(1)
            .stderr(contains(fragment));
    }
    assert!(!config_file(xdg.path()).exists());
}

#[test]
fn config_load_rejects_bad_stamp_values() {
    let xdg = temp();
    let cases = [
        ("stamp = 'yes'\n", "`stamp` must be true or false"),
        ("stamp-created-key = 1\n", "must be a string"),
        ("stamp-updated-key = 1\n", "must be a string"),
        ("stamp-created-key = 'a:b'\n", "invalid property key"),
        ("stamp-updated-key = 'a#b'\n", "invalid property key"),
        ("stamp-format = 1\n", "must be a string"),
        ("stamp-format = '%Q'\n", "timestamp format"),
        ("stamp-exclude = 'x'\n", "must be an array of strings"),
        ("stamp-exclude = [1]\n", "must be an array of strings"),
        ("stamp-exclude = ['/abs']\n", "must be relative paths"),
        ("stamp-exclude = ['']\n", "must not be empty"),
        (
            "stamp-exclude = ['..']\n",
            "must name a place inside the notebook",
        ),
    ];
    for (contents, fragment) in cases {
        write_config(xdg.path(), contents);
        kladde_in(xdg.path())
            .args(["config", "get", "stamp"])
            .assert()
            .code(1)
            .stderr(contains(fragment));
    }
}

#[test]
fn config_bullet_indent_round_trips() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "get", "bullet-indent"])
        .assert()
        .code(1)
        .stdout("");
    for value in ["tab", "spaces"] {
        kladde_in(xdg.path())
            .args(["config", "set", "bullet-indent", value])
            .assert()
            .success();
        kladde_in(xdg.path())
            .args(["config", "get", "bullet-indent"])
            .assert()
            .success()
            .stdout(format!("{value}\n"));
    }
    kladde_in(xdg.path())
        .args(["config", "unset", "bullet-indent"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .args(["config", "get", "bullet-indent"])
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn config_rejects_a_bad_bullet_indent() {
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "bullet-indent", "wide"])
        .assert()
        .code(1)
        .stderr(contains("`bullet-indent` must be \"tab\" or \"spaces\""));
    assert!(!config_file(xdg.path()).exists());
    let cases = [
        ("bullet-indent = 1\n", "`bullet-indent` must be a string"),
        (
            "bullet-indent = 'wide'\n",
            "`bullet-indent` must be \"tab\" or \"spaces\"",
        ),
    ];
    for (contents, fragment) in cases {
        write_config(xdg.path(), contents);
        kladde_in(xdg.path())
            .args(["config", "get", "bullet-indent"])
            .assert()
            .code(1)
            .stderr(contains(fragment));
    }
}

/// Stamping is on with no config at all: a write that creates a note
/// gives it a block holding both stamps in the default datetime shape.
#[test]
fn stamping_is_on_by_default() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert!(
        contents.starts_with("---\nk: v\ncreated: 2"),
        "{contents:?}"
    );
    assert!(contents.contains("\nupdated: 2"), "{contents:?}");
    assert!(contents.ends_with("---\n"), "{contents:?}");
}

/// A stamp format without directives renders the same value every run,
/// making stamped bytes exact.
#[test]
fn stamping_uses_the_configured_keys_and_format() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(
        xdg.path(),
        "stamp-created-key = 'made'\nstamp-updated-key = 'touched'\nstamp-format = 'X'\n",
    );
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
    assert_eq!(contents, "---\nk: v\nmade: X\ntouched: X\n---\n");
}

#[test]
fn append_stamps_a_new_note_and_refreshes_updated() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'A'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ncreated: A\nupdated: A\n---\n- x\n"
    );
    write_config(xdg.path(), "stamp-format = 'B'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- y", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ncreated: A\nupdated: B\n---\n- x\n- y\n"
    );
}

/// A block that predates kladde gains only the updated stamp: created
/// records block creation, and kladde did not create this one.
#[test]
fn append_adds_updated_to_an_existing_block() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    fs::write(nb.path().join("x.md"), "---\nk: v\n---\nbody\n").expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nk: v\nupdated: X\n---\nbody\n- x\n"
    );
}

/// An exclude entry naming a linked folder excludes the notes that land
/// in its target: notes are identified by their canonical path, so the
/// entry is resolved the same way.
#[test]
fn stamping_excludes_through_notebook_links() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::create_dir(nb.path().join("actual")).expect("fixture dir creates");
    link_dir(&nb.path().join("templates"), &nb.path().join("actual"));
    write_config(xdg.path(), "stamp-exclude = ['templates']\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "templates/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("actual").join("x.md")).expect("note reads"),
        "- x\n"
    );
}

/// A `./` spelling names the same place after normalization, so it
/// excludes what it says instead of silently matching nothing.
#[test]
fn stamping_normalizes_curdir_exclude_entries() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    kladde_in(xdg.path())
        .args(["config", "set", "stamp-exclude", "./inbox"])
        .assert()
        .success();
    kladde_in(xdg.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["append", "- x", "inbox/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("inbox").join("x.md")).expect("note reads"),
        "- x\n"
    );
}

#[test]
fn stamping_skips_excluded_paths() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(
        xdg.path(),
        "stamp-exclude = ['inbox', 'archive/2026', 'ghost']\n",
    );
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "inbox/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("inbox").join("x.md")).expect("note reads"),
        "- x\n"
    );
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args([
            "frontmatter",
            "set",
            "k",
            "v",
            "archive/2026/y.md",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("archive").join("2026").join("y.md"))
            .expect("note reads"),
        "---\nk: v\n---\n"
    );
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "elsewhere.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let stamped = fs::read_to_string(nb.path().join("elsewhere.md")).expect("note reads");
    assert!(stamped.starts_with("---\n"), "{stamped:?}");
}

/// A note whose stamp key is unusable still takes the append; the stamp
/// is skipped whole rather than mangling the block.
#[test]
fn append_onto_a_misbehaving_stamp_key_succeeds_unstamped() {
    let nb = temp();
    let state = temp();
    let cases = [
        "---\nupdated: a\nupdated: b\n---\n",
        "---\nupdated:\n  - a\n---\n",
    ];
    for block in cases {
        fs::write(nb.path().join("x.md"), block).expect("fixture writes");
        kladde_state(state.path())
            .args(["append", "- x", "x.md", "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        let contents = fs::read_to_string(nb.path().join("x.md")).expect("note reads");
        assert_eq!(contents, format!("{block}- x\n"), "for {block:?}");
    }
}

/// Adding and removing list items are writes like any other: both
/// stamp, which the format switch makes visible on the removal too.
#[test]
fn stamping_covers_list_edits() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'A'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["frontmatter", "add", "tags", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ntags:\n  - a\ncreated: A\nupdated: A\n---\n"
    );
    write_config(xdg.path(), "stamp-format = 'B'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["frontmatter", "remove", "tags", "a", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ntags: []\ncreated: A\nupdated: B\n---\n"
    );
}

/// Setting a property to the value it already holds writes nothing, so
/// even the updated stamp stays put.
#[test]
fn frontmatter_set_of_the_same_value_bumps_nothing() {
    let nb = temp();
    let state = temp();
    let text = "---\nk: v\ncreated: OLD\nupdated: OLD\n---\n";
    fs::write(nb.path().join("x.md"), text).expect("fixture writes");
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        text
    );
}

/// Dropping the fences must never promote body text into frontmatter:
/// an empty block stays behind, and the stamp lands in it, not in the
/// planted chunk.
#[test]
fn frontmatter_unset_never_promotes_the_body() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    fs::write(
        nb.path().join("x.md"),
        "---\nk: v\n---\n---\nevil: y\n---\nbody\n",
    )
    .expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["frontmatter", "unset", "k", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\nupdated: X\n---\n---\nevil: y\n---\nbody\n"
    );
}

/// A quoted spelling of a property is that property: an edit or a stamp
/// replaces it in place instead of writing a duplicate key beside it.
#[test]
fn frontmatter_replaces_quoted_key_spellings() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    fs::write(
        nb.path().join("q.md"),
        "---\n\"updated\": old\nk: v\n---\nbody\n",
    )
    .expect("fixture writes");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "q.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("q.md")).expect("note reads"),
        "---\nupdated: X\nk: v\n---\nbody\n- x\n"
    );
    fs::write(nb.path().join("s.md"), "---\n\"status\": old\n---\n").expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "status", "new", "s.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("s.md")).expect("note reads"),
        "---\nstatus: new\n---\n"
    );
    fs::write(
        nb.path().join("d.md"),
        "---\n\"status\": a\nstatus: b\n---\n",
    )
    .expect("fixture writes");
    kladde()
        .args(["frontmatter", "get", "status", "d.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("multiple properties named \"status\""));
    fs::write(nb.path().join("e.md"), "---\n\"up\\u0064ated\": old\n---\n")
        .expect("fixture writes");
    kladde_unstamped(state.path(), xdg.path())
        .args(["frontmatter", "set", "updated", "new", "e.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("e.md")).expect("note reads"),
        "---\nupdated: new\n---\n"
    );
    fs::write(nb.path().join("f.md"), "---\nupdated:\told\n---\nbody\n").expect("fixture writes");
    write_config(xdg.path(), "stamp-format = 'X'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "f.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("f.md")).expect("note reads"),
        "---\nupdated: X\n---\nbody\n- x\n"
    );
}

/// An exclusion reaching through a link into a folder that does not
/// exist yet still excludes the first write: the entry's deepest
/// existing ancestor resolves like a note path, and the remainder rides
/// along.
#[test]
fn stamping_excludes_linked_entries_with_missing_suffixes() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    fs::create_dir(nb.path().join("actual")).expect("fixture dir creates");
    link_dir(&nb.path().join("alias"), &nb.path().join("actual"));
    write_config(xdg.path(), "stamp-exclude = ['alias/new']\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "alias/new/note.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("actual").join("new").join("note.md"))
            .expect("note reads"),
        "- x\n"
    );
}

/// An exclusion matches case-folded spellings even before its folder
/// exists, so the first write is excluded like every later one.
#[test]
fn stamping_excludes_case_aliased_paths_before_they_exist() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-exclude = ['Templates']\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args(["append", "- x", "templates/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("templates").join("x.md")).expect("note reads"),
        "- x\n"
    );
}

/// A misbehaving created key skips the whole stamp: the edit lands,
/// nothing else changes.
#[test]
fn stamping_skips_wholly_when_the_created_key_misbehaves() {
    let nb = temp();
    let state = temp();
    kladde_state(state.path())
        .args([
            "frontmatter",
            "add",
            "created",
            "mine",
            "x.md",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ncreated:\n  - mine\n---\n"
    );
}

/// A created value the same edit writes by hand survives the stamp.
#[test]
fn stamping_preserves_a_manual_created_value() {
    let nb = temp();
    let state = temp();
    let xdg = temp();
    write_config(xdg.path(), "stamp-format = 'X'\n");
    kladde_state(state.path())
        .env("XDG_CONFIG_HOME", xdg.path())
        .args([
            "frontmatter",
            "set",
            "created",
            "mine",
            "x.md",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("x.md")).expect("note reads"),
        "---\ncreated: mine\nupdated: X\n---\n"
    );
}

/// Writes read the config for their stamp keys, so a broken config fails
/// them even when `--notebook` pins the notebook.
#[test]
fn writes_report_a_broken_config_despite_a_path_target() {
    let xdg = temp();
    let nb = temp();
    let state = temp();
    write_config(xdg.path(), "not toml [\n");
    kladde_in(xdg.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
    kladde_in(xdg.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("invalid TOML"));
    assert!(!nb.path().join("x.md").exists());
}

/// Windows spells the usage line `kladde.exe`, so the assertion anchors
/// on the subcommand instead of the binary name.
#[test]
fn frontmatter_requires_a_subcommand() {
    kladde()
        .arg("frontmatter")
        .assert()
        .code(2)
        .stderr(contains("frontmatter <COMMAND>"));
}

#[test]
fn frontmatter_rejects_conflicting_targets() {
    let nb = temp();
    kladde()
        .args([
            "frontmatter",
            "get",
            "k",
            "x.md",
            "--name",
            "y",
            "--notebook",
        ])
        .arg(nb.path())
        .assert()
        .code(2)
        .stderr(contains("cannot be used with"));
}

/// The stress test doubles as the create-if-missing race test: all
/// writers start behind a barrier and the first round races to create the
/// note.
#[test]
fn append_concurrent_writers_lose_nothing() {
    const WRITERS: usize = 8;
    const ENTRIES: usize = 5;
    let nb = temp();
    let state = temp();
    let barrier = std::sync::Barrier::new(WRITERS);
    std::thread::scope(|scope| {
        for writer in 0..WRITERS {
            let (barrier, nb, state) = (&barrier, nb.path(), state.path());
            scope.spawn(move || {
                barrier.wait();
                for entry in 0..ENTRIES {
                    kladde_state(state)
                        .args([
                            "append",
                            &format!("- w{writer}e{entry}"),
                            "--date",
                            "2026-01-05",
                            "--notebook",
                        ])
                        .arg(nb)
                        .assert()
                        .success();
                }
            });
        }
    });
    let contents = fs::read_to_string(nb.path().join("2026-01-05.md")).expect("note reads");
    let body: Vec<&str> = contents
        .lines()
        .filter(|line| line.starts_with("- w"))
        .collect();
    assert_eq!(body.len(), WRITERS * ENTRIES);
    let created = contents
        .lines()
        .filter(|line| line.starts_with("created: "))
        .count();
    assert_eq!(created, 1);
    let updated = contents
        .lines()
        .filter(|line| line.starts_with("updated: "))
        .count();
    assert_eq!(updated, 1);
    let lines: std::collections::HashSet<&str> = body.iter().copied().collect();
    for writer in 0..WRITERS {
        for entry in 0..ENTRIES {
            assert!(lines.contains(format!("- w{writer}e{entry}").as_str()));
        }
    }
    assert!(contents.ends_with('\n'));
    assert!(!contents.contains("\n\n"));
}

#[test]
fn daily_uses_the_notebook_config_folder_and_format() {
    let nb = temp();
    write_notebook_config(&nb, "daily-folder = 'Journal'\ndaily-date-format = '%Y'\n");
    kladde()
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("Journal")
                .join("2026.md")
                .display()
        ));
}

/// A key set in both files takes the notebook's value; a key only the
/// base sets still applies.
#[test]
fn notebook_config_wins_over_the_base_key_by_key() {
    let (nb, xdg) = (temp(), temp());
    write_config(
        xdg.path(),
        "daily-folder = 'Base'\ndaily-date-format = '%Y'\n",
    );
    write_notebook_config(&nb, "daily-folder = 'Nb'\n");
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join("Nb").join("2026.md").display()
        ));
}

#[test]
fn notebook_config_layers_under_the_default_notebook() {
    let (nb, xdg) = (temp(), temp());
    write_config(
        xdg.path(),
        &format!("default-notebook = '{}'\n", nb.path().display()),
    );
    write_notebook_config(&nb, "daily-folder = 'Journal'\n");
    kladde_in(xdg.path())
        .args(["path", "--date", "2026-01-05"])
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path())
                .join("Journal")
                .join("2026-01-05.md")
                .display()
        ));
}

#[test]
fn daily_seeds_from_the_notebook_config_template() {
    let (nb, state) = (temp(), temp());
    write_template(&nb, "# {{title}}\n\n## Log\n");
    write_notebook_config(
        &nb,
        "stamp = false\ndaily-template = 'templates/Daily.md'\n",
    );
    kladde_state(state.path())
        .args(["append", "- entry", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(dated_contents(&nb), "# 2026-01-05\n\n## Log\n- entry\n");
}

#[test]
fn stamping_uses_the_notebook_config_keys_and_format() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(
        &nb,
        "stamp-created-key = 'made'\nstamp-updated-key = 'touched'\nstamp-format = 'X'\n",
    );
    kladde_state(state.path())
        .args(["frontmatter", "set", "k", "v", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "---\nk: v\nmade: X\ntouched: X\n---\n");
}

#[test]
fn stamping_skips_paths_the_notebook_config_excludes() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "stamp-exclude = ['skip']\n");
    kladde_state(state.path())
        .args(["append", "- x", "skip/x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(nb.path().join("skip").join("x.md")).expect("note reads"),
        "- x\n"
    );
}

#[test]
fn append_under_bullet_honors_the_notebook_config_indent() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "stamp = false\nbullet-indent = 'spaces'\n");
    fs::write(nb.path().join("x.md"), "1. a\n").expect("fixture writes");
    kladde_state(state.path())
        .args(["append", "- e", "x.md", "--under-bullet", "a", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(x_contents(&nb), "1. a\n   - e\n");
}

/// Every command that consumes notebook-scoped keys fails on a broken
/// notebook config, explicit target or daily, and creates nothing.
#[test]
fn writes_report_a_broken_notebook_config() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "not toml [\n");
    for argv in [
        &["append", "- x", "x.md"][..],
        &["append", "- x", "--date", "2026-01-05"],
        &["new", "x.md"],
        &["frontmatter", "set", "k", "v", "x.md"],
        &["open", "x.md"],
        &["path", "--date", "2026-01-05"],
    ] {
        kladde_state(state.path())
            .args(argv)
            .arg("--notebook")
            .arg(nb.path())
            .assert()
            .code(1)
            .stderr(contains("invalid TOML"))
            .stderr(contains(".kladde.toml"));
    }
    assert!(!nb.path().join("x.md").exists());
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// Commands that consume no notebook-scoped keys never read the notebook
/// config: a broken one does not fail them, and the file never appears
/// in listings or search.
#[test]
fn reads_ignore_a_broken_notebook_config() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "hi\n").expect("fixture writes");
    write_notebook_config(&nb, "not toml [\n");
    kladde()
        .args(["path", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    kladde()
        .args(["read", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("hi\n");
    kladde()
        .args(["list", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\n");
    kladde()
        .args(["search", "hi", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("x.md\n");
    kladde()
        .args(["search", "toml", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stdout("");
}

#[test]
fn writes_reject_a_machine_key_in_the_notebook_config() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "editor = 'vim'\n");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("sets `editor`, which is machine-scoped"))
        .stderr(contains(".kladde.toml"));
    assert!(!nb.path().join("x.md").exists());
}

#[test]
fn writes_report_an_unknown_notebook_config_key() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "unknown = 1\n");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("unknown key `unknown`"))
        .stderr(contains(".kladde.toml"));
}

#[test]
fn writes_report_an_unreadable_notebook_config() {
    let (nb, state) = (temp(), temp());
    fs::write(nb.path().join(".kladde.toml"), [0xFF, 0xFE, 0x00]).expect("obstacle writes");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot read"));
}

/// A directory or any other irregular entry at the config name is
/// rejected outright, for the config surface and the note writes alike.
#[test]
fn commands_reject_an_irregular_notebook_config() {
    let (nb, state) = (temp(), temp());
    fs::create_dir(nb.path().join(".kladde.toml")).expect("obstacle creates");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a regular file"));
    kladde()
        .args(["config", "set", "daily-folder", "Journal", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a regular file"));
}

/// A pipe at the config name fails fast instead of blocking the command
/// forever on a read no writer will ever feed.
#[cfg(unix)]
#[test]
fn commands_reject_a_fifo_notebook_config() {
    let (nb, state) = (temp(), temp());
    let status = std::process::Command::new("mkfifo")
        .arg(nb.path().join(".kladde.toml"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
    kladde_state(state.path())
        .args(["path", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a regular file"));
}

/// Explicit-note commands never touch the notebook config, so a pipe at
/// its name must not stall them either.
#[cfg(unix)]
#[test]
fn explicit_reads_ignore_a_fifo_notebook_config() {
    let nb = temp();
    fs::write(nb.path().join("x.md"), "hi\n").expect("fixture writes");
    let status = std::process::Command::new("mkfifo")
        .arg(nb.path().join(".kladde.toml"))
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
    kladde()
        .args(["path", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    kladde()
        .args(["read", "--name", "x", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("hi\n");
}

/// A bad value in the notebook config names the file, because by then
/// there are two files it could live in.
#[test]
fn writes_report_a_bad_notebook_config_value() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "daily-folder = ''\n");
    kladde_state(state.path())
        .args(["append", "- x", "x.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("in notebook config"))
        .stderr(contains("`daily-folder` must not be empty"));
    assert!(!nb.path().join("x.md").exists());
}

/// Every notebook-scoped key round-trips through the notebook config,
/// without touching the base config or needing a config directory.
#[test]
fn config_notebook_keys_round_trip() {
    let (nb, xdg) = (temp(), temp());
    let cases = [
        ("daily-folder", "Journal", "Journal\n"),
        ("daily-date-format", "%Y", "%Y\n"),
        (
            "daily-template",
            "templates/Daily.md",
            "templates/Daily.md\n",
        ),
        ("stamp", "false", "false\n"),
        ("stamp-created-key", "made", "made\n"),
        ("stamp-updated-key", "touched", "touched\n"),
        ("stamp-format", "%Y-%m-%dT%H:%M", "%Y-%m-%dT%H:%M\n"),
        (
            "stamp-exclude",
            "templates, archive/2026",
            "templates,archive/2026\n",
        ),
        ("bullet-indent", "spaces", "spaces\n"),
    ];
    for (key, value, printed) in cases {
        kladde()
            .args(["config", "get", key, "--notebook"])
            .arg(nb.path())
            .assert()
            .code(1)
            .stdout("");
        kladde()
            .args(["config", "set", key, value, "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        kladde()
            .args(["config", "get", key, "--notebook"])
            .arg(nb.path())
            .assert()
            .success()
            .stdout(printed.to_owned());
        kladde_in(xdg.path())
            .args(["config", "get", key])
            .assert()
            .code(1)
            .stdout("");
        kladde()
            .args(["config", "unset", key, "--notebook"])
            .arg(nb.path())
            .assert()
            .success();
        kladde()
            .args(["config", "get", key, "--notebook"])
            .arg(nb.path())
            .assert()
            .code(1)
            .stdout("");
    }
}

#[test]
fn config_set_rejects_machine_keys_in_a_notebook() {
    let nb = temp();
    for (key, value) in [("editor", "vim"), ("default-notebook", "/notes")] {
        kladde()
            .args(["config", "set", key, value, "--notebook"])
            .arg(nb.path())
            .assert()
            .code(1)
            .stderr(contains(format!(
                "`{key}` is machine-scoped: a notebook config cannot hold it"
            )));
    }
    assert!(!nb.path().join(".kladde.toml").exists());
}

#[test]
fn config_get_rejects_machine_keys_in_a_notebook() {
    let nb = temp();
    kladde()
        .args(["config", "get", "editor", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("`editor` is machine-scoped"));
}

/// `unset` skips the machine-scope guard: it is the repair tool for a
/// hand-edited notebook config that every load rejects.
#[test]
fn config_unset_repairs_a_machine_key_in_a_notebook() {
    let (nb, state) = (temp(), temp());
    write_notebook_config(&nb, "editor = 'vim'\ndaily-folder = 'Journal'\n");
    kladde_state(state.path())
        .args(["append", "- x", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("machine-scoped"));
    kladde()
        .args(["config", "unset", "editor", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    kladde_state(state.path())
        .args(["append", "- x", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("Journal").join("2026-01-05.md").exists());
}

#[test]
fn config_set_validates_through_a_notebook() {
    let nb = temp();
    kladde()
        .args(["config", "set", "daily-folder", "/abs", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("must be a relative path"));
    assert!(!nb.path().join(".kladde.toml").exists());
}

#[test]
fn config_path_prints_the_notebook_config_path() {
    let nb = temp();
    kladde()
        .args(["config", "path", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout(format!(
            "{}\n",
            canonical(nb.path()).join(".kladde.toml").display()
        ));
}

#[test]
fn config_path_reports_a_missing_notebook() {
    let nb = temp();
    kladde()
        .args(["config", "path", "--notebook"])
        .arg(nb.path().join("gone"))
        .assert()
        .code(1)
        .stderr(contains("cannot open notebook"));
}

#[test]
fn config_open_opens_the_notebook_config() {
    let (nb, xdg) = (temp(), temp());
    write_config(xdg.path(), &format!("editor = '{}'\n", creating_editor()));
    kladde_in(xdg.path())
        .args(["config", "open", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join(".kladde.toml").exists());
}

#[test]
fn config_open_notebook_falls_back_to_visual_without_a_config_base() {
    let nb = temp();
    kladde()
        .env("VISUAL", creating_editor())
        .args(["config", "open", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join(".kladde.toml").exists());
}

#[test]
fn config_set_preserves_notebook_config_comments() {
    let nb = temp();
    write_notebook_config(&nb, "# mine\ndaily-folder = 'a'\n");
    kladde()
        .args(["config", "set", "daily-date-format", "%Y", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    let contents = fs::read_to_string(nb.path().join(".kladde.toml")).expect("config reads");
    assert!(contents.contains("# mine"), "{contents}");
    assert!(contents.contains("daily-folder = 'a'"), "{contents}");
}

/// The notebook config's name is reserved: a note command cannot write
/// markdown into it, in any spelling that would alias it.
#[test]
fn note_targets_cannot_name_the_notebook_config() {
    let (nb, state) = (temp(), temp());
    for target in [".kladde.toml", ".KLADDE.TOML", ".kladde.toml/x.md"] {
        kladde_state(state.path())
            .args(["new", target, "--notebook"])
            .arg(nb.path())
            .assert()
            .code(1)
            .stderr(contains("names the notebook config, not a note"));
    }
    kladde()
        .args(["read", ".kladde.toml", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("names the notebook config, not a note"));
    assert!(!nb.path().join(".kladde.toml").exists());
    assert!(!nb.path().join(".KLADDE.TOML").exists());
}

/// Only the root-level name is reserved; deeper down it is an ordinary
/// dot file.
#[test]
fn a_nested_kladde_toml_is_an_ordinary_target() {
    let (nb, state) = (temp(), temp());
    kladde_state(state.path())
        .args(["append", "- x", "sub/.kladde.toml", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(nb.path().join("sub").join(".kladde.toml").exists());
}

/// A linked notebook config is refused before anything reads or writes
/// through it, so a planted link cannot reach outside the notebook.
#[test]
fn config_commands_reject_a_linked_notebook_config() {
    let (nb, target) = (temp(), temp());
    link_dir(&nb.path().join(".kladde.toml"), target.path());
    for argv in [
        &["config", "set", "daily-folder", "Journal"][..],
        &["config", "unset", "daily-folder"],
        &["config", "get", "daily-folder"],
        &["config", "open"],
        &["config", "path"],
    ] {
        kladde()
            .args(argv)
            .arg("--notebook")
            .arg(nb.path())
            .assert()
            .code(1)
            .stderr(contains("is not a regular file"));
    }
}

#[test]
fn writes_reject_a_linked_notebook_config() {
    let (nb, target, state) = (temp(), temp(), temp());
    link_dir(&nb.path().join(".kladde.toml"), target.path());
    kladde_state(state.path())
        .args(["append", "- x", "--date", "2026-01-05", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a regular file"));
    assert!(!nb.path().join("2026-01-05.md").exists());
}

/// The attack the link rule exists for: a planted link aimed at an
/// outside TOML file must not let a config write modify that file.
#[cfg(unix)]
#[test]
fn config_set_refuses_to_write_through_a_planted_link() {
    let (nb, outside) = (temp(), temp());
    let victim = outside.path().join("victim.toml");
    fs::write(&victim, "editor = 'vim'\n").expect("fixture writes");
    std::os::unix::fs::symlink(&victim, nb.path().join(".kladde.toml")).expect("symlink creates");
    kladde()
        .args(["config", "set", "daily-folder", "Journal", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("is not a regular file"));
    assert_eq!(
        fs::read_to_string(&victim).expect("victim reads"),
        "editor = 'vim'\n"
    );
}

/// A hard link is indistinguishable from a regular file, but the rename
/// write path replaces the notebook's directory entry instead of writing
/// through it, so the outside inode never changes.
#[cfg(unix)]
#[test]
fn config_set_never_writes_through_a_hard_link() {
    let (nb, outside) = (temp(), temp());
    let victim = outside.path().join("victim.toml");
    fs::write(&victim, "stamp = false\n").expect("fixture writes");
    fs::hard_link(&victim, nb.path().join(".kladde.toml")).expect("hard link creates");
    kladde()
        .args(["config", "set", "daily-folder", "Journal", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(&victim).expect("victim reads"),
        "stamp = false\n"
    );
    kladde()
        .args(["config", "get", "daily-folder", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("Journal\n");
}

/// A config write touches only its own temp entry: a bystander file
/// with a temp-suffixed name survives untouched, symlink target
/// included.
#[cfg(unix)]
#[test]
fn config_set_leaves_a_bystander_temp_entry_alone() {
    let (nb, outside) = (temp(), temp());
    let victim = outside.path().join("victim.toml");
    fs::write(&victim, "untouched\n").expect("fixture writes");
    let bystander = nb.path().join(".kladde.toml.kladde-tmp");
    std::os::unix::fs::symlink(&victim, &bystander).expect("symlink creates");
    kladde()
        .args(["config", "set", "daily-folder", "Journal", "--notebook"])
        .arg(nb.path())
        .assert()
        .success();
    assert!(
        fs::symlink_metadata(&bystander)
            .expect("metadata reads")
            .is_symlink()
    );
    assert_eq!(
        fs::read_to_string(&victim).expect("victim reads"),
        "untouched\n"
    );
    kladde()
        .args(["config", "get", "daily-folder", "--notebook"])
        .arg(nb.path())
        .assert()
        .success()
        .stdout("Journal\n");
}

/// Temp-suffixed names are kladde's own namespace, so a note command
/// cannot create the file a config write would treat as its workspace.
#[test]
fn note_targets_cannot_use_the_temp_suffix() {
    let (nb, state) = (temp(), temp());
    for target in [".kladde.toml.kladde-tmp", "notes/x.KLADDE-TMP"] {
        kladde_state(state.path())
            .args(["new", target, "--notebook"])
            .arg(nb.path())
            .assert()
            .code(1)
            .stderr(contains("names a kladde temporary file, not a note"));
    }
    kladde_state(state.path())
        .args(["append", "- x", ".kladde.toml.kladde-tmp", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("names a kladde temporary file, not a note"));
    assert!(!nb.path().join(".kladde.toml.kladde-tmp").exists());
    assert!(!nb.path().join("notes").exists());
}

/// A symlinked base config keeps its link; the managed target receives
/// the write.
#[cfg(unix)]
#[test]
fn config_set_follows_a_symlinked_base_config() {
    let (xdg, managed) = (temp(), temp());
    let target = managed.path().join("managed.toml");
    fs::write(&target, "editor = 'vim'\n").expect("fixture writes");
    fs::create_dir_all(xdg.path().join("kladde")).expect("config dir creates");
    std::os::unix::fs::symlink(&target, config_file(xdg.path())).expect("symlink creates");
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "Journal"])
        .assert()
        .success();
    assert!(
        fs::symlink_metadata(config_file(xdg.path()))
            .expect("metadata reads")
            .is_symlink()
    );
    let contents = fs::read_to_string(&target).expect("target reads");
    assert!(contents.contains("editor = 'vim'"), "{contents}");
    assert!(contents.contains("daily-folder"), "{contents}");
}

/// A link installed ahead of the file it manages keeps its link; the
/// write creates the managed target.
#[cfg(unix)]
#[test]
fn config_set_creates_the_target_of_a_dangling_base_link() {
    let (xdg, managed) = (temp(), temp());
    let target = managed.path().join("machine.toml");
    fs::create_dir_all(xdg.path().join("kladde")).expect("config dir creates");
    std::os::unix::fs::symlink(&target, config_file(xdg.path())).expect("symlink creates");
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "Journal"])
        .assert()
        .success();
    assert!(
        fs::symlink_metadata(config_file(xdg.path()))
            .expect("metadata reads")
            .is_symlink()
    );
    assert!(target.exists());
    kladde_in(xdg.path())
        .args(["config", "get", "daily-folder"])
        .assert()
        .success()
        .stdout("Journal\n");
}

/// A dangling chain deeper than any real layout is refused: the rename
/// would replace the link the walk stopped at.
#[cfg(unix)]
#[test]
fn config_set_reports_a_link_chain_too_deep() {
    let xdg = temp();
    let config_dir = xdg.path().join("kladde");
    fs::create_dir_all(&config_dir).expect("config dir creates");
    let mut prev = config_dir.join("gone.toml");
    for index in 0..8 {
        let link = config_dir.join(format!("l{index}.toml"));
        std::os::unix::fs::symlink(&prev, &link).expect("symlink creates");
        prev = link;
    }
    std::os::unix::fs::symlink(&prev, config_file(xdg.path())).expect("symlink creates");
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "Journal"])
        .assert()
        .code(1)
        .stderr(contains("too many levels of links"));
}

/// The Windows twin of the too-deep refusal, over file symlinks.
#[cfg(windows)]
#[test]
fn config_set_reports_a_link_chain_too_deep() {
    let xdg = temp();
    let config_dir = xdg.path().join("kladde");
    fs::create_dir_all(&config_dir).expect("config dir creates");
    let mut prev = config_dir.join("gone.toml");
    for index in 0..8 {
        let link = config_dir.join(format!("l{index}.toml"));
        std::os::windows::fs::symlink_file(&prev, &link).expect("symlink creates");
        prev = link;
    }
    std::os::windows::fs::symlink_file(&prev, config_file(xdg.path())).expect("symlink creates");
    kladde_in(xdg.path())
        .args(["config", "set", "daily-folder", "Journal"])
        .assert()
        .code(1)
        .stderr(contains("too many levels of links"));
}

/// An unverifiable config entry is an error, never treated as absent.
#[cfg(unix)]
#[test]
fn config_path_reports_an_unverifiable_notebook_config() {
    use std::os::unix::fs::PermissionsExt;
    let nb = temp();
    fs::set_permissions(nb.path(), fs::Permissions::from_mode(0o000)).expect("permissions apply");
    kladde()
        .args(["config", "path", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("cannot resolve"));
    fs::set_permissions(nb.path(), fs::Permissions::from_mode(0o755)).expect("permissions restore");
}

/// A hard link to the config carries no reserved spelling, so identity
/// settles it: neither a path target nor name lookup reaches the config
/// through one.
#[test]
fn note_targets_cannot_reach_the_config_through_a_hard_link() {
    let nb = temp();
    fs::write(nb.path().join(".kladde.toml"), "stamp = false\n").expect("fixture writes");
    fs::hard_link(nb.path().join(".kladde.toml"), nb.path().join("alias.md"))
        .expect("hard link creates");
    kladde()
        .args(["read", "alias.md", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("names the notebook config, not a note"));
    kladde()
        .args(["read", "--name", "alias", "--notebook"])
        .arg(nb.path())
        .assert()
        .code(1)
        .stderr(contains("names the notebook config, not a note"));
}

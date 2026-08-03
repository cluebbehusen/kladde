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
/// the failing editor exits unsuccessfully.
#[cfg(not(windows))]
fn creating_editor() -> String {
    "/usr/bin/touch".to_owned()
}

#[cfg(windows)]
fn creating_editor() -> String {
    format!("{}\\System32\\cmd.exe /C copy /Y NUL", systemroot())
}

#[cfg(not(windows))]
fn success_editor() -> String {
    "/usr/bin/true".to_owned()
}

#[cfg(windows)]
fn success_editor() -> String {
    format!("{}\\System32\\cmd.exe /C rem", systemroot())
}

/// `false` ignores its argument; `type` fails on the missing config file the
/// test leaves absent.
#[cfg(not(windows))]
fn failing_editor() -> String {
    "/usr/bin/false".to_owned()
}

#[cfg(windows)]
fn failing_editor() -> String {
    format!("{}\\System32\\cmd.exe /C type", systemroot())
}

#[cfg(windows)]
fn systemroot() -> String {
    std::env::var("SYSTEMROOT").expect("SYSTEMROOT is set on Windows")
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

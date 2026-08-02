use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;

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

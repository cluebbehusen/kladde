//! Locating kladde's configuration.

use std::path::PathBuf;

/// Directory holding kladde's configuration, following the XDG base directory
/// convention on every platform: `$XDG_CONFIG_HOME/kladde`, falling back to
/// `$HOME/.config/kladde`. As the XDG specification requires, a relative path
/// (which includes an empty one) counts as unset.
///
/// Returns `None` when neither variable provides an absolute base.
#[must_use]
pub fn dir(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|path| path.is_absolute())
                .map(|home| home.join(".config"))
        })?;
    Some(base.join("kladde"))
}

/// Path of the config file inside [`dir`].
#[must_use]
pub fn file(xdg_config_home: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    Some(dir(xdg_config_home, home)?.join("config.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

// Verifies: ADR-0029
//! The view state the terminal frontend remembers across sessions (ADR-0029):
//! theme, minimised sidebar and diff layout — the desktop's `localStorage`
//! choices that exist in the terminal. One JSON file in the platform's state
//! directory; a missing or broken file means defaults, never a failed start.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const FILE: &str = "cli-state.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StoredTheme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffView {
    #[default]
    Unified,
    Split,
}

/// Every field is optional with its default, and unknown fields are ignored,
/// so files written by older or newer releases stay readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewState {
    pub theme: StoredTheme,
    pub sidebar_minimised: bool,
    pub diff_view: DiffView,
}

/// The state file's path, from the environment as `var` reports it:
/// `$XDG_STATE_HOME/ocelli` (or `$HOME/.local/state/ocelli`) on XDG systems,
/// `~/Library/Application Support/ocelli` on macOS, `%LOCALAPPDATA%\ocelli`
/// on Windows. `None` when the environment names no usable directory.
pub fn path_from(os: &str, var: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    // The XDG rule for every variable: unset, empty or relative means "ignore".
    let absolute = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
    let dir = match os {
        "windows" => absolute("LOCALAPPDATA")?,
        "macos" => absolute("HOME")?
            .join("Library")
            .join("Application Support"),
        _ => absolute("XDG_STATE_HOME")
            .or_else(|| absolute("HOME").map(|h| h.join(".local").join("state")))?,
    };
    Some(dir.join("ocelli").join(FILE))
}

/// The state file's path on this machine.
pub fn path() -> Option<PathBuf> {
    path_from(std::env::consts::OS, |name| std::env::var(name).ok())
}

/// Reads the state; anything missing or malformed yields the defaults.
pub fn load(path: &Path) -> ViewState {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Writes the state through a temporary file renamed over the old one, so an
/// interrupted write cannot leave a half-written file behind.
pub fn save(path: &Path, state: &ViewState) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(state).map_err(io::Error::other)?;
    fs::write(&tmp, json + "\n")?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    #[test]
    // The expected paths are absolute only by Unix rules, which `Path::is_absolute` applies on
    // the machine running the test; the Windows layout has its own test below.
    #[cfg(unix)]
    fn xdg_state_home_wins_and_falls_back_to_local_state() {
        assert_eq!(
            path_from("linux", env(&[("XDG_STATE_HOME", "/s"), ("HOME", "/h")])),
            Some(PathBuf::from("/s/ocelli/cli-state.json"))
        );
        for unusable in ["", "relative/state"] {
            assert_eq!(
                path_from(
                    "linux",
                    env(&[("XDG_STATE_HOME", unusable), ("HOME", "/h")])
                ),
                Some(PathBuf::from("/h/.local/state/ocelli/cli-state.json")),
                "{unusable:?} must be ignored"
            );
        }
        assert_eq!(path_from("linux", env(&[])), None);
    }

    #[test]
    // The expected paths are absolute only by Unix rules, which `Path::is_absolute` applies on
    // the machine running the test; the Windows layout has its own test below.
    #[cfg(unix)]
    fn macos_uses_application_support() {
        assert_eq!(
            path_from(
                "macos",
                env(&[("HOME", "/Users/a"), ("XDG_STATE_HOME", "/s")])
            ),
            Some(PathBuf::from(
                "/Users/a/Library/Application Support/ocelli/cli-state.json"
            ))
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_local_app_data() {
        assert_eq!(
            path_from(
                "windows",
                env(&[("LOCALAPPDATA", r"C:\Users\a\AppData\Local")])
            ),
            Some(PathBuf::from(
                r"C:\Users\a\AppData\Local\ocelli\cli-state.json"
            ))
        );
    }

    #[test]
    fn the_json_shape_is_the_documented_one() {
        let state = ViewState {
            theme: StoredTheme::Light,
            sidebar_minimised: true,
            diff_view: DiffView::Split,
        };
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            r#"{"theme":"light","sidebarMinimised":true,"diffView":"split"}"#
        );
    }

    #[test]
    fn missing_fields_default_and_unknown_fields_are_ignored() {
        let state: ViewState =
            serde_json::from_str(r#"{"theme":"light","fromTheFuture":1}"#).unwrap();
        assert_eq!(
            state,
            ViewState {
                theme: StoredTheme::Light,
                ..ViewState::default()
            }
        );
    }

    /// A temp directory that removes itself, also when an assertion fails.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "ocelli-cli-state-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn save_creates_the_directory_and_load_reads_it_back() {
        let scratch = Scratch::new();
        let path = scratch.0.join("ocelli").join(FILE);
        let state = ViewState {
            theme: StoredTheme::Light,
            sidebar_minimised: true,
            diff_view: DiffView::Unified,
        };
        save(&path, &state).unwrap();
        assert_eq!(load(&path), state);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "no temp file left"
        );
    }

    #[test]
    fn a_missing_or_broken_file_means_defaults() {
        let scratch = Scratch::new();
        let path = scratch.0.join(FILE);
        assert_eq!(load(&path), ViewState::default());
        fs::create_dir_all(&scratch.0).unwrap();
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), ViewState::default());
    }
}

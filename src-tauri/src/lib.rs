mod commands;

use commands::AppState;
use std::sync::Mutex;

/// The repository named on the command line: the first argument after the program that is not an
/// option, as `ocelli-cli [PATH]` takes it (ADR-0028). Options are left alone, so that what a
/// platform or a launcher adds is never read as a path.
pub fn startup_path(args: impl IntoIterator<Item = String>) -> Option<String> {
    args.into_iter().skip(1).find(|arg| !arg.starts_with('-'))
}

/// The startup path resolved against the working directory, as `ocelli-cli` resolves its own; a
/// path that does not resolve is kept as given, so that opening it reports the error.
fn resolved(path: String) -> String {
    std::fs::canonicalize(&path)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            current_repo: Mutex::new(None),
            startup_repo: Mutex::new(startup_path(std::env::args()).map(resolved)),
        })
        .invoke_handler(tauri::generate_handler![
            commands::startup_repo,
            commands::open_repo,
            commands::browse_repo,
            commands::select_commit,
            commands::get_file_diff,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{resolved, startup_path};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn the_first_argument_after_the_program_names_the_repository() {
        assert_eq!(
            startup_path(args(&["ocelli", "/repos/app"])),
            Some("/repos/app".into())
        );
    }

    #[test]
    fn without_an_argument_no_repository_is_opened() {
        assert_eq!(startup_path(args(&["ocelli"])), None);
        assert_eq!(startup_path(Vec::new()), None);
    }

    #[test]
    fn options_are_never_read_as_a_path() {
        assert_eq!(
            startup_path(args(&["ocelli", "-psn_0_12345", "--flag", "repo"])),
            Some("repo".into())
        );
        assert_eq!(startup_path(args(&["ocelli", "--verbose"])), None);
    }

    #[test]
    fn a_relative_path_is_resolved_against_the_working_directory() {
        let here = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(resolved(".".into()), here.to_string_lossy());
    }

    #[test]
    fn a_path_that_does_not_resolve_is_kept_for_the_error() {
        assert_eq!(resolved("no/such/repository".into()), "no/such/repository");
    }
}

//! What reaches the browser from the server's tabs, scrubbed (ADR-0037, ADR-0035): no path
//! outside the server root leaves the server, the paths of the sidebar's rows sent relative to it,
//! or empty where they lie outside; what git says is replaced; and a transfer's progress names no
//! credential and no path on the server.

use crate::root::Root;
use ocelli_commands::HostError;
use serde_json::Value;
use std::path::Path;

/// What the failure of a write the git adapter reported says in the browser, its refusals
/// included: its words may name a path on the server (ADR-0037).
pub const WRITE_FAILED: &str =
    "The write was refused or failed: reload to see the repository as it is";

/// What a reason that names a path on the server says instead.
pub const READ_FAILED: &str = "Reading the repository failed";

/// The failure of a command that reached the repository as the browser may read it, decided by
/// its kind: Ocelli's own refusals and that no repository is open as they are, and anything git
/// said, which may name a path on the server, as `READ_FAILED` (ADR-0037).
pub fn scrub(error: HostError) -> String {
    match error {
        HostError::NoRepository | HostError::Refused(_) => error.to_string(),
        HostError::Git(_) => READ_FAILED.into(),
    }
}

/// The failure of a write as the browser may read it: `scrub`, with what git said, or the git
/// adapter's refusal, which its type does not tell from it, as `WRITE_FAILED`.
pub fn scrub_write(error: HostError) -> String {
    match error {
        HostError::Git(_) => WRITE_FAILED.into(),
        error => scrub(error),
    }
}

/// A transfer's progress as the browser may read it: a word that is an address keeps its scheme,
/// host, and path but not the user name and password it may carry, and a word that names a path
/// on the server, absolute or with a backslash, reads `<path>`. The git adapter's progress names
/// the remote and counts objects; what a repository's configuration names is untrusted, so what
/// may reach the words is scrubbed here (ADR-0037, ADR-0035).
pub fn scrub_progress(text: &str) -> String {
    let scrubbed: Vec<String> = text.split(' ').map(scrub_word).collect();
    scrubbed.join(" ")
}

fn scrub_word(word: &str) -> String {
    if let Some((scheme, rest)) = word.split_once("://") {
        let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
        let host = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        // A local address's path is a path on the server.
        let path = if scheme.eq_ignore_ascii_case("file") {
            "<path>"
        } else {
            path
        };
        return format!("{scheme}://{host}{path}");
    }
    let bare = word.trim_start_matches(['"', '\'', '(', '[', '<', '`']);
    let drive =
        bare.len() > 2 && bare.as_bytes()[0].is_ascii_alphabetic() && bare[1..].starts_with(":/");
    if bare.starts_with(['/', '~']) || bare.contains('\\') || drive {
        "<path>".into()
    } else {
        word.into()
    }
}

/// A status line with what git said, after a load's error, replaced by `READ_FAILED`.
fn scrub_status(status: &mut Value) {
    let Some(text) = status.as_str() else {
        return;
    };
    let scrubbed = if text.starts_with("Error: ") {
        format!("Error: {READ_FAILED}")
    } else if let Some((rows, _)) = text.split_once(" — loading stopped: ") {
        format!("{rows} — loading stopped: {READ_FAILED}")
    } else {
        return;
    };
    *status = Value::String(scrubbed);
}

/// The payload of `event` with no path outside the server root: the sidebar's working trees,
/// submodules, and parent row carry their path relative to it, or none where they lie outside,
/// which shows the row without its switch, the web interface saying why (ADR-0037); a status
/// line says what git said only as `READ_FAILED`.
pub fn outbound(root: &Root, event: &str, payload: &mut Value) {
    let relative = |value: &mut Value| {
        if let Some(path) = value.as_str() {
            let inside = (!path.is_empty())
                .then(|| root.relative(Path::new(path)))
                .flatten();
            *value = Value::String(inside.unwrap_or_default());
        }
    };
    match event {
        "load-refs" => {
            let refs = &mut payload["refs"];
            for worktree in refs["worktrees"].as_array_mut().into_iter().flatten() {
                relative(&mut worktree["path"]);
            }
            for submodule in refs["submodules"].as_array_mut().into_iter().flatten() {
                relative(&mut submodule["workdir"]);
            }
            if refs["parent"].is_object() {
                relative(&mut refs["parent"]["path"]);
            }
        }
        "set-status" => scrub_status(payload),
        "write-progress" => {
            if let Some(text) = payload.as_str() {
                *payload = Value::String(scrub_progress(text));
            }
        }
        "history-grew" => scrub_status(&mut payload["status"]),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::Tree;
    use super::*;
    use serde_json::json;

    #[test]
    // Verifies: ADR-0037
    fn the_sidebar_s_paths_leave_the_server_relative_to_the_root_or_not_at_all() {
        let tree = Tree::new("refs");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let inside = tree.0.join("root/wt").to_string_lossy().into_owned();
        let outside = tree.0.join("outside").to_string_lossy().into_owned();
        let mut payload = json!({ "refs": {
            "worktrees": [{ "path": inside }, { "path": outside }],
            "submodules": [{ "workdir": inside }, { "workdir": "" }, { "workdir": outside }],
            "parent": { "path": outside },
        }});
        outbound(&root, "load-refs", &mut payload);
        let refs = &payload["refs"];
        assert_eq!(refs["worktrees"][0]["path"], "wt");
        assert_eq!(refs["worktrees"][1]["path"], "");
        assert_eq!(refs["submodules"][0]["workdir"], "wt");
        assert_eq!(refs["submodules"][1]["workdir"], "");
        assert_eq!(refs["submodules"][2]["workdir"], "");
        assert_eq!(
            refs["parent"],
            json!({ "path": "" }),
            "a parent outside shows without its switch"
        );
        let mut parent = json!({ "refs": { "parent": { "name": "wt", "path": inside } } });
        outbound(&root, "load-refs", &mut parent);
        assert_eq!(parent["refs"]["parent"]["path"], "wt");
        let mut none = json!({ "refs": { "worktrees": [], "submodules": [], "parent": null } });
        outbound(&root, "load-refs", &mut none);
        assert_eq!(none["refs"]["parent"], Value::Null);
    }

    #[test]
    // Verifies: ADR-0037
    fn what_git_says_reaches_the_browser_only_as_reading_failed() {
        // What git says is replaced whatever its words, Ocelli's own included.
        for said in [
            "failed to resolve path '/nonexistent/x': No such file or directory".to_string(),
            "could not open C:\\Users\\a\\repo".to_string(),
            crate::root::refused("app"),
        ] {
            assert_eq!(scrub(HostError::Git(said.clone())), READ_FAILED, "{said}");
        }
        // A refusal reaches the browser with its reason, whatever its words.
        for reason in [
            crate::root::refused("app"),
            crate::root::too_large("app"),
            "any reason at all".to_string(),
        ] {
            assert_eq!(scrub(HostError::Refused(reason.clone())), reason);
        }
        assert_eq!(
            scrub(HostError::NoRepository),
            HostError::NoRepository.to_string()
        );
        // A write's failure from git, the git adapter's refusal among them, says that it failed;
        // Ocelli's own refusals pass as they do for a read.
        for said in [
            "the checked-out commit is no longer the one shown: reload to see it as it is",
            "failed to write /srv/repo/.git/index.lock: Permission denied",
        ] {
            assert_eq!(scrub_write(HostError::Git(said.into())), WRITE_FAILED);
        }
        assert_eq!(
            scrub_write(HostError::Refused("A write is still running".into())),
            "A write is still running"
        );
        assert_eq!(
            scrub_write(HostError::NoRepository),
            HostError::NoRepository.to_string()
        );
        let tree = Tree::new("status");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let mut status = json!("Error: failed to resolve path '/srv/x'");
        outbound(&root, "set-status", &mut status);
        assert_eq!(status, format!("Error: {READ_FAILED}"));
        let mut grew = json!({ "rows": 3, "status": "3 commits — loading stopped: object /srv/x" });
        outbound(&root, "history-grew", &mut grew);
        assert_eq!(
            grew["status"],
            format!("3 commits — loading stopped: {READ_FAILED}")
        );
        for plain in ["3 commits", "Loaded 3000 commits...", "Loading..."] {
            let mut status = json!(plain);
            outbound(&root, "set-status", &mut status);
            assert_eq!(status, plain);
        }
    }

    #[test]
    // Verifies: ADR-0035, ADR-0037, Q-6
    fn a_transfer_s_progress_names_no_credential_and_no_path_on_the_server() {
        for (said, read) in [
            (
                "Fetching origin: 3 of 5 objects",
                "Fetching origin: 3 of 5 objects",
            ),
            ("Pushing a: 1 of 2 objects", "Pushing a: 1 of 2 objects"),
            (
                "Fetching https://ada:SECRET@example.org/r.git: authenticating",
                "Fetching https://example.org/r.git: authenticating",
            ),
            (
                "Fetching ssh://ada@host:22: the server is preparing",
                "Fetching ssh://host:22: the server is preparing",
            ),
            (
                "Fetching file:///srv/root/remote.git: 1 of 1 objects",
                "Fetching file://<path> 1 of 1 objects",
            ),
            (
                "from /srv/root/remote.git ('/home/ada/x') C:/Users/ada C:\\Users\\ada ~/r",
                "from <path> <path> <path> <path> <path>",
            ),
        ] {
            assert_eq!(scrub_progress(said), read, "{said}");
        }
        let tree = Tree::new("progress");
        let root = Root::new(&tree.0.join("root")).unwrap();
        let mut payload = json!("Fetching https://u:p@h/r: /srv/x");
        outbound(&root, "write-progress", &mut payload);
        assert_eq!(payload, "Fetching https://h/r: <path>");
    }
}

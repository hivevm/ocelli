//! The one way a test starts `git` (Q-4): with the developer's global and system configuration
//! out of reach, and every program a repository can name for `git` to run turned off on its
//! command line, the hooks, `core.fsmonitor`, the pager, the editors, the signing switches, the
//! credential helpers, and the drivers its configuration names for diffing, filtering, and
//! merging. A test's own `git` then runs nothing the repository brings, so a marker a Q-4 test
//! finds was left by Ocelli. `tests_start_git_one_way` holds every test to it.

use std::path::Path;
use std::process::Command;

/// What `git` would run otherwise, turned off.
const OFF: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.pager", "cat"),
    ("core.editor", "true"),
    ("sequence.editor", "true"),
    ("core.askPass", ""),
    ("core.sshCommand", "ssh"),
    ("credential.helper", ""),
    ("diff.external", ""),
    ("commit.gpgSign", "false"),
    ("tag.gpgSign", "false"),
    ("push.gpgSign", "false"),
];

/// `git` with `dir` as its working directory, ready for a test's arguments.
pub fn git_command(dir: impl AsRef<Path>) -> Command {
    let dir = dir.as_ref();
    let mut command = bare(dir);
    for (key, value) in OFF {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    let hooks = std::env::temp_dir().join("ocelli-tests-no-hooks");
    command
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()));
    // An empty filter turns it off. An empty textconv, diff command, or merge driver would run
    // an empty program: `cat` stands in for a textconv, which then changes nothing, and `false`
    // for the others, a diff or a merge that needs one failing loudly.
    for key in drivers(dir) {
        let off = if key.ends_with(".textconv") {
            "cat"
        } else if key.ends_with(".command") || key.ends_with(".driver") {
            "false"
        } else {
            ""
        };
        command.arg("-c").arg(format!("{key}={off}"));
    }
    command
}

/// `git` in `dir` with nothing but the configuration out of reach and the editors and prompts
/// turned off, environment variables that any `git` honours.
fn bare(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .arg("--no-pager")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            std::env::temp_dir().join("ocelli-tests-no-global-config"),
        )
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    command
}

/// The keys of the repository's own configuration that name a driver's program: a filter's,
/// a diff's, or a merge's. Reading the configuration runs none of them.
fn drivers(dir: &Path) -> Vec<String> {
    let Ok(out) = bare(dir)
        .args([
            "config",
            "--name-only",
            "--get-regexp",
            r"^(filter|diff|merge)\..+\.(clean|smudge|process|command|textconv|driver)$",
        ])
        .output()
    else {
        return Vec::new();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

/// The lines of the Rust source `text` that start `git` some other way, by number.
#[allow(dead_code)] // only the test that holds every test to the helper reads it
pub fn other_ways(text: &str) -> Vec<usize> {
    text.lines()
        .enumerate()
        .filter(|(_, line)| line.contains(concat!("Command::new(", "\"git\")")))
        .map(|(i, _)| i + 1)
        .collect()
}

//! The one way a test starts `git` (Q-4): with the developer's global and system configuration
//! out of reach, and every program a repository can name for `git` to run turned off on its
//! command line, the hooks, `core.fsmonitor`, the pager, the editors, the signing switches, the
//! credential helpers, and the drivers its configuration names for diffing, filtering, and
//! merging. A test's own `git` then runs nothing the repository brings, so a marker a Q-4 test
//! finds was left by Ocelli. `tests_start_git_one_way` holds every test to it.
//!
//! The one exception is `git_running_its_programs`, the control of a Q-4 test: `git` with the
//! same configuration out of reach and the repository's programs left on, which shows that the
//! programs the test names would run where `git` performed what Ocelli performs.

use git2::ConfigLevel;
use std::path::Path;
use std::process::Command;
use std::sync::Once;

/// The developer's global, XDG, and system configuration out of reach of libgit2 in this test
/// binary, the writer's and the reader's, as `bare` puts it out of reach of the test's own `git`:
/// a write and `git` on a twin then both read the repository's own configuration and nothing
/// else. Every way a test makes or starts a repository calls it first, `git` through `bare`, a
/// scratch directory, and the fixture repository, so a new test is covered without a line of
/// its own.
pub fn libgit2_without_developer_configuration() {
    static DONE: Once = Once::new();
    DONE.call_once(|| {
        for level in [ConfigLevel::Global, ConfigLevel::XDG, ConfigLevel::System] {
            // SAFETY: libgit2 reads its search paths with no lock, so no thread may use it while
            // they change. A test reaches libgit2 only on a repository it made or started through
            // one of the callers above, each of which waits here until the paths are set.
            #[allow(unsafe_code)]
            unsafe { git2::opts::set_search_path(level, "") }.unwrap();
        }
    });
}

/// What `git` would run otherwise, turned off.
const OFF: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.pager", "cat"),
    ("core.editor", "true"),
    ("sequence.editor", "true"),
    ("core.askPass", ""),
    ("core.sshCommand", "ssh"),
    ("credential.helper", ""),
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
    // A file, not a directory: no hook can stand below it.
    command.arg("-c").arg("core.hooksPath=/dev/null");
    // An empty filter turns it off. An empty textconv, diff command, external diff, or merge
    // driver would run an empty program: `cat` stands in for a textconv, which then changes
    // nothing, `false` for the others, a diff or a merge that needs one failing loudly, and git's
    // own programs for the ones a fetch and a push start on the remote's side.
    for key in drivers(dir) {
        let off = if key.ends_with(".textconv") {
            "cat"
        } else if key.ends_with(".uploadpack") {
            "git-upload-pack"
        } else if key.ends_with(".receivepack") {
            "git-receive-pack"
        } else if key.ends_with(".command") || key.ends_with(".driver") || key == "diff.external" {
            "false"
        } else {
            ""
        };
        command.arg("-c").arg(format!("{key}={off}"));
    }
    command
}

/// `git` in `dir` that runs the programs the repository names, for the control of a Q-4 test
/// alone: the developer's global and system configuration out of reach, the editors and prompts
/// turned off.
#[allow(dead_code)] // only the controls of the tests of Q-4 start it
pub fn git_running_its_programs(dir: impl AsRef<Path>) -> Command {
    bare(dir.as_ref())
}

/// `git` in `dir` with nothing but the configuration out of reach and the editors and prompts
/// turned off, environment variables that any `git` honours.
fn bare(dir: &Path) -> Command {
    libgit2_without_developer_configuration();
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .arg("--no-pager")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        // The null device: a global configuration that holds nothing, which no one can write to.
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_EDITOR", "true")
        .env("GIT_SEQUENCE_EDITOR", "true")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE");
    // The identity a test's commits carry is the repository's, as the writer's are: `git commit`
    // exports the committing developer's to its hooks, and the pre-commit hook runs the tests.
    for var in [
        "GIT_AUTHOR_NAME",
        "GIT_AUTHOR_EMAIL",
        "GIT_AUTHOR_DATE",
        "GIT_COMMITTER_NAME",
        "GIT_COMMITTER_EMAIL",
        "GIT_COMMITTER_DATE",
    ] {
        command.env_remove(var);
    }
    command
}

/// The keys of the repository's own configuration that name a program: a filter's, a diff's,
/// or a merge's driver, the external diff, and what a fetch or a push starts on the remote's
/// side. Reading the configuration runs none of them.
fn drivers(dir: &Path) -> Vec<String> {
    let Ok(out) = bare(dir)
        .args([
            "config",
            "--name-only",
            "--get-regexp",
            r"^((filter|diff|merge)\..+\.(clean|smudge|process|command|textconv|driver)|diff\.external|remote\..+\.(uploadpack|receivepack))$",
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

/// The lines of the Rust source `text` that start `git` some other way, by number: a command
/// made of `git` by name or by path, however it is spaced, or a name or constant holding `git`
/// for one.
#[allow(dead_code)] // only the test that holds every test to the helper reads it
pub fn other_ways(text: &str) -> Vec<usize> {
    let names_git = |literal: &str| literal == "git" || literal.ends_with("/git");
    text.lines()
        .enumerate()
        .filter(|(_, line)| {
            let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
            let started = line
                .split(concat!("Command::new", "(\""))
                .skip(1)
                .any(|rest| rest.split('"').next().is_some_and(names_git));
            let held = (line.starts_with("let") || line.starts_with("const"))
                && line.split('"').nth(1).is_some_and(names_git);
            started || held
        })
        .map(|(i, _)| i + 1)
        .collect()
}

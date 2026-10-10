//! Every place a repository can name a program for `git` to run (Q-4), set in one go, so that the
//! tests of reading and of writing both prove that Ocelli runs none of them. Each program is a
//! script that leaves a marker named after it; a test that finds no marker afterwards has run
//! none.

use std::path::{Path, PathBuf};

/// Every hook `git help githooks` documents.
pub const HOOKS: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "post-index-change",
];

/// The configuration keys whose value is a program `git` runs when it reads, diffs, filters,
/// merges, signs, edits a message, or asks for a credential; the drivers are those of the
/// attribute `evil`, which [`ATTRIBUTES`] gives every path. A fetch and a push run more, which a
/// remote names: [`REMOTE_PROGRAMS`].
pub const PROGRAMS: &[&str] = &[
    "core.fsmonitor",
    "core.pager",
    "core.editor",
    "core.sshCommand",
    "core.askPass",
    "sequence.editor",
    "diff.external",
    "diff.evil.command",
    "diff.evil.textconv",
    "filter.evil.clean",
    "filter.evil.smudge",
    "filter.evil.process",
    "merge.evil.driver",
    "gpg.program",
    "gpg.openpgp.program",
    "gpg.ssh.program",
    "gpg.x509.program",
    "credential.helper",
];

/// The keys under `remote.<name>` whose value is the program a fetch or a push over the local
/// transport starts on the remote's side. Each script stands in for the program it replaces and
/// then runs it, so that a fetch or a push by `git` goes through and runs the hooks after it.
pub const REMOTE_PROGRAMS: &[(&str, &str)] = &[
    ("uploadpack", "upload-pack"),
    ("receivepack", "receive-pack"),
];

/// The switches that make `git` run a program it would otherwise leave alone.
pub const SWITCHES: &[(&str, &str)] = &[
    ("commit.gpgSign", "true"),
    ("tag.gpgSign", "true"),
    ("push.gpgSign", "true"),
];

/// The attributes that send every path through the `evil` drivers.
pub const ATTRIBUTES: &str = "* diff=evil filter=evil merge=evil\n";

/// The markers the programs of a test leave, in a directory outside its repository.
pub struct Markers(pub PathBuf);

impl Markers {
    /// The names of the programs that ran.
    pub fn ran(&self) -> Vec<String> {
        std::fs::read_dir(&self.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }
}

/// Names a program everywhere [`HOOKS`], [`PROGRAMS`], [`SWITCHES`], and [`ATTRIBUTES`] say, and
/// [`REMOTE_PROGRAMS`] for each remote the repository has, in the repository at `repo`, with the
/// scripts in `tools`, a directory outside it. Set only once the repository is built, so that
/// building it with `git` runs none of them.
pub fn set_everywhere(repo: &Path, tools: &Path) -> Markers {
    use std::os::unix::fs::PermissionsExt;
    let markers = tools.join("markers");
    let hooks = tools.join("hooks");
    std::fs::create_dir_all(&markers).unwrap();
    std::fs::create_dir_all(&hooks).unwrap();
    let script = |path: &Path, marker: &str| {
        let body = format!("#!/bin/sh\ntouch '{}/{marker}'\n", markers.display());
        std::fs::write(path, body).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    let git_dir = git2::Repository::open(repo).unwrap().path().to_path_buf();
    std::fs::create_dir_all(git_dir.join("hooks")).unwrap();
    for hook in HOOKS {
        script(&hooks.join(hook), &format!("hook-{hook}"));
        std::fs::copy(hooks.join(hook), git_dir.join("hooks").join(hook)).unwrap();
    }
    let mut config = git2::Repository::open(repo).unwrap().config().unwrap();
    config
        .set_str("core.hooksPath", hooks.to_str().unwrap())
        .unwrap();
    for key in PROGRAMS {
        let path = tools.join(key);
        script(&path, key);
        config.set_str(key, path.to_str().unwrap()).unwrap();
    }
    let remotes = git2::Repository::open(repo).unwrap().remotes().unwrap();
    for remote in remotes.iter().flatten() {
        for (key, program) in REMOTE_PROGRAMS {
            let key = format!("remote.{remote}.{key}");
            let path = tools.join(&key);
            script(&path, &key);
            let mut body = std::fs::read_to_string(&path).unwrap();
            body.push_str(&format!("exec git {program} \"$@\"\n"));
            std::fs::write(&path, body).unwrap();
            config.set_str(&key, path.to_str().unwrap()).unwrap();
        }
    }
    for (key, value) in SWITCHES {
        config.set_str(key, value).unwrap();
    }
    std::fs::create_dir_all(git_dir.join("info")).unwrap();
    std::fs::write(git_dir.join("info/attributes"), ATTRIBUTES).unwrap();
    Markers(markers)
}

/// The hooks `git help githooks` documents, where its manual is installed.
#[allow(dead_code)] // only the test of reading compares the list with git's manual
pub fn documented_hooks() -> Option<Vec<String>> {
    let out = crate::git_command::git_command(std::env::temp_dir())
        .args(["help", "githooks"])
        .env("MANPAGER", "cat")
        .env("PAGER", "cat")
        .env("MANWIDTH", "100")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let section = text.split_once("\nHOOKS")?.1;
    let section = section.split("\nENVIRONMENT").next().unwrap_or(section);
    let hooks: Vec<String> = section
        .lines()
        .filter(|l| l.starts_with("   ") && !l.starts_with("    "))
        .map(str::trim)
        .filter(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
        .map(str::to_string)
        .collect();
    (out.status.success() && !hooks.is_empty()).then_some(hooks)
}

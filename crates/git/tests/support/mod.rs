//! What the tests of the writes share: a scratch directory, `git` started the one way
//! (`git_command`), a file written, the repository's snapshot, and `refused`, the one way a test
//! takes a refusal of a write. A refused write leaves every part of the snapshot as it was
//! (`docs/CONVENTIONS.md`, a refusal changes nothing); a write compared with `git` on a twin of
//! the repository leaves the snapshot `git` leaves, the commits each side made alone compared by
//! content.

// A test binary uses what it needs of the module and leaves the rest.
#![allow(dead_code)]

#[path = "../git_command/mod.rs"]
pub mod git_command;

pub use git_command::git_command;
use ocelli_core::port::{RepositoryWriter, Write};
use ocelli_git::GitWriter;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// A scratch directory, named for the test binary, the test's `name`, and the process, and
/// removed when the test ends.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ocelli-{}-{name}-{}",
            env!("CARGO_CRATE_NAME"),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The identity the test's own `git` commits as, the one the repositories of the tests keep in
/// their configuration, so that a commit `git` makes on a twin and one the writer makes from the
/// configuration are by the same person.
const IDENTITY: [&str; 4] = [
    "-c",
    "user.name=Ada",
    "-c",
    "user.email=ada@example.invalid",
];

/// Runs `git` and returns whether it succeeded and what it printed: its standard output where it
/// succeeded, its standard error where it failed.
pub fn git_output(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = git_command(dir).args(IDENTITY).args(args).output().unwrap();
    let text = if out.status.success() {
        String::from_utf8(out.stdout).unwrap()
    } else {
        String::from_utf8_lossy(&out.stderr).into_owned()
    };
    (out.status.success(), text)
}

/// What `git` prints, untrimmed, for output whose leading space or NUL separators are read.
pub fn raw(dir: &Path, args: &[&str]) -> String {
    let (ok, text) = git_output(dir, args);
    assert!(ok, "git {args:?}: {text}");
    text
}

/// What `git` prints, trimmed; a failure fails the test.
pub fn git(dir: &Path, args: &[&str]) -> String {
    raw(dir, args).trim().to_string()
}

/// Runs `git` and says whether it succeeded, for the commands that stop on a conflict.
pub fn git_may_fail(dir: &Path, args: &[&str]) -> bool {
    git_output(dir, args).0
}

/// Writes `content` to `path` in `dir`, the directories it needs made.
pub fn write_file(dir: &Path, path: &str, content: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// A copy of the repository, for `git` to do on what the writer does on the original.
pub fn twin(of: &Scratch, name: &str) -> Scratch {
    let t = Scratch::new(name);
    std::fs::remove_dir_all(&t.0).unwrap();
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(&of.0)
        .arg(&t.0)
        .status()
        .unwrap();
    assert!(status.success());
    t
}

/// HEAD as `git` reads it: the branch it is on, or the commit it is detached at.
pub fn head_of(dir: &Path) -> String {
    match git_output(dir, &["symbolic-ref", "--quiet", "--short", "HEAD"]) {
        (true, branch) => branch.trim().to_string(),
        _ => format!("detached at {}", git(dir, &["rev-parse", "HEAD"])),
    }
}

/// The files `git` keeps in its directory while an operation is in progress, and `ORIG_HEAD`,
/// which the operations set.
const IN_PROGRESS: &[&str] = &[
    "MERGE_HEAD",
    "MERGE_MSG",
    "MERGE_MODE",
    "MERGE_AUTOSTASH",
    "AUTO_MERGE",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "ORIG_HEAD",
    "BISECT_LOG",
    "rebase-merge",
    "rebase-apply",
    "sequencer",
];

/// The repository as `git` reads it, everything a write can change.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Snapshot {
    pub head: String,
    /// Every ref, the tags, the remote branches, and the stash included.
    pub refs: String,
    /// The stash's entries, those below its newest included, which no ref names.
    pub stash: String,
    pub index: String,
    /// Each path the status names, ignored and untracked files included, one to a line.
    pub status: String,
    /// The repository's own configuration, where a branch's upstream is kept.
    pub config: String,
    /// Every path of the working directory, `.git` left out: a directory, an empty one included,
    /// which the status omits, by its name and a `/`, and a file with its content, whether it is
    /// executable, or a link's target.
    pub files: BTreeMap<String, String>,
    /// The files of an operation in progress, with their content.
    pub in_progress: BTreeMap<String, String>,
}

pub fn snapshot(dir: &Path) -> Snapshot {
    let status = raw(
        dir,
        &[
            "status",
            "--porcelain",
            "-z",
            "--ignored",
            "--untracked-files=all",
        ],
    );
    let git_dir = PathBuf::from(git(dir, &["rev-parse", "--absolute-git-dir"]));
    let mut in_progress = BTreeMap::new();
    for name in IN_PROGRESS {
        files_below(&git_dir, Path::new(name), &mut in_progress);
    }
    let mut files = BTreeMap::new();
    paths_below(dir, Path::new(""), &mut files);
    Snapshot {
        head: head_of(dir),
        refs: git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        stash: git(dir, &["stash", "list", "--format=%gd %H %gs"]),
        index: git(dir, &["ls-files", "--stage"]),
        // A rename or a copy names its source in the field after its own.
        status: status.trim_end_matches('\0').replace('\0', "\n"),
        config: git(dir, &["config", "--local", "--list"]),
        files,
        in_progress,
    }
}

/// Every path below `path` of the working directory `root`, `.git` left out, with what it holds.
fn paths_below(root: &Path, path: &Path, found: &mut BTreeMap<String, String>) {
    for entry in std::fs::read_dir(root.join(path)).unwrap() {
        let name = path.join(entry.unwrap().file_name());
        if name.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        let full = root.join(&name);
        let meta = full.symlink_metadata().unwrap();
        let shown = name.display().to_string();
        if meta.is_symlink() {
            let target = std::fs::read_link(&full).unwrap();
            found.insert(shown, format!("-> {}", target.display()));
        } else if meta.is_dir() {
            found.insert(format!("{shown}/"), String::new());
            paths_below(root, &name, found);
        } else {
            let content = String::from_utf8_lossy(&std::fs::read(&full).unwrap()).into_owned();
            found.insert(shown, format!("{}{content}", executable(&meta)));
        }
    }
}

#[cfg(unix)]
fn executable(meta: &std::fs::Metadata) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if meta.permissions().mode() & 0o111 != 0 {
        "(executable) "
    } else {
        ""
    }
}

#[cfg(not(unix))]
fn executable(_: &std::fs::Metadata) -> &'static str {
    ""
}

/// Every file at or below `path` in `root`, with its content.
fn files_below(root: &Path, path: &Path, found: &mut BTreeMap<String, String>) {
    let full = root.join(path);
    if full.is_dir() {
        for entry in std::fs::read_dir(&full).unwrap() {
            files_below(root, &path.join(entry.unwrap().file_name()), found);
        }
    } else if let Ok(content) = std::fs::read(&full) {
        let content = String::from_utf8_lossy(&content).into_owned();
        found.insert(path.display().to_string(), content);
    }
}

impl Snapshot {
    /// Every text of the snapshot, for a change made to each alike.
    fn texts(&mut self) -> Vec<&mut String> {
        let mut texts = vec![
            &mut self.head,
            &mut self.refs,
            &mut self.stash,
            &mut self.index,
            &mut self.status,
            &mut self.config,
        ];
        texts.extend(self.files.values_mut());
        texts.extend(self.in_progress.values_mut());
        texts
    }

    /// The snapshot with the files of an operation in progress `files` names left out.
    pub fn without(&self, files: &[&str]) -> Snapshot {
        let mut snapshot = self.clone();
        snapshot
            .in_progress
            .retain(|name, _| !files.contains(&name.as_str()));
        snapshot
    }

    /// The parts that differ from `other`'s, each as both read it.
    pub fn differences(&self, other: &Snapshot, names: (&str, &str)) -> Vec<String> {
        let parts = |s: &Snapshot| {
            [
                ("HEAD", s.head.clone()),
                ("refs", s.refs.clone()),
                ("stash", s.stash.clone()),
                ("index", s.index.clone()),
                ("status", s.status.clone()),
                ("config", s.config.clone()),
                ("files", format!("{:#?}", s.files)),
                ("in progress", format!("{:#?}", s.in_progress)),
            ]
        };
        parts(self)
            .into_iter()
            .zip(parts(other))
            .filter(|(a, b)| a.1 != b.1)
            .map(|((part, a), (_, b))| {
                format!("    {part} {}:\n{a}\n    {part} {}:\n{b}", names.0, names.1)
            })
            .collect()
    }
}

/// The snapshots of the twins `a` and `b`: a commit or an annotated tag one of them holds and the
/// other does not, made on one side alone, stands for its content, its time left out, since the
/// two sides make the same commit at different times. One both hold keeps its id.
pub fn as_twins(a: &Path, b: &Path) -> (Snapshot, Snapshot) {
    snapshots_as_twins((a, snapshot(a)), (b, snapshot(b)))
}

/// [`as_twins`], of the snapshots already taken of each twin.
pub fn snapshots_as_twins(a: (&Path, Snapshot), b: (&Path, Snapshot)) -> (Snapshot, Snapshot) {
    (by_content(a.1, a.0, b.0), by_content(b.1, b.0, a.0))
}

/// Expects the twins `a` and `b` to read alike, as [`as_twins`] reads them.
#[track_caller]
pub fn assert_twins_alike(a: &Path, b: &Path, what: &str) {
    assert_twins_alike_but(a, b, what, &[]);
}

/// [`assert_twins_alike`], but for the files of an operation in progress `known` names, where
/// the writer is known to leave on `a` what `git` does not leave on `b`: each is asserted to
/// still differ, so that a fix shows up as a failure that removes it.
#[track_caller]
pub fn assert_twins_alike_but(a: &Path, b: &Path, what: &str, known: &[&str]) {
    let (ours, theirs) = as_twins(a, b);
    for file in known {
        assert_ne!(
            ours.in_progress.get(*file),
            theirs.in_progress.get(*file),
            "{what}: {file} is as git leaves it now; it is no longer a known difference"
        );
    }
    let (ours, theirs) = (ours.without(known), theirs.without(known));
    assert!(
        ours == theirs,
        "{what}: the twins differ\n{}",
        ours.differences(&theirs, ("here", "on the twin"))
            .join("\n")
    );
}

fn by_content(mut snapshot: Snapshot, dir: &Path, other: &Path) -> Snapshot {
    // Most ids are of objects both hold: asked of the twin at once, they keep their names.
    let ids: Vec<String> = snapshot
        .texts()
        .iter()
        .flat_map(|text| object_ids(text))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut named: BTreeMap<String, String> = ids
        .iter()
        .zip(kinds(other, &refs))
        .filter(|(_, kind)| kind != "missing")
        .map(|(id, _)| (id.clone(), id.clone()))
        .collect();
    for text in snapshot.texts() {
        let ids: BTreeSet<String> = object_ids(text).into_iter().collect();
        for id in ids {
            let name = content_of(&id, dir, other, &mut named);
            if name != id {
                *text = text.replace(&id, &name);
            }
        }
    }
    snapshot
}

/// The object ids in `text`: each run of exactly 40 hexadecimal digits.
fn object_ids(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_hexdigit())
        .filter(|w| w.len() == 40)
        .map(str::to_string)
        .collect()
}

/// What `git cat-file --batch-check` says of each of `ids` in `dir`: its type, or `missing`.
fn kinds(dir: &Path, ids: &[&str]) -> Vec<String> {
    let mut child = git_command(dir)
        .args(["cat-file", "--batch-check=%(objecttype)"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for id in ids {
        writeln!(stdin, "{id}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| l.rsplit(' ').next().unwrap_or(l).to_string())
        .collect()
}

/// The id `id` stands for in `dir`'s snapshot beside its twin `other`: itself where `other`
/// holds it or it is neither a commit nor a tag, else its content with its ids named alike and
/// its times left out.
fn content_of(id: &str, dir: &Path, other: &Path, named: &mut BTreeMap<String, String>) -> String {
    if let Some(name) = named.get(id) {
        return name.clone();
    }
    let in_other = kinds(other, &[id]).remove(0) != "missing";
    let kind = kinds(dir, &[id]).remove(0);
    let name = if in_other || !(kind == "commit" || kind == "tag") {
        id.to_string()
    } else {
        let object = raw(dir, &["cat-file", &kind, id]);
        let (header, message) = object.split_once("\n\n").unwrap_or((&object, ""));
        let header: Vec<String> = header
            .lines()
            .map(|line| match line.split_once(' ') {
                Some((key @ ("author" | "committer" | "tagger"), who)) => {
                    // The last two words are the time and its zone.
                    let who = who.rsplitn(3, ' ').nth(2).unwrap_or(who);
                    format!("{key} {who}")
                }
                Some((key @ ("parent" | "object"), of)) => {
                    format!("{key} {}", content_of(of, dir, other, named))
                }
                _ => line.to_string(),
            })
            .collect();
        format!(
            "<{kind} made on one side: {:?}>",
            format!("{}\n\n{message}", header.join("\n"))
        )
    };
    named.insert(id.to_string(), name.clone());
    name
}

/// Writes `write` and expects it refused with `why` among its words, and the repository's
/// [`Snapshot`] as it was before; returns the refusal. Every refusal test of the writer goes
/// through here, so that none compares less than the whole snapshot.
pub fn refused(dir: &Path, write: &Write, why: &str) -> String {
    refused_reporting(dir, write, why, true, |_| {})
}

/// [`refused`], with the write's progress answered with `go_on`, and `meanwhile` run on the
/// repository at the first report, as another program would change it while the write waits:
/// the snapshot expected after is the one taken once `meanwhile` ran.
pub fn refused_reporting(
    dir: &Path,
    write: &Write,
    why: &str,
    go_on: bool,
    meanwhile: impl FnOnce(&Path),
) -> String {
    let mut before = snapshot(dir);
    let mut meanwhile = Some(meanwhile);
    let mut reports = 0;
    let error = GitWriter
        .write_reporting(dir, write, &mut |_| {
            reports += 1;
            if let Some(change) = meanwhile.take() {
                change(dir);
                before = snapshot(dir);
            }
            go_on
        })
        .unwrap_err();
    assert!(error.to_string().contains(why), "{write:?}: {error}");
    assert!(
        go_on || reports > 0,
        "{write:?}: the write was asked to stop, so it reported"
    );
    assert_eq!(snapshot(dir), before, "{write:?}");
    error.to_string()
}

/// Writes `write` and expects it to stop with `why` among its words after doing part of its
/// work, as a rebase's step past a resolved one stops on a file in its way the way `git`'s does;
/// returns the error. The repository is expected changed: an error that changes nothing is a
/// refusal, and goes through [`refused`].
pub fn stopped_partway(dir: &Path, write: &Write, why: &str) -> String {
    let before = snapshot(dir);
    let error = GitWriter.write(dir, write).unwrap_err();
    assert!(error.to_string().contains(why), "{write:?}: {error}");
    assert_ne!(
        snapshot(dir),
        before,
        "{write:?}: nothing was done, so it was refused"
    );
    error.to_string()
}

/// [`refused`], but for the files of an operation in progress `known` names, which the writer is
/// known to leave changed where it refuses: each is asserted to be changed still, so that a fix
/// shows up as a failure that removes it.
pub fn refused_leaving(dir: &Path, write: &Write, why: &str, known: &[&str]) -> String {
    let before = snapshot(dir);
    let error = GitWriter.write(dir, write).unwrap_err();
    assert!(error.to_string().contains(why), "{write:?}: {error}");
    let after = snapshot(dir);
    for file in known {
        assert_ne!(
            after.in_progress.get(*file),
            before.in_progress.get(*file),
            "{write:?}: {file} is left as it was now; it is no longer a known difference"
        );
    }
    assert_eq!(after.without(known), before.without(known), "{write:?}");
    error.to_string()
}

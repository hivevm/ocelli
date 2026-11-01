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
        git_command::libgit2_without_developer_configuration();
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

/// What a caller may send in place of a string: a NUL, nothing, a path out of the repository or
/// into its git directory, an option, a refspec, a pattern, ref-name and revision syntax, control
/// characters, and lengths past what a file system or a ref takes.
pub fn hostile() -> Vec<String> {
    let mut values: Vec<String> = [
        "\0",
        "a\0b",
        "\0refs/heads/main",
        "main\0",
        "",
        " ",
        "\n",
        "a\nb",
        "\t",
        "\r",
        "\u{1}",
        "\u{1b}[2J",
        "-x",
        "--upload-pack=touch x",
        "+refs/*:refs/*",
        "refs/*",
        "refs/heads/a..b",
        "refs/heads/",
        "refs/heads/a.lock",
        "a.",
        "*",
        "?",
        "[",
        "..",
        "../x",
        "a/../b",
        "a//b",
        "./f",
        "/",
        "/abs",
        "/etc/passwd",
        ".git",
        ".git/config",
        ".GIT/HEAD",
        "\\",
        "C:\\x",
        "\\\\?\\C:\\",
        "~",
        "HEAD",
        "@",
        "@{",
        "@{-1}",
        "main@{0}",
        "main~1",
        "main^",
        "main^{tree}",
        ":",
        "a:b",
        ":/one",
        "%00",
        "file:///tmp",
        "ext::sh -c touch% x",
        "\u{7f}",
        "\u{202e}",
        "\u{feff}",
        "\u{fffd}",
        "\u{10ffff}",
    ]
    .iter()
    .map(|v| v.to_string())
    .collect();
    values.push("a".repeat(100_000));
    values.push("/".repeat(5_000));
    values.push("../".repeat(2_000));
    values
}

/// `text` as a failure names it: its first 40 characters, escaped.
pub fn shortened(text: &str) -> String {
    let start: String = text.chars().take(40).collect();
    let cut = if start.len() < text.len() { "..." } else { "" };
    format!("{start:?}{cut}")
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
    /// Every text of the snapshot but its files, in the same order on every snapshot.
    fn fields(&mut self) -> [&mut String; 6] {
        [
            &mut self.head,
            &mut self.refs,
            &mut self.stash,
            &mut self.index,
            &mut self.status,
            &mut self.config,
        ]
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

/// Every object a repository holds, taken where twins are about to be made of it: what both
/// twins held when they parted, which [`as_twins`] reads by its id.
pub struct Origin(BTreeSet<String>);

impl Origin {
    /// The objects `dir` holds now, those no ref reaches included.
    pub fn of(dir: &Path) -> Origin {
        let all = git(
            dir,
            &[
                "cat-file",
                "--batch-all-objects",
                "--batch-check=%(objectname)",
            ],
        );
        Origin(all.lines().map(str::to_string).collect())
    }

    fn holds(&self, id: &str) -> bool {
        self.0.contains(id)
    }
}

/// The snapshots of the twins `a` and `b`, which parted holding `origin`. An object of `origin`
/// keeps its id; a commit or an annotated tag made since stands for its content, its time left
/// out, since the two sides make the same commit at different times. Where the two read alike
/// but for the ids they name, the ids are taken in pairs, one from each side at the same place: a
/// pair of one id keeps it, and a pair of two commits or two tags made since the twins parted
/// stands for their contents, their parents or objects and the ids their messages name paired
/// alike. Whether the other side holds an id does not decide, since it may have made the same
/// commit elsewhere, at the second this side made it.
pub fn as_twins(a: &Path, b: &Path, origin: &Origin) -> (Snapshot, Snapshot) {
    snapshots_as_twins((a, snapshot(a)), (b, snapshot(b)), origin)
}

/// [`as_twins`], of the snapshots already taken of each twin.
pub fn snapshots_as_twins(
    a: (&Path, Snapshot),
    b: (&Path, Snapshot),
    origin: &Origin,
) -> (Snapshot, Snapshot) {
    let (mut ours, mut theirs) = (a.1, b.1);
    let mut names = Names {
        dirs: [a.0, b.0],
        origin,
        pairs: BTreeMap::new(),
        alone: [BTreeMap::new(), BTreeMap::new()],
    };
    for (x, y) in ours.fields().into_iter().zip(theirs.fields()) {
        names.name_both(x, y);
    }
    names.name_files(&mut ours.files, &mut theirs.files);
    names.name_files(&mut ours.in_progress, &mut theirs.in_progress);
    (ours, theirs)
}

/// Expects the twins `a` and `b`, which parted holding `origin`, to read alike, as [`as_twins`]
/// reads them.
#[track_caller]
pub fn assert_twins_alike(a: &Path, b: &Path, origin: &Origin, what: &str) {
    assert_twins_alike_but(a, b, origin, what, &[]);
}

/// [`assert_twins_alike`], but for the files of an operation in progress `known` names, where
/// the writer is known to leave on `a` what `git` does not leave on `b`: each is asserted to
/// still differ, so that a fix shows up as a failure that removes it.
#[track_caller]
pub fn assert_twins_alike_but(a: &Path, b: &Path, origin: &Origin, what: &str, known: &[&str]) {
    let (ours, theirs) = as_twins(a, b, origin);
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

/// What the ids of the twins stand for, each side's repository in `dirs`, the objects both held
/// when they parted in `origin`.
struct Names<'a> {
    dirs: [&'a Path; 2],
    origin: &'a Origin,
    /// The names of each pair of ids read at the same place on both sides.
    pairs: BTreeMap<(String, String), (String, String)>,
    /// The names of each side's ids at a place the other side does not have.
    alone: [BTreeMap<String, String>; 2],
}

impl Names<'_> {
    /// The files of both sides named: a file both have as a pair of texts, any other alone.
    fn name_files(
        &mut self,
        ours: &mut BTreeMap<String, String>,
        theirs: &mut BTreeMap<String, String>,
    ) {
        for (path, x) in ours.iter_mut() {
            match theirs.get_mut(path) {
                Some(y) => self.name_both(x, y),
                None => self.name_alone(0, x),
            }
        }
        for (path, y) in theirs.iter_mut() {
            if !ours.contains_key(path) {
                self.name_alone(1, y);
            }
        }
    }

    /// `x` and `y`, read at the same place on each side, named: where they read alike but for
    /// their ids, each pair of ids at the same place in them, else each id alone.
    fn name_both(&mut self, x: &mut String, y: &mut String) {
        let (p, q) = (pieces(x), pieces(y));
        let aligned = p.len() == q.len()
            && p.iter().zip(&q).all(|pair| match pair {
                (Piece::Id(_), Piece::Id(_)) => true,
                (Piece::Text(l), Piece::Text(m)) => l == m,
                _ => false,
            });
        if !aligned {
            self.name_alone(0, x);
            self.name_alone(1, y);
            return;
        }
        let (mut named_x, mut named_y) = (String::new(), String::new());
        for pair in p.iter().zip(&q) {
            let (l, m) = match pair {
                (Piece::Id(l), Piece::Id(m)) => self.pair(l, m),
                (Piece::Text(t), _) => (t.to_string(), t.to_string()),
                (Piece::Id(_), Piece::Text(_)) => unreachable!("the pieces are aligned"),
            };
            named_x.push_str(&l);
            named_y.push_str(&m);
        }
        (*x, *y) = (named_x, named_y);
    }

    /// The names of the ids `x` on this side and `y` on the twin, read at the same place: one id
    /// keeps it; two commits or two tags made since the twins parted whose headers have the same
    /// fields stand for their contents, their times left out and their parents or objects and
    /// the ids their messages name paired; any other pair is named alone, so that an object the
    /// twins parted with differs from every other.
    fn pair(&mut self, x: &str, y: &str) -> (String, String) {
        if x == y {
            return (x.to_string(), y.to_string());
        }
        let key = (x.to_string(), y.to_string());
        if let Some(names) = self.pairs.get(&key) {
            return names.clone();
        }
        let made_since = if self.origin.holds(x) || self.origin.holds(y) {
            (None, None)
        } else {
            (made(self.dirs[0], x), made(self.dirs[1], y))
        };
        let names = match made_since {
            (Some(l), Some(m)) if l.kind == m.kind && l.fields() == m.fields() => {
                let (mut ours, mut theirs) = (Vec::new(), Vec::new());
                for (a, b) in l.header.iter().zip(&m.header) {
                    let (a, b) = match (a.split_once(' '), b.split_once(' ')) {
                        (Some((key @ ("parent" | "object"), a)), Some((_, b))) => {
                            let (a, b) = self.pair(a, b);
                            (format!("{key} {a}"), format!("{key} {b}"))
                        }
                        _ => (timeless(a), timeless(b)),
                    };
                    ours.push(a);
                    theirs.push(b);
                }
                // A message names a commit as text, as a revert's, a cherry-pick's with `-x`, or
                // a merge's of a commit does: its ids are paired as the header's are.
                let (mut said, mut said_there) = (l.message.clone(), m.message.clone());
                self.name_both(&mut said, &mut said_there);
                (l.named(&ours, &said), m.named(&theirs, &said_there))
            }
            _ => (self.alone(0, x), self.alone(1, y)),
        };
        self.pairs.insert(key, names.clone());
        names
    }

    /// The ids of `text` on side `side`, at a place the other side does not have, named alone.
    fn name_alone(&mut self, side: usize, text: &mut String) {
        let ids: BTreeSet<String> = pieces(text)
            .into_iter()
            .filter_map(|piece| match piece {
                Piece::Id(id) => Some(id.to_string()),
                Piece::Text(_) => None,
            })
            .collect();
        for id in ids {
            let name = self.alone(side, &id);
            if name != id {
                *text = text.replace(&id, &name);
            }
        }
    }

    /// The id `id` on side `side`, named alone: itself where the twins parted with it or it is
    /// neither a commit nor a tag, else its content, its time left out and its parents or object
    /// and the ids its message names named alone.
    fn alone(&mut self, side: usize, id: &str) -> String {
        if self.origin.holds(id) {
            return id.to_string();
        }
        if let Some(name) = self.alone[side].get(id) {
            return name.clone();
        }
        let name = match made(self.dirs[side], id) {
            Some(object) => {
                let header: Vec<String> = object
                    .header
                    .iter()
                    .map(|line| match line.split_once(' ') {
                        Some((key @ ("parent" | "object"), of)) => {
                            format!("{key} {}", self.alone(side, of))
                        }
                        _ => timeless(line),
                    })
                    .collect();
                let mut said = object.message.clone();
                self.name_alone(side, &mut said);
                object.named(&header, &said)
            }
            None => id.to_string(),
        };
        self.alone[side].insert(id.to_string(), name.clone());
        name
    }
}

/// A piece of a text of a snapshot: an object id, a run of exactly 40 hexadecimal digits, or the
/// text between two.
enum Piece<'a> {
    Id(&'a str),
    Text(&'a str),
}

/// `text` cut into its object ids and the text between them.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let (mut start, mut run) = (0, None);
    let ends = text.char_indices().map(Some).chain([None]);
    for next in ends {
        let at = next.map_or(text.len(), |(i, _)| i);
        match (run, next.is_some_and(|(_, c)| c.is_ascii_hexdigit())) {
            (None, true) => run = Some(at),
            (Some(from), false) => {
                if at - from == 40 {
                    if start < from {
                        pieces.push(Piece::Text(&text[start..from]));
                    }
                    pieces.push(Piece::Id(&text[from..at]));
                    start = at;
                }
                run = None;
            }
            _ => {}
        }
    }
    if start < text.len() {
        pieces.push(Piece::Text(&text[start..]));
    }
    pieces
}

/// A commit or an annotated tag, as `git cat-file` prints it.
struct Made {
    kind: String,
    header: Vec<String>,
    message: String,
}

impl Made {
    /// The field each line of the header starts with.
    fn fields(&self) -> Vec<&str> {
        self.header
            .iter()
            .map(|line| line.split_once(' ').map_or(line.as_str(), |(key, _)| key))
            .collect()
    }

    /// The object as it stands for its content, its header read as `header` and its message as
    /// `message`.
    fn named(&self, header: &[String], message: &str) -> String {
        format!(
            "<{} made apart: {:?}>",
            self.kind,
            format!("{}\n\n{message}", header.join("\n"))
        )
    }
}

/// The commit or annotated tag `id` in `dir`, or `None` where it is another kind of object or
/// missing.
fn made(dir: &Path, id: &str) -> Option<Made> {
    let kind = kinds(dir, &[id]).remove(0);
    if kind != "commit" && kind != "tag" {
        return None;
    }
    let object = raw(dir, &["cat-file", &kind, id]);
    let (header, message) = object.split_once("\n\n").unwrap_or((&object, ""));
    Some(Made {
        header: header.lines().map(str::to_string).collect(),
        message: message.to_string(),
        kind,
    })
}

/// A line of a header, its time left out where it names a person.
fn timeless(line: &str) -> String {
    match line.split_once(' ') {
        Some((key @ ("author" | "committer" | "tagger"), who)) => {
            // The last two words are the time and its zone.
            let who = who.rsplitn(3, ' ').nth(2).unwrap_or(who);
            format!("{key} {who}")
        }
        _ => line.to_string(),
    }
}

/// What `git cat-file --batch-check` says of each of `ids` in `dir`: its type, or `missing`.
fn kinds(dir: &Path, ids: &[&str]) -> Vec<String> {
    if ids.is_empty() {
        return Vec::new();
    }
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

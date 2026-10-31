//! The server root (ADR-0037): the folder the remote server was started with, below which alone a
//! browser opens a repository. A path from the browser is relative to it; the server resolves
//! it, links included, and opens only a repository whose working directory, git directory,
//! common directory, and object stores all lie below the server root, checking it again before
//! every command that reaches it.

use ocelli_git::confine::{Refused, WALK_CAP, below, check_below, still_below};
use serde::Serialize;
use std::path::{Component, Path, PathBuf};

/// A folder of a listing: its name, its path relative to the server root, and whether it holds a
/// `.git` of its own, which marks it as a repository to open; opening decides (ADR-0037).
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Folder {
    pub name: String,
    pub path: String,
    pub repository: bool,
}

/// The folders below one folder: the server's counterpart of the desktop's folder dialog
/// (ADR-0035, ADR-0037).
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct Listing {
    /// The folder listed, relative to the server root, `.` for the root itself.
    pub path: String,
    /// Whether the folder listed is marked as a repository, as its folders are.
    pub repository: bool,
    /// Its folders by name.
    pub folders: Vec<Folder>,
    /// Whether it holds more than a listing reads or lists.
    pub more: bool,
}

/// The most entries of a folder a listing reads, and the most folders it lists.
pub const LIST_READ: usize = 10_000;
pub const LIST_FOLDERS: usize = 1_000;

/// The reason a listing of a path that leads to no folder below the server root is refused with:
/// it names what the browser sent, never where it resolved to.
pub fn no_folder(relative: &str) -> String {
    format!("{relative} is no folder below the server root")
}

/// Whether `dir` holds a `.git` of its own, a folder, a file, or a link: the mark of a repository
/// in a listing, read without following it.
fn marked(dir: &Path) -> bool {
    dir.join(".git").symlink_metadata().is_ok()
}

/// Whether a part of `path` is a `.git`, in any case, as a file system that ignores case reads
/// it: a repository's git directory, which a listing never shows or enters.
fn through_git(path: &Path) -> bool {
    path.components().any(|c| {
        c.as_os_str()
            .to_str()
            .is_some_and(|s| s.eq_ignore_ascii_case(".git"))
    })
}

/// Whether `path` names an absolute path or a drive, which the browser never sends.
fn absolute(path: &Path) -> bool {
    path.components()
        .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
}

pub struct Root {
    /// The server root, resolved once, links included: what every check compares with.
    path: PathBuf,
    /// The most entries of a repository's git directory checked for links when it opens.
    walk_cap: usize,
}

/// A repository a browser may open: where it is, and the path relative to the server root the
/// web interface shows and names it by again.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    pub path: PathBuf,
    pub label: String,
}

/// The one reason every path that leads to no repository below the server root is refused with:
/// it names what the browser sent, never where it resolved to.
pub fn refused(relative: &str) -> String {
    format!("{relative} is no repository below the server root")
}

/// The reason a repository whose git directory holds more than the check walks is refused with.
pub fn too_large(relative: &str) -> String {
    format!(
        "{relative} holds more in its git directory than the server checks, so it is not opened"
    )
}

impl Root {
    /// The server root at `path`, which must be a folder.
    pub fn new(path: &Path) -> Result<Root, String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("The server root {} cannot be read: {e}", path.display()))?;
        if !path.is_dir() {
            return Err(format!("The server root {} is no folder", path.display()));
        }
        Ok(Root {
            path,
            walk_cap: WALK_CAP,
        })
    }

    /// The root walking at most `cap` entries of a repository's git directory when it opens.
    pub fn with_walk_cap(self, cap: usize) -> Root {
        Root {
            walk_cap: cap,
            ..self
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the server root itself is a repository lying below it, which the web interface
    /// opens at its start.
    pub fn is_repository(&self) -> bool {
        check_below(&self.path, &self.path, self.walk_cap).is_ok()
    }

    /// `path`, resolved, relative to the server root with `/` between its parts, `.` for the
    /// root itself; `None` for a path that does not resolve below it.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let path = path.canonicalize().ok()?;
        let below = path.strip_prefix(&self.path).ok()?;
        let label = below
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        Some(if label.is_empty() { ".".into() } else { label })
    }

    /// The repository at `relative`, a path from the browser. An absolute path, `..` leading
    /// out, a link out of the server root, a folder that is not a repository's working directory
    /// itself, and a repository that reaches outside the server root are refused alike
    /// (ADR-0037); one whose git directory holds more than the walk for links checks is refused
    /// as too large. It reads the file system: on the async runtime, call it on the blocking
    /// pool.
    pub fn open(&self, relative: &str) -> Result<Opened, String> {
        let given = Path::new(relative);
        if absolute(given) {
            return Err(refused(relative));
        }
        let path = self
            .path
            .join(given)
            .canonicalize()
            .map_err(|_| refused(relative))?;
        let label = self.relative(&path).ok_or_else(|| refused(relative))?;
        match check_below(&path, &self.path, self.walk_cap) {
            Ok(()) => Ok(Opened { path, label }),
            Err(Refused::TooLarge) => Err(too_large(relative)),
            Err(Refused::Outside) => Err(refused(relative)),
        }
    }

    /// The folders below `relative`, a path from the browser, in place of the desktop's folder
    /// dialog (ADR-0037): resolved as `open` resolves it, refused alike where it leads to no
    /// folder below the server root, and listing no entry that is not a folder below it, so that
    /// a link out of the server root is not followed, nor shown. It reads at most `LIST_READ`
    /// entries and lists at most `LIST_FOLDERS` folders, saying where there are more. It reads
    /// the file system: on the async runtime, call it on the blocking pool.
    pub fn list(&self, relative: &str) -> Result<Listing, String> {
        let given = Path::new(relative);
        if absolute(given) || through_git(given) {
            return Err(no_folder(relative));
        }
        let dir = self
            .path
            .join(given)
            .canonicalize()
            .map_err(|_| no_folder(relative))?;
        let label = self
            .relative(&dir)
            .filter(|_| dir.is_dir() && !self.in_git(&dir))
            .ok_or_else(|| no_folder(relative))?;
        let entries = std::fs::read_dir(&dir).map_err(|_| no_folder(relative))?;
        let (mut folders, mut more) = (Vec::new(), false);
        for (read, entry) in entries.enumerate() {
            if read == LIST_READ || folders.len() == LIST_FOLDERS {
                more = true;
                break;
            }
            let Ok(entry) = entry else { continue };
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if through_git(Path::new(&name)) {
                continue;
            }
            let Ok(target) = entry.path().canonicalize() else {
                continue;
            };
            if !below(&target, &self.path) || !target.is_dir() || self.in_git(&target) {
                continue;
            }
            let path = if label == "." {
                name.clone()
            } else {
                format!("{label}/{name}")
            };
            folders.push(Folder {
                repository: marked(&target),
                name,
                path,
            });
        }
        folders.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Listing {
            path: label,
            repository: marked(&dir),
            folders,
            more,
        })
    }

    /// Whether `resolved`, a resolved path below the server root, lies in a git directory named
    /// `.git`, which a listing never shows or enters.
    fn in_git(&self, resolved: &Path) -> bool {
        resolved.strip_prefix(&self.path).is_ok_and(through_git)
    }

    /// The check made again before every command that reaches an open repository: what changed
    /// on disk since it was opened may lead out of the server root now.
    pub fn verify(&self, path: &Path) -> Result<(), String> {
        if still_below(path, &self.path) {
            return Ok(());
        }
        Err(refused(
            &self
                .relative(path)
                .unwrap_or_else(|| "The repository".into()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch tree: `root/` with a repository `root/app`, a folder `root/plain`, and a
    /// repository `outside/` beside the root.
    struct Tree(PathBuf);

    impl Tree {
        fn new(name: &str) -> Tree {
            let dir =
                std::env::temp_dir().join(format!("ocelli-root-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("root/plain")).unwrap();
            for repo in ["root/app", "outside"] {
                git2::Repository::init(dir.join(repo)).unwrap();
            }
            Tree(dir)
        }

        fn root(&self) -> Root {
            Root::new(&self.0.join("root")).unwrap()
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    // Verifies: ADR-0037
    fn a_repository_below_the_root_opens_named_relative_to_it() {
        let tree = Tree::new("below");
        let root = tree.root();
        let opened = root.open("app").unwrap();
        assert_eq!(opened.label, "app");
        assert_eq!(opened.path, root.path().join("app"));
        assert_eq!(root.open("plain/../app").unwrap().label, "app");
        assert_eq!(root.verify(&opened.path), Ok(()));
        assert!(!root.is_repository());
    }

    #[test]
    // Verifies: ADR-0037
    fn a_path_leading_out_of_the_root_is_refused_naming_nothing_outside() {
        let tree = Tree::new("out");
        let root = tree.root();
        let outside = tree.0.join("outside");
        assert!(outside.join(".git").exists(), "the target is a repository");
        for path in [
            "../outside".to_string(),
            "app/../../outside".to_string(),
            outside.to_string_lossy().into_owned(),
            "..".to_string(),
        ] {
            assert_eq!(root.open(&path), Err(refused(&path)));
        }
        assert_eq!(root.verify(&outside), Err(refused("The repository")));
    }

    #[cfg(unix)]
    #[test]
    // Verifies: ADR-0037
    fn a_link_out_of_the_root_is_refused() {
        let tree = Tree::new("link");
        std::os::unix::fs::symlink(tree.0.join("outside"), tree.0.join("root/link")).unwrap();
        let root = tree.root();
        assert_eq!(root.open("link"), Err(refused("link")));
    }

    #[test]
    // Verifies: ADR-0037
    fn a_folder_inside_a_repository_or_none_opens_nothing() {
        let tree = Tree::new("plain");
        let root = tree.root();
        std::fs::create_dir_all(tree.0.join("root/app/src")).unwrap();
        for path in ["plain", "app/src", "app/.git", "missing"] {
            assert_eq!(root.open(path), Err(refused(path)), "{path}");
        }
    }

    #[test]
    // Verifies: ADR-0037
    fn the_listing_names_the_folders_below_marks_repositories_and_follows_no_link_out() {
        let tree = Tree::new("list");
        let root = tree.root();
        std::fs::write(tree.0.join("root/file"), "").unwrap();
        std::fs::create_dir_all(tree.0.join("root/plain/deep")).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(tree.0.join("outside"), tree.0.join("root/out")).unwrap();
            std::os::unix::fs::symlink(tree.0.join("root/app"), tree.0.join("root/plain/in"))
                .unwrap();
        }
        let listing = root.list(".").unwrap();
        assert_eq!(listing.path, ".");
        assert!(!listing.repository);
        assert!(!listing.more);
        let folder = |name: &str, path: &str, repository| Folder {
            name: name.into(),
            path: path.into(),
            repository,
        };
        assert_eq!(
            listing.folders,
            [folder("app", "app", true), folder("plain", "plain", false)]
        );
        let mut inside = vec![folder("deep", "plain/deep", false)];
        if cfg!(unix) {
            // A link to a folder below the server root is a folder like any other.
            inside.push(folder("in", "plain/in", true));
        }
        assert_eq!(root.list("plain/").unwrap().folders, inside);
        let app = root.list("app").unwrap();
        assert!(app.repository && app.folders.is_empty(), "{app:?}");
        let outside = tree.0.join("outside").to_string_lossy().into_owned();
        let mut refused = vec![
            "..",
            "../outside",
            "plain/../..",
            "file",
            "missing",
            &outside,
        ];
        if cfg!(unix) {
            refused.push("out");
        }
        for path in refused {
            assert_eq!(root.list(path), Err(no_folder(path)), "{path}");
        }
    }

    #[test]
    // Verifies: ADR-0037
    fn a_listing_never_shows_or_enters_a_git_directory_in_any_case_or_through_a_link() {
        let tree = Tree::new("list-git");
        let root = tree.root();
        assert!(tree.0.join("root/app/.git/refs/heads").is_dir());
        std::fs::create_dir_all(tree.0.join("root/plain/.GIT/inner")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(tree.0.join("root/app/.git"), tree.0.join("root/plain/gd"))
            .unwrap();
        // What could be listed is: the folders beside them.
        assert_eq!(root.list("plain").unwrap().folders, []);
        assert_eq!(root.list("app").unwrap().folders, []);
        let mut refused = vec![
            "app/.git",
            "app/.git/refs/heads",
            "app/./.git",
            "app/.GIT",
            "./app/.Git/refs",
            "plain/.GIT",
            "plain/.GIT/inner",
        ];
        if cfg!(unix) {
            refused.extend(["plain/gd", "plain/gd/refs"]);
        }
        for path in refused {
            assert_eq!(root.list(path), Err(no_folder(path)), "{path}");
        }
    }

    #[test]
    // Verifies: ADR-0037
    fn a_listing_stops_at_its_bound_and_says_there_is_more() {
        let tree = Tree::new("list-bound");
        for n in 0..=LIST_FOLDERS {
            std::fs::create_dir(tree.0.join(format!("root/plain/{n:04}"))).unwrap();
        }
        let listing = tree.root().list("plain").unwrap();
        assert_eq!(listing.folders.len(), LIST_FOLDERS);
        assert!(listing.more);
        std::fs::remove_dir(tree.0.join("root/plain/0000")).unwrap();
        let listing = tree.root().list("plain").unwrap();
        assert_eq!(listing.folders.len(), LIST_FOLDERS);
        assert!(!listing.more);
    }

    #[test]
    // Verifies: ADR-0037
    fn the_root_itself_may_be_the_repository() {
        let tree = Tree::new("itself");
        let root = Root::new(&tree.0.join("root/app")).unwrap();
        assert!(root.is_repository());
        assert_eq!(root.open(".").unwrap().label, ".");
        assert_eq!(root.open("").unwrap().label, ".");
    }
}

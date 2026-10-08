//! The server root (ADR-0039): the folder the remote server was started with, below which alone a
//! browser opens a repository. A path from the browser is relative to it; the server resolves
//! it, links included, and refuses whatever does not lie at or below the server root.

use std::path::{Component, Path, PathBuf};

pub struct Root {
    /// The server root, resolved once, links included: what every check compares with.
    path: PathBuf,
}

/// A repository a browser may open: where it is, and the path relative to the server root the
/// web interface shows and names it by again.
#[derive(Debug, PartialEq, Eq)]
pub struct Opened {
    pub path: PathBuf,
    pub label: String,
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
        Ok(Root { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the server root itself is a repository's working directory, which the web
    /// interface opens at its start.
    pub fn is_repository(&self) -> bool {
        self.path.join(".git").exists()
    }

    /// The repository at `relative`, a path from the browser. An absolute path, `..` leading
    /// out, a link out of the server root, and a folder that is not a repository's working
    /// directory are refused alike, with one reason that names no path outside (ADR-0039). The
    /// folder must hold the repository's `.git` itself, so that the git adapter's search for it
    /// stops there and never opens a repository that encloses the server root.
    pub fn open(&self, relative: &str) -> Result<Opened, String> {
        let refused = || format!("{relative} is no repository below the server root");
        let given = Path::new(relative);
        if given
            .components()
            .any(|c| matches!(c, Component::Prefix(_) | Component::RootDir))
        {
            return Err(refused());
        }
        let path = self
            .path
            .join(given)
            .canonicalize()
            .map_err(|_| refused())?;
        let Ok(below) = path.strip_prefix(&self.path) else {
            return Err(refused());
        };
        if !path.join(".git").exists() {
            return Err(refused());
        }
        let label = below
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        Ok(Opened {
            label: if label.is_empty() { ".".into() } else { label },
            path,
        })
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
            for folder in ["root/app/.git", "root/plain", "outside/.git"] {
                std::fs::create_dir_all(dir.join(folder)).unwrap();
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
    // Verifies: ADR-0039
    fn a_repository_below_the_root_opens_named_relative_to_it() {
        let tree = Tree::new("below");
        let root = tree.root();
        let opened = root.open("app").unwrap();
        assert_eq!(opened.label, "app");
        assert_eq!(opened.path, root.path().join("app"));
        assert_eq!(root.open("plain/../app").unwrap().label, "app");
        assert!(!root.is_repository());
    }

    #[test]
    // Verifies: ADR-0039
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
            // The reason names what the browser sent, never where it resolved to.
            assert_eq!(
                root.open(&path).unwrap_err(),
                format!("{path} is no repository below the server root")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    // Verifies: ADR-0039
    fn a_link_out_of_the_root_is_refused() {
        let tree = Tree::new("link");
        std::os::unix::fs::symlink(tree.0.join("outside"), tree.0.join("root/link")).unwrap();
        let root = tree.root();
        assert_eq!(
            root.open("link").unwrap_err(),
            "link is no repository below the server root"
        );
    }

    #[test]
    // Verifies: ADR-0039
    fn a_folder_inside_a_repository_or_none_opens_nothing() {
        let tree = Tree::new("plain");
        let root = tree.root();
        std::fs::create_dir_all(tree.0.join("root/app/src")).unwrap();
        for path in ["plain", "app/src", "missing"] {
            assert!(root.open(path).is_err(), "{path}");
        }
    }

    #[test]
    // Verifies: ADR-0039
    fn the_root_itself_may_be_the_repository() {
        let tree = Tree::new("itself");
        let root = Root::new(&tree.0.join("root/app")).unwrap();
        assert!(root.is_repository());
        assert_eq!(root.open(".").unwrap().label, ".");
        assert_eq!(root.open("").unwrap().label, ".");
    }
}

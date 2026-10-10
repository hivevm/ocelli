//! Verifies: ADR-0033, ADR-0037
//!
//! git2 panics in places, and a release build aborts on a panic, every tab of the remote server
//! with it. Each place of the locked git2's source outside its tests that panics by itself, an
//! `unwrap`, an `expect`, their `_err` forms, `unwrap_unchecked`, or a `panic!`, `unreachable!`,
//! `unimplemented!`, `todo!`, or `assert!` of any kind but a debug one, is audited in
//! `git2_unwraps.txt`: one a string reaches has its method disallowed in the git adapter's
//! `clippy.toml`, used only where its `expect` says why the string is safe, or the audit says why
//! no such string reaches it. A site is keyed by its file, the function it stands in, and its
//! statement as written up to the panicking call, so that moving code does not change the key
//! while changing what it unwraps does: an upgrade of git2 or of libgit2-sys, whose libgit2 the
//! verdicts rest on, fails here, naming each site that changed, until it is audited again.

#![allow(
    clippy::disallowed_methods,
    reason = "a test calls git2's panicking methods to show what the wrappers spare"
)]

mod support;
use ocelli_core::port::{RepositoryReader as _, RepositoryWriter as _, Write};
use ocelli_git::{GitReader, GitWriter};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use support::{Scratch, git, write_file};

/// The audit, beside this test.
const AUDIT: &str = "tests/git2_unwraps.txt";

/// One site: the file below `src/`, the function it stands in, and its statement as written from
/// the `;`, `{`, or `}` before it up to the end of the panicking call, its whitespace collapsed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Site {
    file: String,
    function: String,
    code: String,
}

impl std::fmt::Display for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.file, self.function)
    }
}

/// `source` with every comment, and where `literals` says so every string or character literal,
/// blanked to spaces, byte for byte, line breaks kept, so that a position in it is the same
/// position in `source`.
fn blank(source: &str, literals: bool) -> Vec<u8> {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    let mut i = 0;
    let clear = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    while i < b.len() {
        let rest = &b[i..];
        if rest.starts_with(b"//") {
            let end = rest
                .iter()
                .position(|&c| c == b'\n')
                .map_or(b.len(), |n| i + n);
            clear(&mut out, i, end);
            i = end;
        } else if rest.starts_with(b"/*") {
            let (mut depth, mut j) = (0, i);
            while j < b.len() {
                if b[j..].starts_with(b"/*") {
                    depth += 1;
                    j += 2;
                } else if b[j..].starts_with(b"*/") {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            clear(&mut out, i, j);
            i = j;
        } else if let Some(hashes) = raw_string_at(b, i) {
            let open = i + rest.iter().position(|&c| c == b'"').unwrap() + 1;
            let close: Vec<u8> = std::iter::once(b'"')
                .chain(std::iter::repeat_n(b'#', hashes))
                .collect();
            let end = b[open..]
                .windows(close.len())
                .position(|w| w == close.as_slice())
                .map_or(b.len(), |n| open + n + close.len());
            if literals {
                clear(&mut out, i, end);
            }
            i = end;
        } else if rest[0] == b'"' || rest.starts_with(b"b\"") && !ident_byte(b, i) {
            let mut j = i + rest.iter().position(|&c| c == b'"').unwrap() + 1;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            if literals {
                clear(&mut out, i, (j + 1).min(b.len()));
            }
            i = j + 1;
        } else if rest[0] == b'\'' {
            // A character literal, or a lifetime, which stays.
            let after = &source[i + 1..];
            let mut chars = after.chars();
            let end = match (chars.next(), chars.next()) {
                (Some('\\'), _) => after
                    .find('\'')
                    .filter(|&n| n > 1)
                    .or_else(|| after[2..].find('\'').map(|n| n + 2)),
                (Some(c), Some('\'')) => Some(c.len_utf8()),
                _ => None,
            };
            match end {
                Some(n) => {
                    if literals {
                        clear(&mut out, i, i + n + 2);
                    }
                    i += n + 2;
                }
                None => i += 1,
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The number of `#`s of a raw string literal starting at `i`, if one does.
fn raw_string_at(b: &[u8], i: usize) -> Option<usize> {
    if ident_byte(b, i) {
        return None;
    }
    let mut j = i;
    if b.get(j) == Some(&b'b') {
        j += 1;
    }
    if b.get(j) != Some(&b'r') {
        return None;
    }
    j += 1;
    let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
    (b.get(j + hashes) == Some(&b'"')).then_some(hashes)
}

/// Whether the byte before `i` continues an identifier, so that `b"` or `r"` at `i` is none.
fn ident_byte(b: &[u8], i: usize) -> bool {
    i > 0 && (b[i - 1].is_ascii_alphanumeric() || b[i - 1] == b'_')
}

/// The tokens of blanked code: identifiers, lifetimes, and single punctuation bytes, each with
/// its position.
fn tokens(code: &[u8]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut i = 0;
    while i < code.len() {
        let c = code[i];
        if c.is_ascii_whitespace() {
            i += 1;
        } else if c.is_ascii_alphanumeric() || c == b'_' || c == b'\'' || c >= 0x80 {
            let start = i;
            i += 1;
            while i < code.len()
                && (code[i].is_ascii_alphanumeric() || code[i] == b'_' || code[i] >= 0x80)
            {
                i += 1;
            }
            found.push((start, String::from_utf8_lossy(&code[start..i]).into_owned()));
        } else {
            found.push((i, (c as char).to_string()));
            i += 1;
        }
    }
    found
}

/// The type an `impl` header names, `impl<T> Trait<T> for Type<T> where …` naming `Type`.
fn impl_name(header: &[String]) -> String {
    let header = match header.iter().position(|t| t == "for") {
        Some(n) => &header[n + 1..],
        None => header,
    };
    let mut angle = 0;
    for t in header.iter().take_while(|t| *t != "where") {
        match t.as_str() {
            "<" => angle += 1,
            ">" => angle -= 1,
            _ if angle == 0
                && t.chars()
                    .next()
                    .is_some_and(|c| c.is_alphabetic() || c == '_')
                && !["dyn", "mut", "const", "unsafe"].contains(&t.as_str()) =>
            {
                return t.clone();
            }
            _ => {}
        }
    }
    "?".into()
}

/// The methods that panic on what they are called on.
const PANICKING_METHODS: &[&str] = &[
    "unwrap",
    "expect",
    "unwrap_err",
    "expect_err",
    "unwrap_unchecked",
];

/// The macros that panic, a debug assertion aside, which a release build leaves out.
const PANICKING_MACROS: &[&str] = &[
    "panic",
    "unreachable",
    "unimplemented",
    "todo",
    "assert",
    "assert_eq",
    "assert_ne",
];

/// The index of the token that closes the bracket opened at token `open`.
fn closing(tokens: &[(usize, String)], open: usize) -> usize {
    let mut depth = 0;
    for (n, (_, t)) in tokens.iter().enumerate().skip(open) {
        match t.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => {
                depth -= 1;
                if depth == 0 {
                    return n;
                }
            }
            _ => {}
        }
    }
    tokens.len() - 1
}

/// The sites of one file's `source`, and the modules it declares under `#[cfg(test)]`, whose
/// files are tests too.
fn sites_of(file: &str, source: &str) -> (Vec<Site>, Vec<String>) {
    let code = blank(source, true);
    let written = blank(source, false);
    let tokens = tokens(&code);
    let text = |n: usize| tokens.get(n).map_or("", |(_, t)| t.as_str());
    // The statement of the site whose panicking call spans the tokens `from` to `to`.
    let statement_of = |from: usize, to: usize| {
        let start = (0..from)
            .rev()
            .find(|&m| matches!(text(m), ";" | "{" | "}"))
            .map_or(0, |m| tokens[m].0 + 1);
        let end = tokens[to].0 + tokens[to].1.len();
        String::from_utf8_lossy(&written[start..end])
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (mut sites, mut test_modules) = (Vec::new(), Vec::new());
    // The scopes open, each with the depth its brace opened at; the name the next brace opens.
    let mut scopes: Vec<(usize, String)> = Vec::new();
    let mut pending: Option<String> = None;
    let mut depth = 0;
    let mut n = 0;
    while n < tokens.len() {
        match text(n) {
            "#" if text(n + 1) == "[" => {
                let close = (n..tokens.len())
                    .find(|&m| text(m) == "]")
                    .unwrap_or(tokens.len());
                let attribute: String = (n + 2..close).map(text).collect();
                n = close + 1;
                if attribute == "cfg(test)" || attribute == "test" {
                    // Skip the item, to its `;` or past its braces.
                    let mut inner = 0;
                    let start = n;
                    while n < tokens.len() {
                        match text(n) {
                            ";" if inner == 0 => break,
                            "{" => inner += 1,
                            "}" => {
                                inner -= 1;
                                if inner == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        n += 1;
                    }
                    let item: Vec<&str> = (start..n).map(text).collect();
                    assert!(
                        !item.windows(3).any(|w| w == ["#", "[", "path"]),
                        "{file}: a test module at a #[path] of its own, which the scan does not follow"
                    );
                    if let [.., "mod", name] = item.as_slice() {
                        test_modules.push(name.to_string());
                    }
                    n += 1;
                }
                continue;
            }
            "impl" | "trait" if pending.is_none() => {
                let open = (n..tokens.len())
                    .find(|&m| matches!(text(m), "{" | ";"))
                    .unwrap_or(tokens.len());
                let header: Vec<String> = (n + 1..open).map(|m| text(m).to_string()).collect();
                pending = Some(if text(n) == "impl" {
                    impl_name(&header)
                } else {
                    header.first().cloned().unwrap_or_default()
                });
            }
            "fn" if text(n + 1) != "(" && !text(n + 1).is_empty() => {
                pending = Some(text(n + 1).to_string());
            }
            ";" => pending = None,
            "{" => {
                depth += 1;
                if let Some(name) = pending.take() {
                    scopes.push((depth, name));
                }
            }
            "}" => {
                if scopes.last().is_some_and(|(d, _)| *d == depth) {
                    scopes.pop();
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        // A method called on a value, `x.unwrap()`, or named by its path, `Option::unwrap`.
        let method = (text(n) == "." || text(n) == ":" && text(n + 1) == ":")
            .then(|| n + if text(n) == "." { 1 } else { 2 })
            .filter(|&m| PANICKING_METHODS.contains(&text(m)));
        let called = method.map(|m| {
            if text(m + 1) == "(" {
                closing(&tokens, m + 1)
            } else {
                m
            }
        });
        // A macro, `panic!(…)`, not a method of the same name.
        let invoked = (PANICKING_MACROS.contains(&text(n))
            && text(n + 1) == "!"
            && matches!(text(n + 2), "(" | "[" | "{")
            && !matches!(text(n.wrapping_sub(1)), "." | ":"))
        .then(|| closing(&tokens, n + 2));
        if let Some(end) = called.or(invoked) {
            let function: Vec<&str> = scopes.iter().map(|(_, s)| s.as_str()).collect();
            sites.push(Site {
                file: file.to_string(),
                function: if function.is_empty() {
                    "-".into()
                } else {
                    function.join("::")
                },
                code: statement_of(n, end),
            });
        }
        n += 1;
    }
    (sites, test_modules)
}

/// The version the workspace's `Cargo.lock` holds of the package `name`, which it must hold once,
/// from the crates.io registry.
fn locked_version(name: &str) -> String {
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    let text = std::fs::read_to_string(&lock).expect("the workspace's Cargo.lock is readable");
    let found: Vec<String> = text
        .split("[[package]]")
        .filter(|block| block.lines().any(|l| l == format!("name = \"{name}\"")))
        .filter(|block| block.contains("source = \"registry+"))
        .filter_map(|block| {
            block.lines().find_map(|l| {
                l.strip_prefix("version = \"")
                    .and_then(|v| v.strip_suffix('"'))
                    .map(str::to_string)
            })
        })
        .collect();
    let [version] = found.as_slice() else {
        panic!("Cargo.lock holds one {name} from the registry, not {found:?}");
    };
    version.clone()
}

/// The source directory cargo unpacked the locked `name` at `version` into: below the registry
/// of `CARGO_HOME`, or of `.cargo` in the home directory where that is unset. Building this test
/// unpacks it, git2 being a dependency of the crate, so no other package needs to be fetched.
fn unpacked(name: &str, version: &str) -> PathBuf {
    // As cargo finds its home: CARGO_HOME, else `.cargo` in the user's home directory, which on
    // Windows is USERPROFILE, whatever HOME says.
    let user_home = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os(user_home)
                .unwrap_or_else(|| panic!("CARGO_HOME or {user_home} names cargo's home"));
            PathBuf::from(home).join(".cargo")
        });
    let registries = home.join("registry").join("src");
    // Each registry cargo used holds the same published source; any one of them is read.
    let mut found: Vec<PathBuf> = std::fs::read_dir(&registries)
        .unwrap_or_else(|e| panic!("{} is not readable: {e}", registries.display()))
        .filter_map(|entry| Some(entry.ok()?.path().join(format!("{name}-{version}"))))
        .filter(|dir| dir.is_dir())
        .collect();
    found.sort();
    found.into_iter().next().unwrap_or_else(|| {
        panic!(
            "{name} {version} is not unpacked below {}: build the git adapter's tests, which \
             unpacks it, with CARGO_HOME naming the cargo home they were built with",
            registries.display()
        )
    })
}

/// Every unwrap site of the git2 source below `src`, outside its tests.
fn sites_below(src: &Path) -> Vec<Site> {
    let mut files = Vec::new();
    let mut dirs = vec![src.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).expect("git2's source is readable") {
            let path = entry.expect("an entry").path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                files.push(path);
            }
        }
    }
    let names: Vec<String> = files
        .iter()
        .map(|path| {
            path.strip_prefix(src)
                .expect("below src")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    // A test module's file, and every file below its directory, holds tests only.
    let (mut sites, mut tests) = (Vec::new(), Vec::new());
    for (path, name) in files.iter().zip(&names) {
        let source = std::fs::read_to_string(path).expect("a source file is readable");
        let (found, modules) = sites_of(name, &source);
        sites.push(found);
        for module in modules {
            let base = module_base(name);
            let file = format!("{base}{module}.rs");
            let dir = format!("{base}{module}/");
            assert!(
                names.contains(&file) || names.contains(&format!("{dir}mod.rs")),
                "{name}: the test module {module} is at neither {file} nor {dir}mod.rs"
            );
            tests.push((file, dir));
        }
    }
    let mut sites: Vec<Site> = sites
        .into_iter()
        .flatten()
        .filter(|s| {
            !tests
                .iter()
                .any(|(f, d)| s.file == *f || s.file.starts_with(d))
        })
        .collect();
    sites.sort();
    sites
}

/// The directory, below `src/` and with a trailing `/` unless it is `src/` itself, in which the
/// file `name` finds a module it declares: its own for `lib.rs`, `main.rs`, and `mod.rs`, one
/// named after it for any other.
fn module_base(name: &str) -> String {
    let (dir, file) = name.rsplit_once('/').map_or(("", name), |(d, f)| (d, f));
    let dir = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    match file.strip_suffix(".rs") {
        Some("lib" | "main" | "mod") => dir,
        Some(stem) => format!("{dir}{stem}/"),
        None => panic!("{name} is no Rust file"),
    }
}

/// The audit: the versions of git2 and of libgit2-sys it was made for, and each site with its
/// verdict.
fn audit() -> ([String; 2], Vec<(Site, String)>) {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(AUDIT))
        .expect("the audit is readable");
    let (mut git2, mut sys) = (None, None);
    let mut sites = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
    {
        if let Some(v) = line.strip_prefix("git2 ") {
            git2 = Some(v.trim().to_string());
            continue;
        }
        if let Some(v) = line.strip_prefix("libgit2-sys ") {
            sys = Some(v.trim().to_string());
            continue;
        }
        let mut parts = line.splitn(3, " | ");
        let (Some(at), Some(verdict), Some(code)) = (parts.next(), parts.next(), parts.next())
        else {
            panic!("an audit line reads `file function | verdict | code`: {line}");
        };
        let (file, function) = at.split_once(' ').expect("`file function`");
        sites.push((
            Site {
                file: file.into(),
                function: function.into(),
                code: code.into(),
            },
            verdict.to_string(),
        ));
    }
    (
        [
            git2.expect("the audit names the git2 it audits"),
            sys.expect("the audit names the libgit2-sys its verdicts rest on"),
        ],
        sites,
    )
}

/// The methods the git adapter's `clippy.toml` disallows.
fn disallowed() -> Vec<String> {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("clippy.toml"))
        .expect("clippy.toml is readable");
    text.split("path = \"")
        .skip(1)
        .filter_map(|rest| rest.split_once('"').map(|(path, _)| path.to_string()))
        .collect()
}

/// What is wrong with `verdict`, if anything: it names the methods `clippy.toml` disallows that
/// reach the site, or says why the site holds or why nothing Ocelli does reaches it.
fn verdict_fault(verdict: &str, disallowed: &[String]) -> Option<String> {
    let (kind, why) = verdict.split_once(": ").unwrap_or((verdict, ""));
    let missing: Vec<&str> = why
        .split(", ")
        .map(str::trim)
        .filter(|path| !disallowed.iter().any(|d| d == path))
        .collect();
    match kind {
        "internal" | "unreachable" if !why.trim().is_empty() => None,
        "disallowed" if missing.is_empty() => None,
        "disallowed" => Some(format!(
            "{} not disallowed in clippy.toml",
            missing.join(", ")
        )),
        _ => Some(format!(
            "reads `internal: why`, `disallowed: git2::path, …`, or `unreachable: why`, not \
             `{verdict}`"
        )),
    }
}

#[test]
fn every_unwrap_of_the_locked_git2_is_audited() {
    let version = locked_version("git2");
    let sys_version = locked_version("libgit2-sys");
    let src = unpacked("git2", &version).join("src");
    let ([audited_version, audited_sys], audit) = audit();
    let found = sites_below(&src);
    let mut count: BTreeMap<&Site, i32> = BTreeMap::new();
    for site in &found {
        *count.entry(site).or_default() += 1;
    }
    for (site, _) in &audit {
        *count.entry(site).or_default() -= 1;
    }
    let mut faults = Vec::new();
    for (site, n) in &count {
        if *n > 0 {
            faults.push(format!("not audited ({n}x): {site} | ? | {}", site.code));
        } else if *n < 0 {
            faults.push(format!(
                "audited but gone ({}x): {site} | {}",
                -n, site.code
            ));
        }
    }
    let disallowed = disallowed();
    for (site, verdict) in &audit {
        if let Some(fault) = verdict_fault(verdict, &disallowed) {
            faults.push(format!("{site}: {fault}"));
        }
    }
    if audited_version != version {
        faults.push(format!(
            "the audit is of git2 {audited_version}, the lock file holds {version}: audit the \
             sites again and name the version"
        ));
    }
    if audited_sys != sys_version {
        faults.push(format!(
            "the verdicts rest on libgit2-sys {audited_sys}, the lock file holds {sys_version}: \
             check each verdict against its libgit2 and name the version"
        ));
    }
    assert!(
        faults.is_empty(),
        "git2's unwrap sites in {} differ from {AUDIT}:\n{}",
        src.display(),
        faults.join("\n")
    );
}

#[test]
fn the_scan_keys_a_site_by_its_function_and_skips_comments_strings_and_tests() {
    let source = r#####"
//! `x.unwrap()` in a comment
impl<'a> Builder<'a> {
    /// let r = repo.unwrap();
    pub fn label(&mut self, label: &str) -> &mut Self {
        let s = "a.unwrap() } {";
        let c = '}';
        let r = r#"x.expect("y") }"#;
        self.label = Some(CString::new(label).unwrap());
        self
    }
}
impl<T: Into<u8>> From<T> for Thing {
    fn from(t: T) -> Thing {
        let f = |x: Option<u8>| x
            .expect("one");
        Thing(f(Some(t.into())))
    }
}
fn free() -> impl Iterator<Item = u8> { None.into_iter().map(|x: u8| x).take(1).map(|x| x) }
fn more(r: Result<u8, u8>) {
    let all = [Some(1)].into_iter().map(Option::unwrap);
    r.unwrap_err();
    debug_assert!(true);
    assert_eq!(1, 1, "{}", 2);
    match r { Ok(_) => unreachable!("never"), Err(_) => panic!() }
    x.self_unwrap();
}
#[test]
fn t2() { y.unwrap(); }
#[cfg(test)]
mod tests {
    fn t() { None::<u8>.unwrap(); }
}
#[cfg(test)]
mod test;
"#####;
    let (sites, tests) = sites_of("build.rs", source);
    let keys: Vec<(String, String)> = sites.into_iter().map(|s| (s.function, s.code)).collect();
    let keys: Vec<(&str, &str)> = keys.iter().map(|(f, c)| (f.as_str(), c.as_str())).collect();
    assert_eq!(
        keys,
        [
            (
                "Builder::label",
                "self.label = Some(CString::new(label).unwrap()"
            ),
            ("Thing::from", "let f = |x: Option<u8>| x .expect(\"one\")"),
            ("more", "let all = [Some(1)].into_iter().map(Option::unwrap"),
            ("more", "r.unwrap_err()"),
            ("more", "assert_eq!(1, 1, \"{}\", 2)"),
            ("more", "Ok(_) => unreachable!(\"never\")"),
            (
                "more",
                "Ok(_) => unreachable!(\"never\"), Err(_) => panic!()"
            ),
        ]
    );
    assert_eq!(tests, ["test"]);
}

#[test]
fn a_test_module_is_looked_for_where_rust_looks_for_it() {
    assert_eq!(module_base("lib.rs"), "");
    assert_eq!(module_base("build.rs"), "build/");
    assert_eq!(module_base("sys/mod.rs"), "sys/");
    assert_eq!(module_base("sys/unix.rs"), "sys/unix/");
}

#[test]
fn a_verdict_names_a_disallowed_method_or_says_why() {
    let disallowed = vec![
        "git2::Remote::is_valid_name".to_string(),
        "git2::Tag::is_valid_name".to_string(),
    ];
    assert_eq!(
        verdict_fault(
            "disallowed: git2::Remote::is_valid_name, git2::Tag::is_valid_name",
            &disallowed
        ),
        None
    );
    assert!(
        verdict_fault(
            "disallowed: git2::Tag::is_valid_name, git2::X::y",
            &disallowed
        )
        .is_some()
    );
    assert_eq!(
        verdict_fault("disallowed: git2::Remote::is_valid_name", &disallowed),
        None
    );
    assert_eq!(
        verdict_fault("internal: a name libgit2 read", &disallowed),
        None
    );
    assert_eq!(
        verdict_fault("unreachable: Ocelli builds no proxy", &disallowed),
        None
    );
    assert!(verdict_fault("foreign: a remote's status", &disallowed).is_some());
    assert!(verdict_fault("disallowed: git2::Refspec::dst", &disallowed).is_some());
    assert!(verdict_fault("unreachable:", &disallowed).is_some());
    assert!(verdict_fault("?", &disallowed).is_some());
}

/// A repository with a commit and a stash, the stash's message rewritten to bytes that are not
/// UTF-8, as a repository may hold them.
fn stash_not_utf8(dir: &Path) {
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["config", "user.name", "Ada"]);
    git(dir, &["config", "user.email", "ada@example.invalid"]);
    write_file(dir, "f", "1\n");
    git(dir, &["add", "f"]);
    git(dir, &["commit", "--quiet", "-m", "one"]);
    write_file(dir, "f", "stashed\n");
    git(dir, &["stash", "push", "--quiet"]);
    let log = dir.join(".git/logs/refs/stash");
    let text = std::fs::read(&log).unwrap();
    let tab = text.iter().position(|&c| c == b'\t').unwrap();
    let mut bytes = text[..=tab].to_vec();
    bytes.extend_from_slice(b"WIP \xff\xfe\n");
    std::fs::write(&log, bytes).unwrap();
}

#[test]
// Verifies: ADR-0033, ADR-0037
fn a_stash_message_that_is_not_utf8_is_read_and_dropped_without_a_panic() {
    let scratch = Scratch::new("stash");
    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    stash_not_utf8(&dir);
    let stash = git(&dir, &["rev-parse", "refs/stash"]).trim().to_string();

    // git2's own iteration panics on it, inside a callback whose panic git2 catches where
    // unwinding, and a release build aborts on.
    let ran = panics_in(|| {
        let mut repo = git2::Repository::open(&dir).unwrap();
        let _ = repo.stash_foreach(|_, _, _| true);
    });
    assert!(ran.is_none(), "git2 took the message after all");

    let commits =
        panics_in(|| GitReader.load_first_chunk(&dir, &mut |_| {}).unwrap()).expect("no panic");
    let shown = commits
        .iter()
        .find(|c| c.sha == stash)
        .expect("the stash is shown");
    assert!(shown.message.starts_with("WIP "), "{:?}", shown.message);

    let drop = Write::DropStash {
        index: 0,
        commit: stash,
    };
    panics_in(|| GitWriter.write(&dir, &drop).unwrap()).expect("no panic");
    assert!(!git(&dir, &["stash", "list"]).contains("WIP"));
}

/// What `f` returns, none where it panicked, in a callback whose panic git2 caught too. A panic
/// is recorded by its thread, the test's own.
fn panics_in<T>(f: impl FnOnce() -> T) -> Option<T> {
    static PANICKED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    static HOOK: std::sync::Once = std::sync::Once::new();
    let me = || format!("{:?}", std::thread::current().id());
    HOOK.call_once(|| {
        let earlier = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = format!("{:?}", std::thread::current().id());
            PANICKED
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(thread);
            earlier(info);
        }));
    });
    let mine = |panicked: &mut Vec<String>| {
        let before = panicked.len();
        panicked.retain(|t| *t != me());
        before != panicked.len()
    };
    mine(&mut PANICKED.lock().unwrap());
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    match (result, mine(&mut PANICKED.lock().unwrap())) {
        (Ok(value), false) => Some(value),
        _ => None,
    }
}

#[test]
// Verifies: ADR-0033, ADR-0037
fn a_configuration_key_without_a_value_is_refused_as_git_refuses_it_without_a_panic() {
    let scratch = Scratch::new("valueless");
    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "--quiet", "-b", "main"]);
    git(&dir, &["config", "user.name", "Ada"]);
    git(&dir, &["config", "user.email", "ada@example.invalid"]);
    git(&dir, &["config", "commit.cleanup", "strip"]);
    write_file(&dir, "f", "1\n");
    git(&dir, &["add", "f"]);
    git(&dir, &["commit", "--quiet", "-m", "one"]);
    write_file(&dir, "f", "2\n");
    git(&dir, &["add", "f"]);
    let head = git(&dir, &["rev-parse", "HEAD"]);
    // Written last, and taken out before `git` runs again: `git` refuses a comment character
    // without a value.
    let config = dir.join(".git/config");
    let kept = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{kept}[core]\n\tcommentChar\n")).unwrap();

    let ran = panics_in(|| {
        let config = git2::Repository::open(&dir).unwrap().config().unwrap();
        config
            .get_entry("core.commentchar")
            .unwrap()
            .value()
            .is_none()
    });
    assert!(ran.is_none(), "git2 took the key after all");

    let commit = Write::Commit {
        message: "two\n# a comment\n".to_string(),
        head: Some(head.clone()),
    };
    let refused = panics_in(|| GitWriter.write(&dir, &commit)).expect("no panic");
    let refused = refused.unwrap_err().to_string();
    assert!(
        refused.contains("missing value for 'core.commentchar'"),
        "{refused}"
    );
    std::fs::write(&config, kept).unwrap();
    assert_eq!(
        git(&dir, &["rev-parse", "HEAD"]),
        head,
        "nothing was committed"
    );
}

/// One pkt-line of the git protocol holding `data`.
fn pkt(data: &[u8]) -> Vec<u8> {
    let mut line = format!("{:04x}", data.len() + 4).into_bytes();
    line.extend_from_slice(data);
    line
}

/// The next pkt-line `stream` sends, none at a flush.
fn read_pkt(stream: &mut impl std::io::Read) -> Option<Vec<u8>> {
    let mut length = [0; 4];
    stream.read_exact(&mut length).unwrap();
    let length = usize::from_str_radix(std::str::from_utf8(&length).unwrap(), 16).unwrap();
    let mut data = vec![0; length.checked_sub(4)?];
    stream.read_exact(&mut data).unwrap();
    Some(data)
}

/// A remote served over the git protocol on a port of this machine, for `connections`
/// connections one after another, starting empty. It takes a push of `refs/heads/main` and
/// answers `report`, its status, after `told` on the side band where there is one; once it
/// reported the update taken, it holds `refs/heads/main` at what was sent, which a fetch reads,
/// or at `moved_to` where another push moved it on since.
fn remote_answering(
    report: &'static [u8],
    told: Option<&'static [u8]>,
    connections: usize,
    moved_to: Option<String>,
) -> (String, std::thread::JoinHandle<()>) {
    remote_holding(None, report, told, connections, moved_to)
}

/// [`remote_answering`], the remote holding `refs/heads/main` at `initially` from the start
/// where there is one.
fn remote_holding(
    initially: Option<String>,
    report: &'static [u8],
    told: Option<&'static [u8]>,
    connections: usize,
    moved_to: Option<String>,
) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("git://{}/repo", listener.local_addr().unwrap());
    let served = std::thread::spawn(move || {
        let mut holds: Option<String> = initially;
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(30)))
                .unwrap();
            let request = read_pkt(&mut stream).unwrap();
            let none = [b"0".repeat(40).as_slice(), b" capabilities^{}\0"].concat();
            if request.starts_with(b"git-receive-pack ") {
                let caps: &[u8] = if told.is_some() {
                    b"report-status side-band-64k\n"
                } else {
                    b"report-status\n"
                };
                let first = match &holds {
                    Some(oid) => [format!("{oid} refs/heads/main\0").as_bytes(), caps].concat(),
                    None => [none.as_slice(), caps].concat(),
                };
                let mut advertised = pkt(&first);
                advertised.extend_from_slice(b"0000");
                stream.write_all(&advertised).unwrap();
                let command = read_pkt(&mut stream).unwrap();
                let sent = String::from_utf8_lossy(&command[41..81]).into_owned();
                while read_pkt(&mut stream).is_some() {}
                let mut status = pkt(b"unpack ok\n");
                status.extend(pkt(report));
                status.extend_from_slice(b"0000");
                let answer = match told {
                    Some(told) => {
                        let mut answer = pkt(&[b"\x02", told].concat());
                        answer.extend(pkt(&[b"\x01", status.as_slice()].concat()));
                        answer.extend_from_slice(b"0000");
                        answer
                    }
                    None => status,
                };
                stream.write_all(&answer).unwrap();
                if report.starts_with(b"ok ") {
                    holds = Some(sent);
                }
                if moved_to.is_some() {
                    holds = moved_to.clone();
                }
            } else {
                assert!(request.starts_with(b"git-upload-pack "), "{request:?}");
                let first = match &holds {
                    Some(oid) => format!("{oid} refs/heads/main\0ofs-delta\n").into_bytes(),
                    None => [none.as_slice(), b"ofs-delta\n"].concat(),
                };
                let mut advertised = pkt(&first);
                advertised.extend_from_slice(b"0000");
                stream.write_all(&advertised).unwrap();
            }
            // What the client still sends, the pack of a push among it, until it hangs up.
            let _ = stream.read_to_end(&mut Vec::new());
        }
    });
    (url, served)
}

/// A repository with one commit on `main`, which has no upstream, and the remote `origin` at `url`.
fn pushing_to(dir: &Path, url: &str) {
    git(dir, &["init", "--quiet", "-b", "main"]);
    git(dir, &["config", "user.name", "Ada"]);
    git(dir, &["config", "user.email", "ada@example.invalid"]);
    write_file(dir, "f", "1\n");
    git(dir, &["add", "f"]);
    git(dir, &["commit", "--quiet", "-m", "one"]);
    git(dir, &["remote", "add", "origin", url]);
}

/// Pushes `main` of `dir`, served by `served`, and what the writer answered, with no panic.
fn push_main(dir: &Path, served: std::thread::JoinHandle<()>) -> anyhow::Result<Option<String>> {
    let push = Write::Push {
        branch: "main".into(),
        tip: git(dir, &["rev-parse", "main"]),
    };
    let pushed = panics_in(|| GitWriter.write(dir, &push)).expect("no panic");
    served.join().unwrap();
    assert!(
        !git(dir, &["for-each-ref"]).contains("refs/ocelli/"),
        "the scratch reference is gone"
    );
    pushed
}

#[test]
// Verifies: ADR-0035, ADR-0037
fn a_remote_refusing_a_push_with_a_reason_not_utf8_is_told_without_a_panic() {
    let scratch = Scratch::new("push-refused");
    let (url, served) = remote_answering(b"ng refs/heads/main \xff\xfe declined\n", None, 2, None);
    pushing_to(&scratch.0, &url);
    let refused = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(refused, "origin refused the update");
    assert!(!git(&scratch.0, &["branch", "--remotes"]).contains("origin/main"));
}

#[test]
// Verifies: ADR-0035
fn a_push_a_hook_refuses_says_what_the_hook_said_and_not_to_pull() {
    let scratch = Scratch::new("push-hook");
    let (url, served) = remote_answering(
        b"ng refs/heads/main pre-receive hook declined\n",
        Some(b"checking main\nmain is protected: open a pull request\n"),
        2,
        None,
    );
    pushing_to(&scratch.0, &url);
    let refused = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(
        refused,
        "origin refused the update; the remote said: checking main / main is protected: open a \
         pull request"
    );
}

#[test]
// Verifies: ADR-0035, ADR-0037
fn side_band_text_not_utf8_and_with_control_characters_is_told_without_a_panic() {
    let scratch = Scratch::new("push-told");
    let (url, served) = remote_answering(
        b"ng refs/heads/main hook declined\n",
        Some(b"\x1b[31mno\xff\xfe w\xe2\x80\x8bay\x1b[0m\x07\x1b]0;title\x07 \xe2\x80\xaeyes\n"),
        2,
        None,
    );
    pushing_to(&scratch.0, &url);
    let refused = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(
        refused,
        "origin refused the update; the remote said: no\u{fffd}\u{fffd} way yes"
    );
}

#[test]
// Verifies: ADR-0035
fn a_push_the_remote_reports_taken_moves_the_remote_tracking_branch_and_the_upstream() {
    let scratch = Scratch::new("push-taken");
    // One connection: the remote-tracking branch libgit2 moves tells the push was taken.
    let (url, served) = remote_answering(b"ok refs/heads/main\n", None, 1, None);
    pushing_to(&scratch.0, &url);
    let tip = git(&scratch.0, &["rev-parse", "main"]);
    push_main(&scratch.0, served).unwrap();
    assert_eq!(git(&scratch.0, &["rev-parse", "origin/main"]), tip);
    assert_eq!(
        git(
            &scratch.0,
            &["rev-parse", "--abbrev-ref", "main@{upstream}"]
        ),
        "origin/main"
    );
}

#[test]
// Verifies: ADR-0035
fn a_push_no_fetch_refspec_maps_is_confirmed_by_reading_the_remote_again() {
    let scratch = Scratch::new("push-unmapped");
    let (url, served) = remote_answering(b"ok refs/heads/main\n", None, 2, None);
    pushing_to(&scratch.0, &url);
    git(
        &scratch.0,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    push_main(&scratch.0, served).unwrap();
    assert_eq!(git(&scratch.0, &["config", "branch.main.remote"]), "origin");
    assert_eq!(
        git(&scratch.0, &["config", "branch.main.merge"]),
        "refs/heads/main"
    );
}

#[test]
// Verifies: ADR-0035
fn a_push_a_negative_fetch_refspec_leaves_untracked_is_confirmed_by_reading_the_remote_again() {
    let scratch = Scratch::new("push-negative");
    let (url, served) = remote_answering(b"ok refs/heads/main\n", None, 2, None);
    pushing_to(&scratch.0, &url);
    git(
        &scratch.0,
        &["config", "--add", "remote.origin.fetch", "^refs/heads/main"],
    );
    push_main(&scratch.0, served).unwrap();
    assert!(!git(&scratch.0, &["branch", "--remotes"]).contains("origin/main"));
}

#[test]
// Verifies: ADR-0035
fn a_push_refused_where_no_fetch_refspec_maps_reads_as_refused() {
    let scratch = Scratch::new("push-unmapped-refused");
    let (url, served) = remote_answering(b"ng refs/heads/main stale info\n", None, 2, None);
    pushing_to(&scratch.0, &url);
    git(
        &scratch.0,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    let refused = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(refused, "origin refused the update");
    assert!(
        git_may_fail_quietly(&scratch.0, &["config", "branch.main.merge"]),
        "no upstream is set for a push refused"
    );
}

#[test]
// Verifies: ADR-0035
fn a_push_whose_branch_another_push_moved_on_is_not_confirmed_and_not_called_refused() {
    let scratch = Scratch::new("push-overtaken");
    pushing_to(&scratch.0, "git://127.0.0.1:9/repo");
    let other = git(&scratch.0, &["rev-parse", "main"]);
    write_file(&scratch.0, "f", "2\n");
    git(&scratch.0, &["commit", "--quiet", "-am", "two"]);
    git(
        &scratch.0,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    let (url, served) = remote_answering(b"ok refs/heads/main\n", None, 2, Some(other));
    git(&scratch.0, &["remote", "set-url", "origin", &url]);
    let unconfirmed = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(
        unconfirmed,
        "origin did not confirm the update: its branch now points at another commit"
    );
}

#[test]
// Verifies: ADR-0035, ADR-0039
fn the_read_back_reads_the_address_pushed_to_however_insteadof_would_rewrite_it_again() {
    let scratch = Scratch::new("push-insteadof");
    let root = scratch.0.join("root");
    let outside = scratch.0.join("outside");
    let checked = root.join("ok").join("repo.git");
    let elsewhere = outside.join("repo.git");
    for bare in [&checked, &elsewhere] {
        std::fs::create_dir_all(bare).unwrap();
        git(bare, &["init", "--quiet", "--bare", "-b", "main"]);
    }
    // What lies outside holds a commit of its own on `main`.
    let seed = scratch.0.join("seed");
    std::fs::create_dir_all(&seed).unwrap();
    pushing_to(&seed, elsewhere.to_str().unwrap());
    write_file(&seed, "f", "foreign\n");
    git(&seed, &["commit", "--quiet", "-am", "foreign"]);
    git(&seed, &["push", "--quiet", "origin", "main"]);
    let foreign = git(&seed, &["rev-parse", "main"]);

    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    pushing_to(&dir, "evil:/repo.git");
    let first = format!("url.{}.insteadOf", root.join("ok").display());
    let second = format!("url.{}.insteadOf", outside.display());
    git(&dir, &["config", &first, "evil:"]);
    git(
        &dir,
        &["config", &second, &root.join("ok").display().to_string()],
    );
    git(&dir, &["config", "--unset-all", "remote.origin.fetch"]);
    let push = Write::Push {
        branch: "main".into(),
        tip: git(&dir, &["rev-parse", "main"]),
    };
    panics_in(|| GitWriter.write(&dir, &push).unwrap()).expect("no panic");
    assert_eq!(
        git(&checked, &["rev-parse", "main"]),
        git(&dir, &["rev-parse", "main"])
    );
    assert!(
        git_may_fail_quietly(&dir, &["cat-file", "-e", &foreign]),
        "nothing of the repository outside reached the object store"
    );
    assert!(!git(&dir, &["for-each-ref"]).contains("refs/ocelli/"));
}

#[test]
// Verifies: ADR-0035
fn a_push_the_remote_refuses_for_a_branch_it_has_reads_as_refused_with_what_it_said() {
    let scratch = Scratch::new("push-refused-held");
    pushing_to(&scratch.0, "git://127.0.0.1:9/repo");
    let held = git(&scratch.0, &["rev-parse", "main"]);
    write_file(&scratch.0, "f", "2\n");
    git(&scratch.0, &["commit", "--quiet", "-am", "two"]);
    git(
        &scratch.0,
        &["config", "--unset-all", "remote.origin.fetch"],
    );
    let (url, served) = remote_holding(
        Some(held.clone()),
        b"ng refs/heads/main hook declined\n",
        Some(b"main is frozen\n"),
        2,
        None,
    );
    git(&scratch.0, &["remote", "set-url", "origin", &url]);
    let refused = push_main(&scratch.0, served).unwrap_err().to_string();
    assert_eq!(
        refused,
        "origin refused the update; the remote said: main is frozen"
    );
}

/// Whether `git` fails on `args` in `dir`.
fn git_may_fail_quietly(dir: &Path, args: &[&str]) -> bool {
    !support::git_output(dir, args).0
}
/// A repository with one commit, and a submodule at `sub` whose `.gitmodules` entry `extra`
/// follows.
fn with_submodule(dir: &Path, extra: &[u8]) {
    pushing_to(dir, "git://127.0.0.1:9/repo");
    let head = git(dir, &["rev-parse", "HEAD"]);
    let link = format!("160000,{head},sub");
    git(dir, &["update-index", "--add", "--cacheinfo", &link]);
    let mut modules = b"[submodule \"sub\"]\n\tpath = sub\n\turl = ../sub\n".to_vec();
    modules.extend_from_slice(extra);
    std::fs::write(dir.join(".gitmodules"), modules).unwrap();
}

#[test]
// Verifies: ADR-0037
fn submodules_are_listed_as_configured_and_on_windows_only_where_every_path_is_utf8() {
    let scratch = Scratch::new("submodule-paths");
    let plain = scratch.0.join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    with_submodule(&plain, b"");
    let refs = GitReader.load_refs(&plain).unwrap();
    assert_eq!(refs.submodules.len(), 1, "{:?}", refs.submodules);

    let odd = scratch.0.join("odd");
    std::fs::create_dir_all(&odd).unwrap();
    with_submodule(
        &odd,
        b"[submodule \"odd\"]\n\tpath = odd\xff\n\turl = ../odd\n",
    );
    let refs = panics_in(|| GitReader.load_refs(&odd).unwrap()).expect("no panic");
    // Only on Windows does git2 unwrap the path as UTF-8; elsewhere they are listed as
    // configured.
    if cfg!(windows) {
        assert!(refs.submodules.is_empty(), "{:?}", refs.submodules);
    } else {
        assert!(
            refs.submodules.iter().any(|s| s.name == "sub"),
            "{:?}",
            refs.submodules
        );
    }
}

#[test]
// Verifies: ADR-0037
fn a_working_tree_whose_link_is_not_utf8_is_left_out_on_windows_alone() {
    let scratch = Scratch::new("worktree-path");
    let dir = scratch.0.join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    pushing_to(&dir, "git://127.0.0.1:9/repo");
    let other = scratch.0.join("other");
    git(
        &dir,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "topic",
            other.to_str().unwrap(),
        ],
    );
    let names = |dir: &Path| -> Vec<String> {
        GitReader
            .load_refs(dir)
            .unwrap()
            .worktrees
            .into_iter()
            .map(|w| w.name)
            .collect()
    };
    assert!(
        names(&dir).contains(&"other".to_string()),
        "{:?}",
        names(&dir)
    );
    let link = dir.join(".git/worktrees/other/gitdir");
    let mut bytes = std::fs::read(&link).unwrap();
    bytes.splice(0..0, b"\xff".iter().copied());
    std::fs::write(&link, bytes).unwrap();
    let left = panics_in(|| names(&dir)).expect("no panic");
    // Only on Windows does git2 unwrap the path as UTF-8; elsewhere it is listed as configured.
    assert_eq!(
        left.contains(&"other".to_string()),
        !cfg!(windows),
        "{left:?}"
    );
}

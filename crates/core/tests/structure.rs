//! The structural tests of the core's shape.
//!
//! ADR-0009 keeps every technology and every adapter out of `ocelli-core`: its manifest may name
//! only the pure libraries listed here, and the git adapter reaches no front end. ADR-0010 keeps the layout a functional core: the layout
//! module reaches no file, network, process, thread, clock, environment, or randomness. ADR-0019
//! and G-13 keep the thread that draws free of repository reads and of laying a history out: the
//! terminal application's event loop does neither, and every command of the desktop host that
//! reads the history or the repository runs off its main thread.

use std::fs;
use std::path::{Path, PathBuf};

/// The libraries the core may depend on: none of them performs I/O or names a technology.
const ALLOWED: &[&str] = &["anyhow", "chrono", "serde", "sha2"];

/// The front ends, their technologies, and the other driving adapters, which an adapter on the
/// repository side does not reach (ADR-0009).
const FRONT_ENDS: &[&str] = &[
    "tauri",
    "tauri-build",
    "ratatui",
    "crossterm",
    "wry",
    "ocelli-cli",
    "ocelli",
    "ocelli-measure",
];

/// What the layout module must not use (ADR-0010).
const FORBIDDEN_IN_LAYOUT: &[&str] = &[
    "std::fs",
    "std::io",
    "std::net",
    "std::process",
    "std::thread",
    "std::time",
    "std::env",
    "Utc::now",
    "Local::now",
    "rand::",
];

fn read(rel: &str) -> String {
    fs::read_to_string(format!("{}/{rel}", env!("CARGO_MANIFEST_DIR")))
        .unwrap_or_else(|e| panic!("{rel} is readable: {e}"))
}

/// The dependency names of the manifest's `[dependencies]` tables, target-specific ones included
/// and development and build ones not, in declaration order.
fn dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_deps = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_deps = line.ends_with("dependencies]")
                && !line.contains("dev-dependencies")
                && !line.contains("build-dependencies");
            continue;
        }
        if in_deps
            && !line.is_empty()
            && !line.starts_with('#')
            && let Some((name, _)) = line.split_once('=')
        {
            // `name.workspace = true` names the dependency `name`.
            let name = name.trim().trim_matches('"');
            names.push(name.split('.').next().unwrap_or(name).to_string());
        }
    }
    names
}

/// The source before the `#[cfg(test)]` module: the code that ships.
fn shipped(source: &str) -> &str {
    source.split("#[cfg(test)]").next().unwrap_or(source)
}

#[test]
// Verifies: ADR-0009
fn the_core_depends_on_no_technology_and_no_adapter() {
    let deps = dependencies(&read("Cargo.toml"));
    assert!(!deps.is_empty(), "the manifest's dependencies were read");
    let foreign: Vec<_> = deps
        .iter()
        .filter(|d| !ALLOWED.contains(&d.as_str()))
        .collect();
    assert!(foreign.is_empty(), "ocelli-core depends on {foreign:?}");
}

#[test]
// Verifies: ADR-0009
fn the_dependency_reader_sees_a_technology() {
    let manifest = concat!(
        "[package]\nname = \"x\"\n\n[dependencies]\nserde = \"1\"\n\n",
        "[target.'cfg(unix)'.dependencies]\ngit2 = \"0.20\"\nchrono.workspace = true\n\n",
        "[dev-dependencies]\nserde_json = \"1\"\n",
    );
    assert_eq!(dependencies(manifest), ["serde", "git2", "chrono"]);
}

#[test]
// Verifies: ADR-0009
fn the_git_adapter_depends_on_no_front_end() {
    let deps = dependencies(&read("../git/Cargo.toml"));
    assert!(
        deps.contains(&"ocelli-core".to_string()),
        "the adapter depends on the core"
    );
    let front = front_ends(&deps);
    assert!(front.is_empty(), "ocelli-git depends on {front:?}");
}

/// The dependencies among `deps` that are a front end or another driving adapter.
fn front_ends(deps: &[String]) -> Vec<&String> {
    deps.iter()
        .filter(|d| FRONT_ENDS.contains(&d.as_str()))
        .collect()
}

#[test]
// Verifies: ADR-0009
fn the_front_end_check_names_the_desktop_host_and_the_measurement() {
    let manifest = concat!(
        "[dependencies]\nocelli-core = { workspace = true }\n",
        "ocelli = { path = \"../../src-tauri\" }\nocelli-measure = { path = \"../measure\" }\n",
    );
    assert_eq!(
        front_ends(&dependencies(manifest)),
        ["ocelli", "ocelli-measure"]
    );
}

#[test]
// Verifies: ADR-0010
fn the_layout_performs_no_effect() {
    let source = read("src/graph.rs");
    let code = shipped(&source);
    let found: Vec<_> = FORBIDDEN_IN_LAYOUT
        .iter()
        .filter(|f| code.contains(**f))
        .collect();
    assert!(found.is_empty(), "the layout uses {found:?}");
}

#[test]
// Verifies: ADR-0010
fn the_effect_scan_sees_an_effect() {
    let source = "fn f() { let _ = std::time::Instant::now(); }\n#[cfg(test)]\nmod tests {}";
    assert!(shipped(source).contains("std::time"));
}

#[test]
fn every_crate_takes_the_workspace_lints_that_deny_unsafe_code() {
    let workspace = read("../../Cargo.toml");
    assert!(
        workspace.contains("[workspace.lints.rust]\nunsafe_code = \"deny\""),
        "the workspace denies unsafe code"
    );
    for member in ["core", "git", "measure", "tui", "../src-tauri"] {
        let manifest = read(&format!("../{member}/Cargo.toml"));
        assert!(
            manifest.contains("[lints]\nworkspace = true"),
            "{member} takes the workspace lints"
        );
    }
}

/// What reads a repository or lays a history out, which the terminal application's drawing and
/// key handling must not do (ADR-0019, G-13).
const BLOCKING: &[&str] = &[
    "GitReader",
    "graph::Layout",
    "lay_out(",
    "load_chunks",
    "load::load",
    "load_refs",
    "load_commit_files",
    "load_file_diff",
];

/// The blocking calls a source makes in the code that ships.
fn blocking_in(source: &str) -> Vec<&'static str> {
    let code = shipped(source);
    BLOCKING
        .iter()
        .copied()
        .filter(|b| code.contains(b))
        .collect()
}

#[test]
// Verifies: ADR-0019, G-13
fn the_terminal_application_draws_and_handles_keys_without_reading_or_laying_out() {
    for file in ["app.rs", "ui.rs", "raster.rs"] {
        let source = read(&format!("../tui/src/{file}"));
        assert!(
            blocking_in(&source).is_empty(),
            "crates/tui/src/{file} calls {:?} on the thread that draws",
            blocking_in(&source)
        );
    }
}

#[test]
// Verifies: ADR-0019, G-13
fn the_blocking_scan_sees_a_read() {
    let source = "fn f() { GitReader.load_refs(path); }\n#[cfg(test)]\nmod tests {}";
    assert_eq!(blocking_in(source), ["GitReader", "load_refs"]);
}

/// The desktop host's commands that read the history or the repository and would run on the
/// main thread, which Tauri gives a command that is not async.
fn commands_on_the_main_thread(source: &str) -> Vec<String> {
    let code = shipped(source);
    code.split("#[tauri::command")
        .skip(1)
        .filter_map(|command| {
            let (attribute, rest) = command.split_once(']')?;
            let name = rest
                .split("fn ")
                .nth(1)?
                .split('(')
                .next()?
                .trim()
                .to_string();
            let off_main =
                attribute.contains("async") || rest.split('{').next()?.contains("async fn");
            let reads = ["session", "GitReader", "GitWriter"]
                .iter()
                .any(|uses| rest.contains(uses));
            (reads && !off_main).then_some(name)
        })
        .collect()
}

#[test]
// Verifies: ADR-0019, ADR-0033, G-13
fn every_command_that_reads_or_writes_the_repository_runs_off_the_main_thread() {
    let source = read("../../src-tauri/src/commands.rs");
    assert_eq!(commands_on_the_main_thread(&source), Vec::<String>::new());
}

#[test]
// Verifies: ADR-0019, G-13
fn the_command_scan_sees_a_command_on_the_main_thread() {
    let source = "#[tauri::command]\npub fn read(state: State) -> W { state.session.window(0, 1) }\n\
                  #[tauri::command(async)]\npub fn fine(state: State) { state.session.x() }\n\
                  #[tauri::command]\npub async fn also(state: State) { GitReader.x() }\n\
                  #[tauri::command]\npub fn write(state: State) { GitWriter.write() }\n";
    assert_eq!(commands_on_the_main_thread(source), ["read", "write"]);
}

/// The workspace's members, as the root manifest lists them.
fn members(manifest: &str) -> Vec<String> {
    let list = manifest
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map_or("", |(list, _)| list);
    list.split(',')
        .map(|m| m.trim().trim_matches('"').to_string())
        .filter(|m| !m.is_empty())
        .collect()
}

/// Every Rust source under `dir`, with its path.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(sources(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push((path.clone(), fs::read_to_string(&path).unwrap()));
        }
    }
    out
}

/// What a front end's shipped source names that would write past the core's confirmation rule:
/// the writer port's trait, without which no writer's `write` can be called, a glob import of the
/// port that brings it in unnamed, or the git adapter's write module.
fn writes_past_the_confirmation(source: &str) -> Vec<&'static str> {
    let code = shipped(source);
    ["RepositoryWriter", "port::*", "ocelli_git::write"]
        .into_iter()
        .filter(|name| code.contains(name))
        .collect()
}

#[test]
// Verifies: ADR-0034, Q-6
fn every_front_end_writes_through_the_cores_confirmation_rule() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut front_ends = Vec::new();
    for member in members(&read("../../Cargo.toml")) {
        let manifest = fs::read_to_string(root.join(&member).join("Cargo.toml")).unwrap();
        if member == "crates/git" || !dependencies(&manifest).contains(&"ocelli-git".into()) {
            continue;
        }
        for (path, source) in sources(&root.join(&member).join("src")) {
            let past = writes_past_the_confirmation(&source);
            assert!(past.is_empty(), "{} names {past:?}", path.display());
        }
        front_ends.push(member);
    }
    assert!(
        ["src-tauri", "crates/tui"]
            .iter()
            .all(|f| front_ends.iter().any(|m| m == f)),
        "both front ends are scanned: {front_ends:?}"
    );
    for writer in ["../../src-tauri/src/commands.rs", "../tui/src/loader.rs"] {
        assert!(
            read(writer).contains("port::perform(&GitWriter"),
            "{writer} writes at all"
        );
    }
}

#[test]
// Verifies: ADR-0034
fn the_write_scan_sees_a_write_past_the_confirmation() {
    let through = "use ocelli_git::{GitReader, GitWriter};\n\
                   fn a() { port::perform(&GitWriter, p, w, c) }\n";
    assert_eq!(writes_past_the_confirmation(through), Vec::<&str>::new());
    let renamed =
        "use ocelli_core::port::RepositoryWriter as W;\nfn b() { GitWriter.write(p, w) }\n";
    assert_eq!(writes_past_the_confirmation(renamed), ["RepositoryWriter"]);
    let glob = "use ocelli_core::port::*;\nfn e() { GitWriter.write(p, w) }\n";
    assert_eq!(writes_past_the_confirmation(glob), ["port::*"]);
    let module = "fn c() { ocelli_git::write::write(p, w) }\n";
    assert_eq!(writes_past_the_confirmation(module), ["ocelli_git::write"]);
    let in_tests = "fn d() {}\n#[cfg(test)]\nmod tests { use super::RepositoryWriter; }\n";
    assert_eq!(writes_past_the_confirmation(in_tests), Vec::<&str>::new());
}

#[test]
// Verifies: ADR-0035
fn git2_reaches_remotes_through_its_own_https_and_ssh_transports_only() {
    let manifest = read("../git/Cargo.toml");
    let git2 = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("git2 "))
        .expect("the git adapter depends on git2");
    assert!(git2.contains("default-features = false"), "{git2}");
    for feature in ["\"https\"", "\"ssh\""] {
        assert!(git2.contains(feature), "{feature} missing: {git2}");
    }
    assert!(
        !git2.contains("vendored"),
        "the transports link the system's libraries: {git2}"
    );
}

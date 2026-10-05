//! The structural tests of the core's shape.
//!
//! ADR-0009 keeps every technology and every adapter out of `ocelli-core`: its manifest may name
//! only the pure libraries listed here, and the git adapter reaches no front end. ADR-0010 keeps the layout a functional core: the layout
//! module reaches no file, network, process, thread, clock, environment, or randomness.

use std::fs;

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
            names.push(name.trim().trim_matches('"').to_string());
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
        "[target.'cfg(unix)'.dependencies]\ngit2 = \"0.20\"\n\n",
        "[dev-dependencies]\nserde_json = \"1\"\n",
    );
    assert_eq!(dependencies(manifest), ["serde", "git2"]);
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

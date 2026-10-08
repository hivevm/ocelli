//! Embeds the web interface's build, `dist/` at the workspace's root, in the binary, so the server
//! serves what was built with it and no directory beside it decides what the browser runs
//! (ADR-0037). Without a build the server is compiled with no files and says so when asked.

use std::path::{Path, PathBuf};

fn files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            files(&path, found);
        } else {
            found.push(path);
        }
    }
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = manifest.join("../../dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    let mut found = Vec::new();
    files(&dist, &mut found);
    found.sort();
    if !found.iter().any(|f| f.ends_with("index.html")) {
        println!(
            "cargo:warning=the web interface is not built: run `bun run --cwd src-web build` before \
             building ocelli-server, or it serves no web interface"
        );
    }
    let mut out = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    for file in &found {
        let name = file
            .strip_prefix(&dist)
            .unwrap()
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        let path = file.canonicalize().unwrap();
        out.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            path.to_string_lossy()
        ));
    }
    out.push_str("];\n");
    let target = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("web.rs");
    std::fs::write(target, out).unwrap();
}

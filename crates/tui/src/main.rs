//! `ocelli-cli` — the terminal frontend of Ocelli (ADR-0028).
//!
//! History and refs come from `ocelli-git` and the layout from `ocelli-core`,
//! exactly as for the desktop application; this crate only rasterises and
//! navigates them.

mod app;
mod chips;
mod diffsplit;
mod files;
mod loader;
mod raster;
mod sidebar;
mod state;
mod theme;
mod ui;

use app::App;
use loader::{Msg, Request};
use raster::Glyphs;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

const USAGE: &str = "\
Usage: ocelli-cli [--ascii] [PATH]

Shows the commit graph of the git repository at PATH (default: the current
directory) in the terminal. Nothing in the repository is changed but by a
write the developer asks for, such as tagging a commit.

Options:
  --ascii        Draw the graph with ASCII characters instead of box drawing
  -h, --help     Print this help
  -V, --version  Print the version";

struct Args {
    path: PathBuf,
    glyphs: Glyphs,
}

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Option<Args>, String> {
    let mut path = None;
    let mut glyphs = Glyphs::Unicode;
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("ocelli-cli {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--ascii" => glyphs = Glyphs::Ascii,
            a if a.starts_with('-') => return Err(format!("unknown option: {a}\n\n{USAGE}")),
            a if path.is_none() => path = Some(PathBuf::from(a)),
            a => return Err(format!("unexpected argument: {a}\n\n{USAGE}")),
        }
    }
    Ok(Some(Args {
        path: path.unwrap_or_else(|| PathBuf::from(".")),
        glyphs,
    }))
}

fn main() -> anyhow::Result<()> {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return Ok(()),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    let path = args
        .path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.path.display()))?;

    let mut app = App::new(path, args.glyphs);
    let state_path = state::path();
    if let Some(p) = &state_path {
        app.apply_view_state(state::load(p));
    }
    // `ratatui::run` restores the terminal on return and on panic.
    ratatui::run(|terminal| run(terminal, &mut app, state_path))
}

/// The background threads serving one repository. Dropping it drops the
/// receiver, so results still in flight for that repository go nowhere.
struct Session {
    rx: Receiver<Msg>,
    worker: Sender<Request>,
}

impl Session {
    fn start(path: &Path) -> Self {
        let (tx, rx) = channel();
        loader::spawn_load(path.to_path_buf(), tx.clone());
        let worker = loader::spawn_worker(path.to_path_buf(), tx);
        Self { rx, worker }
    }
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    state_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    let mut session = Session::start(&app.repo);
    // Writes report on a channel of their own, which outlives every session: a
    // refresh while a write runs must not lose what it did or why it failed.
    let (written_tx, written_rx) = channel();
    // Set to stop the write that runs; a new one gets a flag of its own.
    let mut cancel = Arc::new(AtomicBool::new(false));
    let mut saved = app.view_state();

    while !app.quit {
        terminal.draw(|f| ui::draw(f, app))?;
        // Poll with a timeout so worker results are picked up without a key press.
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.on_key(key);
        }
        // Switch before draining, so nothing of the old repository reaches the new state.
        if let Some(path) = app.reload.take() {
            session = Session::start(&path);
        }
        if let Some((write, confirmation)) = app.write.take() {
            cancel = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&cancel);
            loader::spawn_write(
                app.repo.clone(),
                write,
                confirmation,
                flag,
                written_tx.clone(),
            );
        }
        if std::mem::take(&mut app.cancel) {
            cancel.store(true, Ordering::Relaxed);
        }
        while let Ok(msg) = session.rx.try_recv() {
            app.on_msg(msg);
        }
        while let Ok(msg) = written_rx.try_recv() {
            app.on_msg(msg);
        }
        for req in app.requests.drain(..) {
            session.worker.send(req)?;
        }
        if let Some(text) = app.clipboard.take() {
            copy_to_clipboard(&text)?;
        }
        // Remember a changed theme, sidebar or diff layout (ADR-0029). A failed
        // write costs only the memory of it, so it is reported, not fatal.
        let now = app.view_state();
        if now != saved {
            saved = now;
            if let Some(p) = &state_path
                && let Err(e) = state::save(p, &now)
            {
                app.status = format!("Could not save view state to {}: {e}", p.display());
            }
        }
    }
    Ok(())
}

/// Copies through the terminal: OSC 52 asks the terminal emulator to set the
/// clipboard, which also works over SSH and needs no clipboard library.
/// Terminals that do not support it ignore the sequence.
fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
    out.flush()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Option<Args>, String> {
        parse_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_to_the_current_directory_with_unicode_glyphs() {
        let args = parse(&[]).unwrap().unwrap();
        assert_eq!(args.path, PathBuf::from("."));
        assert_eq!(args.glyphs, Glyphs::Unicode);
    }

    #[test]
    fn accepts_a_path_and_the_ascii_flag_in_any_order() {
        let args = parse(&["--ascii", "/repo"]).unwrap().unwrap();
        assert_eq!(args.path, PathBuf::from("/repo"));
        assert_eq!(args.glyphs, Glyphs::Ascii);
    }

    #[test]
    fn rejects_unknown_options_and_a_second_path() {
        assert!(parse(&["--frobnicate"]).is_err());
        assert!(parse(&["a", "b"]).is_err());
    }

    #[test]
    fn base64_matches_the_rfc_4648_test_vectors() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
    }
}

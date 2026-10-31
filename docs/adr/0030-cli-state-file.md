# ADR-0030: The terminal application remembers its view state in one JSON file in the user's state directory

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `crates/tui/src/state.rs` and the state file it writes

## Context

The web interface remembers a few view choices across sessions, each in `localStorage`: the theme
(`theme`, [ADR-0024](0024-themes-tokens-and-palettes.md)), whether the repository sidebar is
minimised (`sidebar`, [ADR-0027](0027-repository-sidebar.md)), the diff layout (`diffView`, unified
or side by side), and the avatar opt-in (`avatars`, [ADR-0025](0025-no-egress-hardened-webview.md)).
The terminal application ([ADR-0029](0029-terminal-application-ratatui.md)) offers the same
choices but the avatars, and a developer who prefers the light theme or a minimised sidebar should
not set it again on every start.

A terminal program has no `localStorage`; remembering anything means writing a file. Where that
file lives, what format it has, and how the program behaves when it is missing or broken constrain
every later setting.

Two forces shape it:

- **The file is Ocelli's own data**, not the repository's: it lives outside every repository, and
  nothing of it lands in a working tree or in `.git`, so every repository stays as the standard
  `git` leaves it (Non-Goals).
- **It never leaves the machine** ([ADR-0025](0025-no-egress-hardened-webview.md)).

## Decision

We will keep the terminal application's view state in **one JSON file, `cli-state.json`, in an
`ocelli` directory under the platform's state location**, read once at start and rewritten whenever
a remembered setting changes.

- **Location.** Linux and other XDG systems: `$XDG_STATE_HOME/ocelli/`, falling back to
  `$HOME/.local/state/ocelli/` when the variable is unset, empty, or not absolute, as the XDG Base
  Directory rules say. macOS: `$HOME/Library/Application Support/ocelli/`. Windows:
  `%LOCALAPPDATA%\ocelli\`. The path is resolved from these environment variables directly; no
  crate is added for it.
- **Content.** Exactly the web interface's remembered choices that exist in the terminal:
  `{"theme": "dark" | "light", "sidebarMinimised": bool, "diffView": "unified" | "split"}`.
  Nothing per repository and no history (recent repositories, last selection).
- **Format.** JSON through `serde` and `serde_json`, which the workspace already locks. Every field
  is optional with its default, and unknown fields are ignored, so the file needs no version and
  stays readable in both directions across releases.
- **Failure never blocks the terminal application.** A missing, unreadable, or malformed file
  means defaults; a failed write is reported in the status line and otherwise ignored. A write goes
  to a temporary file in the same directory and is renamed over the old one, so an interrupted
  write cannot leave half a file.
- **Command-line flags win.** `--ascii` stays a flag of one invocation and is not stored.

**Out of scope:** per-repository state and recent repositories, each a feature of its own; the web
interface's storage.

## Alternatives considered

- **No persistence, flags or environment variables only** (`--light`, `OCELLI_THEME`) — nothing
  written at all, but no parity: the web interface remembers a choice made in the interface, and a
  flag must be repeated on every start or put into a shell alias by hand.
- **TOML in `$XDG_CONFIG_HOME`** — the usual place and format for configuration written by hand
  (guitar keeps `keymap.json`, `layout.json`, and `theme.json` under `~/.config/guitar`). Not
  chosen: these values are remembered state of the interface, which the XDG specification places
  in `$XDG_STATE_HOME` ("view, layout"), and TOML would add a crate for three fields.
- **The `dirs` or `directories` crate for the path** — robust lookup across platforms, but
  `dirs::state_dir` returns nothing on macOS and Windows, so the fallbacks are written by hand
  either way, and three environment variables do not justify a dependency.
- **Sharing the web interface's storage** — impossible without moving the web interface off
  `localStorage`, a change to [ADR-0024](0024-themes-tokens-and-palettes.md)'s theming and the
  WebView setup for a convenience nobody asked for.
- **One file per setting** — three files to keep consistent instead of one atomic write.

## Sources / Prior art

- XDG Base Directory Specification, `$XDG_STATE_HOME` for "state data that should persist between
  (application) restarts" such as view and layout, default `$HOME/.local/state`, relative paths
  ignored — <https://specifications.freedesktop.org/basedir/latest/>
- `dirs::state_dir`, a result on Linux only — <https://docs.rs/dirs/latest/dirs/fn.state_dir.html>
- Apple, *File System Programming Guide*, `~/Library/Application Support` —
  <https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/FileSystemProgrammingGUide/MacOSXDirectories/MacOSXDirectories.html>
- Microsoft, `KNOWNFOLDERID`, `FOLDERID_LocalAppData` —
  <https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid>
- guitar, which keeps its terminal settings as JSON under `~/.config/guitar` —
  <https://github.com/asinglebit/guitar>
- GitGraph's `docs/adr/0026-cli-view-state-file.md` on `main`, where this state file runs.

## Consequences

- Positive: the terminal application keeps the developer's theme, sidebar, and diff layout across
  sessions, like the web interface; one small file whose loss costs nothing but the defaults; no
  crate added to the lockfile.
- Negative / trade-offs: the terminal application writes to the developer's home directory,
  outside every repository; the resolution of the path by hand has to be tested for three
  platforms; the web interface and the terminal application remember their choices separately.
- Follow-ups: per-repository state, recent repositories, and whether the web interface moves its
  choices to the same file, each a decision of its own.

## Enforcement

The tests of `crates/tui/src/state.rs` cite this ADR with a `Verifies:` marker: the state
directory is the platform's on each of the three platforms, the JSON has the documented shape, a
missing or malformed file means defaults, unknown fields are ignored, and a save is read back.

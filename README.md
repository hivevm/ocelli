# Ocelli — View and Edit Git Repositories

[![Checks](https://github.com/mbrigl/git-viewer/actions/workflows/checks.yml/badge.svg)](https://github.com/mbrigl/git-viewer/actions/workflows/checks.yml)

**Ocelli** is named after the three simple eyes on top of a bee's head, with which it reads
light and the horizon at a glance. Ocelli gives a Git repository the same: a commit graph that
shows the whole history at a glance, from which the repository can be edited too. The name
follows the beekeeping vocabulary of the HiveVM family.

The work is driven by a written **specification** ([`docs/SPECIFICATION.md`](docs/SPECIFICATION.md))
and **Architecture Decision Records** ([`docs/adr/`](docs/adr/)), so intent and the reasoning
behind every structural choice stay explicit and reviewable.

> For agent instructions, see [`AGENTS.md`](AGENTS.md), the single source of truth for all coding
> agents.

## Overview

Git history is a directed acyclic graph, but the usual tools flatten it into a list or draw it as
ASCII art that becomes unreadable once several branches run in parallel. Ocelli draws that graph
properly: time runs downwards, each branch keeps its own lane, and merges are drawn as unambiguous
orthogonal edges, on the desktop and in the terminal.
```
 ● abc1234   main   Fix login bug                    Alice      2024-01-12 14:30
 ●─┐ def567         Merge feature/oauth into main    Bob        2024-01-11 09:15
 │ ● ghi890  feat   Add OAuth2 support               Carol      2024-01-10 17:42
 │ ● jkl012         Refactor auth module             Carol      2024-01-09 11:20
 ●─┘ mno345         Update dependencies              Alice      2024-01-08 16:05
 ● pqr678           Initial commit                   Dave       2024-01-01 10:00
```

The layout follows the algorithms described in Pierre Vigier's article
[*Commit Graph Drawing Algorithms*](https://pvigier.github.io/2019/05/06/commit-graph-drawing-algorithms.html).
The problem, the goals, and the success criteria live in
[`docs/SPECIFICATION.md`](docs/SPECIFICATION.md), the algorithm in
[ADR-0015](docs/adr/0015-layout-after-pvigier-by-chunk.md), and the vocabulary in
[`docs/GLOSSARY.md`](docs/GLOSSARY.md).

### Features

| Feature                | Details                                                              |
| ---------------------- | -------------------------------------------------------------------- |
| **Straight branches**  | All commits of a branch share one vertical column                |
| **Merge lines**        | Orthogonal L-shaped lines for merge edges; merge commits carry a double-circle marker |
| **Branch/tag chips**   | Colored pill labels (green = local, blue = remote, amber = tag) in their own column left of the graph, flush against the lanes; the checked-out branch carries a tick and is never collapsed, surplus refs collapse into a `+N` badge |
| **Repository sidebar** | Local branches with ahead/behind, remote branches per remote, working trees, tags, and submodules — filterable, collapsible, minimisable to an icon rail |
| **Repository switching**| Selecting a submodule opens it, selecting a working tree switches to it, and a parent row leads back to the superproject or main working tree |
| **Dark & light theme** | Switchable from the status bar; branch colors stay identical          |
| **Stash & WIP rows**   | Stash entries and uncommitted changes appear as marked graph nodes     |
| **Virtual rendering**  | Only visible rows are painted — smooth on large repositories          |
| **Commit detail**      | Full SHA (click to copy), message, author, committer, and the commit's parents and children as clickable links |
| **Search**             | `Ctrl`/`Cmd+F` — matches message, author, SHA, and refs; results panel, `Enter` cycles through hits |
| **Lineage highlight**  | Selecting a commit keeps its ancestors and descendants in full colour and dims the rest |
| **Keyboard navigation**| `↑`/`↓` walk the commit rows, `Esc` deselects                          |
| **Avatars (opt-in)**   | Gravatar avatars in the detail panel — off by default, no network requests until enabled |
| **Background loading** | History loads off the UI thread; the UI stays responsive              |
| **Whole history**      | The newest 3 000 commits show at once, the rest loads while you scroll; the status bar counts the chunks |

## Prerequisites

- [VS Code](https://code.visualstudio.com/) with the
  [Dev Containers](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)
  extension, or any DevContainer-compatible IDE
- Docker / Podman (rootless) available on the host

## Getting Started

1. Open the repository in VS Code and choose **Reopen in Container**. The Dev Container and the
   preconfigured agent extensions build automatically, and the container enables the repository's
   git hooks ([`.githooks/`](.githooks/)): no commit on `main`, and the checks run before a push.
   Working outside the container? Enable them yourself, once per clone:
   `git config core.hooksPath .githooks`. The push hook also asks `gh` for the last completed
   **Checks** run of the branch pushed to and refuses while it is red, naming the run and its
   failed jobs; the push that fixes it goes through with `OCELLI_PUSH_OVER_RED=1 git push`. Where
   `gh` is missing, not authenticated, offline, or slower than `OCELLI_PUSH_CI_TIMEOUT` seconds
   (default 5), or the branch has no completed run yet, it says it could not tell and goes on.
2. Authenticate your coding agent inside the container (for Claude Code: `claude login`).
3. Start working with the agent. Drive the work from the specification and the ADRs.

## Build, Test & Run

<!-- This section is the single source for build/test/lint/run commands; both humans and agents
     rely on it, and the rule file names it. scripts/check-build.sh runs the build, test, and lint
     commands, locally through scripts/check-all.sh and in CI through the build job. -->

Install the dependencies from the lockfile first, and again after a dependency changes:
`bun install --frozen-lockfile`.

- **Build:** `cargo build --workspace --locked` and `bun run --cwd src-web build`; the desktop
  installers with `bun run tauri build`
- **Test:** `cargo test --workspace --locked` and `bun test src-web`
- **Lint:** `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D
  warnings`, and `bun run --cwd src-web lint`
- **Run:** the desktop application with `bun run tauri dev`; the terminal application with
  `cargo run -p ocelli-cli -- [--ascii] [PATH]`; the remote server with
  `cargo run -p ocelli-server -- --root <folder> [--port <port>] [--listen <address>[:<port>]
  [--cert <file> --key <file>]] [--allow-writes]`, which embeds the web
  interface's build, so `bun run --cwd src-web build` runs before it is built

## Usage

Start the application (`bun run tauri dev`, or the binary produced by `bun run tauri build`) and
open a repository, either via the folder picker in the toolbar or by naming it on the command
line: `ocelli /path/to/repository`, a relative path resolved against the current directory. In
development, `bun run tauri dev -- -- /path/to/repository` passes it through; give an absolute path
there, since the development build runs in `src-tauri/`.

The graph loads in the background: the newest commits show at once, and the older history
follows while you scroll, search, and follow lineage, with the status bar counting what has
arrived. Branch and tag chips mark the commits the refs point at.

The sidebar on the left lists what the repository *contains*: branches, remotes, working trees,
tags, and submodules. Selecting a branch or tag reveals its commit in the graph; selecting a
submodule opens that repository, and selecting a working tree switches to that checkout. When the
open repository is a submodule or a linked working tree, a row above the sections leads back to its
superproject or main working tree.

Select any row — by click or with `↑`/`↓` — to see the commit detail: SHA (click it to copy),
message, author, date and committer, the commit's parents and children as clickable links, plus
the files it changed in a flat or tree view; selecting a file opens its diff over the graph.
Selecting also highlights the commit's lineage — ancestors and descendants — in the graph; `Esc`
deselects (closing the diff first if one is open).

`Ctrl`/`Cmd+F` focuses the search field: matches run over message, author, committer, SHA, and ref
names, a results panel lists them for click-to-jump, and `Enter`/`Shift+Enter` cycle through the
hits. Avatars in the detail panel are loaded from Gravatar only after the toggle in the status bar
is switched on — a fresh installation makes no network requests
([ADR-0023](docs/adr/0023-no-egress-hardened-webview.md)).

### In a browser

`ocelli-server --root <folder>` runs on the machine that holds the repositories and serves the
same web interface to a browser, as VS Code's remote development does. It listens on that
machine's loopback addresses only, on the port `--port` names or a free one, and prints once a
URL that carries a secret token; opening it logs the browser in with a session cookie, and a
request without it, or from another web page, is refused
([ADR-0038](docs/adr/0038-remote-server-localhost-tls-session-cookie.md)).
From another machine, forward the port through SSH (`ssh -L 8000:localhost:8000 host`, the server
started with `--port 8000`) and open the URL there. Or name an address with `--listen
<address>[:<port>]`: the server then listens there alone and only over TLS, with the certificate
and key given with `--cert <file> --key <file>`, or with a self-signed certificate it makes on
the first start and keeps in the user's configuration directory (`ocelli/server/`), printing its
SHA-256 fingerprint to compare with what the browser shows before accepting it. That certificate
names the address listened on, or, for `0.0.0.0` or `::`, the machine's host name alone, and the
server answers only under a name its certificate carries; it refuses to start where the one it
keeps does not name the address or the host name any more, or where only half of it, or a key
other accounts may read, is kept, and says what to remove. The browser opens
the repository the server root is, or one chosen with Open Repo from the folders below the root,
and nothing outside the root
([ADR-0039](docs/adr/0039-git-adapter-confined-to-the-server-root.md));
every restart prints a new URL. The server only reads unless it is started with
`--allow-writes`: then the browser stages, unstages, and commits as the desktop application
does, as the user the server runs as, with the identity git's configuration names for that user, and
whoever holds a session can write; it refuses the other writes the desktop application offers
([ADR-0037](docs/adr/0037-remote-server-websocket-per-tab.md)).

### In the terminal

`ocelli-cli [--ascii] [PATH]` opens the repository at `PATH` (default: the current directory)
in the terminal — the same rows, columns and edges as the desktop app, drawn with box-drawing
characters (`--ascii` for terminals without them). It only reads, and it never touches the network.
The newest commits show at once; the older history loads in the background while you scroll,
search, and follow lineage, and the status line counts what has arrived.

It offers what the desktop window offers, laid out the same way:

- **Title bar** — open another repository (`o`, a path prompt), refresh (`r`), and the search field.
- **Sidebar** — local branches (grouped into folders at their slashes, the checked-out one ticked,
  ahead/behind counts), remotes, working trees, tags, and submodules in collapsible sections, with
  a filter. A branch or tag jumps to its commit; a submodule or working tree opens that repository,
  and a row at the top leads back out. `1` minimises it to a rail of section glyphs with counts.
- **Graph** — ref chips in their own column (surplus refs collapse into `+N`), merge commits
  marked `◉`, stash and uncommitted-changes rows; selecting a commit keeps its ancestors and
  descendants bright and dims the rest.
- **Commit detail** — message, full SHA (`y` copies it through the terminal, OSC 52), author,
  committer, and the parents and children as links to jump along.
- **Changed files** — as a folder tree or a flat list (`t`); a file opens its diff over the graph,
  unified or side by side (`s`).
- **Theme** — dark or light (`T`), with the desktop's colours.

The theme, the minimised sidebar and the diff layout are remembered across sessions, as on the
desktop, in `cli-state.json` under `$XDG_STATE_HOME/ocelli` (default `~/.local/state/ocelli`),
`~/Library/Application Support/ocelli` on macOS, or `%LOCALAPPDATA%\ocelli` on Windows
([ADR-0029](docs/adr/0029-cli-state-file.md)). Deleting the file restores the defaults.

| Key                     | Action                                                   |
| ----------------------- | -------------------------------------------------------- |
| `j`/`k`, `↓`/`↑`        | Move in the focused pane                                  |
| `Ctrl+d`/`Ctrl+u`, `PgDn`/`PgUp` | Half / full page                                 |
| `g`/`G`                 | First / last entry                                        |
| `]` / `[`               | First parent / newest child                               |
| `}` / `{`               | Next / previous commit carrying a ref                     |
| `Tab`/`Shift+Tab`, `h`/`l` | Cycle focus / move left or right: sidebar, graph, detail, files |
| `Enter`                 | Open or toggle the selected entry, jump along a link, open a diff |
| `Ctrl+F`, `/`           | Search message, author, committer, SHA and refs; `Enter`/`Shift+Enter` cycle, `Tab` keeps the query for `n`/`N`. In the sidebar `/` filters it |
| `1`                     | Minimise / expand the sidebar                             |
| `y`                     | Copy the selected commit's SHA                            |
| `t` / `s`               | Files as tree or list / diff unified or side by side      |
| `o` / `r`               | Open another repository / refresh                         |
| `T`                     | Dark / light theme                                        |
| `Esc`                   | Close the diff or input, back to the graph, then deselect |
| `q`, `Ctrl+c`           | Quit                                                      |

## Project Layout

```
README.md             # overview & setup for humans
AGENTS.md             # single source of truth for coding agents
docs/SPECIFICATION.md # the specification: problem, goals, success criteria
docs/GLOSSARY.md      # the vocabulary everyone uses, kept current inline
docs/CONVENTIONS.md   # how this project writes what no check decides, kept current inline
docs/ARCHITECTURE.md  # the system as it currently stands
docs/adr/             # Architecture Decision Records (+ template)
crates/commands/      # the commands both hosts of the web interface answer (ADR-0037)
crates/core/          # the core: commit graph, layout, and the repository port (ADR-0009)
crates/git/           # the repository port implemented with git2 (ADR-0019)
crates/tui/           # the terminal application, ocelli-cli (ADR-0028)
crates/server/        # the remote server, ocelli-server: the web interface in a browser (ADR-0037)
crates/measure/       # ocelli-measure: edge crossings on real repositories (Q-1); with
                      # scripts/measure-memory.sh (Q-3) and scripts/measure-mutants.sh (the git
                      # adapter's guards) run weekly by .github/workflows/measure.yml
src-tauri/            # the desktop host (ADR-0018)
src-web/              # the web interface the desktop host and the remote server show (ADR-0022)
flatpak/              # the Flatpak manifest of the release pipeline (ADR-0025)
scripts/              # consistency checks and sensors (check-all.sh runs them all), run in CI
.githooks/            # git hooks: refuse a commit on main and a push while the checks or CI are red
.github/              # CI workflows, Dependabot, issue & pull request templates, code owners
.devcontainer/        # Dev Container definition (Dockerfile + Features)
.vscode/              # shared editor settings
.editorconfig         # editor-neutral formatting baseline
.gitattributes        # line-ending normalization (LF everywhere)
.agents/skills/       # the procedures of AGENTS.md as Agent Skills, one directory each (ADR-0003)
.claude/CLAUDE.md     # pointer for Claude Code to read AGENTS.md
.claude/skills/       # pointers (one symlink per skill) for Claude Code to read .agents/skills/
.claude/settings.json # Claude Code permissions: deny reading .env files (see SECURITY.md)
```

## Dev Container

The environment is defined entirely in
[`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json). It starts from a prebuilt
base image and layers Dev Container Features and VS Code extensions on top; no Dockerfile or
Compose file is required. Customise the environment by adding Features, switching the base image,
or adding extensions. Features are pinned by major tag and resolved in the committed
`devcontainer-lock.json`. Extensions are listed by identifier only, because a published extension
version cannot be repointed and a version suffix is VS Code-specific.

### Host container management

The Dev Container deliberately has **no access to the host Docker daemon**: the socket is not
mounted ([ADR-0004](docs/adr/0004-dev-container-runtime.md)). To manage the host's containers from
VS Code, run the **Container Tools** extension (`ms-azuretools.vscode-containers`) on the **host**
side: install it in your host VS Code. [`.vscode/settings.json`](.vscode/settings.json) already pins
it to run locally via `remote.extensionKind`, so it keeps talking to the host engine even when this
folder is reopened in the container.

## Coding Agents

This Dev Container preinstalls the **Claude Code** and **Mistral Vibe** VS Code extensions (see
[`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json)); other agents (OpenAI Codex,
Cursor, OpenCode, GitHub Copilot) work too once you add them.

The rules every agent follows live in [`AGENTS.md`](AGENTS.md); that there is exactly one such file
is decided in [ADR-0001](docs/adr/0001-agent-governance-model.md). An agent that cannot read it
natively gets a pointer file instead. [`.claude/CLAUDE.md`](.claude/CLAUDE.md) is the one shipped
here; it carries the pointer and no rules of its own ([`AGENTS.md` §3](AGENTS.md#3-adr-rules)). It
exists for one gap only: Claude Code loads `CLAUDE.md`, not `AGENTS.md`, tracked upstream as
[anthropics/claude-code#34235](https://github.com/anthropics/claude-code/issues/34235). Once that
lands, delete the pointer file rather than keeping a second file to maintain.

The procedures those rules name (proposing an ADR, planning a feature, reviewing a change,
revising the design) are Agent Skills under [`.agents/skills/`](.agents/skills/), the open format
and the directory Codex, Gemini CLI, and Cursor read
([ADR-0003](docs/adr/0003-procedures-as-skills.md)). An agent invokes one by name (`/review`) or
picks it up when the task matches its description. Claude Code reads `.claude/skills/` only, so
that directory holds one symlink per skill into `.agents/skills/`; a directory-level symlink is
not followed. The symlinks are pointers of the same kind as `CLAUDE.md`, tracked upstream as
[anthropics/claude-code#31005](https://github.com/anthropics/claude-code/issues/31005). Delete them
when that lands, and add one for every skill added meanwhile; the documentation check fails on a
skill without its pointer. A rule is enforced only by what every agent hits: the git hooks under
[`.githooks/`](.githooks/), the checks, and the repository settings below. A tool's own
configuration is not used to enforce one, because a gate that one agent honours and the next does
not looks like a rule and is not one.

## Repository settings

Some of this project's rules cannot be enforced by files in the repository
([`AGENTS.md` §6](AGENTS.md#6-project-rules)). They are settings on the GitHub repository itself,
configured once by a maintainer and worth re-checking after a repository move or transfer:

- a **ruleset on `main`** that requires pull requests, requires the
  <!-- required-checks begin — compared with the workflow's jobs by scripts/check-docs.sh -->
  `docs`, `traceability`, `devcontainer`, `sensors`, `shell`, and `build`
  <!-- required-checks end -->
  jobs of the **Checks** workflow as required status checks (rulesets list checks by their job
  name), and blocks force pushes and branch deletion;
- enable **require a review from Code Owners** on that ruleset, so the two artifacts only a human
  decides, [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md) and everything under
  [`docs/adr/`](docs/adr/) ([`AGENTS.md` §3](AGENTS.md#3-adr-rules)), cannot be merged without the
  owner named in [`.github/CODEOWNERS`](.github/CODEOWNERS), which has to carry a real handle or
  team for the requirement to mean anything;
- enable **secret scanning with push protection**, the only mechanical backstop behind the secrets
  rule ([`AGENTS.md` §7](AGENTS.md#7-secrets)), which is otherwise carried by review alone;
- enable **private vulnerability reporting** (see [`SECURITY.md`](SECURITY.md)).

## Template

- **Template release:** v0.1.0 — the release of [NUC](https://github.com/hivevm/nuc) this
  repository carries; a project moves the line when it takes up a later one
  ([ADR-0005](docs/adr/0005-template-releases.md)).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the workflow (specification- and ADR-driven, small
reviewable changes) and [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) for the community standards we
expect of everyone taking part. Security issues: please follow [`SECURITY.md`](SECURITY.md) instead
of opening a public issue.

## License

Released under the MIT License. See [`LICENSE`](LICENSE).

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
| **Commit limit**       | Up to 3 000 commits per load; the status bar says when the limit cut the history |

## Prerequisites

- [VS Code](https://code.visualstudio.com/) with the
  [Dev Containers](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)
  extension, or any DevContainer-compatible IDE
- Docker / Podman (rootless) available on the host
- Outside the Dev Container: bash 4.4 or later for the checks, 3.2 for the git hooks
  ([ADR-0002](docs/adr/0002-decisions-verified-by-tests.md)); macOS ships 3.2

## Getting Started

1. Open the repository in VS Code and choose **Reopen in Container**. The Dev Container and the
   preconfigured agent extensions build automatically, and the container enables the repository's
   git hooks with the merge settings they rely on ([Git hooks](#git-hooks)). Working outside the
   container? Run the three commands listed there yourself, once per clone.
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
  `cargo run -p ocelli-cli -- [--ascii] [PATH]`

## Usage

Start the application (`bun run tauri dev`, or the binary produced by `bun run tauri build`) and
open a repository — either via the folder picker in the toolbar or by passing a path.

The graph loads in the background, with progress shown in the status bar — including a note when
the commit limit truncated the history. Scroll through the history; branch and tag chips mark the
commits the refs point at.

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

### In the terminal

`ocelli-cli [--ascii] [PATH]` opens the repository at `PATH` (default: the current directory)
in the terminal — the same rows, columns and edges as the desktop app, drawn with box-drawing
characters (`--ascii` for terminals without them). It only reads, and it never touches the network.

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
CONTRIBUTING.md       # how to contribute, pointing at the rules
SECURITY.md           # how to report a vulnerability, and what the configuration guarantees
CODE_OF_CONDUCT.md    # the standards everyone taking part is held to
LICENSE               # the license the project is released under
docs/SPECIFICATION.md # the specification: problem, goals, success criteria
docs/GLOSSARY.md      # the vocabulary everyone uses, kept current inline
docs/CONVENTIONS.md   # how this project writes what no check decides, kept current inline
docs/ARCHITECTURE.md  # the system as it currently stands
docs/adr/             # Architecture Decision Records (+ template)
docs/plan/            # open work: feature specs and their tickets, deleted as they land
crates/core/          # the core: commit graph, layout, and the repository port (ADR-0009)
crates/git/           # the repository port implemented with git2 (ADR-0019)
crates/tui/           # the terminal application, ocelli-cli (ADR-0028)
src-tauri/            # the desktop host (ADR-0018)
src-web/              # the web interface the desktop host shows (ADR-0022)
flatpak/              # the Flatpak manifest of the release pipeline (ADR-0025)
scripts/check-all.sh  # runs the checks, sensors, and self-tests, as the hooks and CI do
scripts/check-*.sh    # checks: each fails on what breaks a rule
scripts/sensor-*.sh   # sensors: report for the reviewer; sensor-repo-settings.sh runs by hand
scripts/test-*.sh     # self-tests of the checks, the sensors, the hooks, the setup, and the scripts
                      # run by hand
scripts/lib/          # what the scripts share: the ADR header, git, the ticket path, the self-tests
scripts/*.sh          # run by hand, steps 7 and 8 of the integration branches: number-adrs.sh,
                      # ready-for-main.sh, drop-merged-branch.sh
.githooks/            # git hooks: nothing lands on main locally; checks before a merge and a push
.github/workflows/    # checks.yml runs the checks; review-binding.yml reads each pull request's
                      # hand-off; release.yml releases on a tag
.github/              # Dependabot, issue & pull request templates, code owners
.devcontainer/        # Dev Container definition (Dockerfile + Features)
.vscode/              # shared editor settings
.editorconfig         # editor-neutral formatting baseline
.gitattributes        # line-ending normalization (LF everywhere), the ADR index merged by its driver
.gitignore            # what git leaves out, .scratch/ for session artifacts among it
.agents/skills/       # the procedures of AGENTS.md as Agent Skills, one directory each (ADR-0003),
                      # with the scripts and templates they carry (claim.sh, remap.sh, ticket.md)
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

The procedures those rules name (proposing an ADR, planning a feature, working its tickets
alone or in parallel, reviewing a change, revising the design, normalizing the record) are Agent
Skills under [`.agents/skills/`](.agents/skills/), the open format and the directory Codex,
Gemini CLI, and Cursor read
([ADR-0003](docs/adr/0003-procedures-as-skills.md)). An agent invokes one by name (`/review`) or
picks it up when the task matches its description. Claude Code reads `.claude/skills/` only, so
that directory holds one symlink per skill into `.agents/skills/`; a directory-level symlink is
not followed. The symlinks are pointers of the same kind as `CLAUDE.md`, tracked upstream as
[anthropics/claude-code#31005](https://github.com/anthropics/claude-code/issues/31005). Delete them
when that lands, and add one for every skill added meanwhile; the documentation check fails on a
skill without its pointer. What enforces a rule is what every agent hits
([`AGENTS.md` §6](AGENTS.md#6-project-rules)): the git hooks under [`.githooks/`](.githooks/),
the checks, and the repository settings below.

Agents work tickets in parallel without any one vendor's mechanism: a dispatching session claims
each ticket by its branch in a git worktree of its own and starts a fresh session there, with the
non-interactive command of whichever agent the project's harness ADR names, and the session works
the ticket through the `work-tickets` skill.

## Git hooks

Git does not version its hooks, so the repository keeps them under [`.githooks/`](.githooks/).
They hold for every agent and every human. The Dev Container enables them, with the merge
settings they rely on, on every start; outside it, run once per clone:

```sh
git config core.hooksPath .githooks
git config merge.ff false
git config pull.ff only
git config merge.adr-index.driver '.githooks/merge-adr-index %O %A %B %S %X %Y'
```

`merge.ff false` makes every merge a merge commit, as a `feat/` branch keeps them
([`AGENTS.md` §6](AGENTS.md#6-project-rules)); a fast-forward would make no commit and run no
hook. `pull.ff only` keeps a pull from then making a merge. Without the two settings the hooks
still hold: a fast-forward skips the documentation check of `pre-merge-commit`, and the push
still gates what it brought. The merge driver of the ADR index keeps the rows both sides add and
stops on its prose ([`.githooks/merge-adr-index`](.githooks/merge-adr-index)); without it, every
row two branches append conflicts.

| Hook | Refuses | Bypassed by |
|---|---|---|
| `reference-transaction` | moving the local `main` to a commit `origin/main` lacks: by a commit, merge, cherry-pick, reset, or `git branch -f`. It lets through `git merge --ff-only origin/main`, a reset to it, `git fetch origin main:main`, a write of `main` where it already is, and a deletion. Needs git 2.28 or later. | running git without the hooks (`git -c core.hooksPath=/dev/null`), or moving `origin/main` by hand; not by `--no-verify` |
| `pre-commit` | a commit on `main`; the conclusion of a conflicted merge while `scripts/check-all.sh --changed` is red | `--no-verify` |
| `pre-merge-commit` | a merge on `main`; a merge into any other branch while `scripts/check-docs.sh` is red | `--no-verify` |
| `pre-push` | a push to the remote's `main`, a deletion included; any other push while `scripts/check-all.sh --changed` is red, except one that only deletes, and one from a clean working tree whose tree the checks passed on before, as after a rework that only regroups | `--no-verify`, as the claim on a ticket does ([`AGENTS.md` §6](AGENTS.md#6-project-rules)); a tree written by hand into `checks-green-trees` of the git directory |

With `--changed`, the self-tests of what the branch left untouched since `main` are skipped; CI
runs every one before anything reaches `main`. A bypass is a deliberate act, not a default;
[`AGENTS.md` §6](AGENTS.md#6-project-rules) says when an agent may. Each refusal says what to do
instead.
[`scripts/test-git-hooks.sh`](scripts/test-git-hooks.sh) exercises every row.

## Integration branches

A change reaches `main` through a pull request. Several branches may integrate first on a
`feat/` branch, by local merges or by pull requests against it, and it stays the human's to
rework until its pull request into `main` merges ([`AGENTS.md` §6](AGENTS.md#6-project-rules)).
Each step waits for a human's instruction.

1. Start it at the remote's `main`: `git fetch origin`, then
   `git switch -c feat/<topic> origin/main`.
2. Merge a reviewed branch into it with its hand-off as the message, written to a file since an
   editor strips its headings: `git merge --no-ff -F .scratch/<hand-off>.md <branch>`. The
   documentation check gates the merge, and the push of step 6 runs the rest. One that stops on a
   conflict is concluded, once resolved and staged, with `git commit -F .scratch/<hand-off>.md`,
   and all the checks gate that commit.
3. Or merge it by a pull request against the `feat/` branch, so that its pipelines run first
   ([`AGENTS.md` §6](AGENTS.md#6-project-rules)). With both branches pushed, on the branch:
   `gh pr create --base feat/<topic> --body-file .scratch/<hand-off>.md`, and once it is green,
   `gh pr merge <number> --merge --body-file .scratch/<hand-off>.md`, never by the web interface,
   whose merge commit drops the hand-off. Then, on the `feat/` branch, `git fetch origin` and
   `git merge --ff-only origin/feat/<topic>`.
4. Rework it as often as needed: rebase it, regroup its commits, or drop what did not hold,
   with `git rebase -i --autosquash --rebase-merges`. `--autosquash` folds in the fixups a review
   leaves, as the `review` skill says; without `--rebase-merges` the merges flatten, and their
   hand-offs with them. A pull request still open against the `feat/` branch is rebased onto the
   rework.
5. After a rework, review the whole branch at its head
   ([`AGENTS.md` §5](AGENTS.md#5-quality-bar--definition-of-done)) and name that commit on the
   `Reviewed at:` line of the pull request of step 6, which
   [`review-binding.yml`](.github/workflows/review-binding.yml) reads on every push and edit.
6. Test it in the pipelines: `git push -u origin feat/<topic>`, then open a draft pull request,
   `gh pr create --draft --base main`, which runs them on every push against what the merge
   into `main` would produce. A branch kept without a pull request, such as an experiment, runs
   them by hand: `gh workflow run checks.yml --ref feat/<topic>`. A rework already pushed goes out
   with `git push --force-with-lease`.
7. Make the pull request ready: [`scripts/ready-for-main.sh`](scripts/ready-for-main.sh) refuses
   a fixup steps 4 to 6 left, takes up the remote's `main`, and numbers the draft ADRs
   ([ADR-0002](docs/adr/0002-decisions-verified-by-tests.md)); then the push it names and
   `gh pr ready`. What `main` gains later is taken up the same way, and an ADR it gains moves the
   branch's numbers on ([mechanics](docs/adr/README.md#mechanics)).
8. Once the pull request into `main` has merged as a merge commit, take it up and drop the branch:
   on it, [`scripts/drop-merged-branch.sh`](scripts/drop-merged-branch.sh) refuses a branch
   `origin/main` lacks, takes up the remote's `main`, and deletes the local branch; then the
   `git push origin --delete feat/<topic>` it names.

## Repository settings

Some of this project's rules cannot be enforced by files in the repository
([`AGENTS.md` §6](AGENTS.md#6-project-rules)). They are settings on the GitHub repository itself,
configured once by a maintainer and worth re-checking after a repository move or transfer:

- a **ruleset on `main`** that requires pull requests and allows the merge method **merge commit**
  only, since squash and rebase discard the merges a `feat/` branch carries and the commits it
  was reworked into ([`AGENTS.md` §6](AGENTS.md#6-project-rules)); requires the
  <!-- required-checks begin — compared with the workflow's jobs by scripts/check-docs.sh -->
  `docs`, `traceability`, `devcontainer`, `sensors`, `shell`, and `build`
  <!-- required-checks end -->
  jobs of the **Checks** workflow as required status checks (rulesets list checks as they report:
  a job by its name, a job with a matrix by one check run per entry, such as
  `build (ubuntu-latest)`, which this list then names instead of the job) and a branch up to date
  with `main` before it merges, so that the checks see what the merge produces, such as two ADRs
  proposed in parallel under one number, and blocks force pushes and branch deletion;
- enable **require a review from Code Owners** on that ruleset, so the two artifacts only a human
  decides, [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md) and everything under
  [`docs/adr/`](docs/adr/) ([`AGENTS.md` §3](AGENTS.md#3-adr-rules)), cannot be merged without the
  owner named in [`.github/CODEOWNERS`](.github/CODEOWNERS), which has to carry a real handle or
  team for the requirement to mean anything;
- enable **secret scanning with push protection**, the only mechanical backstop behind the secrets
  rule ([`AGENTS.md` §7](AGENTS.md#7-secrets)), which is otherwise carried by review alone;
- enable **private vulnerability reporting** (see [`SECURITY.md`](SECURITY.md)).

[`scripts/sensor-repo-settings.sh`](scripts/sensor-repo-settings.sh) reads these settings
through `gh` and reports each one that differs from this list. It runs on a human's
instruction ([ADR-0002](docs/adr/0002-decisions-verified-by-tests.md)).

## Template

- **Template release:** v0.6.0 — the release of [NUC](https://github.com/hivevm/nuc) this
  repository carries; a project moves the line when it takes up a later one
  ([ADR-0005](docs/adr/0005-template-releases.md)).

## Contributing

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the workflow (specification- and ADR-driven, small
reviewable changes) and [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) for the community standards we
expect of everyone taking part. Security issues: please follow [`SECURITY.md`](SECURITY.md) instead
of opening a public issue.

## License

Released under the MIT License. See [`LICENSE`](LICENSE).

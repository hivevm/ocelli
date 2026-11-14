# Contributing

Thanks for your interest in contributing. This project is **specification- and ADR-driven**. The
working rules in [`AGENTS.md`](AGENTS.md) are the single source of truth, and they govern coding
agents and human contributors alike. This file carries no rules of its own: it points at them and
adds only the contributor-facing procedure and the mechanics the rule file delegates to it: what
the git hooks refuse, the steps of an integration branch, and the repository settings.

## Before you start

Read what [`AGENTS.md` §2](AGENTS.md#2-start-here) names before any non-trivial work, then
[`AGENTS.md`](AGENTS.md) itself, the working rules for humans and agents alike, and
[`docs/CONVENTIONS.md`](docs/CONVENTIONS.md), how this project writes code and tests where no
check decides it.

Authority runs **specification → accepted ADRs → task**
([NUC-decisions-are-proposed-then-accepted §7](docs/adr/NUC-decisions-are-proposed-then-accepted.md)).

## Workflow

1. **Settle scope and intent first** when they are not yet agreed
   ([`AGENTS.md` §4](AGENTS.md#4-working-style)).
2. **Follow [`AGENTS.md`](AGENTS.md)**: when a decision needs an ADR and how it is reviewed
   ([NUC-decisions-are-proposed-then-accepted](docs/adr/NUC-decisions-are-proposed-then-accepted.md)),
   when a change counts as done ([§5](AGENTS.md#5-quality-bar--definition-of-done)), and the rules
   of integration ([NUC-main-only-via-pr](docs/adr/NUC-main-only-via-pr.md)). Run
   [`scripts/check-all.sh`](scripts/check-all.sh) locally before pushing
   ([§5](AGENTS.md#5-quality-bar--definition-of-done)). It runs the same checks CI runs on every
   pull request, and the repository's git hooks run them for you ([Git hooks](#git-hooks)). They
   need bash 4.4 or later and coreutils, already in the Dev Container. Only the shell lint
   additionally needs [ShellCheck](https://www.shellcheck.net), which the Dev Container installs;
   elsewhere it skips itself where that is missing, and CI runs it always. On macOS, whose bash
   is 3.2, install a current one, such as Homebrew's. On Windows they run under Git Bash in a
   clone made with `git clone -c core.symlinks=true`, so that `.claude/skills/` holds links and
   not copies; links need the Developer Mode of Windows. `ln -s` makes a link there under
   `MSYS=winsymlinks:nativestrict`; without such a setting it copies, and the documentation check
   refuses the copy.
3. **Open a pull request** against `main` and fill in the PR template.

## Project Layout

```
README.md             # overview & setup for those who use the project
AGENTS.md             # single source of truth for coding agents
CONTRIBUTING.md       # how to work on the project: this file, pointing at the rules
SECURITY.md           # how to report a vulnerability, and what the configuration guarantees
CODE_OF_CONDUCT.md    # the standards everyone taking part is held to
LICENSE               # the license the project is released under
docs/SPECIFICATION.md # the specification: problem, goals, success criteria
docs/GLOSSARY.md      # the vocabulary everyone uses, kept current inline
docs/CONVENTIONS.md   # how this project writes what no check decides, kept current inline
docs/ARCHITECTURE.md  # the system as it currently stands
docs/adr/             # Architecture Decision Records (+ template): the template's NUC- base,
                      # held by its blob list NUC.blobs, and the project's own
docs/plan/            # open work: feature specs and their tickets, deleted as they land
Cargo.toml            # the Cargo workspace of the crates and the desktop host (ARCH-rust-core-typescript-web)
Cargo.lock            # the locked Rust dependencies
rust-toolchain.toml   # the pinned Rust toolchain and its components (ARCH-rust-core-typescript-web)
.cargo/               # Cargo's build configuration: linkers and the development profile
package.json          # the web interface's dependencies and scripts, run by Bun (ARCH-rust-core-typescript-web)
bun.lock              # the locked JavaScript dependencies
crates/commands/      # the commands both hosts of the web interface answer (API-websocket-per-tab)
crates/core/          # the core: commit graph, layout, and the repository port (ARCH-ports-and-adapters)
crates/git/           # the repository port implemented with git2 (GIT-git2-reads-repositories)
crates/tui/           # the terminal application, ocelli-cli (ARCH-ratatui-terminal-application)
crates/server/        # the remote server, ocelli-server: the web interface in a browser (API-websocket-per-tab)
crates/measure/       # ocelli-measure: edge crossings on real repositories (Q-1); with
                      # scripts/measure-memory.sh (Q-3) and scripts/measure-mutants.sh (the git
                      # adapter's guards) run weekly by .github/workflows/measure.yml
src-tauri/            # the desktop host (ARCH-tauri-desktop-host)
src-web/              # the web interface the desktop host and the remote server show (ARCH-svelte-web-interface)
flatpak/              # the Flatpak manifest of the release pipeline (OPS-tag-builds-draft-release)
scripts/check-all.sh  # runs the checks, sensors, and self-tests, as the hooks and CI do
scripts/check-*.sh    # checks: each fails on what breaks a rule
scripts/sensor-*.sh   # sensors: report for the reviewer; sensor-repo-settings.sh,
                      # sensor-work-measures.sh, sensor-release-notes.sh, and
                      # sensor-red-commits.sh run by hand
scripts/test-*.sh     # self-tests of the checks, the sensors, the hooks, the setup, and the scripts
                      # run by hand
scripts/lib/          # what the scripts share: the bash floor (bash.sh), the ADR header (adr.sh),
                      # the findings of a check (check.sh), git (git.sh), the root a script
                      # reads (root.sh): every scripts/*.sh but the self-tests and check-all.sh
                      # takes it, as check-shell.sh checks; the plain rules (rules.sh), the
                      # self-tests (selftest.sh), the ticket path (ticket.sh)
scripts/fixtures/     # the cases of a check as a table, which its self-test reads
scripts/*.sh          # run by hand, steps 7 and 8 of the integration branches: ready-for-main.sh,
                      # drop-merged-branch.sh
.githooks/            # git hooks: nothing lands on main locally; checks before a commit, a merge,
                      # and a push
.github/workflows/    # checks.yml runs the checks; review-binding.yml reads each pull request's
                      # hand-off; measure.yml measures weekly; release.yml releases on a tag
.github/              # Dependabot, issue & pull request templates, code owners
.devcontainer/        # Dev Container definition (Dockerfile + Features)
.vscode/              # shared editor settings
.editorconfig         # editor-neutral formatting baseline
.gitattributes        # line-ending normalization (LF everywhere), the ADR index merged by its driver
.gitignore            # what git leaves out, .scratch/ for session artifacts among it
.agents/skills/       # the procedures of AGENTS.md as Agent Skills, one directory each,
                      # with their scripts, templates, and maps (claim.sh, remap.sh, ticket.md)
.claude/CLAUDE.md     # pointer for Claude Code to read AGENTS.md
.claude/skills/       # pointers (one symlink per skill) for Claude Code to read .agents/skills/
.claude/settings.json # Claude Code permissions: deny reading .env files (see SECURITY.md)
```

A script `scripts/*.sh` of the project, other than its self-tests, takes its root as the
template's do ([NUC-checks-are-bash §9](docs/adr/NUC-checks-are-bash.md)): it reads its own
repository, or another one only as `--root <dir>`, through `enter_root` of `scripts/lib/root.sh`,
and `rest_args` refuses an argument it does not read. The rule `shell/root` of
`scripts/check-shell.sh` fails on one that does not. The rule `shell/mode` fails on a tracked
script `scripts/*.sh` or `.agents/skills/<name>/*.sh` whose mode in the index is not 100755
([NUC-checks-are-bash §9](docs/adr/NUC-checks-are-bash.md)).

## Dev Container

The environment is defined in [`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json)
and the [`Dockerfile`](.devcontainer/Dockerfile) it builds from, which adds the desktop host's
system libraries to the base image, with Dev Container Features and VS Code extensions layered on
top ([NUC-container-hides-host-daemon](docs/adr/NUC-container-hides-host-daemon.md),
[DEV-container-from-dockerfile](docs/adr/DEV-container-from-dockerfile.md)). Customise the
environment by adding Features or extensions. Features are pinned by major tag and resolved in the
committed `devcontainer-lock.json`. Extensions are listed by identifier only, because a published
extension version cannot be repointed and a version suffix is VS Code-specific.

### Host configuration the container takes up

The container mounts the host's `~/.claude` and `~/.config/gh`, so the agent's login and the `gh`
login survive a rebuild; VS Code also copies the host's `~/.gitconfig` into it. **`gh` shares the
host's login only where the host keeps the token in that directory.** `gh` keeps it in the system
keyring by default; the container cannot reach the keyring and logs in on its own. To share the
host's login, log in on the host once with `gh auth login --insecure-storage`, which stores the
token in plain text in `~/.config/gh/hosts.yml`. The container can then read every `gh` login of the
host and write its configuration ([SECURITY.md](SECURITY.md)). On Windows the host's `gh` keeps its
configuration under `%AppData%`, so nothing is shared there.

### Host container management

The Dev Container deliberately has **no access to the host Docker daemon**: the socket is not
mounted ([NUC-container-hides-host-daemon §2](docs/adr/NUC-container-hides-host-daemon.md)).
To manage the host's containers from VS Code, run the **Container Tools** extension
(`ms-azuretools.vscode-containers`) on the **host** side: install it in your host VS Code.
[`.vscode/settings.json`](.vscode/settings.json) already pins it to run locally via
`remote.extensionKind`, so it keeps talking to the host engine even when this folder is reopened in
the container.

## Coding Agents

This Dev Container preinstalls the **Claude Code** VS Code extension (see
[`.devcontainer/devcontainer.json`](.devcontainer/devcontainer.json)); other agents (OpenAI Codex,
Cursor, OpenCode, GitHub Copilot, Mistral Vibe) work too once you add them.

The rules every agent follows live in [`AGENTS.md`](AGENTS.md); that there is exactly one such file
is decided in [NUC-documents-are-fixed §3](docs/adr/NUC-documents-are-fixed.md). An agent that
cannot read it natively gets a pointer file instead. [`.claude/CLAUDE.md`](.claude/CLAUDE.md) is
the one shipped here; it carries the pointer and no rules of its own
([NUC-documents-are-fixed §7](docs/adr/NUC-documents-are-fixed.md)). It exists for one gap only:
Claude Code loads `CLAUDE.md`, not `AGENTS.md`, tracked upstream as
[anthropics/claude-code#34235](https://github.com/anthropics/claude-code/issues/34235). Once that
lands, delete the pointer file rather than keeping a second file to maintain.

The procedures those rules name (proposing an ADR, planning a feature, working its tickets
alone or in parallel, reviewing a change, revising the design, normalizing the record) are Agent
Skills under [`.agents/skills/`](.agents/skills/), the open format and the directory Codex,
Gemini CLI, and Cursor read
([NUC-procedures-are-skills §1](docs/adr/NUC-procedures-are-skills.md)). An agent invokes one by
name (`/review`) or picks it up when the task matches its description. Claude Code reads
`.claude/skills/` only, so that directory holds one symlink per skill into `.agents/skills/`; a
directory-level symlink is not followed. The symlinks are pointers of the same kind as `CLAUDE.md`,
tracked upstream as
[anthropics/claude-code#31005](https://github.com/anthropics/claude-code/issues/31005). Delete them
when that lands, and add one for every skill added meanwhile; the documentation check fails on a
skill without its pointer. What enforces a rule is what every agent hits
([NUC-main-only-via-pr §7](docs/adr/NUC-main-only-via-pr.md)): the git hooks under
[`.githooks/`](.githooks/), the checks, and the repository settings below.

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
([NUC-main-only-via-pr §5](docs/adr/NUC-main-only-via-pr.md)); a fast-forward would make no commit
and run no hook. `pull.ff only` keeps a pull from then making a merge. Without the two settings the
hooks still hold: a fast-forward skips the documentation check of `pre-merge-commit`, and the push
still gates what it brought. The merge driver of the ADR index keeps the rows both sides add and
stops on its prose ([`.githooks/merge-adr-index`](.githooks/merge-adr-index)); without it, every row
two branches append conflicts.

| Hook | Refuses | Bypassed by |
|---|---|---|
| `reference-transaction` | moving the local `main` to a commit `origin/main` lacks: by a commit, merge, cherry-pick, reset, or `git branch -f`. It lets through `git merge --ff-only origin/main`, a reset to it, `git fetch origin main:main`, a write of `main` where it already is, and a deletion. Needs git 2.28 or later. | running git without the hooks (`git -c core.hooksPath=/dev/null`), or moving `origin/main` by hand; not by `--no-verify` |
| `pre-commit` | a commit on `main`; any other commit while `scripts/check-docs.sh` is red on the working tree; the conclusion of a conflicted merge while `scripts/check-all.sh --changed` is red | `--no-verify` |
| `pre-merge-commit` | a merge on `main`; a merge into any other branch while `scripts/check-docs.sh` is red | `--no-verify` |
| `pre-push` | a push to the remote's `main`, a deletion included; any other push while `scripts/check-all.sh --changed` is red, except one that only deletes, and one from a clean working tree whose tree the checks passed on before, as after a rework that only regroups | `--no-verify`, as the claim on a ticket does ([NUC-main-only-via-pr §6](docs/adr/NUC-main-only-via-pr.md)); a tree written by hand into `checks-green-trees` of the git directory |

With `--changed`, the self-tests of what the branch left untouched since `main` are skipped; CI runs
every one before anything reaches `main`. A bypass is a deliberate act, not a default;
[NUC-main-only-via-pr §6](docs/adr/NUC-main-only-via-pr.md) says when an agent may. Each refusal
says what to do instead.
[`scripts/test-git-hooks.sh`](scripts/test-git-hooks.sh) exercises every row.

## Integration branches

A change reaches `main` through a pull request. Several branches may integrate first on a
`feat/` branch, by local merges or by pull requests against it, and it stays the human's to
rework until its pull request into `main` merges
([NUC-main-only-via-pr §5](docs/adr/NUC-main-only-via-pr.md)). Each step waits for a human's
instruction ([NUC-main-only-via-pr §2](docs/adr/NUC-main-only-via-pr.md)).

1. Start it at the remote's `main`: `git fetch origin`, then
   `git switch -c feat/<topic> origin/main`.
2. Merge a reviewed branch into it with its hand-off as the message, written to a file since an
   editor strips its headings: `git merge --no-ff -F .scratch/<hand-off>.md <branch>`. The
   documentation check gates the merge, and the push of step 6 runs the rest. One that stops on a
   conflict is concluded, once resolved and staged, with `git commit -F .scratch/<hand-off>.md`,
   and all the checks gate that commit.
3. Or merge it by a pull request against the `feat/` branch, so that its pipelines run what the
   local gate cannot, such as another platform or a service
   ([NUC-main-only-via-pr §5](docs/adr/NUC-main-only-via-pr.md)). With both branches pushed, on
   the branch: `gh pr create --base feat/<topic> --body-file .scratch/<hand-off>.md`, and once it
   is green, `gh pr merge <number> --merge --body-file .scratch/<hand-off>.md`, never by the web
   interface, whose merge commit drops the hand-off. Then, on the `feat/` branch,
   `git fetch origin` and `git merge --ff-only origin/feat/<topic>`.
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
   a fixup steps 4 to 6 left and takes up the remote's `main`; then the push it names and
   `gh pr ready`. What `main` gains later is taken up the same way.
8. Once the pull request into `main` has merged as a merge commit, take it up and drop the branch:
   on it, [`scripts/drop-merged-branch.sh`](scripts/drop-merged-branch.sh) refuses a branch
   `origin/main` lacks, takes up the remote's `main`, and deletes the local branch; then the
   `git push origin --delete feat/<topic>` it names.

## Cutting a release

A human cuts a release
([NUC-releases-are-semver-tags §2](docs/adr/NUC-releases-are-semver-tags.md)). Its notes say what
changed and what a derived project does to take it up. Every hand-off says the latter under
Doubts, in the field "A derived project acts on".

1. Open a pull request into `main` that moves the `Template release` line of the README to
   `vX.Y.Z`. The maintainer picks the version by
   [NUC-releases-are-semver-tags §3](docs/adr/NUC-releases-are-semver-tags.md).
2. Once it has merged, run
   [`scripts/sensor-release-notes.sh`](scripts/sensor-release-notes.sh) on `main`, after
   `git fetch --tags origin`. It lists that field for each merge since the last tag `v*`, from
   the pull request's description or the merge message. `--since <tag>` names another tag. The
   field of a `feat/` branch's pull request and those of its tickets may overlap; both print
   unless their words are the same.
3. Write the notes from what changed and from that list. Where the release merges, splits, or
   removes a `NUC-` ADR, the notes carry the map from each old identifier and clause to its new
   home ([NUC-releases-are-semver-tags §7](docs/adr/NUC-releases-are-semver-tags.md)).
4. Tag the merge commit of the pull request of step 1 with those notes as its annotation:
   `git tag -a vX.Y.Z -F .scratch/<notes>.md <merge>`, then `git push origin vX.Y.Z`.
5. Build the GitHub release from that tag, with the same notes.

The sensor reads GitHub through `gh` and runs on a human's instruction
([NUC-checks-are-bash §7](docs/adr/NUC-checks-are-bash.md)). A derived project that ships
releases of its own may run the sensor for their notes.

## Repository settings

Some of this project's rules cannot be enforced by files in the repository
([NUC-main-only-via-pr §7](docs/adr/NUC-main-only-via-pr.md)). They are settings on the GitHub
repository itself, configured once by a maintainer and worth re-checking after a repository move
or transfer:

- a **ruleset on `main`** that requires pull requests and allows the merge method **merge commit**
  only, since squash and rebase discard the merges a `feat/` branch carries and the commits it
  was reworked into ([NUC-main-only-via-pr §4](docs/adr/NUC-main-only-via-pr.md)); requires the
  <!-- required-checks begin — compared with the workflow's jobs by scripts/check-docs.sh -->
  `docs`, `traceability`, `devcontainer`, `sensors`, `shell`, `build (ubuntu-latest)`,
  `build (macos-latest)`, and `build (windows-latest)`
  <!-- required-checks end -->
  jobs of the **Checks** workflow as required status checks (rulesets list checks as they report:
  a job by its name, a job with a matrix by one check run per entry, such as
  `build (ubuntu-latest)`, which this list then names instead of the job) and a branch up to date
  with `main` before it merges, so that the checks see what the merge produces, such as two ADRs
  proposed in parallel under one identifier, and blocks force pushes and branch deletion;
- enable **require a review from Code Owners** on that ruleset, so the two artifacts only a human
  decides, [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md) and everything under
  [`docs/adr/`](docs/adr/)
  ([NUC-decisions-are-proposed-then-accepted §5](docs/adr/NUC-decisions-are-proposed-then-accepted.md)),
  cannot be merged without the owner named in [`.github/CODEOWNERS`](.github/CODEOWNERS), which has
  to carry a real handle or team for the requirement to mean anything;
- enable **secret scanning with push protection**, the only mechanical backstop behind the secrets
  rule ([`AGENTS.md` §7](AGENTS.md#7-secrets)), which is otherwise carried by review alone;
- enable **private vulnerability reporting** (see [`SECURITY.md`](SECURITY.md)).

[`scripts/sensor-repo-settings.sh`](scripts/sensor-repo-settings.sh) reads these settings
through `gh` and reports each one that differs from this list. It runs on a human's
instruction ([NUC-checks-are-bash §7](docs/adr/NUC-checks-are-bash.md)).

## Reporting security issues

Do **not** open a public issue for vulnerabilities. Follow [`SECURITY.md`](SECURITY.md).

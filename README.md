# NUC — an Agentic, Specification-Oriented Starter Template

[![Checks](https://github.com/hivevm/nuc/actions/workflows/checks.yml/badge.svg)](https://github.com/hivevm/nuc/actions/workflows/checks.yml)
[![Release](https://img.shields.io/github/v/release/hivevm/nuc)](https://github.com/hivevm/nuc/releases/latest)
[![License](https://img.shields.io/github/license/hivevm/nuc)](LICENSE)

**NUC** is named after the small *nucleus colony* a full hive grows from in beekeeping. It is a
starting point for building software **with coding agents** inside a ready-to-use Dev Container.
The work is driven by a written **specification**
([`docs/SPECIFICATION.md`](docs/SPECIFICATION.md)) and **Architecture Decision Records**
([`docs/adr/`](docs/adr/)), so intent and the reasoning behind every structural choice stay
explicit and reviewable.

> For agent instructions, see [`AGENTS.md`](AGENTS.md), the single source of truth for all coding
> agents.

> [!NOTE]
> **This project still carries its template setup.** The one-time steps that turn the scaffold
> into your own project are in [`TEMPLATE-SETUP.md`](TEMPLATE-SETUP.md); ask your agent to run the
> `setup-project` skill to be walked through them, one question at a time. Once you are through
> them, `scripts/test-template-setup.sh --close` removes that file and this note.

## Why NUC

NUC is a paved road for agentic coding: a repository template, not a tool to install, for how
humans and agents decide, review, and enforce. It ships no toolchain. A project starts from it
with what a codebase built by several humans and agents over years needs on day one:

- **One rule file for every agent.** [`AGENTS.md`](AGENTS.md) and the Agent Skills under
  [`.agents/skills/`](.agents/skills/) are read by Claude Code, Codex, Gemini CLI, Cursor, and
  others alike. No vendor's CLI or IDE is required.
- **Intent and decisions on record.** The specification is the constitution. Every structural
  choice is an ADR that a human accepts, and a normalization keeps the record readable as it
  grows.
- **Rules with teeth.** Git hooks and CI refuse what breaks the record: an accepted ADR no test
  cites, a broken link, a placeholder left behind, a commit or push on `main`.
- **Proportionate.** A small project takes a short path through the setup. A complex one grows
  to tickets worked by agents in parallel, each in a worktree of its own.

How it compares with other spec-driven approaches, as of October 2026:

| | NUC | [Spec Kit](https://github.com/github/spec-kit) | [Kiro](https://kiro.dev) | [OpenSpec](https://github.com/Fission-AI/OpenSpec) | [BMAD Method](https://github.com/bmad-code-org/BMAD-METHOD) |
|---|---|---|---|---|---|
| Form | repository template | CLI and templates | IDE | CLI | agent personas |
| Agents | any that reads `AGENTS.md` | many, by slash commands | its own | several | several |
| Unit of work | specification, ADRs, tickets | spec, plan, tasks per feature | requirements, design, tasks per feature | change proposal with spec deltas | PRD, architecture, stories |
| Decisions kept as | ADRs with a lifecycle | constitution and plans | design documents | specs, updated by each change | architecture document |
| Enforced by | git hooks and CI checks | the agent's commands | IDE hooks | a validate command | the agent's workflow |

Where another fits better: OpenSpec is lighter for small changes to existing code. Spec Kit and
Kiro generate the spec, plan, and tasks of a feature by command. NUC asks for more setup in
exchange for checks that hold for every agent and every human, and it assumes GitHub.

## Overview

Describe what this project does, who it is for, and its main goals. The full problem statement and
goals live in [`docs/SPECIFICATION.md`](docs/SPECIFICATION.md), the vocabulary in
[`docs/GLOSSARY.md`](docs/GLOSSARY.md).

## Prerequisites

- [VS Code](https://code.visualstudio.com/) with the
  [Dev Containers](https://marketplace.visualstudio.com/items?itemName=ms-vscode-remote.remote-containers)
  extension, or any DevContainer-compatible IDE
- Docker / Podman (rootless) available on the host
- Outside the Dev Container: bash 4.4 or later for the checks, 3.2 for the git hooks
  ([ADR-0002](docs/adr/0002-decisions-verified-by-tests.md)); macOS ships 3.2

## Getting Started

> Setting the project up for the first time? Create the repository from the template, with
> **Use this template** on GitHub or with
> `gh repo create <owner>/<name> --template hivevm/nuc --public --clone`. A private repository
> needs a paid GitHub plan for the rulesets of [Repository settings](#repository-settings).
> The one-time steps are in [`TEMPLATE-SETUP.md`](TEMPLATE-SETUP.md); the close of the setup
> removes this paragraph with that file.

1. Open the repository in VS Code and choose **Reopen in Container**. The Dev Container and the
   preconfigured agent extensions build automatically, and the container enables the repository's
   git hooks with the merge settings they rely on ([Git hooks](#git-hooks)). Working outside the
   container? Run the three commands listed there yourself, once per clone.
2. Authenticate your coding agent inside the container (for Claude Code: `claude login`).
3. Start working with the agent. Drive the work from the specification and the ADRs.

## Build, Test & Run

<!-- Fill in once the toolchain is chosen. This section is the single source for
     build/test/lint/run commands; both humans and agents rely on it, and the rule file names
     it. -->

- **Build:** TODO <!-- e.g. `make build` -->
- **Test:** TODO <!-- e.g. `make test` -->
- **Lint:** TODO <!-- e.g. `make lint` -->
- **Run:** TODO <!-- e.g. `make run` -->

## Usage

<!-- Once there is something to use, show how to use the built software: the primary commands or
     API, a minimal example, and the expected output. Keep build/test/run mechanics in the section
     above; this section is about using the result, not producing it. -->

TODO — show a minimal example of using the project.

## Project Layout

```
TEMPLATE-SETUP.md     # one-time template setup; delete it when the project is yours
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
scripts/check-all.sh  # runs the checks, sensors, and self-tests, as the hooks and CI do
scripts/check-*.sh    # checks: each fails on what breaks a rule
scripts/sensor-*.sh   # sensors: report for the reviewer; sensor-repo-settings.sh runs by hand
scripts/test-*.sh     # self-tests of the checks, the sensors, the hooks, the setup, and the scripts
                      # run by hand
scripts/lib/          # what the scripts share: the ADR header, git, the ticket path, the self-tests
scripts/*.sh          # run by hand, steps 7 and 8 of the integration branches: number-adrs.sh,
                      # ready-for-main.sh, drop-merged-branch.sh
.githooks/            # git hooks: nothing lands on main locally; checks before a merge and a push
.github/workflows/    # checks.yml runs the checks; review-binding.yml reads each pull request's hand-off
.github/              # Dependabot, issue & pull request templates, code owners
.devcontainer/        # Dev Container definition (base image + Features)
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
  `docs`, `traceability`, `devcontainer`, `sensors`, and `shell`
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

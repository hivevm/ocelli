# Specification — Ocelli

> The **specification** of this project: the problem it solves, where it is going, and what has to
> be true for it to have succeeded, written in the vocabulary of [`GLOSSARY.md`](GLOSSARY.md). It
> is the **constitution**: every Architecture Decision Record in [`adr/`](adr/) derives from it and
> must not contradict it ([ADR-0001](adr/0001-agent-governance-model.md)).

## Problem

Developers who work with Git need to see the shape of a repository's history: how branches
diverge, where they merge, and which commits a branch carries. Of the tools in wide use, GitKraken
draws the commit graph well enough to read that shape at a glance. GitKraken is not an option for
a broad audience: it is paid software, and it is built on Electron, which costs a desktop
application far more memory and CPU than the task needs. The free tools in wide use, the `git log
--graph` of the shell included, draw a graph that becomes hard to read once a history has more
than a few parallel branches. And no tool in wide use offers the same view in the three places
developers work: on the desktop, in a browser against a remote machine, and in a terminal.

It is worth solving now because there is a concrete idea for a layout that draws the whole commit
graph, every branch included, as readably as GitKraken does: each branch in a lane of its own, the
main branch as a straight line on the left, and few crossing edges.

## Mission

Give every developer a free, lightweight way to view and edit a Git repository through a
readable commit graph, on the desktop, in the browser, and in the terminal.

## Vision

A developer opens any repository, from a small project to one with a million commits, and reads
its whole history at a glance: every branch in its own lane, the main branch on the left, and the
places where work diverged and merged plain to see. From that commit graph they do their daily
Git work, and rework history where they need to, without switching to the shell.

The same Ocelli runs where the developer is: as a desktop application, as a web interface to a
repository on a remote machine, and as a terminal application in a shell. It is free to use, and
it starts and runs on a fraction of the resources an Electron application takes.

## Strategy

Lead with the commit graph. Ocelli starts from GitGraph, the read-only viewer this repository
built before: its layout, its look, and its behaviour carry over unchanged onto Ocelli's
architecture first, because the layout is what no free tool offers and GitGraph has proven it.
Editing and the remote server follow once the viewer stands on the new architecture.
Every front end offers the same commit graph and the same operations, so that a developer who
learns Ocelli in one place knows it in the others.

## Goals / Success Criteria

Acceptance criteria a test can decide, where it helps in the form *When <trigger>, the system
shall <response>*. Each starts with a bold `G-n`, which a test cites with `Verifies: G-n`. Until
that test exists the criterion is pending work, and the traceability check lists it
([ADR-0002](adr/0002-decisions-verified-by-tests.md)). Identifiers are never renumbered or reused.

1. **G-1** — When a repository is opened, the system shall show its commit graph with every
   branch and tag.
2. **G-2** — When the commit graph is drawn, the system shall keep each branch in one lane over the
   whole length it is shown.
3. **G-3** — When the commit graph is drawn, the system shall draw the main line as one straight
   line down the leftmost lane, and nothing else in that lane.
4. **G-4** — The system shall let the developer stage changes, commit, create, switch, and delete
   branches, and fetch, pull, and push.
5. **G-5** — The system shall let the developer merge, rebase, cherry-pick, revert, and reset from
   the commit graph.
6. **G-6** — When a merge, rebase, cherry-pick, or revert stops on a conflict, the system shall let
   the developer resolve it, and continue or abort the operation.
7. **G-7** — The system shall let the developer create, apply, and drop stashes, and create and
   delete tags.
8. **G-8** — The system shall offer G-1 to G-7 and G-9 to G-17 in each front end: the desktop
   application, the web interface, and the terminal application.
9. **G-9** — When the commit graph is drawn, the system shall draw an edge between lanes as an
   orthogonal L-shape whose bend follows its type, a branch edge bending at its parent's row and a
   merge edge at its child's row, a merge out of the main line running up a track of its own with
   a bend at each end, and shall route no edge through a commit it does not join, an edge down to
   a late child excepted.
10. **G-10** — When the commit graph is drawn, the system shall place every commit above its
    parents, except a late child, a commit whose committer date runs backwards against a parent
    laid out before it, which stands below that parent, and order commits by committer date where
    the topology leaves the order open, newest at the top.
11. **G-11** — When the commit graph is drawn, the system shall show each local branch, remote
    branch, and tag as a ref chip on the commit it points at.
12. **G-12** — When the developer selects a commit, the system shall show its SHA, message,
    author, date, parents, children, and changed files, and the diff of a changed file on request.
13. **G-13** — While a repository loads, or a fetch, pull, or push runs, the system shall keep
    responding to input and report its progress, and let the developer stop a fetch, and a pull
    while it fetches.
14. **G-14** — When the commit graph is drawn, the system shall draw only the rows in view and the
    edges that cross them.
15. **G-15** — When the same repository state is opened twice, the system shall assign the same
    rows, lanes, and edges both times, the working-directory node's row excepted.
16. **G-16** — When the developer searches, the system shall find every commit whose message,
    author, committer, SHA, or refs contain the search text, ignoring case, in row order.
17. **G-17** — When the developer selects a commit, the system shall set apart its lineage, the
    commits it descends from and those that descend from it, from every other commit.

## Quality Goals

The few properties that drive decisions, in priority order. Every ADR weighs its *Consequences*
against them. Each has a measure concrete enough to settle a disagreement and starts with a bold
`Q-n`, cited like a goal. Security goals, what the system must never do, belong here, derived
from the threats below.

- **Q-1** — Readability: the commit graph is what Ocelli exists for. Each branch keeps its lane
  (G-2), and the layout pays for that with edge crossings, never more than recorded for the corpus;
  a recorded count changes only with a decision about the layout. Measured on a fixed corpus of
  public repositories, each at a fixed commit of its main branch over the first chunk of its
  history: the layout draws no more edge crossings than the count recorded for that commit.
- **Q-2** — Performance: a repository of one million commits stays usable. Measured on such a
  repository: the first screen of the commit graph appears within 2 s of opening it, and
  scrolling never waits more than 100 ms for the next screen.
- **Q-3** — Resource footprint: being lighter than an Electron application is half of the
  problem. Measured on Linux on a repository of 100 000 commits: showing the commit graph adds less
  than 150 MB to the memory of the desktop application, its WebView's processes included, over the
  application with no repository open.
- **Q-4** — Security: working with a repository must not run code the repository brings.
  Measured on a repository whose configuration names programs (such as `core.fsmonitor`,
  `core.sshCommand`, `credential.helper`) and whose hooks are set: showing its commit graph,
  every write Ocelli makes to it, and fetching, pulling, and pushing it start none of them.
- **Q-5** — Security: the remote server hands control of a machine to whoever reaches it.
  Measured against a running remote server: every request without valid authentication is
  refused, the server accepts none from a web page of another origin, and no request reads
  anything outside the server root.
- **Q-6** — Safety: an operation that discards work cannot happen by accident. Measured for each
  such operation (a hard reset, dropping a stash, applying a stash and dropping it, deleting a
  tag, deleting a branch that is not merged, aborting an operation in progress, taking one side
  of a conflicted file): it runs only after the developer confirms it.
- **Q-7** — Security: a repository must not obtain the developer's credentials. Measured on a
  repository whose remote address and configuration are written to obtain them, a line break in
  the address's user name, an address of another host, a remote over plain HTTP, a credential
  helper of its own: no credential of the developer's leaves for another host or unencrypted,
  and the repository's own helper does not run.

## Non-Goals

What this project explicitly does not try to do.

- Hosting: no pull requests, issues, or code review. Ocelli works on the repository only.
- An editor or IDE: Ocelli shows diffs and does not edit files, except to resolve a conflict.
- Its own Git: no storage format and no extension of its own. Every repository Ocelli touches
  stays fully usable with the standard `git`.
- A shared server: the remote server serves one developer, not a team.

## Constraints

What the solution has to live with regardless of the better choice: platforms, standards,
systems to interoperate with, limits of time, budget, and team. A constraint not written here is
rediscovered as a rejected alternative in an ADR.

- Every front end runs on Linux, macOS, and Windows.
- Ocelli is free and released under the MIT License.
- The web interface works like VS Code's remote development: a remote server that is part of
  the environment runs next to the repository, and the browser talks to it.
- Ocelli authenticates against remote repositories through the developer's existing Git setup,
  the SSH agent and the credential helpers of the developer's global and system configuration,
  and keeps no credentials of its own.
- One maintainer.

## Threats & Forbidden Actions

The assets, the actors who threaten them, and the trust boundaries between the parts. Then what
the system must never do, regardless of what a task asks. A prohibition not written here does not
exist for an agent. One a test can decide is restated as a `Q-n`, so that it is verified.

- **Assets:** the repositories and their unpublished work, the developer's credentials for remote
  repositories (SSH keys, credential helpers), and the machine the remote server runs on.
- **Actors and trust boundaries:**
  - the developer, the one trusted actor;
  - a repository's content, untrusted: a cloned repository may carry configuration and hooks
    written by anyone;
  - remote repositories and the network to them, untrusted;
  - anyone who can reach the remote server over the network, and any web page open in the
    developer's browser, untrusted: the boundary runs between the browser and the remote server;
  - other users of the machine the remote server runs on, untrusted.
- The system shall never run a program a repository names (Q-4).
- The system shall never act on a request to the remote server that is not authenticated (Q-5).
- The system shall never let a browser read a repository or a file outside the server root (Q-5).
- The system shall never discard work without the developer's confirmation (Q-6).
- The system shall never store, log, or send a credential anywhere but to the remote repository
  it belongs to (Q-7).

## Assumptions

What is believed true today and would invalidate the decisions above if it turned out false:
expected scale, the shape of the data, who operates the result, which dependency stays
maintained.

- Repositories reach up to one million commits and a few thousand branches and tags.
- The developer has Git installed and configured, credentials included, on every machine
  Ocelli works on.
- The developer may start a long-running process under their own account on a remote machine,
  as VS Code's remote development does.
- Terminals Ocelli runs in show Unicode line drawing and 256 colours.
- Browsers are current versions of Chrome, Firefox, Safari, and Edge.

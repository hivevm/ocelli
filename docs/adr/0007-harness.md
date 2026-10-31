# ADR-0007: One agent implements, a fresh session reviews, and the maintainer reads every change

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** every change to the repository, who makes it, and who reviews it

## Context

Ocelli has one maintainer and is built with a coding agent.
[`AGENTS.md` §4](../../AGENTS.md#4-working-style) lets a task run unattended only within the
parts and the cap the harness ADR keeps with the human, and
[§5](../../AGENTS.md#5-quality-bar--definition-of-done) has a fresh session review every change
before it is handed off. Nothing yet says which agent works here, whether anything runs
without the maintainer, or which parts are never worked without the maintainer.

The specification's *Threats & Forbidden Actions* name three places where a wrong change does
real harm: the remote server and its authentication (Q-5), anything that could run code a
repository brings (Q-4), and the operations that discard work (Q-7). A one-person project
starts with the smallest harness and grows it when the work does.

## Decision

We will have one agent, Claude Code with an Opus model, implement every change, have the
`review` skill review it in a fresh session before the pull request opens, and have the
maintainer read every pull request, with nothing running unattended and no parallel sessions.

The parts never worked without the maintainer, however small the ticket: the remote server's
authentication and the origins it accepts, every path by which Ocelli starts a program, and every
operation that discards work.

**Out of scope:** a second agent or a reviewer of a different kind; the model version, which
follows the agent's current default; and the cap on unattended work, which is zero until an ADR
raises it.

## Alternatives considered

- **Implementer and reviewer apart** — a different agent or model as reviewer, and a second
  reviewer for the protected parts. Worth it once reviews miss what a second kind would catch;
  with one maintainer reading every pull request it adds a tool before it adds a finding.
- **Unattended tickets under a cap** — tickets run without the maintainer in worktrees of their
  own. Premature while the golden path and the structural tests do not exist yet: there is
  nothing an unattended session could be held to.
- **A process-owning framework** — brings agents, artifacts, and pipeline in one vendor's format;
  ADR-0003 and ADR-0006 decline it.

## Sources / Prior art

- Birgitta Böckeler, *Harness engineering* —
  <https://martinfowler.com/articles/harness-engineering.html>: guides and sensors, and committing
  early to what the sensors can hold.
- Addy Osmani, *Agentic autonomy levels* — <https://addyosmani.com/blog/agentic-autonomy-levels/>:
  autonomy follows how quickly a wrong result is noticed and how cleanly it is undone.

## Consequences

- Positive: every change passes two readers, the review session and the maintainer; the parts
  the threats protect are named, so a ticket that touches them says so.
- Negative / trade-offs: the maintainer's reading time is the throughput; nothing is
  parallelised.
- Follow-ups: when reviews pile up, a second reviewer of a different kind, or unattended tickets
  under a cap.

## Enforcement

**Not mechanically decidable:** who implements, who reviews, and whether a session ran
unattended happen outside the tree. What the harness relies on mechanically is checked
elsewhere: the git hooks under `.githooks/`, and the required checks and the Code Owners review
of the repository settings listed in the README.

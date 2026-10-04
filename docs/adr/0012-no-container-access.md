# ADR-0012: The work in the Dev Container reaches no container engine

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** `.devcontainer/` and the tests that need a service or a container

## Context

Ocelli's tests need Git repositories, which they create in temporary directories, and a remote
server, which runs as a process. No test needs a database, a service, or a container, so the
template's state, no engine and no socket, is the simplest that holds.

## Decision

We will keep the Dev Container without a container engine and without a socket, as the template
ships it.

**Out of scope:** nothing beyond the sentence above.

## Alternatives considered

None — cheap to reverse: an engine inside or a Compose file is added by superseding this ADR once
a test needs one.

## Sources / Prior art

None — cheap to reverse.

## Consequences

None — cheap to reverse.

## Enforcement

`scripts/check-devcontainer.sh` refuses a host socket mount (ADR-0004).

**Not mechanically decidable:** that no engine runs inside, because a Feature
or a package could add one under any name; review reads every change to `.devcontainer/`.

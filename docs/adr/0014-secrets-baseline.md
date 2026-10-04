# ADR-0014: Secrets are guarded by GitHub's secret scanning and push protection, with no scanner of the project's own

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** every secret the project's CI uses, and `SECURITY.md`

## Context

The specification names the developer's credentials as an asset and forbids Ocelli to store, log,
or send them; Ocelli reaches remote repositories through the developer's own Git setup and keeps
none. The project itself has one maintainer and, until installers are signed, no secret but the
`GITHUB_TOKEN` GitHub issues per run. The catalogue's baseline fits one developer with few
credentials in GitHub's scanning patterns.

## Decision

We will rely on GitHub's **secret scanning with push protection** as the repository setting, keep
the project's secrets as GitHub Actions secrets, and add no scanner of our own until a secret of a
shape GitHub does not recognise, such as an installer signing certificate, joins them.

**Out of scope:** how Ocelli treats the developer's credentials at runtime, which the
specification's threats, ADR-0035 for the remotes, and ADR-0037 for the remote server decide.

## Alternatives considered

- **gitleaks in CI from the start** — catches shapes GitHub does not know, such as a base64
  certificate; there is no such secret yet.
- **Encrypted files (sops) or a secret manager** — for configuration that travels with its
  secrets, or several environments; Ocelli has neither.

## Sources / Prior art

- GitHub, *About secret scanning* —
  <https://docs.github.com/en/code-security/secret-scanning/introduction/about-secret-scanning>.
- GitHub, *Remediating a leaked secret* —
  <https://docs.github.com/en/code-security/tutorials/remediate-leaked-secrets/remediating-a-leaked-secret>.
- gitleaks — <https://github.com/gitleaks/gitleaks>.

## Consequences

- Positive: nothing to run or maintain; push protection stops a recognised secret before it
  lands.
- Negative / trade-offs: a secret of an unrecognised shape is caught by review alone.
- Follow-ups: a scanner when signing secrets arrive.

## Enforcement

**Not mechanically decidable:** push protection is a repository setting outside the tree, listed
under *Repository settings* in the README, and no check in the repository can see whether it is
on; a secret of an unrecognised shape is review's to catch.

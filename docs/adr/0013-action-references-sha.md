# ADR-0013: GitHub Actions are pinned to a full commit SHA and kept current by Dependabot

- **Status:** 🟢 accepted
- **Date:** 2026-10-04
- **Deciders:** Markus Brigl
- **Applies to:** every `uses:` reference in `.github/workflows/`, and `.github/dependabot.yml`

## Context

The template references actions by major version tag. Ocelli will run third-party actions (a Bun
setup, a desktop bundler, a Flatpak builder, as the earlier code base on `main` did) in a release
pipeline whose token can write releases. A tag can be repointed; a repointed tag runs in CI before
anyone sees a diff, as with `tj-actions/changed-files` in 2025. The catalogue's entry fits a full
SHA to third-party actions, a release pipeline, or a token with write access, and Ocelli has all
three.

## Decision

We will reference every action by its **full commit SHA with the version as a comment**, and let
Dependabot's `github-actions` ecosystem update the pin and its comment.

**Out of scope:** local actions (`./…`), which are exempt; which actions the workflows use.

## Alternatives considered

- **Major version tag with Dependabot** — the template's default; fixes within a major arrive
  without a pull request, and so would a repointed tag.
- **Tags for `actions/*`, SHAs for the rest** — a third form; its check needs a list of trusted
  owners, and the release token is reachable from either.
- **SHA pins without an updater** — freezes security fixes out.

## Sources / Prior art

- GitHub, *Security hardening for GitHub Actions* —
  <https://docs.github.com/en/actions/security-for-github-actions/security-guides/security-hardening-for-github-actions>:
  a full SHA is the only immutable reference.
- GitHub Advisory GHSA-mrrh-fwg8-r2c3 / CVE-2025-30066 —
  <https://github.com/advisories/GHSA-mrrh-fwg8-r2c3>.
- GitHub, *Keeping your actions up to date with Dependabot* —
  <https://docs.github.com/en/code-security/dependabot/working-with-dependabot/keeping-your-actions-up-to-date-with-dependabot>.

## Consequences

- Positive: every change of the code CI runs is a reviewed diff.
- Negative / trade-offs: a Dependabot pull request for every release of every action; an SHA is
  unreadable without its comment.
- Follow-ups: zizmor's workflow audits, if the workflows grow.

## Enforcement

`scripts/sensor-action-refs.sh --require sha` runs in `scripts/check-all.sh` and in the `sensors`
job of `checks.yml`, beside a comment that cites this ADR with a `Verifies:` marker; it fails on
any other form and on a missing `github-actions` entry in `.github/dependabot.yml`.

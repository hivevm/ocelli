# Ticket: The next normalization carries what the merge editor's spec left for the record

<!-- Filed by the design revision of 2026-11-03, which closed docs/plan/merge-editor/. Decided in
     docs/adr/0004-feature-layer.md. -->

## What to build

When the human next instructs a normalization (skill `normalize`), the ADR that restates ADR-0031
carries four things the merge editor's spec and its tickets left for it, which no other document may
hold, since ADR-0031 is accepted and only a normalization rewrites it
([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 5):

1. **CodeMirror's "alone".** The maintainer answered on 2026-11-03 (commit 4914b37, in the spec now
   deleted): ADR-0031's "`@codemirror/state`, `@codemirror/view`, and `@codemirror/commands` alone"
   means the web interface's direct dependencies, as built. What `@codemirror/commands` brings in as
   its own is allowed: today `@codemirror/language`, `@codemirror/streamparser`, `@lezer/common`,
   `@lezer/highlight`, `@lezer/lr`, and the small packages `bun.lock` lists under them. The restated
   ADR says so in *Components* and lists the transitive packages under its *Sources*, read off
   `bun.lock` at the time. No supersession before then.
2. **No limit on the size of a file the editor opens.** Ticket merge-editor/06 measured it (commit
   970f6cf) in release builds on a 70,004-line file with one block and a 35,009-line file with 3,001
   blocks: the core's read 92 / 137 ms; the terminal application's key and frame about 26 / 9 ms;
   the WebView opening it in 262 / 431 ms, adding 180 / 210 MB. No interaction exceeded 50 ms, so
   the maintainer decided on no limit and no change to how often the blocks are found. A reader
   without that conversation would take the missing limit for an oversight
   ([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 2): the restated ADR records it under
   *Consequences* with the numbers.
3. **The text edited in place.** Commit 076d780 (the squashed tickets 01-04) changed the text of
   ADR-0031, accepted on 2026-10-31, together with the code: how a block's base is found from a run
   of blocks, and how the terminal application sets a base section aside. Both describe the system
   as built, but an accepted ADR is changed only by superseding it, and a commit that changes an
   ADR's text changes nothing else ([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rules 4 and
   5). The restatement is what makes that text the record's; the normalization's map names it.
4. **One decision per ADR.** ADR-0031 decides two things, its title says so: that every write is
   what `git` writes, and that a conflicted file's text is resolved in a merge editor of Ocelli's
   own, with its components ([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 1). The
   normalization weighs restating its section *The merge editor* as an ADR of its own, beside the
   one on writes, without changing either decision, and moves the `Verifies:` markers of the merge
   editor's tests by its map.

## Acceptance criteria

- [ ] Given the next normalization, when the ADR restating ADR-0031 lands, then its *Components*
      says the CodeMirror packages named are the direct dependencies, and its *Sources* list the
      transitive ones `bun.lock` holds.
- [ ] Given that ADR, when a reader looks for a size limit, then its *Consequences* say there is
      none and give the measurements above.
- [ ] Given the normalization, when the human decides whether the merge editor becomes an ADR of its
      own, then the decision is in the normalization's hand-off either way.

## Read first

- `.agents/skills/normalize/SKILL.md`
- docs/adr/0031-writes-what-git-writes.md, section *The merge editor* and *Components*
- `git show 4914b37 970f6cf` and `git show 076d780 -- docs/adr/0031-writes-what-git-writes.md`

## Blocked by

- None (can start immediately) — but it runs only inside a normalization the human instructs.

## Mode

with the human — a normalization runs only on a human's instruction
([`AGENTS.md` §3](../../../AGENTS.md#3-adr-rules), rule 5), and whether ADR-0031 splits is the
human's call.

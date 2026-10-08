# Glossary — Ocelli

> The **vocabulary** of this project: the terms everyone, humans and agents alike, uses in code,
> comments, documentation, and ADRs. It defines words and decides nothing. Whoever resolves a term
> adds it here in the same change ([ADR-0001](adr/0001-agent-governance-model.md)).

One term per concept, only terms specific to this domain. Say what a term *is* in one or two
sentences, and list under _Avoid:_ the synonyms it displaces, where there are any. A term the
specification, an ADR, or the code uses without an entry here is a gap to close, not a convention
to invent.

- **Branch edge** — an edge that leaves the lane of a commit to reach a child in another lane,
  where a branch forks off.
  _Avoid:_ fork line.
- **Chunk** — a run of up to 3 000 commits in the order the walk over a repository's refs yields
  them, the unit in which a history is loaded and laid out; each chunk is laid out below the ones
  before it.
  _Avoid:_ page, batch, slice.
- **Commit graph** — the commits of a repository drawn as nodes, each joined by an edge to its
  parents, with every branch and tag shown.
  _Avoid:_ tree, history graph, log graph.
- **Connection token** — the random secret the remote server prints at each start, in a URL,
  which a browser's first request exchanges for a session cookie.
  _Avoid:_ password, API key.
- **Conflict** — a file the index holds in more than one stage after an operation in progress
  stopped on it; resolved by taking ours, taking theirs, or as the developer edited it.
  _Avoid:_ clash, collision.
- **Default remote** — the remote a fetch reaches without one named, and a push of a branch
  without an upstream: the checked-out branch's upstream's remote, else `origin`, else the only
  remote there is.
  _Avoid:_ main remote, primary remote.
- **Departure** — a write whose outcome differs on purpose from what `git` does for the same
  command, an ADR deciding it.
  _Avoid:_ deviation, known difference.
- **Edge** — the line joining a commit to one of its children in the commit graph; a branch edge
  or a merge edge.
  _Avoid:_ link, connector.
- **Edge crossing** — a point where one line of the commit graph passes another from one side to
  the other, other than at a commit; a point counts once however many edges pass it, and two edges
  that cross at two points count twice. In Ocelli's layout it is where the horizontal segment of
  one edge meets the vertical segment of another strictly inside both. In the ASCII graph of
  `git log --graph`, where a line moves between even text columns with `/`, `\`, and `_`, it is a
  point on a row boundary in a column whose line runs on above and below, which a moving line
  passes from one side to the other; joining the column from one side, at a merge or a fork, is
  not a crossing. The count Q-1 compares.
  _Avoid:_ intersection, overlap.
- **Front end** — one of the three ways a developer uses Ocelli: the desktop application, the web
  interface, and the terminal application.
  _Avoid:_ client, UI, frontend.
- **History operation** — a merge, rebase, cherry-pick, revert, or reset of the checked-out
  branch, offered on a commit of the commit graph (G-5); one stopped on a conflict is an operation
  in progress.
  _Avoid:_ action.
- **Lane** — the column of the commit graph in which a branch is drawn.
  _Avoid:_ column, rail.
- **Lane colour** — the colour a lane is drawn in, taken from one theme-independent list so a
  commit keeps its colour across a theme switch; deliberately not a theme token
  ([ADR-0021](adr/0021-themes-tokens-and-palettes.md)).
  _Avoid:_ branch colour, since a branch is a ref and a lane outlives it.
- **Late child** — a commit the walk yields after its parent, because its committer date runs
  backwards against it; it is laid out in its chunk, below its parent.
- **Layout** — the rows, lanes, and edges the core computes for a loaded repository. A front end
  draws a layout and never computes one.
- **Lineage** — the commits a commit descends from and those that descend from it.
  _Avoid:_ ancestry, family.
- **Load session** — one pass of reading a repository and laying it out. Opening another repository
  starts a new one, and no result of the previous session reaches the new state.
- **Main branch** — the branch a repository treats as its line of record, usually `main` or
  `master`.
  _Avoid:_ trunk, default branch, master branch.
- **Main line** — the main branch's tip and each first parent down from it until one laid out
  before its child, drawn straight in the leftmost lane; where no main branch exists, the
  checked-out commit's.
- **Merge edge** — an edge that joins a parent in another lane to a merge commit, where a branch
  is merged.
  _Avoid:_ merge line.
- **Operation in progress** — a merge, rebase, cherry-pick, or revert stopped on a conflict, in
  `git`'s own state, whoever started it; continued or aborted once its conflicts are resolved.
  _Avoid:_ stopped write, pending merge.
- **Parent row** — in the sidebar, the row above the sections that leads out of a submodule or a
  linked working tree to the repository one level up, read from Git and never remembered from the
  way in. Not a commit's parent.
- **Rasteriser** — the terminal application's counterpart of the canvas painter: it turns a layout
  into a grid of glyphs and colours.
- **Ref chip** — the label on a commit in the commit graph that names a branch, remote branch, or
  tag pointing at it.
  _Avoid:_ badge, tag label.
- **Ref column** — the column of ref chips left of the commit graph, hugging the lane of the commit
  each chip marks; surplus chips on one row collapse into a `+N` badge.
- **Remote server** — the part of Ocelli that runs on the machine holding a repository and serves
  the web interface to the developer's browser, as VS Code's remote development does.
  _Avoid:_ backend, daemon, agent.
- **Server root** — the folder the remote server is started with; a browser opens only the
  repositories below it.
  _Avoid:_ workspace, base directory.
- **Stash node** — the node of the commit graph that stands for one entry of the stash, joined to
  the commit it was made on, and carrying its place `n` of `stash@{n}`.
  _Avoid:_ stash commit, stash entry node.
- **Submodule state** — what the sidebar says about a configured submodule: on the pinned commit,
  modified when its working copy has drifted from it, or not initialized when it was never checked
  out; determined without cloning or fetching.
- **Track** — the lane a merge out of the main line runs up between its parent's row and its
  child's, which the layout keeps free over those rows.
- **Transfer** — a write that reaches a remote: a fetch, a pull, which fetches and then
  fast-forwards, or a push. A transfer tells its progress as it goes, the repository is read again
  after it whether it was written or refused, and a fetch, a pull's fetch included, can be stopped
  while it runs.
  _Avoid:_ sync, network write, a write that takes long.
- **Upstream** — the remote branch a local branch follows, which a pull fetches and fast-forwards
  to and a push sends to; set where the branch tracks one.
  _Avoid:_ tracking branch (that is the local branch), remote counterpart.
- **View state** — what a front end remembers between sessions: the theme, the sidebar's width or
  minimised rail, and the diff layout; never anything read from a repository.
- **Window** — the range of rows a front end is drawing, the same term in both front ends since
  the terminal has no viewport in the browser's sense; the core answers it with those rows and the
  edges crossing them.
  _Avoid:_ viewport.
- **Working-directory node** — the node at the top of the commit graph that stands for the
  uncommitted changes of the working directory.
  _Avoid:_ WIP node, workdir commit.
- **Write** — a change Ocelli makes to a repository at the developer's request, such as creating a
  tag or applying a stash, naming the state it expects to find and refused with nothing changed
  where the repository no longer holds it; a write that discards work runs only once the
  developer has confirmed it.
  _Avoid:_ operation, which names a history operation or an operation in progress, action,
  mutation.

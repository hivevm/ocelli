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
- **Commit graph** — the commits of a repository drawn as nodes, each joined by an edge to its
  parents, with every branch and tag shown.
  _Avoid:_ tree, history graph, log graph.
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
- **Lane** — the column of the commit graph in which a branch is drawn.
  _Avoid:_ track, column, rail.
- **Lane colour** — the colour a lane is drawn in, taken from one theme-independent list so a
  commit keeps its colour across a theme switch; deliberately not a theme token
  ([ADR-0021](adr/0021-themes-tokens-and-palettes.md)).
  _Avoid:_ branch colour, since a branch is a ref and a lane outlives it.
- **Layout** — the rows, lanes, and edges the core computes for a loaded repository. A front end
  draws a layout and never computes one.
- **Lineage** — the commits a commit descends from and those that descend from it.
  _Avoid:_ ancestry, family.
- **Load session** — one pass of reading a repository and laying it out. Opening another repository
  starts a new one, and no result of the previous session reaches the new state.
- **Main branch** — the branch a repository treats as its line of record, usually `main` or
  `master`.
  _Avoid:_ trunk, default branch, master branch.
- **Merge edge** — an edge that joins a parent in another lane to a merge commit, where a branch
  is merged.
  _Avoid:_ merge line.
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
- **Submodule state** — what the sidebar says about a configured submodule: on the pinned commit,
  modified when its working copy has drifted from it, or not initialized when it was never checked
  out; determined without cloning or fetching.
- **View state** — what a front end remembers between sessions: the theme, the sidebar's width or
  minimised rail, and the diff layout; never anything read from a repository.
- **Window** — the range of rows a front end is drawing, the same term in both front ends since
  the terminal has no viewport in the browser's sense.
  _Avoid:_ viewport.
- **Working-directory node** — the node at the top of the commit graph that stands for the
  uncommitted changes of the working directory.
  _Avoid:_ WIP node, workdir commit.

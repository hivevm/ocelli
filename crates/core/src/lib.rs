//! Ocelli's core (ARCH-ports-and-adapters): the commit graph and its layout, and the port through which the
//! core reaches a repository. It depends on no adapter and on no technology; the layout in
//! [`graph`] is pure functions over values (GRAPH-layout-pure-functions-by-chunk). [`history`] keeps the laid-out graph and
//! answers the front ends' reads of it: windows of rows, search, and lineage (GRAPH-core-serves-row-windows).

pub mod graph;
pub mod history;
pub mod load;
pub mod merge;
pub mod model;
pub mod operations;
pub mod port;
pub mod transfer;

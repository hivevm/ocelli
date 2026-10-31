//! Ocelli's core (ADR-0008): the commit graph and its layout, and the port through which the
//! core reaches a repository. It depends on no adapter and on no technology; the layout in
//! [`graph`] is pure functions over values (ADR-0012). [`history`] keeps the laid-out graph and
//! answers the front ends' reads of it: windows of rows, search, and lineage (ADR-0019).

pub mod graph;
pub mod history;
pub mod load;
pub mod merge;
pub mod model;
pub mod operations;
pub mod port;
pub mod transfer;

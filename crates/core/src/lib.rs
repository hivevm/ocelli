//! Ocelli's core (ADR-0009): the commit graph and its layout, and the port through which the
//! core reaches a repository. It depends on no adapter and on no technology; the layout in
//! [`graph`] is pure functions over values (ADR-0010).

pub mod graph;
pub mod model;
pub mod port;

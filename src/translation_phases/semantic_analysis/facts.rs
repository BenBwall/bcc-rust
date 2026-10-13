//! Retained semantic control and lifetime facts. Read `control::ControlFacts`
//! for the consumer seam, `builder::Builder` for traversal-time production,
//! and `ancestry::Ancestry` for linear construction and logarithmic queries.
//! Regions distinguish lexical scope, VM declaration protection and GNU block
//! expressions; transitions are compact parent-chain runs, never ancestor
//! lists. C99: §6.2.1, pp. 29-30; PDF pp. 41-42; §6.2.4p6, p. 32;
//! PDF p. 44; §6.8.4-6.8.6, pp. 133-139; PDF pp. 145-151.

mod ancestry;
pub(super) mod builder;
mod control;

pub(crate) use control::*;

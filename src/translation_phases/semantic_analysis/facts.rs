//! Retained semantic facts. `control`/`builder`/`ancestry` own control and
//! lifetime production and queries. `correspondence` retains declaration
//! occurrences and parameter metadata; `parameters` records incoming/body
//! aliases; `sizes`/`bounds` finalize shared size DAGs and typed bound sites.
//! `resolved` produces normalized designations, point-of-use calls, owning
//! member identities and statement-expression tails alongside ordinary
//! traversal. Regions distinguish lexical scope, VM declaration protection and
//! GNU block expressions; transitions are compact parent-chain runs, never
//! ancestor lists. C99: §6.2.1, pp. 29-30; PDF pp. 41-42; §6.2.4p6, p. 32;
//! PDF p. 44; §6.8.4-6.8.6, pp. 133-139; PDF pp. 145-151.

mod ancestry;
mod bounds;
pub(super) mod builder;
mod builtin_instances;
mod control;
pub(super) mod correspondence;
mod parameters;
pub(super) mod resolved;
mod sizes;

pub(crate) use builtin_instances::{
    BuiltinCallFacts,
    BuiltinOperand,
};
pub(crate) use control::*;
pub(crate) use correspondence::{
    CorrespondenceFacts,
    OccurrenceId,
    ParameterFacts,
};
pub(crate) use resolved::{
    CallFacts,
    Callee,
    ResolvedFacts,
};
pub(crate) use sizes::*;

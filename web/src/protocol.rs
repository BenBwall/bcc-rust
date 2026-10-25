//! Messages between the page's engine proxy (`engine.rs`) and the engine
//! Worker (`bin/engine_worker.rs`). Both ends are built from this crate, and
//! messages cross as plain JavaScript values through `serde_wasm_bindgen`.

use serde::{
    Deserialize,
    Serialize,
};

use crate::snapshot::Snapshot;

/// A page-to-Worker request; the Worker answers each with [`Reply::Done`].
#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    pub id:      u32,
    pub command: Command,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Command {
    /// Compile new source, replacing the current program.
    Load(String),
    /// Execute one operation of the current program.
    Step,
}

/// A Worker-to-page message.
#[derive(Debug, Deserialize, Serialize)]
pub enum Reply {
    /// The Worker's Wasm has started and it accepts requests. Requests posted
    /// earlier could be lost, so the page waits for this first.
    Ready,
    Done {
        id:     u32,
        result: Result<Box<Snapshot>, String>,
    },
}

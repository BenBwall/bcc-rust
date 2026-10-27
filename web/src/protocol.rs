//! Messages between the page's engine proxy (`engine.rs`) and the engine
//! Worker (`bin/engine_worker.rs`). Both ends are built from this crate, and
//! messages cross as plain JavaScript values through `serde_wasm_bindgen`.

use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    pipeline::Compilation,
    snapshot::Snapshot,
};

/// A page-to-Worker request; the Worker answers each with [`Reply::Done`].
#[derive(Debug, Deserialize, Serialize)]
pub struct Request {
    pub id:      u32,
    pub command: Command,
}

#[derive(Debug, Deserialize, Serialize)]
pub enum Command {
    /// Compile new source for the abstract machine, replacing the current
    /// program; answered with a [`Answer::Snapshot`].
    Load(String),
    /// Execute one operation of the current program.
    Step,
    /// Run bcc's real compiler pipeline over `source`; answered with
    /// [`Answer::Compiled`], or an error while the Worker has no in-browser
    /// compiler (see `explorer.rs`).
    Compile(String),
}

/// A Worker's answer to one request.
#[derive(Debug, Deserialize, Serialize)]
pub enum Answer {
    Snapshot(Box<Snapshot>),
    Compiled(Box<Compilation>),
}

/// A Worker-to-page message.
#[derive(Debug, Deserialize, Serialize)]
pub enum Reply {
    /// The Worker's Wasm has started and it accepts requests. Requests posted
    /// earlier could be lost, so the page waits for this first.
    Ready,
    Done {
        id:     u32,
        result: Result<Answer, String>,
    },
}

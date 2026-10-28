//! Messages between the page's Worker proxy (`engine.rs`) and its two
//! Workers, the abstract machine (`bin/engine_worker.rs`) and the compiler
//! (`bin/compiler_worker.rs`). All ends are built from this crate, and
//! messages cross as plain JavaScript values through `serde_wasm_bindgen`.

use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    pipeline::Compilation,
    snapshot::Snapshot,
    wasm::{
        WasmControl,
        WasmModule,
        WasmProgram,
        WasmSnapshot,
    },
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
    /// Run bcc's real compiler over `source`, at `-O2` when `optimize`;
    /// answered by the compiler Worker with [`Answer::Compiled`] (see
    /// `explorer.rs`).
    Compile { source: String, optimize: bool },
    /// Load a WebAssembly module into the abstract machine Worker's stepper
    /// (see `wasm.rs`), replacing the previous one; answered with
    /// [`Answer::WasmLoaded`].
    WasmLoad(WasmModule),
    /// Step the loaded module; answered with [`Answer::Wasm`].
    Wasm(WasmControl),
}

/// A Worker's answer to one request.
#[derive(Debug, Deserialize, Serialize)]
pub enum Answer {
    Snapshot(Box<Snapshot>),
    Compiled(Box<Compilation>),
    WasmLoaded(Box<WasmProgram>),
    Wasm(Box<WasmSnapshot>),
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
    /// The Worker's Rust code panicked, with this message. Its module traps
    /// right after, so the page reports the message rather than the trap.
    Panicked(String),
}

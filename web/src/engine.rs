//! Proxy for the page's two Workers: the abstract machine
//! (`bin/engine_worker.rs`) and bcc's compiler (`bin/compiler_worker.rs`),
//! which the page starts only once it needs a live compilation, so the
//! machine starts without downloading the compiler.
//!
//! Compilation and execution never run on the UI thread. This proxy posts
//! [`Request`]s to a Worker and matches its [`Reply`]s by request id. A
//! timeout terminates the Worker, including a non-yielding call, and rejects
//! every pending request.

use std::{
    cell::{
        Cell,
        RefCell,
    },
    collections::HashMap,
    rc::{
        Rc,
        Weak,
    },
};

use futures_channel::oneshot;
use gloo_timers::callback::Timeout;
use js_sys::Reflect;
use wasm_bindgen::{
    JsCast,
    JsValue,
    closure::Closure,
};
use web_sys::{
    ErrorEvent,
    MessageEvent,
    Worker,
    WorkerOptions,
};

use crate::{
    pipeline::Compilation,
    protocol::{
        Answer,
        Command,
        Reply,
        Request,
    },
    snapshot::Snapshot,
    wasm::{
        WasmControl,
        WasmModule,
        WasmProgram,
        WasmSnapshot,
    },
};

/// Which Worker to start, and how to talk about it.
pub struct Kind {
    /// The Worker's name in the browser's developer tools.
    name:            &'static str,
    /// The `<meta>` that records its content-hashed loader URL.
    meta:            &'static str,
    /// The loader's URL when the page was not built by Trunk.
    fallback:        &'static str,
    connect_timeout: u32,
    call_timeout:    u32,
    /// Why a request failed: the Worker could not start, timed out, or
    /// stopped.
    unstartable:     &'static str,
    timed_out:       &'static str,
    stopped:         &'static str,
    disconnected:    &'static str,
}

/// The abstract machine, which holds the stepping session.
pub const MACHINE: Kind = Kind {
    name:            "bcc-rust-engine",
    meta:            crate::ENGINE_META,
    fallback:        "./engine-worker_loader.js",
    connect_timeout: 20_000,
    call_timeout:    10_000,
    unstartable:     "The Rust engine Worker could not start",
    timed_out:       "Rust engine timed out and was terminated. Retry engine to start a fresh \
                      instance.",
    stopped:         "Rust engine stopped.",
    disconnected:    "Rust engine disconnected. Retry engine to start a fresh instance.",
};

/// bcc's compiler. It keeps no state between requests, so the page simply
/// starts a new one after a timeout or a crash.
pub const COMPILER: Kind = Kind {
    name:            "bcc-rust-compiler",
    meta:            crate::COMPILER_META,
    fallback:        "./compiler-worker_loader.js",
    // The compiler's Wasm is several times the machine's.
    connect_timeout: 30_000,
    call_timeout:    COMPILE_TIMEOUT_MS,
    unstartable:     "bcc's in-browser compiler could not start",
    timed_out:       "bcc's in-browser compiler took longer than 10 seconds, so the page stopped \
                      it.",
    stopped:         "bcc's in-browser compiler stopped.",
    disconnected:    "bcc's in-browser compiler stopped.",
};

impl Kind {
    /// The error a request gets when the Worker ran out of time.
    pub fn timed_out(&self) -> &'static str {
        self.timed_out
    }

    /// The error a request gets when the Worker was stopped.
    pub fn stopped(&self) -> &'static str {
        self.stopped
    }
}

/// How long one compilation may run before its Worker is terminated.
pub const COMPILE_TIMEOUT_MS: u32 = 10_000;

/// Why a Worker's module trapped, from the trap's message and the panic it
/// reported first, if any. A Rust panic reports its message. A trap
/// without one is an `unreachable` that Rust reached when an allocation
/// failed (each of the compiler's arenas is one fixed region of linear
/// memory, and a full region is an allocation failure), or an overflow of
/// the engine's call stack.
pub fn trap_reason(trap: &str, panic: Option<&str>) -> String {
    match panic {
        | Some(panic) => format!("{INTERNAL_ERROR}{panic}"),
        | None if trap.contains("unreachable") => OUT_OF_MEMORY.to_owned(),
        | None if trap.contains("call stack") || trap.contains("too much recursion") =>
            TOO_DEEP.to_owned(),
        | None => trap.to_owned(),
    }
}

/// How a trap that followed a panic is reported, before the panic's message.
pub const INTERNAL_ERROR: &str = "bcc hit an internal error and stopped: ";

/// How a trap from a failed allocation is reported.
pub const OUT_OF_MEMORY: &str =
    "bcc ran out of memory compiling this program in your browser. There each of its arenas is \
     one fixed 32 MiB region, and this program filled one: a macro that expands enormously can, \
     even below the size limit. Make the program smaller, or compile it with the native bcc.";

/// How an overflow of the engine's call stack is reported.
pub const TOO_DEEP: &str = "This program nests too deeply for bcc's in-browser compiler, whose \
                            stack is much smaller than a native one.";

/// One connected Worker.
pub struct Engine {
    inner: Rc<Inner>,
}

impl Engine {
    /// Start a fresh Worker of `kind` and wait for its Wasm to start.
    pub async fn connect(kind: &'static Kind) -> Result<Self, String> {
        let global = js_sys::global();
        if !has(&global, "Worker") || !has(&global, "WebAssembly") {
            return Err("This browser needs Web Workers and WebAssembly to execute C.".into());
        }
        let options = WorkerOptions::new();
        options.set_name(kind.name);
        let worker = Worker::new_with_options(&worker_url(kind), &options)
            .map_err(|error| format!("{}: {}", kind.unstartable, describe(&error)))?;
        let inner = Rc::new(Inner {
            kind,
            worker,
            pending: RefCell::default(),
            sequence: Cell::new(0),
            connected: Cell::new(true),
            panicked: RefCell::default(),
            on_message: RefCell::default(),
            on_failure: RefCell::default(),
        });
        inner.listen();
        let engine = Self { inner };
        // The Worker announces itself once its Wasm runs; requests posted
        // before then could be lost.
        let ready = engine.inner.expect(READY, kind.connect_timeout);
        if let Err(error) = engine.inner.answer(ready).await {
            engine.dispose(kind.stopped);
            return Err(error);
        }
        Ok(engine)
    }

    pub async fn load(&self, source: &str) -> Result<Snapshot, String> {
        match self.request(Command::Load(source.to_owned())).await? {
            | Answer::Snapshot(snapshot) => Ok(*snapshot),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    pub async fn step(&self) -> Result<Snapshot, String> {
        match self.request(Command::Step).await? {
            | Answer::Snapshot(snapshot) => Ok(*snapshot),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    /// Load a WebAssembly module into the machine Worker's stepper.
    pub async fn wasm_load(&self, module: WasmModule) -> Result<WasmProgram, String> {
        match self.request(Command::WasmLoad(module)).await? {
            | Answer::WasmLoaded(program) => Ok(*program),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    /// Step the loaded WebAssembly module.
    pub async fn wasm(&self, control: WasmControl) -> Result<WasmSnapshot, String> {
        match self.request(Command::Wasm(control)).await? {
            | Answer::Wasm(snapshot) => Ok(*snapshot),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    /// Run bcc's compiler over `source` in the Worker, at `-O2` when
    /// `optimize`.
    pub async fn compile(&self, source: &str, optimize: bool) -> Result<Compilation, String> {
        let command = Command::Compile {
            source: source.to_owned(),
            optimize,
        };
        match self.request(command).await? {
            | Answer::Compiled(compilation) => Ok(*compilation),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    /// Compile `source` to WebAssembly with bcc in the Worker. The inner
    /// error is bcc's refusal, with its diagnostics.
    pub async fn compile_wasm(&self, source: &str) -> Result<Result<WasmModule, String>, String> {
        match self
            .request(Command::CompileWasm(source.to_owned()))
            .await?
        {
            | Answer::CompiledWasm(result) => Ok(*result),
            | _ => Err(UNEXPECTED.into()),
        }
    }

    pub fn connected(&self) -> bool {
        self.inner.connected.get()
    }

    /// Terminate the Worker and reject pending requests with `reason`.
    pub fn dispose(&self, reason: &str) {
        self.inner.dispose(reason);
    }

    async fn request(&self, command: Command) -> Result<Answer, String> {
        let receiver = self.inner.send(command)?;
        self.inner
            .answer(receiver)
            .await?
            .ok_or_else(|| "The Rust engine sent no answer.".into())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.inner.dispose(self.inner.kind.stopped);
    }
}

/// The Worker URL the build recorded (see `src/bin/prerender.rs`), or the
/// unhashed source layout when the page was not built by Trunk.
fn worker_url(kind: &Kind) -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector(&format!("meta[name=\"{}\"]", kind.meta))
                .ok()
                .flatten()
        })
        .and_then(|meta| meta.get_attribute("content"))
        .unwrap_or_else(|| kind.fallback.into())
}

const UNEXPECTED: &str = "The Rust engine sent an answer of the wrong kind.";
/// The pending-request slot for the Worker's [`Reply::Ready`]; requests count
/// from 1.
const READY: u32 = 0;

/// A request's outcome: an answer, or nothing for [`Reply::Ready`].
type Outcome = Result<Option<Answer>, String>;
type MessageHandler = Closure<dyn FnMut(MessageEvent)>;
type FailureHandler = Closure<dyn FnMut(JsValue)>;

struct Pending {
    reply:  oneshot::Sender<Outcome>,
    _timer: Timeout,
}

struct Inner {
    kind:       &'static Kind,
    worker:     Worker,
    pending:    RefCell<HashMap<u32, Pending>>,
    sequence:   Cell<u32>,
    connected:  Cell<bool>,
    /// The message of a panic the Worker reported before it trapped.
    panicked:   RefCell<Option<String>>,
    // Keep the Worker's event listeners alive for the engine's lifetime.
    on_message: RefCell<Option<MessageHandler>>,
    on_failure: RefCell<Vec<FailureHandler>>,
}

impl Inner {
    fn send(self: &Rc<Self>, command: Command) -> Result<oneshot::Receiver<Outcome>, String> {
        if !self.connected.get() {
            return Err(self.kind.disconnected.into());
        }
        let id = self.sequence.get() + 1;
        self.sequence.set(id);
        let receiver = self.expect(id, self.kind.call_timeout);
        let message = serde_wasm_bindgen::to_value(&Request { id, command })
            .map_err(|error| error.to_string())
            .and_then(|message| {
                self.worker
                    .post_message(&message)
                    .map_err(|error| describe(&error))
            });
        if let Err(error) = message {
            self.dispose(&format!(
                "Unable to send a request to the Rust Worker: {error}"
            ));
        }
        Ok(receiver)
    }

    /// Register a pending answer for `id`; the Worker is terminated if none
    /// arrives within `timeout` milliseconds.
    fn expect(self: &Rc<Self>, id: u32, timeout: u32) -> oneshot::Receiver<Outcome> {
        let weak = Rc::downgrade(self);
        let timer = Timeout::new(timeout, move || {
            if let Some(inner) = weak.upgrade() {
                inner.dispose(inner.kind.timed_out);
            }
        });
        let (reply, receiver) = oneshot::channel();
        _ = self.pending.borrow_mut().insert(
            id,
            Pending {
                reply,
                _timer: timer,
            },
        );
        receiver
    }

    async fn answer(&self, receiver: oneshot::Receiver<Outcome>) -> Outcome {
        receiver
            .await
            .unwrap_or_else(|_| Err(self.kind.stopped.into()))
    }

    fn listen(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        let on_message = MessageHandler::new(move |event: MessageEvent| {
            if let Some(inner) = weak.upgrade() {
                inner.receive(event.data());
            }
        });
        self.worker
            .set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        let on_error = self.listener(|inner, event| {
            event.prevent_default();
            let message = event
                .dyn_ref::<ErrorEvent>()
                .map(ErrorEvent::message)
                .unwrap_or_default();
            if message.is_empty() {
                format!(
                    "{}. Serve the built site over HTTP.",
                    inner.kind.unstartable
                )
            } else {
                trap_reason(&message, inner.panicked.borrow_mut().take().as_deref())
            }
        });
        self.worker
            .set_onerror(Some(on_error.as_ref().unchecked_ref()));
        let on_message_error =
            self.listener(|_, _| "Unable to decode the Rust Worker response.".into());
        self.worker
            .set_onmessageerror(Some(on_message_error.as_ref().unchecked_ref()));
        *self.on_message.borrow_mut() = Some(on_message);
        *self.on_failure.borrow_mut() = vec![on_error, on_message_error];
    }

    /// A Worker event that disposes the engine with the returned reason.
    fn listener(self: &Rc<Self>, reason: fn(&Self, &web_sys::Event) -> String) -> FailureHandler {
        let weak: Weak<Self> = Rc::downgrade(self);
        Closure::new(move |event: JsValue| {
            if let Some(inner) = weak.upgrade() {
                inner.dispose(&reason(&inner, event.unchecked_ref()));
            }
        })
    }

    fn receive(&self, data: JsValue) {
        let (id, answer) = match serde_wasm_bindgen::from_value(data) {
            | Ok(Reply::Ready) => (READY, Ok(None)),
            | Ok(Reply::Done { id, result }) => (id, result.map(Some)),
            | Ok(Reply::Panicked(message)) => {
                *self.panicked.borrow_mut() = Some(message);
                return;
            },
            | Err(error) => {
                self.dispose(&format!(
                    "The Rust Worker sent an unreadable reply ({error}). Reload the page."
                ));
                return;
            },
        };
        let Some(pending) = self.pending.borrow_mut().remove(&id) else {
            return;
        };
        _ = pending.reply.send(answer);
    }

    fn dispose(&self, reason: &str) {
        self.connected.set(false);
        self.worker.terminate();
        // Take the map first: a rejected receiver may run before this returns.
        let pending = std::mem::take(&mut *self.pending.borrow_mut());
        for (_, request) in pending {
            _ = request.reply.send(Err(reason.into()));
        }
    }
}

fn has(target: &JsValue, key: &str) -> bool {
    Reflect::has(target, &key.into()).unwrap_or(false) && !get(target, key).is_undefined()
}

fn get(target: &JsValue, key: &str) -> JsValue {
    Reflect::get(target, &key.into()).unwrap_or(JsValue::UNDEFINED)
}

fn describe(error: &JsValue) -> String {
    error
        .dyn_ref::<js_sys::Error>()
        .map(|error| String::from(error.message()))
        .or_else(|| error.as_string())
        .unwrap_or_else(|| "unknown error".into())
}

//! Proxy for the engine Worker (`bin/engine_worker.rs`).
//!
//! Compilation and execution never run on the UI thread. This proxy posts
//! [`Request`]s to the Worker and matches its [`Reply`]s. A timeout terminates
//! the Worker, including a non-yielding call, and rejects every pending
//! request.

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
    protocol::{
        Command,
        Reply,
        Request,
    },
    snapshot::Snapshot,
};

/// One connected engine Worker.
pub struct Engine {
    inner: Rc<Inner>,
}

impl Engine {
    /// Start a fresh Worker and wait for it to instantiate the engine.
    pub async fn connect() -> Result<Self, String> {
        let global = js_sys::global();
        if !has(&global, "Worker") || !has(&global, "WebAssembly") {
            return Err("This browser needs Web Workers and WebAssembly to execute C.".into());
        }
        let options = WorkerOptions::new();
        options.set_name("bcc-rust-engine");
        let worker = Worker::new_with_options(&worker_url(), &options).map_err(|error| {
            format!(
                "The Rust engine Worker could not start: {}",
                describe(&error)
            )
        })?;
        let inner = Rc::new(Inner {
            worker,
            pending: RefCell::default(),
            sequence: Cell::new(0),
            connected: Cell::new(true),
            on_message: RefCell::default(),
            on_failure: RefCell::default(),
        });
        inner.listen();
        let engine = Self { inner };
        // The Worker announces itself once its Wasm runs; requests posted
        // before then could be lost.
        let ready = engine.inner.expect(READY, CONNECT_TIMEOUT_MS);
        if let Err(error) = Inner::answer(ready).await {
            engine.dispose("Rust engine stopped.");
            return Err(error);
        }
        Ok(engine)
    }

    pub async fn load(&self, source: &str) -> Result<Snapshot, String> {
        self.request(Command::Load(source.to_owned())).await
    }

    pub async fn step(&self) -> Result<Snapshot, String> {
        self.request(Command::Step).await
    }

    pub fn connected(&self) -> bool {
        self.inner.connected.get()
    }

    /// Terminate the Worker and reject pending requests with `reason`.
    pub fn dispose(&self, reason: &str) {
        self.inner.dispose(reason);
    }

    async fn request(&self, command: Command) -> Result<Snapshot, String> {
        let receiver = self.inner.send(command)?;
        Inner::answer(receiver)
            .await?
            .ok_or_else(|| "The Rust engine sent no snapshot.".into())
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.inner.dispose("Rust engine stopped.");
    }
}

/// The Worker URL the build recorded (see `src/bin/prerender.rs`), or the
/// unhashed source layout when the page was not built by Trunk.
fn worker_url() -> String {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| {
            document
                .query_selector(&format!("meta[name=\"{}\"]", crate::ENGINE_META))
                .ok()
                .flatten()
        })
        .and_then(|meta| meta.get_attribute("content"))
        .unwrap_or_else(|| "./engine-worker_loader.js".into())
}

const CONNECT_TIMEOUT_MS: u32 = 20_000;
const CALL_TIMEOUT_MS: u32 = 10_000;
/// The pending-request slot for the Worker's [`Reply::Ready`]; requests count
/// from 1.
const READY: u32 = 0;

/// A request's outcome: a snapshot, or nothing for [`Reply::Ready`].
type Answer = Result<Option<Snapshot>, String>;
type MessageHandler = Closure<dyn FnMut(MessageEvent)>;
type FailureHandler = Closure<dyn FnMut(JsValue)>;

struct Pending {
    reply:  oneshot::Sender<Answer>,
    _timer: Timeout,
}

struct Inner {
    worker:     Worker,
    pending:    RefCell<HashMap<u32, Pending>>,
    sequence:   Cell<u32>,
    connected:  Cell<bool>,
    // Keep the Worker's event listeners alive for the engine's lifetime.
    on_message: RefCell<Option<MessageHandler>>,
    on_failure: RefCell<Vec<FailureHandler>>,
}

impl Inner {
    fn send(self: &Rc<Self>, command: Command) -> Result<oneshot::Receiver<Answer>, String> {
        if !self.connected.get() {
            return Err("Rust engine disconnected. Retry engine to start a fresh instance.".into());
        }
        let id = self.sequence.get() + 1;
        self.sequence.set(id);
        let receiver = self.expect(id, CALL_TIMEOUT_MS);
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
    fn expect(self: &Rc<Self>, id: u32, timeout: u32) -> oneshot::Receiver<Answer> {
        let weak = Rc::downgrade(self);
        let timer = Timeout::new(timeout, move || {
            if let Some(inner) = weak.upgrade() {
                inner.dispose(
                    "Rust engine timed out and was terminated. Retry engine to start a fresh \
                     instance.",
                );
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

    async fn answer(receiver: oneshot::Receiver<Answer>) -> Answer {
        receiver
            .await
            .unwrap_or_else(|_| Err("Rust engine stopped.".into()))
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
        let on_error = self.listener(|event| {
            event.prevent_default();
            let message = event
                .dyn_ref::<ErrorEvent>()
                .map(ErrorEvent::message)
                .unwrap_or_default();
            if message.is_empty() {
                "The Rust engine Worker could not start. Serve the built site over HTTP.".into()
            } else {
                message
            }
        });
        self.worker
            .set_onerror(Some(on_error.as_ref().unchecked_ref()));
        let on_message_error =
            self.listener(|_| "Unable to decode the Rust Worker response.".into());
        self.worker
            .set_onmessageerror(Some(on_message_error.as_ref().unchecked_ref()));
        *self.on_message.borrow_mut() = Some(on_message);
        *self.on_failure.borrow_mut() = vec![on_error, on_message_error];
    }

    /// A Worker event that disposes the engine with the returned reason.
    fn listener(self: &Rc<Self>, reason: fn(&web_sys::Event) -> String) -> FailureHandler {
        let weak: Weak<Self> = Rc::downgrade(self);
        Closure::new(move |event: JsValue| {
            if let Some(inner) = weak.upgrade() {
                inner.dispose(&reason(event.unchecked_ref()));
            }
        })
    }

    fn receive(&self, data: JsValue) {
        let (id, answer) = match serde_wasm_bindgen::from_value(data) {
            | Ok(Reply::Ready) => (READY, Ok(None)),
            | Ok(Reply::Done { id, result }) => (id, result.map(|snapshot| Some(*snapshot))),
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

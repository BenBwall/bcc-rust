//! The compiler Web Worker. Trunk builds it as its own Wasm module, apart from
//! the abstract machine's, because it holds the whole of bcc's compiler: the
//! page loads it only when it first needs a live compilation, and can
//! terminate it (after a timeout, or a trap that leaves the module unusable)
//! without losing the abstract machine's session.

use bcc_web::{
    explorer,
    protocol::{
        Reply,
        Request,
    },
};
use wasm_bindgen::{
    JsCast,
    closure::Closure,
};
use web_sys::{
    DedicatedWorkerGlobalScope,
    MessageEvent,
};

fn main() {
    console_error_panic_hook::set_once();
    let scope: DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let replies = scope.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // The proxy only sends valid requests; anything else times out there.
        if let Ok(Request { id, command }) = serde_wasm_bindgen::from_value(event.data()) {
            let result = explorer::run(command);
            post(&replies, &Reply::Done { id, result });
        }
    });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The handler lives as long as the Worker.
    on_message.forget();
    post(&scope, &Reply::Ready);
}

fn post(scope: &DedicatedWorkerGlobalScope, reply: &Reply) {
    let message = serde_wasm_bindgen::to_value(reply).expect("replies are serializable");
    scope
        .post_message(&message)
        .expect("the page accepts plain values");
}

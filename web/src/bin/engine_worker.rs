//! The engine Web Worker. Trunk builds it as a separate Wasm module with a
//! generated loader script; it runs the bcc abstract machine off the UI
//! thread so the page can terminate a hung or crashed engine.

use std::cell::RefCell;

use bcc_web::{
    machine::Machine,
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
    let machine = RefCell::new(Machine::default());
    let replies = scope.clone();
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        // The proxy only sends valid requests; anything else times out there.
        if let Ok(Request { id, command }) = serde_wasm_bindgen::from_value(event.data()) {
            let result = machine.borrow_mut().run(command).map(Box::new);
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

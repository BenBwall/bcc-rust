//! The workbench's insight panels. In abstract-machine mode: the run as a
//! multitrack tape (`tape`), the cells under its playhead, the note on the
//! step (`narration`), the expression being evaluated as a schematic, and
//! the objects' datasheet.
//! In pipeline mode: the compiler's output stage by stage, with what each
//! front-end stage did to the program. In WebAssembly mode: the module's
//! listing and the stepper's state. Engine data is always rendered as text
//! nodes, never as HTML.

mod cells;
mod datasheet;
mod front_end;
mod knob;
mod narration;
mod pipeline;
mod schematic;
mod tape;
mod wasm;

pub use cells::Cells;
pub use datasheet::Datasheet;
pub use front_end::source_marks;
pub use knob::Knob;
use leptos::prelude::*;
pub use narration::Narration;
pub use pipeline::{
    LiveCompile,
    PipelineView,
};
pub use schematic::Schematic;
pub use tape::{
    Deck,
    px,
    value_text,
};
pub use wasm::{
    WasmListing,
    WasmView,
};

/// `text` with each `` `code` `` span rendered as `<code>`.
fn rich_text(text: &str) -> AnyView {
    text.split('`')
        .enumerate()
        .map(|(index, piece)| {
            let piece = piece.to_owned();
            if index % 2 == 1 {
                view! { <code>{piece}</code> }.into_any()
            } else {
                piece.into_any()
            }
        })
        .collect_view()
        .into_any()
}

/// A human label for an event category.
fn category_label(category: &str) -> &'static str {
    match category {
        | "evaluate" => "Evaluate",
        | "convert" => "Convert",
        | "memory" => "Memory",
        | "lifetime" => "Lifetime",
        | "control" => "Control flow",
        | "call" => "Call",
        | "fault" => "Stopped",
        | _ => "Step",
    }
}

/// What a kind of step does, for the badge's tooltip.
fn category_hint(category: &str) -> &'static str {
    match category {
        | "evaluate" => "Evaluate: an operator or constant produces a value",
        | "convert" => "Convert: a value is read from an object or changes type or form",
        | "memory" => "Memory: an object is designated, stored into or updated",
        | "lifetime" => "Lifetime: a block begins or ends, or an object is created",
        | "control" => "Control flow: a condition or jump picks what runs next",
        | "call" => "Call: a function call begins or returns",
        | "fault" => "Stopped: the run cannot go on, for undefined behavior or a limit of this machine",
        | _ => "One evaluation of the abstract machine",
    }
}

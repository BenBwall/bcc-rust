//! The workbench's insight panels. In abstract-machine mode: the narration of
//! the last step, the expression tree being evaluated, and the memory diagram.
//! In pipeline mode: the compiler's output stage by stage. Engine data is
//! always rendered as text nodes, never as HTML.

mod memory;
mod narration;
mod pipeline;
mod tree;

use leptos::prelude::*;
pub use memory::Memory;
pub use narration::Narration;
pub use pipeline::PipelineView;
pub use tree::TreeView;

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

//! The expression tree of the full expression being evaluated. Values fill in
//! bottom-up as the machine evaluates each operand; the node just evaluated is
//! highlighted.

use leptos::prelude::*;

use crate::{
    palette,
    snapshot::{
        Snapshot,
        Tree,
    },
};

#[component]
pub fn TreeView(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let tree = Memo::new(move |_| {
        snapshot.with(|state| state.as_ref().and_then(|state| state.tree.clone()))
    });
    view! {
        <section class="card tree-card" aria-labelledby="tree-title">
            <header class="card-header">
                <h2 id="tree-title">"Expression tree"</h2>
                <span class="card-subtitle">
                    {move || tree.with(|tree| tree.as_ref().map_or_else(String::new, |tree| format!("in {}()", tree.function)))}
                </span>
            </header>
            {move || tree.with(|tree| match tree {
                | Some(tree) if !tree.nodes.is_empty() => view! {
                    <div class="tree-scroll">
                        <div class="tree" role="tree" aria-label="Expression being evaluated">
                            {node(tree, tree.nodes.len() - 1)}
                        </div>
                    </div>
                }.into_any(),
                // Before the engine loads there is no step yet.
                | _ if snapshot.with(Option::is_none) => view! {
                    <p class="empty-state">
                        "The expression being evaluated appears here as a tree, and fills in from the leaves up as the machine evaluates it."
                    </p>
                }.into_any(),
                | _ => view! {
                    <p class="empty-state">
                        "This step is not part of an expression. When the machine evaluates one, its tree appears here and fills in from the leaves up."
                    </p>
                }.into_any(),
            })}
            <footer class="tree-legend">
                <span><i class="legend-node is-value"></i>"value"</span>
                <span><i class="legend-node is-lvalue"></i>"lvalue: designates an object"</span>
                <span><i class="legend-node is-pending"></i>"not evaluated yet"</span>
            </footer>
        </section>
    }
}

fn node(tree: &Tree, index: usize) -> AnyView {
    let entry = &tree.nodes[index];
    let colour = entry
        .object
        .as_deref()
        .map(palette::class)
        .unwrap_or_default();
    let state = if entry.value.is_none() {
        "is-pending"
    } else if entry.lvalue {
        "is-lvalue"
    } else {
        "is-value"
    };
    let value = match &entry.value {
        | None => "·".to_owned(),
        // A leaf's value repeats its label: say what kind of thing it is.
        | Some(value) if *value == entry.label && entry.lvalue => "object".to_owned(),
        | Some(value) if *value == entry.label => "int".to_owned(),
        | Some(value) => value.clone(),
    };
    // One class string: a `class:` toggle beside a changing `class` would be
    // dropped when the pill re-renders.
    let active = if entry.active { " is-active" } else { "" };
    let children = entry.children.clone();
    let label = entry.label.clone();
    let description = format!("{label}: {value}");
    view! {
        <div class="tnode" role="treeitem" aria-label=description>
            <div class=format!("tnode-pill {state} {colour}{active}")>
                <span class="tnode-label">{label}</span>
                <span class="tnode-value">{value}</span>
            </div>
            {(!children.is_empty()).then(|| view! {
                <div class="tnode-children" role="group">
                    {children.into_iter().map(|child| node(tree, child)).collect_view()}
                </div>
            })}
        </div>
    }
    .into_any()
}

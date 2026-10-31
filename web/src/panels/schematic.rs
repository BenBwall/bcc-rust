//! Figure 2: the full expression being evaluated, wired up as a schematic.
//! Its operands enter at the left edge like a circuit's inputs; each operator
//! is a gate placed by its logic level, so the full expression's operator
//! sits at the right. A wire carries its value once the machine has
//! evaluated it, dashed until then; the node the last step evaluated is lit.

use leptos::prelude::*;

use super::tape::px;
use crate::snapshot::{
    Snapshot,
    Tree,
};

const COLUMN: f64 = 132.0;
const ROW: f64 = 34.0;
/// Width of one character of the labels, in pixels.
const CHAR: f64 = 8.0;

#[component]
pub fn Schematic(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let tree = Memo::new(move |_| {
        snapshot.with(|state| state.as_ref().and_then(|state| state.tree.clone()))
    });
    view! {
        <figure class="schematic" aria-labelledby="schematic-caption">
            <figcaption class="module-caption" id="schematic-caption">
                <b>"Figure 2"</b>
                {move || tree.with(|tree| match tree {
                    | Some(tree) if !tree.nodes.is_empty() => format!(" — The full expression in {}(), wired up", tree.function),
                    | _ => " — The full expression, wired up".to_owned(),
                })}
            </figcaption>
            {move || tree.with(|tree| match tree {
                | Some(tree) if !tree.nodes.is_empty() => view! {
                    <div class="schematic-scroll">{drawing(tree)}</div>
                    <ol class="sr-only" aria-label="Expression being evaluated">{outline(tree, tree.nodes.len() - 1)}</ol>
                }.into_any(),
                | _ if snapshot.with(Option::is_none) => view! {
                    <p class="schematic-empty">"The expression being evaluated is drawn here as a circuit, its wires filling with values as the machine evaluates it."</p>
                }.into_any(),
                | _ => view! {
                    <p class="schematic-empty">"This step is not part of an expression. When the machine evaluates one, it is wired up here."</p>
                }.into_any(),
            })}
        </figure>
    }
}

/// Where each node sits: its logic level (0 for an operand) and its row.
fn place(tree: &Tree) -> (Vec<usize>, Vec<f64>, usize) {
    let count = tree.nodes.len();
    let mut level = vec![0; count];
    // Children come first in the list, so one pass computes the levels.
    for index in 0..count {
        level[index] = tree.nodes[index]
            .children
            .iter()
            .map(|&child| level[child] + 1)
            .max()
            .unwrap_or(0);
    }
    let mut row = vec![0.0; count];
    let mut leaves = 0;
    walk(tree, count - 1, &mut row, &mut leaves);
    (level, row, leaves)
}

/// Rows from the leaves up: each operand its own, each operator midway
/// between its first and last operand.
fn walk(tree: &Tree, index: usize, row: &mut [f64], leaves: &mut usize) -> f64 {
    let node = &tree.nodes[index];
    let y = if node.children.is_empty() {
        *leaves += 1;
        px(*leaves - 1)
    } else {
        let rows: Vec<f64> = node.children.iter().map(|&child| walk(tree, child, row, leaves)).collect();
        f64::midpoint(rows[0], rows[rows.len() - 1])
    };
    row[index] = y;
    y
}

fn drawing(tree: &Tree) -> impl IntoView {
    let (level, row, leaves) = place(tree);
    let root = tree.nodes.len() - 1;
    let x = |index: usize| px(level[index]) * COLUMN + 10.0;
    let y = |index: usize| row[index] * ROW + ROW / 2.0 + 6.0;
    let width_of = |index: usize| {
        let label = px(tree.nodes[index].label.chars().count());
        if tree.nodes[index].children.is_empty() {
            label * CHAR + 16.0
        } else {
            (label * CHAR + 14.0).max(30.0)
        }
    };
    let width = px(level[root]) * COLUMN + width_of(root) + 70.0;
    let height = px(leaves.max(1)) * ROW + 14.0;
    let mut parent = vec![None; tree.nodes.len()];
    for (index, node) in tree.nodes.iter().enumerate() {
        for &child in &node.children {
            parent[child] = Some(index);
        }
    }
    let wires = (0..tree.nodes.len())
        .map(|index| {
            let node = &tree.nodes[index];
            let (from, y0) = (x(index) + width_of(index), y(index));
            let (to, y1) = match parent[index] {
                | Some(parent) => (x(parent), y(parent)),
                | None => (from + 26.0, y0),
            };
            let bend = to - 14.0;
            let state = wire_state(node.value.is_some(), node.active);
            let shown = node.value.as_ref().filter(|value| !(node.children.is_empty() && **value == node.label));
            let value = shown.map(|value| if node.lvalue { "object".to_owned() } else { value.clone() });
            view! {
                <path class=format!("wire {state}") d=format!("M{from:.1} {y0:.1} H{bend:.1} V{y1:.1} H{to:.1}") />
                {value.map(|value| view! { <text class="wire-value" x=from + 4.0 y=y0 - 5.0>{value}</text> })}
                {parent[index].is_none().then(|| view! {
                    <text class="wire-value" x=to + 4.0 y=y0 + 4.0>{node.value.clone().unwrap_or_else(|| "?".to_owned())}</text>
                })}
            }
        })
        .collect_view();
    let nodes = tree
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let (x0, y0) = (x(index), y(index));
            let active = if node.active { " is-active" } else { "" };
            if node.children.is_empty() {
                view! {
                    <g class=format!("schematic-input{active}")>
                        <circle cx=x0 + 3.0 cy=y0 r="3" />
                        <text x=x0 + 10.0 y=y0 + 4.0>{node.label.clone()}</text>
                    </g>
                }
                .into_any()
            } else {
                let box_width = width_of(index);
                view! {
                    <g class=format!("schematic-gate{active}")>
                        <rect x=x0 y=y0 - 12.0 width=box_width height="24" />
                        <text x=x0 + box_width / 2.0 y=y0 + 4.5 text-anchor="middle">{node.label.clone()}</text>
                    </g>
                }
                .into_any()
            }
        })
        .collect_view();
    view! {
        <svg class="schematic-svg" width=width height=height viewBox=format!("0 0 {width:.0} {height:.0}") aria-hidden="true">
            {wires}
            {nodes}
        </svg>
    }
}

fn wire_state(evaluated: bool, active: bool) -> &'static str {
    match (evaluated, active) {
        | (_, true) => "is-active",
        | (true, false) => "is-value",
        | (false, false) => "is-pending",
    }
}

/// The tree as nested lists, for screen readers.
fn outline(tree: &Tree, index: usize) -> AnyView {
    let node = &tree.nodes[index];
    let value = node.value.clone().unwrap_or_else(|| "not evaluated yet".to_owned());
    let children = node.children.clone();
    view! {
        <li>
            {format!("{}: {value}", node.label)}
            {(!children.is_empty()).then(|| view! {
                <ol>{children.into_iter().map(|child| outline(tree, child)).collect_view()}</ol>
            })}
        </li>
    }
    .into_any()
}

//! Figure 2: the full expression being evaluated, wired up as a schematic.
//! Its operands enter at the left edge like a circuit's inputs; each operator
//! is a gate placed by its logic level, so the full expression's operator
//! sits at the right. A wire carries its value once the machine has
//! evaluated it, dashed until then; the node the last step evaluated is lit.
//! An operand that `&&`, `||` or `?:` skipped stays faint, its wire dotted
//! and labelled "skipped", for it is never evaluated.

use leptos::prelude::*;

use super::tape::px;
use crate::snapshot::{
    Snapshot,
    Tree,
};

/// The narrowest step from one logic level to the next, in pixels.
const COLUMN: f64 = 132.0;
const ROW: f64 = 34.0;
/// Width of one character of the labels, in pixels.
const CHAR: f64 = 8.0;
/// Width of one character of a wire's value (10.5 px type), in pixels.
const VALUE_CHAR: f64 = 6.6;
/// The run of wire a level keeps before the next level's bend.
const GAP: f64 = 30.0;
/// The most characters an operand's label or a wire's value shows; a
/// string literal is cut short, as on the tape, and the outline for screen
/// readers keeps it whole.
const SHOWN: usize = 16;

#[component]
pub fn Schematic(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let tree = Memo::new(move |_| {
        snapshot.with(|state| state.as_ref().and_then(|state| state.tree.clone()))
    });
    // A call's first step shows the called function, which has no
    // expression yet; the caller's expression waits on the call.
    let calling = Memo::new(move |_| {
        snapshot.with(|state| {
            let state = state
                .as_ref()
                .filter(|state| state.event.category == "call")?;
            let (called, waiting) = (state.stack.first()?, state.stack.get(1)?);
            Some((
                waiting.name.clone(),
                waiting.waiting.clone()?,
                called.name.clone(),
            ))
        })
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
                    {pointer_legend(tree)}
                }.into_any(),
                | _ if calling.with(Option::is_some) => {
                    let (waiting, call, called) = calling.get().unwrap_or_default();
                    view! {
                        <p class="schematic-empty">
                            "The call "<code>{call}</code>" is part of an expression in "{waiting}"(), which waits for it to return. "{called}"()'s own expressions are wired up here as they run."
                        </p>
                    }.into_any()
                },
                // Before the first step: what will be drawn here, in a card
                // like the blank tape's, unless the run stopped there.
                | _ if snapshot.with(|state| state.as_ref().is_some_and(|state| state.step == 0 && !state.can_step())) => view! {
                    <p class="schematic-empty">"The machine stopped before its first step, so it evaluated no expression."</p>
                }.into_any(),
                | _ if snapshot.with(|state| state.as_ref().is_none_or(|state| state.step == 0)) => view! {
                    <div class="figure-empty">
                        <p class="figure-empty-label">"Nothing wired yet"</p>
                        <p class="figure-empty-text">
                            "Each full expression the machine evaluates is drawn here as a circuit: its operands \
                             feed its operators, and the wires fill with values step by step."
                        </p>
                    </div>
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
        let rows: Vec<f64> = node
            .children
            .iter()
            .map(|&child| walk(tree, child, row, leaves))
            .collect();
        f64::midpoint(rows[0], rows[rows.len() - 1])
    };
    row[index] = y;
    y
}

fn drawing(tree: &Tree) -> impl IntoView {
    let (level, row, leaves) = place(tree);
    let root = tree.nodes.len() - 1;
    let width_of = |index: usize| width_of(tree, index);
    let shown_value = |index: usize| shown_value(tree, index);
    let start = columns(tree, &level);
    let x = |index: usize| start[level[index]];
    let y = |index: usize| row[index] * ROW + ROW / 2.0 + 6.0;
    let result = tree.nodes[root]
        .value
        .as_ref()
        .map_or(1, |value| shown(value).chars().count());
    let width = x(root) + width_of(root) + 34.0 + px(result) * VALUE_CHAR;
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
            let state = if node.skipped {
                "is-skipped"
            } else {
                wire_state(node.value.is_some(), node.active)
            };
            // The full expression's own value is written once, at the end
            // of its wire.
            let value = shown_value(index).filter(|_| parent[index].is_some());
            // An operand `&&`, `||` or `?:` skipped says so once, on the
            // wire into the operator that skipped it.
            let skipped = node.skipped
                && parent[index].is_some_and(|parent| !tree.nodes[parent].skipped);
            // A pointer to a function leads to code, not to an object: its
            // value is set in the colour of calls.
            let value_class = if node.function { "wire-value is-function" } else { "wire-value" };
            view! {
                <path class=format!("wire {state}") d=format!("M{from:.1} {y0:.1} H{bend:.1} V{y1:.1} H{to:.1}") />
                {value.map(|value| view! { <text class=value_class x=from + 4.0 y=y0 - 5.0>{value}</text> })}
                {skipped.then(|| view! { <text class="wire-value is-skipped" x=from + 4.0 y=y0 - 5.0>"skipped"</text> })}
                {parent[index].is_none().then(|| view! {
                    <text class="wire-value" x=to + 4.0 y=y0 + 4.0>{node.value.as_deref().map_or_else(|| "?".to_owned(), shown)}</text>
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
            let active = match (node.active, node.skipped) {
                | (true, _) => " is-active",
                | (false, true) => " is-skipped",
                | (false, false) => "",
            };
            if node.children.is_empty() {
                view! {
                    <g class=format!("schematic-input{active}")>
                        <circle cx=x0 + 3.0 cy=y0 r="3" />
                        <text x=x0 + 10.0 y=y0 + 4.0>{shown(&node.label)}</text>
                    </g>
                }
                .into_any()
            } else {
                let box_width = width_of(index);
                view! {
                    <g class=format!("schematic-gate{active}")>
                        <rect x=x0 y=y0 - 12.0 width=box_width height="24" />
                        <text x=x0 + box_width / 2.0 y=y0 + 4.5 text-anchor="middle">{shown(&node.label)}</text>
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

/// The left edge of each logic level. Each level is as wide as its widest
/// node and the value on its wire, so a long operand (a string literal)
/// never runs into its operator.
fn columns(tree: &Tree, level: &[usize]) -> Vec<f64> {
    let count = level.iter().max().map_or(1, |&deepest| deepest + 1);
    let mut start = vec![10.0; count + 1];
    for column in 0..count {
        let widest = (0..tree.nodes.len())
            .filter(|&index| level[index] == column)
            .map(|index| {
                let characters = shown_value(tree, index).map_or(0, |value| value.chars().count());
                let characters = if tree.nodes[index].skipped {
                    characters.max("skipped".len())
                } else {
                    characters
                };
                let value = if characters == 0 {
                    0.0
                } else {
                    px(characters) * VALUE_CHAR + 8.0
                };
                width_of(tree, index) + value.max(GAP)
            })
            .fold(0.0, f64::max);
        start[column + 1] = start[column] + widest.max(COLUMN);
    }
    start
}

/// The width of node `index`'s label: an operand's name, or an operator's
/// box.
fn width_of(tree: &Tree, index: usize) -> f64 {
    let label = px(shown(&tree.nodes[index].label).chars().count());
    if tree.nodes[index].children.is_empty() {
        label * CHAR + 16.0
    } else {
        (label * CHAR + 14.0).max(30.0)
    }
}

/// The value written on node `index`'s wire, if it has one to show: not an
/// operand's own spelling, and "object" for an lvalue.
fn shown_value(tree: &Tree, index: usize) -> Option<String> {
    let node = &tree.nodes[index];
    node.value
        .as_ref()
        .filter(|value| !(node.children.is_empty() && **value == node.label))
        .map(|value| {
            if node.lvalue {
                "object".to_owned()
            } else {
                shown(value)
            }
        })
}

/// `text` cut to [`SHOWN`] characters, ending in `…` when it was longer.
fn shown(text: &str) -> String {
    if text.chars().count() <= SHOWN {
        text.to_owned()
    } else {
        let mut cut: String = text.chars().take(SHOWN - 1).collect();
        cut.push('…');
        cut
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
    let value = node.value.clone().map_or_else(
        || {
            if node.skipped {
                "skipped, never evaluated".to_owned()
            } else {
                "not evaluated yet".to_owned()
            }
        },
        |value| match value.strip_prefix('&') {
            // `&square` is read out as what it is.
            | Some(function) if node.function => format!("a pointer to the function {function}"),
            | _ => value,
        },
    );
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

/// A line under the drawing when a wire carries a pointer: the wires write
/// a pointer as `&` would make it, where the tape, the cells and Table 1
/// draw the same value as an arrow, so the reader can match the two. A
/// pointer into a string literal is left out: the other panels spell it by
/// the literal's text, which the wire does not repeat.
fn pointer_legend(tree: &Tree) -> Option<impl IntoView + use<>> {
    let (written, drawn) = legend_spellings(tree)?;
    Some(view! {
        <p class="schematic-legend">
            "A wire writes a pointer as "<code>"&"</code>" would make it, "<code>{written}</code>"; the tape, the cells and Table 1 draw the same pointer as an arrow, "<code>{drawn}</code>"."
        </p>
    })
}

/// The first pointer a wire carries, as the wire writes it and as the tape
/// draws it: `&t[0]` and `→t[0]`, `&square` and `→square()`.
fn legend_spellings(tree: &Tree) -> Option<(String, String)> {
    let pointer = tree.nodes.iter().find(|node| {
        !node.lvalue
            && node
                .value
                .as_deref()
                .is_some_and(|value| value.starts_with('&') && !value.starts_with("&\""))
    })?;
    let written = pointer.value.clone().unwrap_or_default();
    let name = written.trim_start_matches('&');
    let drawn = if pointer.function {
        format!("→{name}()")
    } else {
        format!("→{name}")
    };
    Some((written, drawn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::TreeNode;

    fn node(value: &str, function: bool) -> TreeNode {
        TreeNode {
            label: String::from("x"),
            children: Vec::new(),
            value: Some(value.to_owned()),
            lvalue: false,
            active: false,
            skipped: false,
            function,
            object: None,
            range: None,
        }
    }

    // Part 15's N-c: the legend matches a wire's `&` with the arrow the
    // other panels draw, and leaves out what it cannot spell as they do.
    #[test]
    fn the_legend_spells_a_pointer_as_both_panels_do() {
        let tree = |nodes| Tree {
            function: String::from("main"),
            nodes,
        };
        assert_eq!(
            legend_spellings(&tree(vec![node("3", false), node("&t[0]", false)])),
            Some((String::from("&t[0]"), String::from("→t[0]")))
        );
        assert_eq!(
            legend_spellings(&tree(vec![node("&square", true)])),
            Some((String::from("&square"), String::from("→square()")))
        );
        assert_eq!(
            legend_spellings(&tree(vec![node("&\"%d\\n\"[0]", false), node("5", false)])),
            None
        );
    }
}

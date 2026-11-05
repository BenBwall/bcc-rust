//! Under the head: the objects at the playhead as a strip of cells, one run
//! per active call, innermost first, then the objects with static storage
//! duration, the way a tape passes under a read/write head. An array is a
//! run of element cells, a structure a run of its members' cells with a
//! padding cell wherever the layout leaves unused bytes. The cell the step wrote is lit,
//! the one it read underlined, the one it only designated outlined; an
//! element a pointer points at names that pointer. The objects of returned
//! calls fold into one cell.

use leptos::prelude::*;

use super::tape::value_text;
use crate::{
    palette,
    snapshot::{
        Object,
        Snapshot,
        Value,
    },
};

#[component]
pub fn Cells(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    view! {
        <section class="head-cells" aria-labelledby="cells-title">
            <h2 class="module-caption" id="cells-title">
                <b>"Under the head"</b>" the objects at the playhead, by call"
            </h2>
            {move || snapshot.with(|snapshot| match snapshot {
                | Some(snapshot) if !snapshot.objects.is_empty() => strip(snapshot).into_any(),
                | Some(_) => view! { <p class="cells-empty">"No objects yet. Each declaration that runs puts one on the tape."</p> }.into_any(),
                | None => view! { <p class="cells-empty">"The objects of the run appear here as cells, under the playhead."</p> }.into_any(),
            })}
        </section>
    }
}

/// One cell of the strip: a scalar, or the padding after a member.
enum Part {
    Cell {
        value: Value,
        /// How the source names it: `total`, `values[1]`, `p.x`.
        name:  String,
        ty:    String,
        /// Its scalar index in the object, for an array or a structure.
        leaf:  Option<usize>,
    },
    Padding {
        bytes: usize,
        /// The member it follows.
        after: String,
    },
}

/// The cells of `object`: one per scalar, in memory order, with a padding
/// cell after each structure member that has padding.
fn parts(object: &Object) -> Vec<Part> {
    let mut parts = Vec::new();
    let ty = object.ty.split('[').next().unwrap_or_default().trim().to_owned();
    match &object.value {
        | Value::Array { .. } | Value::Struct { .. } =>
            collect(&object.value, &object.name, &ty, &mut 0, &mut parts),
        | value => parts.push(Part::Cell {
            value: value.clone(),
            name: object.name.clone(),
            ty,
            leaf: None,
        }),
    }
    parts
}

fn collect(value: &Value, name: &str, ty: &str, leaf: &mut usize, parts: &mut Vec<Part>) {
    match value {
        | Value::Array { elements } =>
            for (index, element) in elements.iter().enumerate() {
                collect(element, &format!("{name}[{index}]"), ty, leaf, parts);
            },
        | Value::Struct { members, .. } =>
            for member in members {
                let path = format!("{name}.{}", member.name);
                let ty = member.ty.split('[').next().unwrap_or_default().trim();
                collect(&member.value, &path, ty, leaf, parts);
                if member.padding > 0 {
                    parts.push(Part::Padding {
                        bytes: member.padding,
                        after: path,
                    });
                }
            },
        | scalar => {
            parts.push(Part::Cell {
                value: scalar.clone(),
                name:  name.to_owned(),
                ty:    ty.to_owned(),
                leaf:  Some(*leaf),
            });
            *leaf += 1;
        },
    }
}

#[expect(clippy::too_many_lines, reason = "One block per kind of cell.")]
fn strip(snapshot: &Snapshot) -> impl IntoView {
    let access = |object: usize, cell: Option<usize>| {
        snapshot
            .event
            .access
            .iter()
            .find(|access| {
                access.object == object
                    && (access.cell == cell
                        || access.cell.is_none()
                        || (cell.is_none() && access.cell == Some(0)))
            })
            .map(|access| access.kind.clone())
    };
    // Which pointer points at which cell, to name it under the cell.
    let pointers: Vec<(usize, i32, String)> = snapshot
        .objects
        .iter()
        .filter(|object| object.alive)
        .filter_map(|object| match object.value {
            | Value::Pointer {
                target: Some(target),
                offset,
                ended: false,
                ..
            } => Some((target, offset, object.name.clone())),
            | _ => None,
        })
        .collect();
    let pointed = |object: usize, offset: usize| {
        pointers
            .iter()
            .filter(|(target, at, _)| {
                *target == object && usize::try_from(*at).ok() == Some(offset)
            })
            .map(|(_, _, name)| format!("◂{name}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let cells = |objects: Vec<&Object>| {
        objects
            .into_iter()
            .flat_map(|object| {
                parts(object).into_iter().map(move |part| match part {
                    | Part::Cell { value, name, ty, leaf } => {
                        let kind = access(object.id, leaf);
                        let text = value_text(&value, &snapshot.objects);
                        let indeterminate = text.is_none();
                        let marks = pointed(object.id, leaf.unwrap_or(0));
                        let description = format!(
                            "{name}, {ty}: {}{}",
                            text.as_deref().unwrap_or("indeterminate"),
                            kind.as_deref().map(|kind| format!(", {}", verb(kind))).unwrap_or_default()
                        );
                        view! {
                            <li
                                class=format!("cell {}", palette::class(&object.name))
                                class:is-indeterminate=indeterminate
                                data-access=kind
                                aria-label=description
                            >
                                <span class="cell-type" aria-hidden="true">{ty}</span>
                                <b class="cell-value" aria-hidden="true">{text.unwrap_or_else(|| "?".to_owned())}</b>
                                <span class="cell-name" aria-hidden="true">{name}<span class="cell-pointers">{marks}</span></span>
                            </li>
                        }
                        .into_any()
                    },
                    | Part::Padding { bytes, after } => {
                        let plural = if bytes == 1 { "" } else { "s" };
                        view! {
                            <li
                                class="cell is-padding"
                                aria-label=format!("{bytes} byte{plural} of padding after {after}, holding no value")
                                title=format!("{bytes} byte{plural} of unnamed padding after {after}: no member lives here, and its bytes hold unspecified values")
                            >
                                <span class="cell-type" aria-hidden="true">"pad"</span>
                                <b class="cell-value" aria-hidden="true">{format!("{bytes}B")}</b>
                                <span class="cell-name" aria-hidden="true">"padding"</span>
                            </li>
                        }
                        .into_any()
                    },
                })
                .collect::<Vec<_>>()
            })
            .collect_view()
    };
    let statics: Vec<&Object> = snapshot
        .objects
        .iter()
        .filter(|object| object.frame_id == 0 && object.alive)
        .collect();
    let statics = (!statics.is_empty()).then(|| {
        view! {
            <li class="cell-frame is-static">
                <span class="cell-frame-name">"static"</span>
                <ol class="cell-run" aria-label="Static storage: objects that exist for the whole run">{cells(statics)}</ol>
            </li>
        }
    });
    let frames = snapshot
        .stack
        .iter()
        .map(|frame| {
            let objects: Vec<&Object> = snapshot
                .objects
                .iter()
                .filter(|object| object.frame_id == frame.id && object.alive)
                .collect();
            view! {
                <li class="cell-frame">
                    <span class="cell-frame-name">{format!("{}()", frame.name)}</span>
                    <ol class="cell-run" aria-label=format!("{}() frame", frame.name)>{cells(objects)}</ol>
                </li>
            }
        })
        .collect_view();
    let ended: Vec<&Object> = snapshot
        .objects
        .iter()
        .filter(|object| !object.alive)
        .collect();
    view! {
        <ol class="cells">
            {frames}
            {statics}
            {(!ended.is_empty()).then(|| {
                let names = tally(ended.iter().map(|object| object.name.as_str()));
                let label = format!("Lifetimes over: {names}");
                view! {
                    <li class="cell is-ended" aria-label=label>
                        <span class="cell-type" aria-hidden="true">"ended"</span>
                        <b class="cell-value" aria-hidden="true">{ended.len()}</b>
                        <span class="cell-name" aria-hidden="true">{names}</span>
                    </li>
                }
            })}
        </ol>
    }
}

/// Names, each once, with how many times it repeats: `x ×30 before`.
pub(super) fn tally<'a>(names: impl Iterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for name in names {
        match counts.iter_mut().find(|(known, _)| *known == name) {
            | Some((_, count)) => *count += 1,
            | None => counts.push((name, 1)),
        }
    }
    counts
        .into_iter()
        .map(|(name, count)| {
            if count > 1 {
                format!("{name} ×{count}")
            } else {
                name.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a step did to a cell, in words.
fn verb(kind: &str) -> &'static str {
    match kind {
        | "read" => "read by this step",
        | "write" => "written by this step",
        | "create" => "created by this step",
        | "end" => "its lifetime ends",
        | "address" => "its address taken",
        | _ => "designated by this step",
    }
}

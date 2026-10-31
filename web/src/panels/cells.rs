//! Under the head: the objects at the playhead as a strip of cells, one run
//! per active call, innermost first, the way a tape passes under a read/write
//! head. An array is a run of element cells. The cell the step wrote is lit,
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

#[expect(clippy::too_many_lines, reason = "One block per kind of cell.")]
fn strip(snapshot: &Snapshot) -> impl IntoView {
    let access = |object: usize, cell: Option<usize>| {
        snapshot
            .event
            .access
            .iter()
            .find(|access| {
                access.object == object
                    && (access.cell == cell || access.cell.is_none() || (cell.is_none() && access.cell == Some(0)))
            })
            .map(|access| access.kind.clone())
    };
    // Which pointer points at which element, to name it under the element.
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
            .filter(|(target, at, _)| *target == object && usize::try_from(*at).ok() == Some(offset))
            .map(|(_, _, name)| format!("◂{name}"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let frames = snapshot
        .stack
        .iter()
        .map(|frame| {
            let objects: Vec<&Object> = snapshot
                .objects
                .iter()
                .filter(|object| object.frame_id == frame.id && object.alive)
                .collect();
            let cells = objects
                .into_iter()
                .flat_map(|object| {
                    let element = matches!(object.value, Value::Array { .. });
                    let parts: Vec<(Value, String, Option<usize>)> = match &object.value {
                        | Value::Array { elements } => elements
                            .iter()
                            .enumerate()
                            .map(|(index, element)| (element.clone(), format!("{}[{index}]", object.name), Some(index)))
                            .collect(),
                        | value => vec![(value.clone(), object.name.clone(), None)],
                    };
                    let ty = object.ty.split('[').next().unwrap_or_default().trim().to_owned();
                    parts.into_iter().map(move |(value, name, index)| {
                        let kind = access(object.id, if element { index } else { None });
                        let text = value_text(&value, &snapshot.objects);
                        let indeterminate = text.is_none();
                        let marks = pointed(object.id, index.unwrap_or(0));
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
                                <span class="cell-type" aria-hidden="true">{ty.clone()}</span>
                                <b class="cell-value" aria-hidden="true">{text.unwrap_or_else(|| "?".to_owned())}</b>
                                <span class="cell-name" aria-hidden="true">{name}<span class="cell-pointers">{marks}</span></span>
                            </li>
                        }
                    }).collect::<Vec<_>>()
                })
                .collect_view();
            view! {
                <li class="cell-frame">
                    <span class="cell-frame-name">{format!("{}()", frame.name)}</span>
                    <ol class="cell-run" aria-label=format!("{}() frame", frame.name)>{cells}</ol>
                </li>
            }
        })
        .collect_view();
    let ended: Vec<&Object> = snapshot.objects.iter().filter(|object| !object.alive).collect();
    view! {
        <ol class="cells">
            {frames}
            {(!ended.is_empty()).then(|| {
                let names = ended.iter().map(|object| object.name.as_str()).collect::<Vec<_>>().join(" ");
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

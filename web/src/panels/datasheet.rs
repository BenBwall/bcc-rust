//! Table 1: the objects at the playhead as a datasheet: name, type, the call
//! it lives in (or static storage), the step it was created at and its
//! value. A structure's members follow it, one row each, with their byte
//! offsets and the padding between them. The objects the step used are lit;
//! those whose lifetime has ended are struck through.

use leptos::prelude::*;

use super::tape::{
    Reel,
    value_text,
};
use crate::{
    palette,
    snapshot::{
        Object,
        Snapshot,
        Value,
    },
};

#[component]
pub fn Datasheet(
    snapshot: Memo<Option<Snapshot>>,
    history: RwSignal<Vec<Snapshot>>,
    /// The tape, whose lane names (`x #2`) the rows take.
    reel: Memo<Reel>,
) -> impl IntoView {
    // The step each object was created at.
    let born = Memo::new(move |_| {
        history.with(|history| {
            let mut born = std::collections::HashMap::new();
            for snapshot in history {
                for object in &snapshot.objects {
                    born.entry(object.id).or_insert(snapshot.step);
                }
            }
            born
        })
    });
    view! {
        <figure class="datasheet" aria-labelledby="datasheet-caption">
            <figcaption class="module-caption" id="datasheet-caption">
                <b>"Table 1"</b>
                {move || snapshot.with(|snapshot| snapshot.as_ref().map_or_else(|| " — Objects".to_owned(), |snapshot| format!(" — Objects at step {}", snapshot.step)))}
            </figcaption>
            <div class="sheet-scroll">
            <table>
                <thead>
                    <tr>
                        <th scope="col">"Object"</th>
                        <th scope="col">"Type"</th>
                        <th scope="col">"Lives in"</th>
                        <th scope="col">"Since"</th>
                        <th scope="col">"Value"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || snapshot.with(|snapshot| {
                        let Some(snapshot) = snapshot else {
                            return Vec::new();
                        };
                        // The objects of returned calls fold into one row,
                        // unless the step used one; those it used are
                        // listed, not counted.
                        let shown = |object: &Object| {
                            object.alive || snapshot.event.access.iter().any(|access| access.object == object.id)
                        };
                        let ended: Vec<&str> = snapshot.objects.iter().filter(|object| !shown(object)).map(|object| object.name.as_str()).collect();
                        born.with(|born| {
                            snapshot
                                .objects
                                .iter()
                                .filter(|object| shown(object) || ended.len() <= 3)
                                .map(|object| {
                                    let used = snapshot.event.access.iter().any(|access| access.object == object.id);
                                    let value = if let (true, Value::Struct { size, .. }) = (object.alive, &object.value) {
                                        // Its members' rows hold the values.
                                        format!("{size} bytes")
                                    } else if object.alive {
                                        value_text(&object.value).unwrap_or_else(|| "indeterminate".to_owned())
                                    } else {
                                        "lifetime over".to_owned()
                                    };
                                    let place = place(object);
                                    let row = view! {
                                        <tr class:is-used=used class:is-ended=!object.alive>
                                            <th scope="row" class=palette::class(&object.name) title=object.literal.then(|| object.name.clone())>
                                                <span class="sheet-swatch" aria-hidden="true"></span>{reel.with(|reel| reel.label_of(object.id)).unwrap_or_else(|| object.name.clone())}
                                            </th>
                                            <td>{object.ty.clone()}</td>
                                            <td>{place}</td>
                                            <td>{born.get(&object.id).map(ToString::to_string).unwrap_or_default()}</td>
                                            <td class="sheet-value">{value}</td>
                                        </tr>
                                    };
                                    let members = if object.alive { members(object) } else { Vec::new() };
                                    view! { {row}{members} }
                                })
                                .map(IntoAny::into_any)
                                .chain((ended.len() > 3).then(|| view! {
                                    <tr class="is-ended is-folded">
                                        <th scope="row">{format!("{} ended", ended.len())}</th>
                                        <td colspan="4">{super::cells::tally(ended.iter().copied())}</td>
                                    </tr>
                                }.into_any()))
                                .collect::<Vec<_>>()
                        })
                    })}
                </tbody>
            </table>
            </div>
        </figure>
    }
}

/// Where an object lives: its call, or static storage.
fn place(object: &Object) -> String {
    if object.storage == "static" {
        return object.scope.as_ref().map_or_else(
            || "static storage".to_owned(),
            |function| format!("static, {function}()"),
        );
    }
    if object.function.is_empty() {
        "ended call".to_owned()
    } else {
        format!("{}()", object.function)
    }
}

/// A row per member of a structure object (nested structures indented),
/// with the member's byte offset and size, and a row for each run of
/// padding.
fn members(object: &Object) -> Vec<AnyView> {
    let mut rows = Vec::new();
    collect(&object.value, &object.name, 0, 1, object, &mut rows);
    rows
}

fn collect(
    value: &Value,
    path: &str,
    base: usize,
    depth: usize,
    object: &Object,
    rows: &mut Vec<AnyView>,
) {
    let Value::Struct { members, .. } = value else {
        return;
    };
    for member in members {
        let name = format!("{path}.{}", member.name);
        let offset = base + member.offset;
        // A nested structure's own members follow it, with the values.
        let shown = if matches!(member.value, Value::Struct { .. }) {
            String::new()
        } else {
            value_text(&member.value).unwrap_or_else(|| "?".to_owned())
        };
        let bytes = if member.size == 1 {
            format!("byte {offset}")
        } else {
            format!("bytes {}–{}", offset, offset + member.size - 1)
        };
        rows.push(
            view! {
                <tr class="is-member" style:--depth=depth.to_string()>
                    <th scope="row" class=palette::class(&object.name)>
                        <span class="sheet-member">{format!(".{}", member.name)}</span>
                    </th>
                    <td>{member.ty.clone()}</td>
                    <td class="sheet-offset">{bytes}</td>
                    <td></td>
                    <td class="sheet-value">{shown}</td>
                </tr>
            }
            .into_any(),
        );
        collect(&member.value, &name, offset, depth + 1, object, rows);
        if member.padding > 0 {
            let start = offset + member.size;
            let plural = if member.padding == 1 { "" } else { "s" };
            rows.push(
                view! {
                    <tr class="is-member is-padding" style:--depth=depth.to_string()>
                        <th scope="row"><span class="sheet-member">"padding"</span></th>
                        <td>{format!("{} byte{plural}", member.padding)}</td>
                        <td class="sheet-offset">{if member.padding == 1 { format!("byte {start}") } else { format!("bytes {}–{}", start, start + member.padding - 1) }}</td>
                        <td></td>
                        <td class="sheet-value">"no value"</td>
                    </tr>
                }
                .into_any(),
            );
        }
    }
}

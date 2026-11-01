//! Table 1: the objects at the playhead as a datasheet: name, type, the call
//! it lives in, the step it was created at and its value. The objects the
//! step used are lit; those whose lifetime has ended are struck through.

use leptos::prelude::*;

use super::tape::value_text;
use crate::{
    palette,
    snapshot::Snapshot,
};

#[component]
pub fn Datasheet(
    snapshot: Memo<Option<Snapshot>>,
    history: RwSignal<Vec<Snapshot>>,
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
                        let shown = |object: &crate::snapshot::Object| {
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
                                    let value = if object.alive {
                                        value_text(&object.value, &snapshot.objects).unwrap_or_else(|| "indeterminate".to_owned())
                                    } else {
                                        "lifetime over".to_owned()
                                    };
                                    let place = if object.function.is_empty() { "ended call".to_owned() } else { format!("{}()", object.function) };
                                    view! {
                                        <tr class:is-used=used class:is-ended=!object.alive>
                                            <th scope="row" class=palette::class(&object.name)>
                                                <span class="sheet-swatch" aria-hidden="true"></span>{object.name.clone()}
                                            </th>
                                            <td>{object.ty.clone()}</td>
                                            <td>{place}</td>
                                            <td>{born.get(&object.id).map(ToString::to_string).unwrap_or_default()}</td>
                                            <td class="sheet-value">{value}</td>
                                        </tr>
                                    }
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
        </figure>
    }
}

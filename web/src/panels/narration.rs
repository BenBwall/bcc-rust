//! The step narration: what the last operation did, why, and the rule of C
//! that says so.

use leptos::prelude::*;

use super::{
    category_label,
    rich_text,
};
use crate::snapshot::Snapshot;

#[component]
pub fn Narration(
    snapshot: Memo<Option<Snapshot>>,
    /// Recent steps before the current one: history index, title, category.
    recent: Memo<Vec<(usize, String, String)>>,
    on_jump: Callback<usize>,
    /// The lesson's summary, shown before the engine has loaded.
    placeholder: Memo<&'static str>,
) -> impl IntoView {
    let event =
        Memo::new(move |_| snapshot.with(|state| state.as_ref().map(|state| state.event.clone())));
    let category = move || {
        event.with(|event| {
            event
                .as_ref()
                .map_or_else(|| "control".to_owned(), |event| event.category.clone())
        })
    };
    let diagnostic = Memo::new(move |_| {
        snapshot.with(|state| state.as_ref().and_then(|state| state.diagnostic.clone()))
    });
    let returned = Memo::new(move |_| {
        snapshot.with(|state| {
            state.as_ref().and_then(|state| match &state.return_value {
                | Some(crate::snapshot::Value::Integer { text }) => Some(text.clone()),
                | _ => None,
            })
        })
    });
    view! {
        <article class="card narration" data-category=category aria-labelledby="narration-title">
            <div class="narration-meta">
                <span class="category-badge">{move || category_label(&category())}</span>
                <span class="narration-step">
                    {move || snapshot.with(|state| state.as_ref().map_or_else(String::new, |state| format!("Step {}", state.step)))}
                </span>
                <span class="sequence-chip" hidden=move || !event.with(|event| event.as_ref().is_some_and(|event| event.sequence_point))>
                    <span class="sequence-dot" aria-hidden="true"></span>"Sequence point"
                </span>
            </div>
            <h2 id="narration-title" class="narration-title">
                {move || event.with(|event| match event {
                    | Some(event) => rich_text(&event.title),
                    | None => "Loading the abstract machine…".into_any(),
                })}
            </h2>
            <p class="narration-body">
                {move || event.with(|event| match event {
                    | Some(event) => rich_text(&event.explanation),
                    | None => placeholder.get().into_any(),
                })}
            </p>
            <p class="return-callout" hidden=move || returned.with(Option::is_none)>
                "Program finished: main returned "<strong>{move || returned.get().unwrap_or_default()}</strong>
            </p>
            <div class="fault-callout" hidden=move || diagnostic.with(Option::is_none)>
                <p class="fault-kind">{move || diagnostic.with(|d| d.as_ref().map(crate::snapshot::Diagnostic::kind_label).unwrap_or_default())}</p>
                <p class="fault-message">{move || diagnostic.with(|d| d.as_ref().map(|d| d.message.clone()).unwrap_or_default())}</p>
                <p class="fault-note" hidden=move || diagnostic.with(|d| d.as_ref().is_none_or(|d| d.note.is_empty()))>
                    {move || diagnostic.with(|d| d.as_ref().map(|d| d.note.clone()).unwrap_or_default())}
                </p>
            </div>
            <ul class="clauses" aria-label="C standard rules applied">
                {move || event.with(|event| {
                    event
                        .as_ref()
                        .map(|event| event.clauses.clone())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|clause| view! {
                            <li class="clause">
                                <span class="clause-id" title=clause.citation.clone()>{format!("C99 §{}", clause.id)}</span>
                                <span class="clause-title">{clause.title}</span>
                                <span class="clause-gloss">{clause.gloss}</span>
                            </li>
                        })
                        .collect_view()
                })}
            </ul>
            <div class="recent" hidden=move || recent.with(Vec::is_empty)>
                <h3>"Earlier steps"</h3>
                <ol>
                    {move || recent.get().into_iter().map(|(index, title, category)| view! {
                        <li>
                            <button type="button" class="recent-step" data-category=category on:click=move |_| on_jump.run(index)>
                                <span class="recent-dot" aria-hidden="true"></span>
                                <span class="recent-index">{index}</span>
                                <span class="recent-title">{title.replace('`', "")}</span>
                            </button>
                        </li>
                    }).collect_view()}
                </ol>
            </div>
        </article>
    }
}

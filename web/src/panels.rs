//! Machine-state panels: call stack, objects, evaluation and sequencing, and
//! the diagnostic banner. Engine data is rendered as text.

use std::{
    collections::HashMap,
    fmt::Write,
};

use leptos::prelude::*;

use crate::snapshot::{
    Diagnostic,
    Object,
    Snapshot,
    Value,
};

#[component]
pub fn StackPanel(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let frames = Memo::new(move |_| {
        snapshot.with(|state| {
            state
                .as_ref()
                .map(|state| state.stack.clone())
                .unwrap_or_default()
        })
    });
    view! {
        <section class="stack-panel" aria-labelledby="stack-title">
            <div class="panel-heading">
                <div><span class="panel-index">"02"</span><h2 id="stack-title">"Call stack"</h2></div>
                <span class="count">{move || plural(frames.with(Vec::len), "frame")}</span>
            </div>
            <ol class="call-stack">
                {move || {
                    let frames = frames.get();
                    if frames.is_empty() {
                        let empty = if snapshot.with(Option::is_some) {
                            "No active function calls."
                        } else {
                            "Load a program to inspect its call stack."
                        };
                        return view! { <li class="empty-state">{empty}</li> }.into_any();
                    }
                    frames
                        .into_iter()
                        .enumerate()
                        .map(|(index, frame)| {
                            let active = index == 0;
                            view! {
                                <li class="stack-frame" class:is-active=active aria-current=active.then_some("true")>
                                    <span class="frame-depth">{format!("#{}", frame.depth)}</span>
                                    <span class="frame-name">{format!("{}()", frame.name)}</span>
                                    <span class="frame-meta">{frame.label}</span>
                                </li>
                            }
                        })
                        .collect_view()
                        .into_any()
                }}
            </ol>
        </section>
    }
}

#[component]
pub fn ObjectsPanel(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let objects = Memo::new(move |_| {
        snapshot.with(|state| {
            state
                .as_ref()
                .map(|state| state.objects.clone())
                .unwrap_or_default()
        })
    });
    view! {
        <section class="objects-panel" aria-labelledby="objects-title">
            <div class="panel-heading">
                <div><span class="panel-index">"03"</span><h2 id="objects-title">"Objects"</h2></div>
                <span class="count">{move || plural(objects.with(Vec::len), "object")}</span>
            </div>
            <div class="object-legend">
                <span><i class="legend-dot live-dot"></i>" Alive"</span>
                <span><i class="legend-dot ended-dot"></i>" Lifetime ended"</span>
                <span class="mono">"→ pointer target"</span>
            </div>
            <div class="objects" role="region" tabindex="0" aria-label="Object values">
                {move || {
                    let objects = objects.get();
                    if objects.is_empty() {
                        let empty = if snapshot.with(Option::is_some) {
                            "No objects in this state."
                        } else {
                            "Object identity, storage, lifetime, and values will appear here."
                        };
                        return view! { <p class="empty-state">{empty}</p> }.into_any();
                    }
                    let names: HashMap<usize, String> = objects.iter().map(|object| (object.id, object.name.clone())).collect();
                    objects.into_iter().map(|object| object_card(object, &names)).collect_view().into_any()
                }}
            </div>
        </section>
    }
}

#[component]
pub fn EffectsPanel(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let with_state = move |present: fn(&Snapshot) -> String, absent: &'static str| {
        move || snapshot.with(|state| state.as_ref().map_or_else(|| absent.to_owned(), present))
    };
    let effects = Memo::new(move |_| {
        snapshot.with(|state| state.as_ref().map(|state| state.effects.clone()))
    });
    let return_value = Memo::new(move |_| {
        snapshot.with(|state| {
            state
                .as_ref()
                .and_then(|state| state.return_value.as_ref())
                .map(|value| {
                    let text = match value {
                        | Value::Integer { text }
                        | Value::Pointer {
                            text: Some(text), ..
                        } => text.clone(),
                        | Value::Indeterminate => "indeterminate".into(),
                        | Value::Array { .. } => "array".into(),
                        | Value::Pointer { .. } => "pointer".into(),
                    };
                    format!("main returned {text}")
                })
        })
    });
    view! {
        <section class="effects-panel" aria-labelledby="effects-title">
            <div class="panel-heading">
                <div><span class="panel-index">"04"</span><h2 id="effects-title">"Evaluation & sequencing"</h2></div>
                <span class="tag">{with_state(|_| "Atomic stores · prototype".into(), "Awaiting engine")}</span>
            </div>
            <div class="effects-content">
                <div class="current-event">
                    <p class="eyebrow">"LAST COMPLETED OPERATION"</p>
                    <p>
                        {with_state(
                            |state| if state.event.is_empty() { "Ready for the next engine evaluation.".into() } else { state.event.clone() },
                            "Load the Rust engine to begin stepping.",
                        )}
                    </p>
                    <p class="muted">
                        {with_state(
                            |state| state.sequence_explanation.clone(),
                            "Load a program to inspect the prototype’s sequencing model.",
                        )}
                    </p>
                    <p class="return-value" hidden=move || return_value.with(Option::is_none)>
                        {move || return_value.get().unwrap_or_default()}
                    </p>
                </div>
                <div class="pending-effects">
                    <h3>"Pending effects "<span class="count">{move || effects.with(|effects| effects.as_ref().map_or(0, Vec::len))}</span></h3>
                    <ul id="effects-list">
                        {move || match effects.get() {
                            | None => view! { <li class="muted">"No execution state loaded."</li> }.into_any(),
                            | Some(effects) if effects.is_empty() => view! { <li class="muted">"No pending effects."</li> }.into_any(),
                            | Some(effects) => effects.into_iter().map(|effect| view! { <li>{effect}</li> }).collect_view().into_any(),
                        }}
                    </ul>
                </div>
            </div>
        </section>
    }
}

#[component]
pub fn DiagnosticBanner(diagnostic: Memo<Option<Diagnostic>>) -> impl IntoView {
    let field = move |read: fn(&Diagnostic) -> String| {
        move || diagnostic.with(|diagnostic| diagnostic.as_ref().map(read).unwrap_or_default())
    };
    view! {
        <section class="diagnostic" aria-labelledby="diagnostic-title" hidden=move || diagnostic.with(Option::is_none)>
            <div class="diagnostic-label">{field(|diagnostic| diagnostic.kind_label().into())}</div>
            <h2 id="diagnostic-title">{field(|diagnostic| non_empty(&diagnostic.title, "Execution stopped"))}</h2>
            <p>{field(|diagnostic| non_empty(&diagnostic.message, "The engine stopped execution."))}</p>
            <pre hidden=move || diagnostic.with(|diagnostic| diagnostic.as_ref().is_none_or(|diagnostic| diagnostic.detail.is_empty()))>
                {field(|diagnostic| diagnostic.detail.clone())}
            </pre>
            <p class="citation">{field(|diagnostic| non_empty(&diagnostic.citation, "No standard citation was supplied by the engine."))}</p>
        </section>
    }
}

fn object_card(object: Object, names: &HashMap<usize, String>) -> impl IntoView + use<> {
    let lifetime = if object.alive {
        "Alive"
    } else {
        "Lifetime ended"
    };
    let name = non_empty(&object.name, "(unnamed)");
    let storage = format!("{} storage", non_empty(&object.storage, "unspecified"));
    let scope = (!object.scope.is_empty()).then(|| view! { <span>{object.scope}</span> });
    view! {
        <article class="object-card" class:is-ended=!object.alive id=object_anchor(object.id) tabindex="-1">
            <div class="object-title">
                <h3>{name}</h3>
                <span class="object-id">{format!("#{}", object.id)}</span>
                <span class="object-type">{object.ty}</span>
            </div>
            <div class="object-details">
                <span>{storage}</span>
                <span class="object-lifetime">{lifetime}</span>
                {scope}
            </div>
            <div class="object-value">{value_view(&object.value, names)}</div>
        </article>
    }
}

fn value_view(value: &Value, names: &HashMap<usize, String>) -> AnyView {
    match value {
        | Value::Indeterminate =>
            view! { <span class="indeterminate">"? indeterminate"</span> }.into_any(),
        | Value::Integer { text } => view! { <span>{text.clone()}</span> }.into_any(),
        | Value::Array { elements } => elements
            .iter()
            .enumerate()
            .map(|(index, item)| {
                view! {
                    <span class="array-item">
                        <span class="array-index">{format!("[{index}]")}</span>
                        {value_view(item, names)}
                    </span>
                }
            })
            .collect_view()
            .into_any(),
        | Value::Pointer {
            target: None, text, ..
        } => view! { <span>{text.clone().unwrap_or_else(|| "null pointer".into())}</span> }
            .into_any(),
        | Value::Pointer {
            target: Some(target),
            offset,
            one_past,
            ended,
            ..
        } => {
            let name = names.get(target);
            let mut label = format!("→ #{target}");
            if let Some(name) = name {
                label.push(' ');
                label.push_str(name);
            }
            if *offset != 0 {
                _ = write!(label, " + {offset}");
            }
            if *one_past {
                label.push_str(" (one past)");
            }
            if *ended {
                label.push_str(" (lifetime ended)");
            }
            if name.is_some() {
                view! { <a class="pointer-link" href=format!("#{}", object_anchor(*target))>{label}</a> }.into_any()
            } else {
                view! { <span class="pointer-link">{format!("{label} · not in snapshot")}</span> }
                    .into_any()
            }
        },
    }
}

fn object_anchor(id: usize) -> String {
    format!("object-{id}")
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

fn non_empty(text: &str, fallback: &str) -> String {
    if text.is_empty() { fallback } else { text }.to_owned()
}

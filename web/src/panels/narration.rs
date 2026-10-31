//! The step narration: what the last operation did, why, and the rule of C
//! that says so, in layers. One sentence leads, with the values the step took
//! and produced drawn as a row of chips and the cited paragraphs as links to
//! the standard; the full explanation, each paragraph's gloss and the
//! glossary entries wait behind "How C defines this step". The first use of
//! each term of the standard is marked, with its definition on hover or
//! focus, and the names of objects take their colour from the memory
//! diagram.

use std::collections::HashMap;

use leptos::prelude::*;

use super::{
    category_hint,
    category_label,
};
use crate::{
    glossary::{
        self,
        Piece,
        Term,
    },
    palette,
    snapshot::{
        FlowPart,
        Snapshot,
        StepEvent,
    },
};

#[component]
pub fn Narration(
    snapshot: Memo<Option<Snapshot>>,
    /// Recent steps before the current one: history index, title, category.
    recent: Memo<Vec<(usize, String, String)>>,
    on_jump: Callback<usize>,
    /// The lesson's summary, shown before the engine has loaded.
    placeholder: Memo<&'static str>,
    /// Whether Run is stepping: the steps it passes through flash by, so
    /// they do not count as seen.
    running: Signal<bool>,
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
    let names = object_names(snapshot);
    // Whether the learner opened the details; it stays open (or closed)
    // from step to step.
    let details = RwSignal::new(false);
    let tip = RwSignal::new(None::<Tip>);
    // A new step's text replaces the term the tip belongs to.
    Effect::new(move || {
        event.track();
        tip.set(None);
    });
    let card = NodeRef::<leptos::html::Article>::new();
    // Each note shows on the first step the learner sees it at (by step
    // number, so Back to that step shows it again); steps Run passes through
    // do not use it up. The run starts over at step 0.
    let first_seen = StoredValue::new(HashMap::<String, usize>::new());
    let note = Memo::new(move |_| {
        snapshot.with(|state| {
            let state = state.as_ref()?;
            let note = &state.event.note;
            if state.step == 0 {
                first_seen.update_value(HashMap::clear);
            }
            if note.is_empty() || running.get() {
                return None;
            }
            let first = first_seen
                .try_update_value(|seen| *seen.entry(note.clone()).or_insert(state.step))?;
            (first == state.step).then(|| note.clone())
        })
    });
    view! {
        <article class="card narration" data-category=category aria-labelledby="narration-title" node_ref=card>
            <div class="narration-meta">
                // No step has run before the engine loads, so no kind of step.
                <span class="category-badge" title=move || category_hint(&category()) hidden=move || event.with(Option::is_none)>{move || category_label(&category())}</span>
                <span class="narration-step">
                    {move || snapshot.with(|state| state.as_ref().map_or_else(String::new, |state| format!("Step {}", state.step)))}
                </span>
                <span class="sequence-chip" hidden=move || !event.with(|event| event.as_ref().is_some_and(|event| event.sequence_point))>
                    <span class="sequence-dot" aria-hidden="true"></span>"Sequence point"
                </span>
            </div>
            <h2 id="narration-title" class="narration-title" tabindex="-1">
                {move || event.with(|event| match event {
                    | Some(event) => super::rich_text(&event.title),
                    | None => "Loading the abstract machine…".into_any(),
                })}
            </h2>
            <p class="flow" aria-hidden="true" hidden=move || event.with(|event| event.as_ref().is_none_or(|event| event.flow.is_empty()))>
                {move || event.with(|event| event.as_ref().map(|event| flow_groups(&event.flow)))}
            </p>
            <p class="narration-lead">
                {move || event.with(|event| match event {
                    | Some(event) => names.with(|names| explained(lead(event), &mut Vec::new(), names, tip, card)),
                    | None => placeholder.get().into_any(),
                })}
            </p>
            <aside class="narration-note" aria-label="Why it matters" hidden=move || note.with(Option::is_none)>
                <h3>"Why it matters"</h3>
                <p>
                    {move || note.get().and_then(|note| event.with(|event| event.as_ref().map(|event| {
                        // Terms the lead marked stay plain here.
                        let mut seen = Vec::new();
                        let _ = explained_pieces(lead(event), &mut seen);
                        names.with(|names| explained(&note, &mut seen, names, tip, card))
                    })))}
                </p>
            </aside>
            <p class="return-callout" hidden=move || returned.with(Option::is_none)>
                "Program finished: main returned "<strong>{move || returned.get().unwrap_or_default()}</strong>
            </p>
            <div class="fault-callout" hidden=move || diagnostic.with(Option::is_none)>
                <p class="fault-kind">{move || diagnostic.with(|d| d.as_ref().map(crate::snapshot::Diagnostic::kind_label).unwrap_or_default())}</p>
                <p class="fault-message">{move || diagnostic.with(|d| d.as_ref().map(|d| super::rich_text(&sentence_case(&d.message))))}</p>
                <p class="fault-note" hidden=move || diagnostic.with(|d| d.as_ref().is_none_or(|d| d.note.is_empty()))>
                    {move || diagnostic.with(|d| d.as_ref().map(|d| d.note.clone()).unwrap_or_default())}
                </p>
            </div>
            <Clauses event />
            <Details event note names open=details tip card />
            <Recent recent on_jump />
            <TermTip tip />
        </article>
    }
}

/// The names of the objects the program has created, so the text can colour
/// them as the source, the tree and the memory diagram do.
fn object_names(snapshot: Memo<Option<Snapshot>>) -> Memo<Vec<String>> {
    Memo::new(move |_| {
        snapshot.with(|state| {
            state.as_ref().map_or_else(Vec::new, |state| {
                state
                    .objects
                    .iter()
                    .map(|object| object.name.clone())
                    .collect()
            })
        })
    })
}

/// The details behind the lead: the full explanation, what each cited
/// paragraph says, and the step's terms in the glossary.
#[component]
fn Details(
    event: Memo<Option<StepEvent>>,
    note: Memo<Option<String>>,
    names: Memo<Vec<String>>,
    /// Whether the learner opened them; they stay open from step to step.
    open: RwSignal<bool>,
    tip: RwSignal<Option<Tip>>,
    card: NodeRef<leptos::html::Article>,
) -> impl IntoView {
    view! {
        <details
            class="narration-details"
            hidden=move || event.with(|event| event.as_ref().is_none_or(|event| event.explanation.is_empty() || event.explanation == event.summary))
            prop:open=move || open.get()
            on:toggle=move |toggle| {
                if let Some(now) = is_open(&toggle) {
                    open.set(now);
                }
            }
        >
            <summary>"How C defines this step"</summary>
            // The badge's meaning, which its tooltip gives only to a mouse.
            <p class="narration-kind">
                {move || event.with(|event| event.as_ref().map(|event| category_hint(&event.category)))}
            </p>
            <p class="narration-body">
                {move || event.with(|event| event.as_ref().map(|event| {
                    // Terms marked in the lead or the note stay plain.
                    let mut seen = Vec::new();
                    let _ = explained_pieces(lead(event), &mut seen);
                    if let Some(note) = note.get() {
                        let _ = explained_pieces(&note, &mut seen);
                    }
                    names.with(|names| explained(&event.explanation, &mut seen, names, tip, card))
                }))}
            </p>
            <Glosses event />
            <Terms event note />
        </details>
    }
}

/// The paragraphs of C99 the step applied, each a link to the standard
/// with its heading.
#[component]
fn Clauses(event: Memo<Option<StepEvent>>) -> impl IntoView {
    view! {
        <ul class="clauses" aria-label="C standard rules applied" hidden=move || event.with(|event| event.as_ref().is_none_or(|event| event.clauses.is_empty()))>
            {move || event.with(|event| {
                event
                    .as_ref()
                    .map(|event| event.clauses.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|clause| {
                        let id = format!("C99 §{}", clause.id);
                        let label = match clause.link() {
                            | Some(link) => view! {
                                <a class="clause-id" href=link target="_blank" rel="noopener" title=format!("{}: open the standard at this paragraph", clause.citation)>
                                    {id}<span class="sr-only">" (opens the standard in a new tab)"</span>
                                </a>
                            }.into_any(),
                            | None => view! { <span class="clause-id" title=clause.citation.clone()>{id}</span> }.into_any(),
                        };
                        view! {
                            <li class="clause">
                                {label}
                                <span class="clause-title">{clause.title}</span>
                            </li>
                        }
                    })
                    .collect_view()
            })}
        </ul>
    }
}

/// What each cited paragraph says, in plain words.
#[component]
fn Glosses(event: Memo<Option<StepEvent>>) -> impl IntoView {
    view! {
        <dl class="clause-glosses" aria-label="What the cited rules say">
            {move || event.with(|event| {
                event
                    .as_ref()
                    .map(|event| event.clauses.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .map(|clause| view! {
                        <dt>{format!("§{}", clause.id)}</dt>
                        <dd>{clause.gloss}</dd>
                    })
                    .collect_view()
            })}
        </dl>
    }
}

/// Links to the glossary entries of the terms this step's text uses.
#[component]
fn Terms(event: Memo<Option<StepEvent>>, note: Memo<Option<String>>) -> impl IntoView {
    let terms = move || {
        event.with(|event| {
            let Some(event) = event else {
                return Vec::new();
            };
            let mut seen = Vec::new();
            let _ = glossary::pieces(lead(event), &mut seen);
            if let Some(note) = note.get() {
                let _ = glossary::pieces(&note, &mut seen);
            }
            let _ = glossary::pieces(&event.explanation, &mut seen);
            glossary::TERMS
                .iter()
                .filter(|term| seen.contains(&term.id))
                .collect::<Vec<_>>()
        })
    };
    view! {
        <p class="narration-terms" hidden=move || terms().is_empty()>
            <span class="narration-terms-label">"In the glossary: "</span>
            {move || {
                let terms = terms();
                let count = terms.len();
                terms
                    .into_iter()
                    .enumerate()
                    .map(|(index, term)| view! {
                        <a href=format!("#term-{}", term.id)>{term.name}</a>
                        {(index + 1 < count).then_some(", ")}
                    })
                    .collect_view()
            }}
        </p>
    }
}

/// The sentence the card leads with: the summary, or the explanation for
/// an engine that sends none.
fn lead(event: &StepEvent) -> &str {
    if event.summary.is_empty() {
        &event.explanation
    } else {
        &event.summary
    }
}

/// Whether a toggled `<details>` is now open.
#[cfg(feature = "hydrate")]
fn is_open(event: &leptos::ev::Event) -> Option<bool> {
    use wasm_bindgen::JsCast;
    event
        .target()?
        .dyn_into::<web_sys::Element>()
        .ok()
        .map(|element| element.has_attribute("open"))
}

#[cfg(not(feature = "hydrate"))]
fn is_open(_event: &leptos::ev::Event) -> Option<bool> {
    None
}

/// The steps before this one, to jump back to, folded under their count so
/// the expression tree and memory stay near the step.
#[component]
fn Recent(recent: Memo<Vec<(usize, String, String)>>, on_jump: Callback<usize>) -> impl IntoView {
    view! {
        <details class="recent" hidden=move || recent.with(Vec::is_empty)>
            <summary>
                <h3>"Earlier steps"</h3>
                <span class="recent-latest">
                    {move || recent.with(|recent| recent.first().map(|(index, title, _)| format!("{index} · {}", title.replace('`', ""))))}
                </span>
            </summary>
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
        </details>
    }
}

/// The open definition: which term, and where in the card to draw it.
#[derive(Clone, Copy, PartialEq)]
struct Tip {
    term:  &'static Term,
    style: Placement,
}

#[derive(Clone, Copy, PartialEq)]
struct Placement {
    left:   f64,
    width:  f64,
    /// Below the term, from the card's top; or above it, from the card's
    /// bottom.
    offset: f64,
    above:  bool,
}

/// The definition of the term under the pointer or focus, drawn in the card
/// below (or above) the term. It stays while the pointer moves onto it, and
/// Escape closes it.
#[component]
fn TermTip(tip: RwSignal<Option<Tip>>) -> impl IntoView {
    let style = move || {
        tip.with(|tip| {
            tip.map_or_else(String::new, |tip| {
                let place = tip.style;
                let vertical = if place.above { "bottom" } else { "top" };
                format!(
                    "left: {:.0}px; width: {:.0}px; {vertical}: {:.0}px",
                    place.left, place.width, place.offset
                )
            })
        })
    };
    view! {
        <div
            class="term-tip"
            class:is-above=move || tip.with(|tip| tip.is_some_and(|tip| tip.style.above))
            id="term-tip"
            role="tooltip"
            hidden=move || tip.with(Option::is_none)
            style=style
            on:mouseleave=move |event| {
                if !leaves_for_term(&event) {
                    tip.set(None);
                }
            }
        >
            {move || tip.get().map(|tip| view! {
                <span class="term-tip-word">{tip.term.name}</span>
                <span class="term-tip-text">{tip.term.definition}</span>
                <span class="term-tip-clause">{format!("C99 §{}", tip.term.clause)}</span>
            })}
        </div>
    }
}

/// `text` with its `` `code` `` spans and the first use of each glossary
/// term not in `seen` marked; code naming one of the program's objects takes
/// its colour.
fn explained(
    text: &str,
    seen: &mut Vec<&'static str>,
    names: &[String],
    tip: RwSignal<Option<Tip>>,
    card: NodeRef<leptos::html::Article>,
) -> AnyView {
    explained_pieces(text, seen)
        .into_iter()
        .map(|(code, piece)| match piece {
            | Piece::Text(text) if code && names.iter().any(|name| name == text) =>
                view! { <code class=format!("is-object {}", palette::class(text))>{text.to_owned()}</code> }
                    .into_any(),
            | Piece::Text(text) if code => view! { <code>{text.to_owned()}</code> }.into_any(),
            | Piece::Text(text) => prose(text),
            | Piece::Term(written, term) => term_button(written.to_owned(), term, tip, card),
        })
        .collect_view()
        .into_any()
}

/// `text` with its first letter capitalized, as the start of a sentence.
fn sentence_case(text: &str) -> String {
    let mut characters = text.chars();
    characters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(characters).collect()
    })
}

/// Plain text, with "x86-64" kept on one line.
fn prose(text: &str) -> AnyView {
    text.split("x86-64")
        .enumerate()
        .map(|(index, part)| {
            let part = part.to_owned();
            if index == 0 {
                part.into_any()
            } else {
                view! { <span class="nowrap">"x86-64"</span>{part} }.into_any()
            }
        })
        .collect_view()
        .into_any()
}

/// `text` split into code spans (`true`) and prose, the prose split at
/// glossary terms.
fn explained_pieces<'a>(text: &'a str, seen: &mut Vec<&'static str>) -> Vec<(bool, Piece<'a>)> {
    let mut pieces = Vec::new();
    for (index, part) in text.split('`').enumerate() {
        if index % 2 == 1 {
            pieces.push((true, Piece::Text(part)));
        } else {
            pieces.extend(
                glossary::pieces(part, seen)
                    .into_iter()
                    .map(|piece| (false, piece)),
            );
        }
    }
    pieces
}

fn term_button(
    written: String,
    term: &'static Term,
    tip: RwSignal<Option<Tip>>,
    card: NodeRef<leptos::html::Article>,
) -> AnyView {
    let open = move || tip.with(|tip| tip.is_some_and(|tip| std::ptr::eq(tip.term, term)));
    let show = move |target: Option<web_sys::EventTarget>| {
        if let Some(style) = placement(target, card) {
            tip.set(Some(Tip { term, style }));
        }
    };
    view! {
        <button
            type="button"
            class="term"
            data-term=term.id
            aria-describedby=move || open().then_some("term-tip")
            // A tap on a touch screen also sends pointer and mouse events,
            // so hovering opens the tip only for a mouse, and a click (or
            // tap) only ever opens it; blur, Escape and leaving close it.
            on:pointerenter=move |event| {
                if event.pointer_type() == "mouse" {
                    show(event.target());
                }
            }
            on:focus=move |event| show(event.target())
            on:click=move |event| show(event.target())
            on:pointerleave=move |event| {
                if event.pointer_type() == "mouse" && !leaves_for_tip(&event) {
                    tip.set(None);
                }
            }
            on:blur=move |_| tip.set(None)
            on:keydown=move |event| {
                if event.key() == "Escape" && open() {
                    event.stop_propagation();
                    tip.set(None);
                }
            }
        >
            {written}
        </button>
    }
    .into_any()
}

/// Where the tip for the term `target` goes inside the card: under the term,
/// or above it when the window has no room below, and never wider than the
/// card.
#[cfg(feature = "hydrate")]
fn placement(
    target: Option<web_sys::EventTarget>,
    card: NodeRef<leptos::html::Article>,
) -> Option<Placement> {
    use wasm_bindgen::JsCast;
    let term = target?
        .dyn_into::<web_sys::Element>()
        .ok()?
        .get_bounding_client_rect();
    let card = card.get_untracked()?.get_bounding_client_rect();
    let width = (card.width() - 24.0).clamp(0.0, 340.0);
    let left = (term.left() - card.left()).clamp(12.0, (card.width() - width - 12.0).max(12.0));
    let window = window();
    let height = window.inner_height().ok()?.as_f64()?;
    let above = term.bottom() + 170.0 > height && term.top() > 190.0;
    let offset = if above {
        card.bottom() - term.top() + 6.0
    } else {
        term.bottom() - card.top() + 6.0
    };
    Some(Placement {
        left,
        width,
        offset,
        above,
    })
}

#[cfg(not(feature = "hydrate"))]
fn placement(
    _target: Option<web_sys::EventTarget>,
    _card: NodeRef<leptos::html::Article>,
) -> Option<Placement> {
    None
}

/// Whether the pointer leaves a term for the open tip, which keeps it open.
fn leaves_for_tip(event: &leptos::ev::MouseEvent) -> bool {
    related_has_class(event, "term-tip")
}

/// Whether the pointer leaves the tip for a term, whose own events decide.
fn leaves_for_term(event: &leptos::ev::MouseEvent) -> bool {
    related_has_class(event, "term")
}

#[cfg(feature = "hydrate")]
fn related_has_class(event: &leptos::ev::MouseEvent, class: &str) -> bool {
    use wasm_bindgen::JsCast;
    event
        .related_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
        .and_then(|element| element.closest(&format!(".{class}")).ok().flatten())
        .is_some()
}

#[cfg(not(feature = "hydrate"))]
fn related_has_class(_event: &leptos::ev::MouseEvent, _class: &str) -> bool {
    false
}

/// The value flow's chips, each arrow or operator grouped with the chip
/// after it so a line never ends on an arrow.
fn flow_groups(flow: &[FlowPart]) -> AnyView {
    let mut groups: Vec<Vec<&FlowPart>> = Vec::new();
    let mut pending = Vec::new();
    for part in flow {
        pending.push(part);
        if !matches!(part.role.as_str(), "arrow" | "operator") {
            groups.push(std::mem::take(&mut pending));
        }
    }
    if !pending.is_empty() {
        groups.push(pending);
    }
    groups
        .into_iter()
        .map(|group| view! { <span class="flow-group">{group.into_iter().map(chip).collect_view()}</span> })
        .collect_view()
        .into_any()
}

/// One chip of the value flow.
fn chip(part: &FlowPart) -> AnyView {
    let colour = part
        .object
        .as_deref()
        .map(palette::class)
        .unwrap_or_default();
    let role = part.role.clone();
    let text = part.text.clone();
    let ty = part.ty.clone();
    match role.as_str() {
        | "arrow" => view! {
            <span class="flow-arrow"><span class="flow-verb">{text}</span></span>
        }
        .into_any(),
        | "operator" | "separator" =>
            view! { <span class=format!("flow-{role}")>{text}</span> }.into_any(),
        | _ => view! {
            <span class=format!("flow-chip is-{role} {colour}")>
                <span class="flow-text">{text}</span>
                {ty.map(|ty| view! { <span class="flow-type">{ty}</span> })}
            </span>
        }
        .into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        explained_pieces,
        lead,
    };
    use crate::{
        glossary::Piece,
        snapshot::StepEvent,
    };

    #[test]
    fn the_card_leads_with_the_summary() {
        let mut event = StepEvent {
            summary: "One sentence.".to_owned(),
            explanation: "The details.".to_owned(),
            ..StepEvent::default()
        };
        assert_eq!(lead(&event), "One sentence.");
        event.summary.clear();
        assert_eq!(lead(&event), "The details.");
    }

    #[test]
    fn a_term_the_lead_marks_stays_plain_in_the_details() {
        let terms = |text: &str, seen: &mut Vec<&'static str>| {
            explained_pieces(text, seen)
                .into_iter()
                .filter_map(|(_, piece)| match piece {
                    | Piece::Term(_, term) => Some(term.id),
                    | Piece::Text(_) => None,
                })
                .collect::<Vec<_>>()
        };
        let mut seen = Vec::new();
        assert_eq!(
            terms("The name `x` is an lvalue.", &mut seen),
            ["lvalue"]
        );
        assert_eq!(
            terms("An lvalue designates an object.", &mut seen),
            ["object"]
        );
    }
}

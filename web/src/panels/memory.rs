//! The memory diagram: one box per active call, innermost on top, holding its
//! objects. Arrays show their elements; a pointer is drawn as an arrow to the
//! element it points at, including the one-past-the-end position. Objects
//! whose lifetime has ended stay visible, faded, so their end can be seen.

use std::collections::HashSet;

use leptos::{
    html,
    prelude::*,
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
pub fn Memory(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let canvas = NodeRef::<html::Div>::new();
    let arrows = RwSignal::new(Vec::<Arrow>::new());
    let size = RwSignal::new((0.0, 0.0));
    Effect::new(move || {
        snapshot.track();
        arrows::schedule(canvas, arrows, size);
    });
    #[cfg(feature = "hydrate")]
    Effect::new(move || {
        let handle = window_event_listener(leptos::ev::resize, move |_| {
            arrows::schedule(canvas, arrows, size);
        });
        on_cleanup(move || handle.remove());
    });
    let layout = Memo::new(move |_| snapshot.with(|state| state.as_ref().map(layout)));
    view! {
        <section class="card memory-card" aria-labelledby="memory-title">
            <header class="card-header">
                <h2 id="memory-title">"Memory"</h2>
                <span class="card-subtitle">"Call stack, innermost call on top"</span>
            </header>
            <div class="memory-canvas" node_ref=canvas>
                {move || layout.with(|layout| match layout {
                    | Some(layout) if !layout.frames.is_empty() || !layout.ended.is_empty() => view! {
                        <div class="frames">
                            {layout.frames.iter().map(|frame| view! {
                                <div class="frame" class:is-active=frame.active>
                                    <div class="frame-head">
                                        <span class="frame-name">{format!("{}()", frame.name)}</span>
                                        {frame.waiting.clone().map(|call| view! {
                                            <span class="frame-waiting">"waiting for "<code>{call}</code></span>
                                        })}
                                        <span class="frame-id">{format!("frame #{}", frame.id)}</span>
                                    </div>
                                    <div class="frame-objects">
                                        {if frame.objects.is_empty() {
                                            view! { <p class="frame-empty">"No objects yet"</p> }.into_any()
                                        } else {
                                            frame.objects.iter().map(|object| object_view(object, layout)).collect_view().into_any()
                                        }}
                                    </div>
                                </div>
                            }).collect_view()}
                        </div>
                        {(!layout.ended.is_empty()).then(|| view! {
                            <div class="ended-calls">
                                <h3>"Ended with their calls"</h3>
                                <div class="frame-objects">
                                    {layout.ended.iter().map(|object| object_view(object, layout)).collect_view()}
                                </div>
                            </div>
                        })}
                    }.into_any(),
                    | Some(_) => view! { <p class="empty-state">"No objects yet. They appear as declarations run."</p> }.into_any(),
                    | None => view! { <p class="empty-state">"Objects, their values and their lifetimes will appear here."</p> }.into_any(),
                })}
                <svg
                    class="arrows"
                    aria-hidden="true"
                    width=move || size.get().0
                    height=move || size.get().1
                >
                    <defs>
                        <marker id="arrowhead" viewBox="0 0 10 10" refX="8.5" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
                            <path d="M0 0L10 5L0 10z" class="arrowhead" />
                        </marker>
                    </defs>
                    {move || arrows.get().into_iter().map(|arrow| view! {
                        <path d=arrow.path class=format!("arrow {}", arrow.class) marker-end="url(#arrowhead)" />
                        <circle cx=arrow.start.0 cy=arrow.start.1 r="3.2" class=format!("arrow-origin {}", arrow.class) />
                    }).collect_view()}
                </svg>
            </div>
        </section>
    }
}

/// One drawn pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    path:  String,
    start: (f64, f64),
    class: String,
}

#[derive(PartialEq)]
struct Layout {
    frames:   Vec<FrameBox>,
    /// Objects of calls that have returned.
    ended:    Vec<Object>,
    /// Objects touched by the last step.
    touched:  HashSet<usize>,
    /// Array objects some pointer targets one past the end.
    one_past: HashSet<usize>,
}

#[derive(PartialEq)]
struct FrameBox {
    id:      usize,
    name:    String,
    active:  bool,
    waiting: Option<String>,
    objects: Vec<Object>,
}

fn layout(snapshot: &Snapshot) -> Layout {
    let live: HashSet<usize> = snapshot.stack.iter().map(|frame| frame.id).collect();
    let mut one_past = HashSet::new();
    for object in &snapshot.objects {
        let mut visit = |value: &Value| {
            if let Value::Pointer {
                target: Some(target),
                one_past: true,
                ..
            } = value
            {
                one_past.insert(*target);
            }
        };
        match &object.value {
            | Value::Array { elements } => elements.iter().for_each(&mut visit),
            | value => visit(value),
        }
    }
    Layout {
        frames: snapshot
            .stack
            .iter()
            .enumerate()
            .map(|(index, frame)| FrameBox {
                id:      frame.id,
                name:    frame.name.clone(),
                active:  index == 0,
                waiting: frame.waiting.clone(),
                objects: snapshot
                    .objects
                    .iter()
                    .filter(|object| object.frame_id == frame.id)
                    .cloned()
                    .collect(),
            })
            .collect(),
        ended: snapshot
            .objects
            .iter()
            .filter(|object| !live.contains(&object.frame_id))
            .cloned()
            .collect(),
        touched: snapshot.event.objects.iter().copied().collect(),
        one_past,
    }
}

fn object_view(object: &Object, layout: &Layout) -> AnyView {
    let colour = palette::class(&object.name);
    let touched = layout.touched.contains(&object.id);
    let cells = match &object.value {
        | Value::Array { elements } => {
            let mut cells: Vec<AnyView> = elements
                .iter()
                .enumerate()
                .map(|(index, value)| cell(object, index, value, true))
                .collect();
            if layout.one_past.contains(&object.id) {
                cells.push(
                    view! {
                        <div class="cell is-one-past" data-cell=format!("{}-{}", object.id, elements.len())>
                            <span class="cell-index">{format!("[{}]", elements.len())}</span>
                            <span class="cell-value">"end"</span>
                        </div>
                    }
                    .into_any(),
                );
            }
            cells
        },
        | value => vec![cell(object, 0, value, false)],
    };
    let lifetime = if object.alive { "" } else { "lifetime ended" };
    view! {
        <div
            class=format!("object {colour}")
            class:is-ended=!object.alive
            class:is-touched=touched
            data-object=object.id
        >
            <div class="object-head">
                <span class="object-swatch" aria-hidden="true"></span>
                <span class="object-name">{object.name.clone()}</span>
                <span class="object-type">{object.ty.clone()}</span>
                {object.parameter.then(|| view! { <span class="object-tag">"parameter"</span> })}
                <span class="object-lifetime">{lifetime}</span>
            </div>
            <div class="cells">{cells}</div>
        </div>
    }
    .into_any()
}

fn cell(object: &Object, index: usize, value: &Value, indexed: bool) -> AnyView {
    let id = format!("{}-{index}", object.id);
    let label = indexed.then(|| view! { <span class="cell-index">{format!("[{index}]")}</span> });
    match value {
        | Value::Indeterminate => view! {
            <div class="cell is-indeterminate" data-cell=id title="Indeterminate: no value has been stored">
                {label}<span class="cell-value">"?"</span>
            </div>
        }
        .into_any(),
        | Value::Integer { text } => view! {
            <div class="cell" data-cell=id>{label}<span class="cell-value">{text.clone()}</span></div>
        }
        .into_any(),
        | Value::Pointer { target: None, .. } => view! {
            <div class="cell is-pointer is-null" data-cell=id>{label}<span class="cell-value">"NULL"</span></div>
        }
        .into_any(),
        | &Value::Pointer {
            target: Some(target),
            offset,
            ended,
            ..
        } => view! {
            <div
                class="cell is-pointer"
                class:is-dangling=ended
                data-cell=id
                data-points=format!("{target}-{offset}")
                data-colour=palette::class(&object.name)
            >
                {label}<span class="cell-value">{if ended { "dangling" } else { "•" }}</span>
            </div>
        }
        .into_any(),
        | Value::Array { .. } => view! { <div class="cell" data-cell=id>{label}"{…}"</div> }.into_any(),
    }
}

mod arrows {
    use leptos::{
        html,
        prelude::*,
    };

    use super::Arrow;

    /// Recompute the pointer arrows after the next layout.
    pub fn schedule(
        canvas: NodeRef<html::Div>,
        arrows: RwSignal<Vec<Arrow>>,
        size: RwSignal<(f64, f64)>,
    ) {
        request_animation_frame(move || {
            let Some(canvas) = canvas.get_untracked() else {
                return;
            };
            let (drawn, extent) = measure(&canvas);
            size.set(extent);
            arrows.set(drawn);
        });
    }

    fn measure(canvas: &web_sys::HtmlDivElement) -> (Vec<Arrow>, (f64, f64)) {
        let origin = canvas.get_bounding_client_rect();
        let extent = (
            f64::from(canvas.scroll_width()),
            f64::from(canvas.scroll_height()),
        );
        let mut drawn = Vec::new();
        let Ok(sources) = canvas.query_selector_all("[data-points]") else {
            return (drawn, extent);
        };
        for index in 0..sources.length() {
            let Some(source) = sources
                .item(index)
                .and_then(|node| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(node).ok())
            else {
                continue;
            };
            let Some(points) = source.get_attribute("data-points") else {
                continue;
            };
            let Ok(Some(target)) = canvas.query_selector(&format!("[data-cell=\"{points}\"]"))
            else {
                continue;
            };
            let from = source.get_bounding_client_rect();
            let to = target.get_bounding_client_rect();
            let scroll = (
                f64::from(canvas.scroll_left()),
                f64::from(canvas.scroll_top()),
            );
            let local =
                |x: f64, y: f64| (x - origin.left() + scroll.0, y - origin.top() + scroll.1);
            let start = local(
                from.left() + from.width() / 2.0,
                from.top() + from.height() / 2.0 + 4.0,
            );
            let downward = to.top() > from.bottom();
            let upward = to.bottom() < from.top();
            let end = if downward {
                local(to.left() + to.width() / 2.0, to.top())
            } else if upward {
                local(to.left() + to.width() / 2.0, to.bottom())
            } else {
                local(to.left() + to.width() / 2.0, to.top())
            };
            let bend = if upward { -36.0 } else { 36.0 };
            let path = if downward || upward {
                format!(
                    "M{:.1} {:.1} C{:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
                    start.0,
                    start.1,
                    start.0,
                    start.1 + bend,
                    end.0,
                    end.1 - bend,
                    end.0,
                    end.1
                )
            } else {
                // Same row: loop under the cells, clear of the index labels.
                let from = local(from.left() + from.width() / 2.0, from.bottom());
                let to = local(to.left() + to.width() / 2.0, to.bottom() + 1.0);
                let drop = from.1.max(to.1) + 26.0;
                format!(
                    "M{:.1} {:.1} C{:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}",
                    from.0, from.1, from.0, drop, to.0, drop, to.0, to.1
                )
            };
            let start = if downward || upward {
                start
            } else {
                local(from.left() + from.width() / 2.0, from.bottom())
            };
            drawn.push(Arrow {
                path,
                start,
                class: source.get_attribute("data-colour").unwrap_or_default(),
            });
        }
        (drawn, extent)
    }
}

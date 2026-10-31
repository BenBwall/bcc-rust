//! The run as a multitrack tape (Figure 1 of the machine view). Every object
//! the program has created is a lane over time, as in Marey's 1885 train
//! graph and in a digital timing diagram: a lane is born, carries its value
//! in hexagonal bus segments (cross-hatched while indeterminate), changes and
//! ends. A call is a rule over its objects' lanes, solid while it runs and
//! dashed while it waits on a call of its own. What a store or a call took
//! its value from is a slanted line between lanes. The playhead marks the
//! step the page shows; the steps recorded after it are dimmed, and the tape
//! past the last recorded step is blank. Reels at both ends turn as the
//! playhead moves, the left one taking up what has played.
//!
//! Calls are grouped by function and depth on the call stack, so a loop's
//! repeated calls reuse one set of lanes, as a timing diagram reuses a
//! register's line; a group with no live call folds to one row that still
//! shows each call as a bar.

use std::{
    collections::HashMap,
    fmt::Write as _,
};

use leptos::{
    html,
    prelude::*,
};

use super::knob::capture;
use crate::{
    palette,
    snapshot::{
        Object,
        Snapshot,
        Value,
    },
};

/// Height of one lane, in pixels.
const LANE: f64 = 28.0;
/// Height of the step axis and clock lane above the lanes.
const AXIS: f64 = 30.0;
/// Half the height of a bus segment.
const BUS: f64 = 9.0;
/// The narrowest and widest step a fitted tape gives each step.
const FIT_MIN: f64 = 14.0;
const FIT_MAX: f64 = 48.0;
/// The narrowest and widest step the zoom knob reaches: zoomed out, a run
/// of a thousand steps fits a laptop's screen.
const ZOOM_MIN: f64 = 1.0;
const ZOOM_MAX: f64 = 64.0;
/// Steps of blank tape drawn after the last recorded one.
const LEADER: usize = 6;

/// `count` as a drawing coordinate. Counts here (steps, lanes, characters)
/// stay far below 2^32.
pub fn px(count: usize) -> f64 {
    f64::from(u32::try_from(count).unwrap_or(u32::MAX))
}

/// The step under `x` pixels into a tape of `step`-pixel steps.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Clamped to zero and below the run's length by the caller."
)]
fn step_at(x: f64, step: f64) -> usize {
    (x / step).floor().max(0.0) as usize
}

/// A scroll offset in whole pixels.
#[expect(clippy::cast_possible_truncation, reason = "Tapes are far narrower than 2^31 pixels.")]
fn whole(x: f64) -> i32 {
    x.round() as i32
}

/// Every object of the run over the recorded steps (history indices).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reel {
    pub groups:     Vec<Group>,
    pub flows:      Vec<Flow>,
    /// Each recorded step's number, and whether it ended at a sequence
    /// point.
    pub steps:      Vec<(usize, bool)>,
    /// Whether the engine can record another step after the last.
    pub open:       bool,
    /// What each step did to each object it used: `read`, `write`,
    /// `create`, `end`, `designate` or `address`.
    pub access:     Vec<Vec<(usize, String)>>,
}

/// The calls of one function at one depth of the call stack, and the lanes
/// of their objects.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub key:   String,
    pub name:  String,
    pub depth: usize,
    pub calls: Vec<Call>,
    pub lanes: Vec<Lane>,
}

/// One call: the steps its frame existed at, and those it spent waiting.
#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub frame:   usize,
    pub first:   usize,
    pub last:    usize,
    /// Runs of steps (start, end exclusive) the call waited on another, and
    /// the source of that call.
    pub waiting: Vec<(usize, usize, String)>,
}

/// One object name of a group, over every call of the group.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub key:      String,
    pub name:     String,
    pub ty:       String,
    pub objects:  Vec<usize>,
    pub segments: Vec<Segment>,
    /// Steps at which an object of the lane ended.
    pub ends:     Vec<usize>,
}

/// A run of steps over which one object held one value; `None` while it was
/// indeterminate.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub from:   usize,
    pub to:     usize,
    pub object: usize,
    pub value:  Option<String>,
}

/// A value read from one object at one step and stored into (or passed to)
/// another at a later one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flow {
    pub from: (usize, usize),
    pub to:   (usize, usize),
}

impl Reel {
    /// The tape of `history`, in order.
    #[expect(clippy::too_many_lines, reason = "One pass over the history: calls, objects, flows.")]
    pub fn record(history: &[Snapshot]) -> Self {
        let mut reel = Self::default();
        let mut group_of_frame: HashMap<usize, usize> = HashMap::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        // Objects read since the last store, call or full expression.
        let mut reads: Vec<(usize, usize)> = Vec::new();
        for (step, snapshot) in history.iter().enumerate() {
            reel.steps
                .push((snapshot.step, snapshot.event.sequence_point));
            let height = snapshot.stack.len();
            for (place, frame) in snapshot.stack.iter().enumerate() {
                let depth = height - place;
                let key = format!("{}@{depth}", frame.name);
                let at = *index.entry(key.clone()).or_insert_with(|| {
                    reel.groups.push(Group {
                        key,
                        name: frame.name.clone(),
                        depth,
                        calls: Vec::new(),
                        lanes: Vec::new(),
                    });
                    reel.groups.len() - 1
                });
                group_of_frame.insert(frame.id, at);
                let group = &mut reel.groups[at];
                let call = match group.calls.iter_mut().find(|call| call.frame == frame.id) {
                    | Some(call) => call,
                    | None => {
                        group.calls.push(Call {
                            frame:   frame.id,
                            first:   step,
                            last:    step,
                            waiting: Vec::new(),
                        });
                        group.calls.last_mut().expect("just pushed")
                    },
                };
                call.last = step;
                if let Some(on) = &frame.waiting {
                    match call.waiting.last_mut() {
                        | Some((_, end, source)) if *end == step && source == on => *end = step + 1,
                        | _ => call.waiting.push((step, step + 1, on.clone())),
                    }
                }
            }
            for object in &snapshot.objects {
                let Some(&at) = group_of_frame.get(&object.frame_id) else {
                    continue;
                };
                let group = &mut reel.groups[at];
                let lane = match group.lanes.iter_mut().position(|lane| lane.name == object.name) {
                    | Some(lane) => &mut group.lanes[lane],
                    | None => {
                        group.lanes.push(Lane {
                            key:      format!("{}:{}", group.key, object.name),
                            name:     object.name.clone(),
                            ty:       object.ty.clone(),
                            objects:  Vec::new(),
                            segments: Vec::new(),
                            ends:     Vec::new(),
                        });
                        group.lanes.last_mut().expect("just pushed")
                    },
                };
                if !lane.objects.contains(&object.id) {
                    lane.objects.push(object.id);
                }
                let last = lane.segments.last_mut();
                if object.alive {
                    let value = value_text(&object.value, &snapshot.objects);
                    match last {
                        | Some(segment)
                            if segment.to == step
                                && segment.object == object.id
                                && segment.value == value =>
                            segment.to = step + 1,
                        | _ => lane.segments.push(Segment {
                            from: step,
                            to: step + 1,
                            object: object.id,
                            value,
                        }),
                    }
                } else if last.is_some_and(|segment| segment.object == object.id && segment.to == step) {
                    lane.ends.push(step);
                }
            }
            let event = &snapshot.event;
            reel.access.push(
                event
                    .access
                    .iter()
                    .map(|access| (access.object, access.kind.clone()))
                    .collect(),
            );
            reads.extend(
                event
                    .access
                    .iter()
                    .filter(|access| access.kind == "read")
                    .map(|access| (access.object, step)),
            );
            let call = event.kind == "call";
            let stores: Vec<usize> = event
                .access
                .iter()
                .filter(|access| access.kind == "write" || (call && access.kind == "create"))
                .map(|access| access.object)
                .collect();
            for &object in &stores {
                for &read in &reads {
                    reel.flows.push(Flow {
                        from: read,
                        to:   (object, step),
                    });
                }
            }
            // A return keeps what its expression read: the value flows on
            // into the caller's store.
            if !stores.is_empty() || (event.sequence_point && !call && event.kind != "return") {
                reads.clear();
            }
        }
        reel.open = history.last().is_some_and(Snapshot::can_step);
        reel
    }
}

/// An object's value as the tape and the cells write it, or `None` while it
/// is indeterminate.
pub fn value_text(value: &Value, objects: &[Object]) -> Option<String> {
    match value {
        | Value::Indeterminate => None,
        | Value::Integer { text } => Some(text.clone()),
        | Value::Array { elements } => Some(format!(
            "{{{}}}",
            elements
                .iter()
                .map(|element| value_text(element, objects).unwrap_or_else(|| "?".to_owned()))
                .collect::<Vec<_>>()
                .join(",")
        )),
        | Value::Pointer { target: None, .. } => Some("null".to_owned()),
        | Value::Pointer { ended: true, .. } => Some("→ ended".to_owned()),
        | &Value::Pointer {
            target: Some(id),
            offset,
            one_past,
            ..
        } => {
            let target = objects.iter().find(|object| object.id == id);
            let name = target.map_or_else(|| format!("#{id}"), |object| object.name.clone());
            Some(match target.map(|object| &object.value) {
                | Some(Value::Array { .. }) if one_past => format!("→{name}+{offset}"),
                | Some(Value::Array { .. }) => format!("→{name}[{offset}]"),
                | _ => format!("→{name}"),
            })
        },
    }
}

/// One row of the deck: the axis, a group's header, or one of its lanes.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Group { at: usize, folded: bool, live: bool },
    Lane { group: usize, lane: usize },
}

fn rows(reel: &Reel, cursor: usize, opened: &[String]) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut order: Vec<usize> = (0..reel.groups.len()).collect();
    order.sort_by_key(|&at| {
        let group = &reel.groups[at];
        (group.depth, group.calls.first().map_or(0, |call| call.first))
    });
    for at in order {
        let group = &reel.groups[at];
        // A call stays open through the step that ends it, so its lanes are
        // on screen while their objects' lifetimes end.
        let live = group
            .calls
            .iter()
            .any(|call| call.first <= cursor && cursor <= call.last + 1);
        let folded = !live && !opened.contains(&group.key);
        rows.push(Row::Group { at, folded, live });
        if !folded {
            rows.extend((0..group.lanes.len()).map(|lane| Row::Lane { group: at, lane }));
        }
    }
    rows
}

/// The deck: reels, the lane labels, and the scrolling tape with its
/// playhead. Dragging the tape, or the playhead, moves through the recorded
/// steps; the arrow keys step (through the page's shortcuts), Home and End go
/// to the first and last recorded step.
#[component]
#[expect(clippy::too_many_lines, reason = "The deck's parts and its pointer and key handling.")]
pub fn Deck(
    history: RwSignal<Vec<Snapshot>>,
    cursor: RwSignal<Option<usize>>,
    /// The scale, as a factor of the one that fits the recorded run to the
    /// screen.
    zoom: RwSignal<f64>,
    /// Where the playhead is on screen, relative to the start of the tape's
    /// viewport, for the line it drops into the note.
    head: RwSignal<Option<f64>>,
    on_jump: Callback<usize>,
) -> impl IntoView {
    let reel = Memo::new(move |_| history.with(|history| Reel::record(history)));
    let opened = RwSignal::new(Vec::<String>::new());
    let tape = NodeRef::<html::Div>::new();
    let width = RwSignal::new(760.0_f64);
    let scroll = RwSignal::new(0.0_f64);
    let at = move || cursor.get().unwrap_or(0);
    let rows = Memo::new(move |_| {
        let cursor = at();
        reel.with(|reel| opened.with(|opened| rows(reel, cursor, opened)))
    });
    let step_width = Memo::new(move |_| {
        let count = px(reel.with(|reel| reel.steps.len()));
        let fit = (width.get() / (count + 3.0)).floor().clamp(FIT_MIN, FIT_MAX);
        (fit * zoom.get()).clamp(ZOOM_MIN, ZOOM_MAX)
    });
    observe_width(tape, width);
    // A step moves the tape, not the head: keep the playhead in the middle
    // stretch of the viewport.
    Effect::new(move || {
        let x = (px(at()) + 0.5) * step_width.get();
        let Some(element) = tape.get() else {
            return;
        };
        let view = f64::from(element.client_width());
        let left = f64::from(element.scroll_left());
        if x < left + view * 0.15 || x > left + view * 0.8 {
            let target = (x - view * 0.7).max(0.0);
            element.set_scroll_left(whole(target));
        }
    });
    Effect::new(move || {
        let x = (px(at()) + 0.5) * step_width.get() - scroll.get();
        let shown = history.with(|history| !history.is_empty()) && x >= 0.0 && x <= width.get();
        head.set(shown.then_some(x));
    });
    let dragging = StoredValue::new(false);
    let index_at = move |client_x: i32| -> Option<usize> {
        let element = tape.get_untracked()?;
        let left = element.get_bounding_client_rect().left();
        let x = f64::from(client_x) - left + f64::from(element.scroll_left());
        let count = history.with_untracked(Vec::len);
        (count > 0).then(|| step_at(x, step_width.get_untracked()).min(count - 1))
    };
    let label = move || {
        cursor.get().and_then(|index| {
            history.with(|history| {
                history.get(index).map(|snapshot| {
                    format!(
                        "Step {} of {}: {}",
                        snapshot.step,
                        history.last().map_or(0, |last| last.step),
                        snapshot.event.title.replace('`', "")
                    )
                })
            })
        })
    };
    let toggle = move |key: String| {
        opened.update(|opened| {
            if let Some(place) = opened.iter().position(|open| *open == key) {
                opened.remove(place);
            } else {
                opened.push(key);
            }
        });
    };
    let played = move || {
        let count = reel.with(|reel| reel.steps.len().max(2)) - 1;
        px(at()) / px(count)
    };
    view! {
        <figure class="deck" aria-labelledby="deck-caption">
            <figcaption class="deck-caption" id="deck-caption">
                <b>"Figure 1"</b>" — Every object of the run, recorded on tape. Drag the tape, or use "<kbd>"←"</kbd>" "<kbd>"→"</kbd>"."
            </figcaption>
            <Spool side="left" fill=Signal::derive(played) turn=Signal::derive(move || px(at()) * 37.0) />
            <div class="deck-labels" aria-hidden="true" style:--lane=format!("{LANE}px")>
                <div class="deck-label is-axis">"Step"</div>
                {move || rows.with(|rows| reel.with(|reel| rows.iter().map(|row| match *row {
                    | Row::Group { at, folded, live } => {
                        let group = &reel.groups[at];
                        let key = group.key.clone();
                        let count = group.calls.len();
                        view! {
                            <div class="deck-label is-group" class:is-folded=folded class:is-live=live>
                                <button type="button" tabindex="-1" on:click=move |_| toggle(key.clone())>
                                    <span class="deck-fold">{if folded { "▸" } else { "▾" }}</span>
                                    {format!("{}()", group.name)}
                                    {(count > 1).then(|| view! { <span class="deck-count">{format!("×{count}")}</span> })}
                                </button>
                            </div>
                        }.into_any()
                    },
                    | Row::Lane { group, lane } => {
                        let lane = &reel.groups[group].lanes[lane];
                        view! {
                            <div class=format!("deck-label is-lane {}", palette::class(&lane.name))>
                                <span class="deck-swatch"></span>
                                <span class="deck-name">{lane.name.clone()}</span>
                                <span class="deck-type">{lane.ty.clone()}</span>
                            </div>
                        }.into_any()
                    },
                }).collect_view()))}
            </div>
            <div
                class="tape"
                node_ref=tape
                tabindex="0"
                role="slider"
                aria-label="Playhead: the step shown"
                aria-valuemin="0"
                aria-valuemax=move || history.with(|history| history.len().saturating_sub(1)).to_string()
                aria-valuenow=move || at().to_string()
                aria-valuetext=label
                on:scroll=move |_| {
                    if let Some(element) = tape.get_untracked() {
                        scroll.set(f64::from(element.scroll_left()));
                    }
                }
                on:pointerdown=move |event| {
                    if event.button() != 0 {
                        return;
                    }
                    dragging.set_value(true);
                    capture(&event);
                    if let Some(index) = index_at(event.client_x()) {
                        on_jump.run(index);
                    }
                }
                on:pointermove=move |event| {
                    if dragging.get_value()
                        && let Some(index) = index_at(event.client_x())
                        && Some(index) != cursor.get_untracked()
                    {
                        on_jump.run(index);
                    }
                }
                on:pointerup=move |_| dragging.set_value(false)
                on:pointercancel=move |_| dragging.set_value(false)
                on:keydown=move |event| {
                    let count = history.with_untracked(Vec::len);
                    let target = match event.key().as_str() {
                        | "Home" => Some(0),
                        | "End" => count.checked_sub(1),
                        | "PageUp" => Some(cursor.get_untracked().unwrap_or(0).saturating_sub(10)),
                        | "PageDown" => Some((cursor.get_untracked().unwrap_or(0) + 10).min(count.saturating_sub(1))),
                        | _ => None,
                    };
                    if let Some(target) = target.filter(|_| count > 0) {
                        event.prevent_default();
                        on_jump.run(target);
                    }
                }
            >
                {move || tape_svg(reel, rows, step_width.get(), at(), width.get())}
            </div>
            <Spool side="right" fill=Signal::derive(move || 1.0 - played()) turn=Signal::derive(move || px(at()) * 37.0) />
        </figure>
    }
}

/// The tape itself, as one SVG.
#[expect(clippy::too_many_lines, reason = "One block per layer of the drawing.")]
fn tape_svg(reel: Memo<Reel>, rows: Memo<Vec<Row>>, step: f64, cursor: usize, view: f64) -> AnyView {
    reel.with(|reel| {
        rows.with(|rows| {
            let count = reel.steps.len();
            let width = (px(count + LEADER) * step).max(view);
            let height = AXIS + px(rows.len()) * LANE + 8.0;
            let x = move |index: usize| px(index) * step;
            let mut y_of_group = HashMap::new();
            let mut y_of_object = HashMap::new();
            for (place, row) in rows.iter().enumerate() {
                let y = AXIS + px(place) * LANE + LANE / 2.0;
                match *row {
                    | Row::Group { at, .. } => {
                        y_of_group.insert(at, y);
                    },
                    | Row::Lane { group, lane } => {
                        for &object in &reel.groups[group].lanes[lane].objects {
                            y_of_object.insert(object, y);
                        }
                    },
                }
            }
            let ahead = move |index: usize| if index > cursor { "is-ahead" } else { "" };
            // Axis: a tick and a number at the first round interval that
            // leaves room for the number.
            let every = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000]
                .into_iter()
                .find(|&every| px(every) * step >= 44.0)
                .unwrap_or(2000);
            let ticks = (0..count)
                .filter(|index| index % every == 0)
                .map(|index| view! {
                    <line class="tape-grid" x1=x(index) y1=AXIS - 4.0 x2=x(index) y2=height />
                    <text class="tape-number" x=x(index) + 3.0 y="11">{reel.steps[index].0}</text>
                })
                .collect_view();
            // The clock lane: one pulse per step, taller at a sequence point
            // (a flat line where steps are too narrow to tell apart).
            let mut clock = format!("M0 {}", AXIS - 6.0);
            for (index, &(_, sequence)) in reel.steps.iter().enumerate().filter(|_| step >= 3.0) {
                let high = if sequence { AXIS - 19.0 } else { AXIS - 13.0 };
                let (left, right) = (x(index) + 1.0, x(index) + step * 0.55);
                _ = write!(
                    clock,
                    " L{left:.1} {low} L{left:.1} {high} L{right:.1} {high} L{right:.1} {low}",
                    low = AXIS - 6.0
                );
            }
            _ = write!(clock, " L{:.1} {}", x(count), AXIS - 6.0);
            let calls = rows
                .iter()
                .filter_map(|row| match *row {
                    | Row::Group { at, .. } => Some(at),
                    | Row::Lane { .. } => None,
                })
                .flat_map(|at| {
                    let y = y_of_group[&at];
                    reel.groups[at].calls.iter().map(move |call| {
                        let mut parts = Vec::new();
                        let mut start = call.first;
                        for (from, to, on) in &call.waiting {
                            if *from > start {
                                parts.push(view! { <line class=format!("tape-call {}", ahead(start)) x1=x(start) y1=y x2=x(*from) y2=y /> }.into_any());
                            }
                            parts.push(view! { <line class=format!("tape-call is-waiting {}", ahead(*from)) x1=x(*from) y1=y x2=x(*to) y2=y /> }.into_any());
                            if x(*to) - x(*from) > 170.0 {
                                parts.push(view! {
                                    <text class=format!("tape-waiting {}", ahead(*from)) x=x(*from) + 6.0 y=y - 5.0>{format!("waiting for {on}")}</text>
                                }.into_any());
                            }
                            start = *to;
                        }
                        if call.last + 1 > start {
                            parts.push(view! { <line class=format!("tape-call {}", ahead(start)) x1=x(start) y1=y x2=x(call.last + 1) y2=y /> }.into_any());
                        }
                        for edge in [call.first, call.last + 1] {
                            if edge < count {
                                parts.push(view! { <line class=format!("tape-edge {}", ahead(edge)) x1=x(edge) y1=y - 6.0 x2=x(edge) y2=y + 6.0 /> }.into_any());
                            }
                        }
                        parts.into_iter().collect_view()
                    })
                })
                .collect_view();
            let lanes = rows
                .iter()
                .filter_map(|row| match *row {
                    | Row::Lane { group, lane } => Some(&reel.groups[group].lanes[lane]),
                    | Row::Group { .. } => None,
                })
                .map(|lane| {
                    let y = y_of_object.get(&lane.objects[0]).copied().unwrap_or(0.0);
                    let used = |object: usize| {
                        reel.access
                            .get(cursor)
                            .and_then(|access| access.iter().find(|(id, _)| *id == object))
                            .map(|(_, kind)| kind.clone())
                    };
                    let segments = lane
                        .segments
                        .iter()
                        .map(|segment| {
                            let here = segment.from <= cursor && cursor < segment.to;
                            bus(segment, y, step, cursor, here.then(|| used(segment.object)).flatten())
                        })
                        .collect_view();
                    let ends = lane.ends.iter().map(|&end| view! {
                        <line class=format!("tape-end {}", ahead(end)) x1=x(end) y1=y - 7.0 x2=x(end) y2=y + 7.0 />
                    }).collect_view();
                    view! { <g class=palette::class(&lane.name)>{segments}{ends}</g> }
                })
                .collect_view();
            let flows = reel
                .flows
                .iter()
                .filter_map(|flow| {
                    let (from, to) = (*y_of_object.get(&flow.from.0)?, *y_of_object.get(&flow.to.0)?);
                    if (from - to).abs() < 1.0 && flow.from.1 == flow.to.1 {
                        return None;
                    }
                    let down = if to > from { 1.0 } else { -1.0 };
                    let (x0, x1) = (x(flow.from.1) + step / 2.0, x(flow.to.1) + 2.0);
                    Some(view! {
                        <g class=format!("tape-flow {}", ahead(flow.to.1))>
                            <line x1=x0 y1=from + down * 7.0 x2=x1 y2=to - down * 7.0 marker-end="url(#tape-arrow)" />
                            <circle cx=x0 cy=from r="2.4" />
                        </g>
                    })
                })
                .collect_view();
            let head = x(cursor) + step / 2.0;
            let number = reel.steps.get(cursor).map_or(0, |step| step.0).to_string();
            let flag = px(number.len()) * 7.0 + 10.0;
            view! {
                <svg class="tape-svg" width=width height=height viewBox=format!("0 0 {width} {height}") aria-hidden="true">
                    <defs>
                        <pattern id="tape-hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                            <line class="tape-hatch" x1="0" y1="0" x2="0" y2="5" />
                        </pattern>
                        <marker id="tape-arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">
                            <path class="tape-arrowhead" d="M0 0L10 5L0 10z" />
                        </marker>
                    </defs>
                    <rect class="tape-now" x=x(cursor) y=AXIS - 2.0 width=step height=height />
                    {ticks}
                    <path class="tape-clock" d=clock />
                    {calls}
                    {lanes}
                    {flows}
                    <rect class="tape-blank" x=x(count) y=AXIS - 2.0 width=(width - x(count)).max(0.0) height=height />
                    {reel.open.then(|| view! { <text class="tape-unrun" x=x(count) + 10.0 y=AXIS + 14.0>"NOT YET RUN"</text> })}
                    {(count > 0).then(|| view! {
                        <line class="tape-head" x1=head y1="14" x2=head y2=height />
                        <rect class="tape-flag" x=head - flag / 2.0 y="0" width=flag height="15" />
                        <text class="tape-flag-number" x=head y="11" text-anchor="middle">{number.clone()}</text>
                    })}
                </svg>
            }
            .into_any()
        })
    })
}

/// One bus segment: a hexagon between two transitions, with its value
/// written inside (repeated along a long segment, so one is always in view).
/// `used` is what the step at the playhead did to it, if it is the segment
/// there: a store lights it, a read outlines it in the flow's blue, a
/// designation dashes it.
fn bus(segment: &Segment, y: f64, step: f64, cursor: usize, used: Option<String>) -> AnyView {
    let (x0, x1) = (px(segment.from) * step + 1.5, px(segment.to) * step - 1.5);
    let slope = (4.0_f64).min((x1 - x0) / 2.0);
    let points = format!(
        "{x0:.1},{y:.1} {:.1},{:.1} {:.1},{:.1} {x1:.1},{y:.1} {:.1},{:.1} {:.1},{:.1}",
        x0 + slope,
        y - BUS,
        x1 - slope,
        y - BUS,
        x1 - slope,
        y + BUS,
        x0 + slope,
        y + BUS
    );
    let here = segment.from <= cursor && cursor < segment.to;
    let mut class = String::from("tape-bus");
    if segment.value.is_none() {
        class.push_str(" is-indeterminate");
    }
    if here {
        class.push_str(" is-now");
    }
    if segment.from > cursor {
        class.push_str(" is-ahead");
    }
    let text = segment.value.clone().unwrap_or_else(|| "?".to_owned());
    let text_width = px(text.chars().count()) * 7.6 + 6.0;
    let mut labels = Vec::new();
    let mut at = x0 + slope + 3.0;
    while at + text_width <= x1 - slope {
        labels.push(at);
        at += (text_width + 60.0).max(240.0);
    }
    view! {
        <g class=class data-object=segment.object data-access=used>
            <polygon points=points />
            {labels.into_iter().map(|at| view! { <text x=at y=y + 4.0>{text.clone()}</text> }).collect_view()}
        </g>
    }
    .into_any()
}

/// A reel: its hub turns as the playhead moves, and its pack of tape grows
/// (on the left) or shrinks (on the right) with what has played.
#[component]
fn Spool(side: &'static str, fill: Signal<f64>, turn: Signal<f64>) -> impl IntoView {
    view! {
        <div class=format!("spool is-{side}") aria-hidden="true">
            <svg viewBox="-28 -28 56 56" width="56" height="56">
                <g class="spool-turn" style:transform=move || format!("rotate({:.0}deg)", turn.get())>
                    <circle class="spool-pack" r=move || format!("{:.1}", 11.0 + 15.0 * fill.get()) />
                    <circle class="spool-rim" r="26" />
                    <rect class="spool-spoke" x="-2.5" y="-24" width="5" height="13" />
                    <rect class="spool-spoke" x="-2.5" y="-24" width="5" height="13" transform="rotate(120)" />
                    <rect class="spool-spoke" x="-2.5" y="-24" width="5" height="13" transform="rotate(240)" />
                    <circle class="spool-hub" r="6" />
                    <circle class="spool-axle" r="2.2" />
                </g>
            </svg>
        </div>
    }
}

/// Keep `width` at the element's width as the window resizes.
fn observe_width(element: NodeRef<html::Div>, width: RwSignal<f64>) {
    #[cfg(feature = "hydrate")]
    Effect::new(move |observing: Option<bool>| {
        use wasm_bindgen::JsCast;
        if observing == Some(true) {
            return true;
        }
        let Some(node) = element.get() else {
            return false;
        };
        width.set(f64::from(node.client_width()));
        // A change of view unmounts the deck, and the observer reports its
        // removal after the signals are gone; it then does nothing.
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(node)) = element.try_get_untracked() {
                _ = width.try_set(f64::from(node.client_width()));
            }
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref()) {
            observer.observe(&node);
        }
        closure.forget();
        true
    });
    #[cfg(not(feature = "hydrate"))]
    let _ = (element, width);
}

#[cfg(all(test, feature = "worker"))]
mod tests {
    use super::*;
    use crate::{
        demos,
        machine::Machine,
        protocol::{
            Answer,
            Command,
        },
    };

    fn run(key: &str) -> Vec<Snapshot> {
        let mut machine = Machine::default();
        let snapshot = |answer| match answer {
            | Ok(Answer::Snapshot(snapshot)) => *snapshot,
            | _ => panic!("a snapshot"),
        };
        let source = demos::find(key).expect("a lesson").source.to_owned();
        let mut history = vec![snapshot(machine.run(Command::Load(source)))];
        while history.last().is_some_and(Snapshot::can_step) {
            history.push(snapshot(machine.run(Command::Step)));
        }
        history
    }

    #[test]
    fn the_tape_draws_each_value_an_object_held_and_where_it_came_from() {
        let history = run("path");
        let reel = Reel::record(&history);
        assert_eq!(reel.steps.len(), history.len());
        assert!(!reel.open, "the run finished");
        let names: Vec<_> = reel.groups.iter().map(|group| group.key.as_str()).collect();
        assert_eq!(names, ["main@1", "square@2"]);
        let lane = |group: usize, name: &str| {
            reel.groups[group]
                .lanes
                .iter()
                .find(|lane| lane.name == name)
                .expect("a lane")
        };
        // `side` is indeterminate until its initializer stores 3.
        let values: Vec<_> = lane(0, "side").segments.iter().map(|segment| segment.value.as_deref()).collect();
        assert_eq!(values, [None, Some("3")]);
        assert_eq!(lane(1, "n").segments[0].value.as_deref(), Some("4"));
        assert_eq!(lane(1, "n").ends.len(), 1, "n ends when square returns");
        // main waits while square runs.
        assert_eq!(reel.groups[0].calls[0].waiting[0].2, "square(side + 1)");
        // side flows into n at the call; n flows into area through the
        // return.
        let side = lane(0, "side").objects[0];
        let n = lane(1, "n").objects[0];
        let area = lane(0, "area").objects[0];
        assert!(reel.flows.iter().any(|flow| flow.from.0 == side && flow.to.0 == n));
        assert!(reel.flows.iter().any(|flow| flow.from.0 == n && flow.to.0 == area));
    }

    #[test]
    fn repeated_calls_share_lanes_and_fold_once_they_return() {
        let history = run("tour");
        let reel = Reel::record(&history);
        let bump = reel
            .groups
            .iter()
            .position(|group| group.name == "bump")
            .expect("bump's calls");
        assert_eq!(reel.groups[bump].calls.len(), 3, "one group for the loop's three calls");
        let last_call = reel.groups[bump].calls[2].last;
        let after = rows(&reel, last_call + 2, &[]);
        assert!(after.contains(&Row::Group {
            at:     bump,
            folded: true,
            live:   false,
        }));
        let opened = rows(&reel, last_call + 2, &[reel.groups[bump].key.clone()]);
        assert!(opened.len() > after.len(), "an opened group shows its lanes");
    }

    #[test]
    fn pointers_name_what_they_point_at() {
        let objects = vec![Object {
            id:        7,
            name:      "values".to_owned(),
            ty:        "int[3]".to_owned(),
            storage:   "automatic".to_owned(),
            alive:     true,
            frame_id:  1,
            function:  "main".to_owned(),
            parameter: false,
            value:     Value::Array {
                elements: vec![Value::Indeterminate; 3],
            },
        }];
        let pointer = |offset, one_past| Value::Pointer {
            target: Some(7),
            offset,
            one_past,
            ended: false,
        };
        assert_eq!(value_text(&pointer(1, false), &objects).as_deref(), Some("→values[1]"));
        assert_eq!(value_text(&pointer(3, true), &objects).as_deref(), Some("→values+3"));
        assert_eq!(value_text(&objects[0].value, &objects).as_deref(), Some("{?,?,?}"));
    }
}

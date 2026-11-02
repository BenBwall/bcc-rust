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
        Range,
        Snapshot,
        Tree,
        Value,
    },
};

/// Height of one lane, in pixels: a bus segment and a gap for a slant's
/// label.
const LANE: f64 = 40.0;
/// Height of the step axis and clock lane above the lanes.
const AXIS: f64 = 32.0;
/// Half the height of a bus segment.
const BUS: f64 = 10.0;
/// Width of one character of a segment's value (Space Mono at 13 px).
const CHAR: f64 = 8.0;
/// The narrowest and widest step a fitted tape gives each step.
const FIT_MIN: f64 = 14.0;
const FIT_MAX: f64 = 48.0;
/// The narrowest step the zoom knob reaches short of the whole run (which
/// it fits, however long, turned fully left), and the widest.
const ZOOM_MIN: f64 = 1.0;
const ZOOM_MAX: f64 = 64.0;
/// Room the tape keeps right of the playhead for the end of the run: the
/// blank tape and "NOT YET RUN" after the last recorded step.
const ROOM: f64 = 130.0;
/// Below this width the deck is compact (a phone): the tape takes the
/// whole width, each lane's name rides on its lane, and the end of the run
/// keeps less room.
const COMPACT_BELOW: f64 = 560.0;
const ROOM_COMPACT: f64 = 44.0;
/// How long the tape takes to glide to a new scale, in milliseconds.
const GLIDE_MS: f64 = 260.0;

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

/// Every object of the run over the recorded steps (history indices).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reel {
    pub groups: Vec<Group>,
    pub flows:  Vec<Flow>,
    /// Each recorded step's number, and whether it ended at a sequence
    /// point.
    pub steps:  Vec<(usize, bool)>,
    /// Whether the engine can record another step after the last.
    pub open:   bool,
    /// What each step did to each object it used: `read`, `write`,
    /// `create`, `end`, `designate` or `address`.
    pub access: Vec<Vec<(usize, String)>>,
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

/// One object name of a group, over every call of the group. Objects of
/// the same name whose lifetimes overlap (a block's `x` shadowing the
/// function's) each get a lane of their own, numbered from 1.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    pub key:      String,
    pub name:     String,
    pub ordinal:  usize,
    /// The object on the lane now, while recording.
    occupant:     Option<usize>,
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

/// The values read (object, step) that a store or a call's parameter took
/// its value from, and the source of the expression that combined them, for
/// the slant's label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Flow {
    pub from:  Vec<(usize, usize)>,
    pub to:    (usize, usize),
    pub label: Option<Range>,
}

impl Reel {
    /// The tape of `history`, in order.
    #[expect(
        clippy::too_many_lines,
        reason = "One pass over the history: calls, objects, flows."
    )]
    pub fn record(history: &[Snapshot]) -> Self {
        let mut reel = Self::default();
        let mut group_of_frame: HashMap<usize, usize> = HashMap::new();
        let mut lane_of: HashMap<usize, usize> = HashMap::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        // Values read since the last store, call or full expression, and the
        // operands of `[]` and `*` in that time: reads inside one computed an
        // address, not a value.
        let mut reads: Vec<(usize, usize, Option<Range>)> = Vec::new();
        let mut addresses: Vec<Range> = Vec::new();
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
            let alive: Vec<usize> = snapshot
                .objects
                .iter()
                .filter(|object| object.alive)
                .map(|object| object.id)
                .collect();
            for object in &snapshot.objects {
                let Some(&at) = group_of_frame.get(&object.frame_id) else {
                    continue;
                };
                let group = &mut reel.groups[at];
                let place = *lane_of
                    .entry(object.id)
                    .or_insert_with(|| group.place(object, |occupant| alive.contains(&occupant)));
                let lane = &mut group.lanes[place];
                lane.occupant = Some(object.id);
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
                } else if last
                    .is_some_and(|segment| segment.object == object.id && segment.to == step)
                {
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
                    .map(|access| (access.object, step, event.range)),
            );
            if matches!(event.kind.as_str(), "subscript" | "dereference")
                && let Some(range) = event.range
            {
                addresses.push(range);
            }
            let values: Vec<(usize, usize, Option<Range>)> = reads
                .iter()
                .copied()
                .filter(|&(_, _, range)| {
                    range.is_none_or(|range| {
                        !addresses
                            .iter()
                            .any(|&address| address != range && within(address, range))
                    })
                })
                .collect();
            let call = event.kind == "call";
            if call {
                reel.flows.extend(arguments(history, step, &values));
            } else if let Some(store) = event.access.iter().find(|access| access.kind == "write")
                && !values.is_empty()
            {
                // The expression whose value was stored: the operand of the
                // assignment, initializer or return that took the values.
                let label = history[values[values.len() - 1].1]
                    .tree
                    .as_ref()
                    .and_then(|tree| {
                        let ranges: Vec<Range> =
                            values.iter().filter_map(|&(_, _, range)| range).collect();
                        operand(tree, &ranges)
                    });
                reel.flows.push(Flow {
                    from: values.iter().map(|&(object, at, _)| (object, at)).collect(),
                    to: (store.object, step),
                    label,
                });
            }
            let stored = call || event.access.iter().any(|access| access.kind == "write");
            // A return keeps what its expression read: the value flows on
            // into the caller's store.
            if stored || (event.sequence_point && event.kind != "return") {
                reads.clear();
                addresses.clear();
            }
        }
        reel.open = history.last().is_some_and(Snapshot::can_step);
        reel
    }
}

impl Reel {
    /// What the tape calls `object`'s lane: its name, numbered where
    /// another object of that name has a lane of its own (`x #2`).
    pub fn label_of(&self, object: usize) -> Option<String> {
        self.groups
            .iter()
            .flat_map(|group| &group.lanes)
            .find(|lane| lane.objects.contains(&object))
            .map(Lane::label)
    }
}

impl Lane {
    fn label(&self) -> String {
        if self.ordinal > 1 {
            format!("{} #{}", self.name, self.ordinal)
        } else {
            self.name.clone()
        }
    }
}

impl Group {
    /// The lane for a new `object`: the first of its name whose occupant is
    /// no longer alive, or a new one beside them.
    fn place(&mut self, object: &Object, alive: impl Fn(usize) -> bool) -> usize {
        // A lane holds one type: an `int x` and a later `int *x` of the
        // same name get lanes of their own.
        let same = |lane: &&Lane| lane.name == object.name && lane.ty == object.ty;
        if let Some(place) = self
            .lanes
            .iter()
            .position(|lane| same(&lane) && lane.occupant.is_none_or(|occupant| !alive(occupant)))
        {
            return place;
        }
        let ordinal = self
            .lanes
            .iter()
            .filter(|lane| lane.name == object.name)
            .count()
            + 1;
        self.lanes.push(Lane {
            key: format!("{}:{}:{ordinal}", self.key, object.name),
            name: object.name.clone(),
            ordinal,
            occupant: None,
            ty: object.ty.clone(),
            objects: Vec::new(),
            segments: Vec::new(),
            ends: Vec::new(),
        });
        self.lanes.len() - 1
    }
}

/// Whether `inner` lies within `outer`.
fn within(outer: Range, inner: Range) -> bool {
    let at = |position: crate::snapshot::Position| (position.line, position.column);
    at(outer.start) <= at(inner.start) && at(inner.end) <= at(outer.end)
}

/// Whether a tree node takes its operand's value: an assignment, an
/// initializer, a call or a return.
fn consumes(label: &str) -> bool {
    label == "return"
        || label.ends_with("()")
        || (label.ends_with('=') && !matches!(label, "==" | "!=" | "<=" | ">="))
}

/// The operand that combined the values read at `ranges`: the smallest node
/// of `tree` holding them all, widened to the whole operand of the node
/// that consumed it (`side + 1` in `square(side + 1)`).
fn operand(tree: &Tree, ranges: &[Range]) -> Option<Range> {
    let mut at = tree.nodes.iter().position(|node| {
        node.range
            .is_some_and(|range| ranges.iter().all(|&read| within(range, read)))
    })?;
    if consumes(&tree.nodes[at].label) {
        return None;
    }
    while let Some(parent) = tree
        .nodes
        .iter()
        .position(|node| node.children.contains(&at))
        && !consumes(&tree.nodes[parent].label)
    {
        at = parent;
    }
    tree.nodes[at].range
}

/// What a call at `step` passed: each parameter it created took the values
/// read inside its own argument.
fn arguments(
    history: &[Snapshot],
    step: usize,
    values: &[(usize, usize, Option<Range>)],
) -> Vec<Flow> {
    let event = &history[step].event;
    let parameters = event.access.iter().filter(|access| access.kind == "create");
    let Some(call) = step
        .checked_sub(1)
        .and_then(|before| history[before].tree.as_ref())
        .and_then(|tree| {
            tree.nodes
                .iter()
                .find(|node| node.range == event.range && node.label.ends_with("()"))
                .map(|node| (tree, node))
        })
    else {
        return Vec::new();
    };
    let (tree, node) = call;
    parameters
        .zip(&node.children)
        .filter_map(|(parameter, &argument)| {
            let range = tree.nodes[argument].range?;
            let from: Vec<(usize, usize)> = values
                .iter()
                .filter(|&&(_, _, read)| read.is_some_and(|read| within(range, read)))
                .map(|&(object, at, _)| (object, at))
                .collect();
            (!from.is_empty()).then_some(Flow {
                from,
                to: (parameter.object, step),
                label: Some(range),
            })
        })
        .collect()
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

/// One row of the deck: a group's header, one of its lanes, or a ladder of
/// folded groups of one function at consecutive depths (a recursion's
/// returned or waiting calls), each drawn as a thin rung of call bars.
#[derive(Clone, Debug, PartialEq)]
enum Row {
    Group {
        at:     usize,
        folded: bool,
        live:   bool,
    },
    Lane {
        group: usize,
        lane:  usize,
    },
    Ladder {
        groups: Vec<usize>,
    },
}

/// How many live groups of one function stay open: the outermost call and
/// the two innermost, where a recursion is doing its work.
const OPEN_DEPTHS: usize = 3;

/// The deck's rows. Functions come in the order of their first call, each
/// function's groups by depth. Every folded group of a function shares one
/// ladder row, however its calls interleave with another function's, so a
/// recursion takes a bounded number of rows.
fn rows(reel: &Reel, cursor: usize, opened: &[String]) -> Vec<Row> {
    let mut functions: Vec<&str> = Vec::new();
    let mut first: Vec<usize> = (0..reel.groups.len()).collect();
    first.sort_by_key(|&at| reel.groups[at].calls.first().map_or(0, |call| call.first));
    for &at in &first {
        if !functions.contains(&reel.groups[at].name.as_str()) {
            functions.push(&reel.groups[at].name);
        }
    }
    // A call stays open through the step that ends it, so its lanes are on
    // screen while their objects' lifetimes end.
    let live = |at: usize| {
        reel.groups[at]
            .calls
            .iter()
            .any(|call| call.first <= cursor && cursor <= call.last + 1)
    };
    let mut rows: Vec<Row> = Vec::new();
    for name in functions {
        let mut groups: Vec<usize> = (0..reel.groups.len())
            .filter(|&at| reel.groups[at].name == name)
            .collect();
        groups.sort_by_key(|&at| reel.groups[at].depth);
        let mut folded: Vec<bool> = groups.iter().map(|&at| !live(at)).collect();
        // A deep recursion keeps its outermost and innermost calls open.
        let open: Vec<usize> = (0..groups.len()).filter(|&place| !folded[place]).collect();
        if open.len() > OPEN_DEPTHS {
            for &place in &open[1..open.len() + 1 - OPEN_DEPTHS] {
                folded[place] = true;
            }
        }
        for (place, &at) in groups.iter().enumerate() {
            if opened.contains(&reel.groups[at].key) {
                folded[place] = false;
            }
        }
        let ladder: Vec<usize> = groups
            .iter()
            .zip(&folded)
            .filter(|&(_, &folded)| folded)
            .map(|(&at, _)| at)
            .collect();
        let mut laddered = false;
        for (&at, &folded) in groups.iter().zip(&folded) {
            if !folded {
                rows.push(Row::Group {
                    at,
                    folded: false,
                    live: live(at),
                });
                rows.extend(
                    (0..reel.groups[at].lanes.len()).map(|lane| Row::Lane { group: at, lane }),
                );
            } else if ladder.len() < 2 {
                rows.push(Row::Group {
                    at,
                    folded: true,
                    live: live(at),
                });
            } else if !laddered {
                laddered = true;
                rows.push(Row::Ladder {
                    groups: ladder.clone(),
                });
            }
        }
    }
    rows
}

/// How many steps the fitted scale makes room for: the run's length
/// rounded up to a power of two from 16, so a short run fits with room to
/// spare and a run stepped by hand rescales only twice or so before the
/// scale reaches its narrowest step and the tape scrolls instead.
fn fit_slots(count: usize) -> usize {
    count.max(16).next_power_of_two()
}

/// The width of a step for `count` recorded steps on `tape` pixels (the
/// window less the room kept for the end of the run), at `zoom` times the
/// fitted scale.
fn fitted_step(count: usize, tape: f64, zoom: f64) -> f64 {
    let fit = (tape / px(fit_slots(count)))
        .floor()
        .clamp(FIT_MIN, FIT_MAX);
    // Turned fully left, ZOOM fits the whole recorded run, however long.
    let whole = ((tape - 10.0) / px(count.max(1))).max(0.05);
    (fit * zoom).clamp(whole.min(ZOOM_MIN), ZOOM_MAX)
}

/// The source of `range`, on one line, for a slant's label; `None` for a
/// range over several lines.
fn source_text(source: &str, range: Range) -> Option<String> {
    if range.start.line != range.end.line {
        return None;
    }
    let line = source.lines().nth(range.start.line.checked_sub(1)?)?;
    // Columns count UTF-16 code units from 1.
    let units: Vec<u16> = line.encode_utf16().collect();
    let text = String::from_utf16_lossy(
        units.get(range.start.column.checked_sub(1)?..range.end.column.checked_sub(1)?)?,
    );
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    Some(shorten(&text, 26))
}

/// `text` cut to `chars` characters, ending in `…` when it was longer. An
/// array keeps whole elements: `{3,1,4,…}`.
fn shorten(text: &str, chars: usize) -> String {
    if text.chars().count() <= chars {
        return text.to_owned();
    }
    if text.starts_with('{') {
        let mut shown = String::from("{");
        for element in text.trim_matches(['{', '}']).split(',') {
            if shown.chars().count() + element.chars().count() + 3 > chars {
                break;
            }
            shown.push_str(element);
            shown.push(',');
        }
        if shown.len() > 1 {
            shown.push_str("…}");
            return shown;
        }
    }
    let mut shown: String = text.chars().take(chars.saturating_sub(1)).collect();
    shown.push('…');
    shown
}

/// The deck: reels, the lane labels, and the scrolling tape with its
/// playhead. Dragging the tape, or the playhead, moves through the recorded
/// steps; the arrow keys step (through the page's shortcuts), Home and End go
/// to the first and last recorded step.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "The deck's parts and its pointer and key handling."
)]
pub fn Deck(
    history: RwSignal<Vec<Snapshot>>,
    cursor: RwSignal<Option<usize>>,
    /// The run's tape (`Reel::record` of the history).
    reel: Memo<Reel>,
    /// The program the engine runs, for the slants' labels.
    source: Signal<String>,
    /// The scale, as a factor of the one that fits the recorded run to the
    /// screen.
    zoom: RwSignal<f64>,
    /// Whether Run is stepping: the tape then jumps to the playhead rather
    /// than glide behind it.
    running: Signal<bool>,
    /// Where the playhead is on screen, relative to the start of the tape's
    /// viewport, for the line it drops into the note.
    head: RwSignal<Option<f64>>,
    on_jump: Callback<usize>,
) -> impl IntoView {
    let opened = RwSignal::new(Vec::<String>::new());
    // The deck's body scrolls both ways, like a spreadsheet: the lane names
    // stay at its left and the ruler at its top.
    let body = NodeRef::<html::Div>::new();
    let tape = NodeRef::<html::Div>::new();
    let width = RwSignal::new(760.0_f64);
    // The deck body's whole width, names and tape: below `COMPACT_BELOW` the
    // names ride on the lanes.
    let full = RwSignal::new(900.0_f64);
    let compact = Memo::new(move |_| full.get() < COMPACT_BELOW);
    let room = move || if compact.get() { ROOM_COMPACT } else { ROOM };
    let scroll = RwSignal::new(0.0_f64);
    // Whether rows lie below the deck's window, for the key at its foot,
    // and whether any lie outside it.
    let more = RwSignal::new(false);
    let overflows = RwSignal::new(false);
    let strip = NodeRef::<html::Button>::new();
    // Whether rows lie below the body's foot, and whether they would without
    // the strip: while it shows it takes its own height from the body, so
    // measured against the shortened body it would never hide again. A
    // resize observer may call it after the deck is gone; it then does
    // nothing.
    let measure = move |element: &web_sys::Element| {
        let taken = if overflows.try_get_untracked() == Some(true) {
            strip
                .try_get_untracked()
                .flatten()
                .map_or(0, |strip| strip.offset_height())
        } else {
            0
        };
        _ = more
            .try_set(element.scroll_top() + element.client_height() + 2 < element.scroll_height());
        _ = overflows.try_set(element.client_height() + taken + 2 < element.scroll_height());
    };
    let at = move || cursor.get().unwrap_or(0);
    let rows = Memo::new(move |_| {
        let cursor = at();
        reel.with(|reel| opened.with(|opened| rows(reel, cursor, opened)))
    });
    // The scale the run fits at, with room for the end of the tape, so a
    // short run fits the window without scrolling.
    let fitted = Memo::new(move |_| {
        let count = reel.with(|reel| reel.steps.len());
        fitted_step(count, width.get() - room(), zoom.get())
    });
    // The scale drawn: it glides to a new fitted scale as the run outgrows
    // the old one or ZOOM turns, and jumps on a resize, while Run steps,
    // and when motion is reduced.
    let step_width = RwSignal::new(fitted.get_untracked());
    let glides = StoredValue::new((0_u32, width.get_untracked()));
    Effect::new(move || {
        let to = fitted.get();
        let view = width.get();
        let from = step_width.get_untracked();
        let (generation, last_view) = glides.get_value();
        glides.set_value((generation + 1, view));
        if (from - to).abs() < 0.5
            || (view - last_view).abs() > 0.5
            || running.get_untracked()
            || reduced_motion()
        {
            step_width.set(to);
        } else {
            glide(
                step_width,
                glides,
                generation + 1,
                (from, to),
                js_sys::Date::now(),
            );
        }
    });
    observe_width(body, width, full);
    observe_rows(body, measure);
    // A step moves the tape, not the head: keep the playhead in the middle
    // stretch of the viewport. The tape glides a short way, and jumps when
    // Run is stepping, for a long way, or when motion is reduced.
    Effect::new(move || {
        let x = (px(at()) + 0.5) * step_width.get();
        // While the scale glides the tape follows it frame by frame.
        let running =
            running.get() || (step_width.get_untracked() - fitted.get_untracked()).abs() > 0.01;
        let Some(element) = body.get() else {
            return;
        };
        let view = width.get_untracked();
        let room = room();
        let left = f64::from(element.scroll_left());
        // The head keeps the room for the end of the run to its right: past
        // that the tape moves just enough, so a short run that fits never
        // scrolls.
        let target = if x > left + view - room {
            Some(x + room - view)
        } else if x < left + view * 0.15 {
            Some((x - view * 0.3).max(0.0))
        } else {
            None
        };
        if let Some(target) = target {
            let glide = !running && (target - left).abs() < view && !reduced_motion();
            let options = web_sys::ScrollToOptions::new();
            options.set_left(target.round());
            options.set_behavior(if glide {
                web_sys::ScrollBehavior::Smooth
            } else {
                web_sys::ScrollBehavior::Instant
            });
            element.scroll_to_with_scroll_to_options(&options);
        }
    });
    // New rows (a call, an opened fold) may run past the deck's foot.
    Effect::new(move || {
        rows.track();
        if let Some(element) = body.get() {
            measure(&element);
        }
    });
    Effect::new(move || {
        let x = (px(at()) + 0.5) * step_width.get() - scroll.get();
        let shown = history.with(|history| !history.is_empty()) && x >= 0.0 && x <= width.get();
        head.set(shown.then_some(x));
    });
    let dragging = StoredValue::new(false);
    let index_at = move |client_x: i32| -> Option<usize> {
        // The tape moves with the deck's scroll, so its own box places the
        // pointer.
        let element = tape.get_untracked()?;
        let x = f64::from(client_x) - element.get_bounding_client_rect().left();
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
    let toggle = move |keys: Vec<String>| {
        opened.update(|opened| {
            if keys.iter().all(|key| opened.contains(key)) {
                opened.retain(|open| !keys.contains(open));
            } else {
                opened.extend(keys);
            }
        });
    };
    let played = move || {
        let count = reel.with(|reel| reel.steps.len().max(2)) - 1;
        px(at()) / px(count)
    };
    view! {
        <figure class="deck" class:is-compact=move || compact.get() aria-labelledby="deck-caption">
            <figcaption class="deck-caption" id="deck-caption">
                <b>"Figure 1"</b>" — Every object of the run, recorded on tape. Drag the tape, or use "<kbd>"←"</kbd>" "<kbd>"→"</kbd>"."
            </figcaption>
            <Spool side="left" fill=Signal::derive(played) turn=Signal::derive(move || px(at()) * 37.0) />
            // The labels and the tape scroll together where a run has more
            // rows than the deck's height; past either end the page scrolls.
            <div class="deck-window">
            <div
                class="deck-body"
                node_ref=body
                on:scroll=move |_| {
                    if let Some(element) = body.get_untracked() {
                        scroll.set(f64::from(element.scroll_left()));
                        measure(&element);
                    }
                }
            >
            // The lane names repeat what Table 1 lists for screen readers;
            // the fold keys are real buttons.
            <div class="deck-labels" style:--lane=format!("{LANE}px") style:--axis=format!("{AXIS}px")>
                <div class="deck-label is-axis" aria-hidden="true">"Step"</div>
                {move || rows.with(|rows| reel.with(|reel| rows.iter().map(|row| match row {
                    | &Row::Group { at, folded, live } => {
                        let group = &reel.groups[at];
                        let keys = vec![group.key.clone()];
                        let count = group.calls.len();
                        // The depth tells a recursion's calls apart.
                        let recursive = reel.groups.iter().filter(|other| other.name == group.name).count() > 1;
                        view! {
                            <div class="deck-label is-group" class:is-folded=folded class:is-live=live>
                                <button
                                    type="button"
                                    aria-expanded=(!folded).to_string()
                                    aria-label=format!("{}(){}: {} its objects on the tape", group.name, if recursive { format!(" at depth {}", group.depth) } else { String::new() }, if folded { "show" } else { "fold" })
                                    on:click=move |_| toggle(keys.clone())
                                >
                                    <span class="deck-fold" aria-hidden="true">{if folded { "▸" } else { "▾" }}</span>
                                    <span class="deck-call">{format!("{}()", group.name)}</span>
                                    {recursive.then(|| view! { <span class="deck-depth" title=format!("depth {} on the call stack", group.depth)>{format!("d{}", group.depth)}</span> })}
                                    {(count > 1).then(|| view! { <span class="deck-count">{format!("×{count}")}</span> })}
                                </button>
                            </div>
                        }.into_any()
                    },
                    | Row::Ladder { groups } => {
                        let first = &reel.groups[groups[0]];
                        let last = &reel.groups[groups[groups.len() - 1]];
                        let keys: Vec<String> = groups.iter().map(|&at| reel.groups[at].key.clone()).collect();
                        view! {
                            <div class="deck-label is-group is-folded is-ladder">
                                <button
                                    type="button"
                                    aria-expanded="false"
                                    aria-label=format!("{}() at depths {} to {}: show their objects on the tape", first.name, first.depth, last.depth)
                                    on:click=move |_| toggle(keys.clone())
                                >
                                    <span class="deck-fold" aria-hidden="true">"▸"</span>
                                    <span class="deck-call">{format!("{}()", first.name)}</span>
                                    <span class="deck-depth" title=format!("depths {}–{} on the call stack", first.depth, last.depth)>{format!("d{}–{}", first.depth, last.depth)}</span>
                                </button>
                            </div>
                        }.into_any()
                    },
                    | &Row::Lane { group, lane } => {
                        let lane = &reel.groups[group].lanes[lane];
                        view! {
                            <div class=format!("deck-label is-lane {}", palette::class(&lane.name)) title=format!("{} {}", lane.ty, lane.name) aria-hidden="true">
                                <span class="deck-swatch"></span>
                                <span class="deck-name">{lane.name.clone()}</span>
                                {(lane.ordinal > 1).then(|| view! { <span class="deck-ordinal">{format!("#{}", lane.ordinal)}</span> })}
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
                {move || ruler_svg(reel, step_width.get(), at(), (width.get(), room()))}
                {move || tape_svg(reel, rows, step_width.get(), at(), (width.get(), compact.get()))}
                // The slants' labels, placed in the tape's visible window,
                // and on a compact tape the values, kept in it. The layer
                // is one pixel drawn with visible overflow, so it never
                // widens what the deck can scroll.
                {move || {
                    let step = step_width.get();
                    let (cursor, left, view, compact) = (at(), scroll.get(), width.get(), compact.get());
                    reel.with(|reel| rows.with(|rows| source.with(|source| {
                        let window = (left, left + view);
                        // A compact tape's labels stay left of the head:
                        // past it is only the narrow blank.
                        let (avoid, labelled) = if compact {
                            (tags(reel, rows, left), (left, window.1.min((px(cursor) + 0.5) * step - 3.0)))
                        } else {
                            (Vec::new(), window)
                        };
                        let layer = place_labels(reel, rows, source, step, cursor, labelled, &avoid);
                        let values = if compact { pinned_values(reel, rows, step, cursor, window) } else { Vec::new() };
                        view! {
                            <svg class="tape-labels" aria-hidden="true" width="1" height="1">
                                {values.into_iter().map(|value| view! {
                                    <g class=value.class data-access=value.access>
                                        <text x=value.left y=value.y>{value.text}</text>
                                    </g>
                                }).collect_view()}
                                {layer.into_iter().map(|label| view! {
                                    <text class="tape-flow-label" x=label.left y=label.middle + 3.5>{label.text}</text>
                                }).collect_view()}
                            </svg>
                        }
                    })))
                }}
            </div>
            </div>
            // At the deck's foot, under the rows rather than over them,
            // while the rows run past the deck's height.
            <button
                type="button"
                class="deck-more"
                node_ref=strip
                hidden=move || !overflows.get()
                on:click=move |_| {
                    if let Some(element) = body.get_untracked() {
                        let options = web_sys::ScrollToOptions::new();
                        options.set_top(if more.get_untracked() { f64::from(element.scroll_height()) } else { 0.0 });
                        element.scroll_to_with_scroll_to_options(&options);
                    }
                }
            >
                {move || if more.get() { "▾ More rows below" } else { "▴ Back to the top rows" }}
            </button>
            </div>
            <Spool side="right" fill=Signal::derive(move || 1.0 - played()) turn=Signal::derive(move || px(at()) * 37.0) />
        </figure>
    }
}

/// Height of a lane's name riding on a compact tape, at the top of its row.
const TAG: f64 = 13.0;

/// Where the names ride on a compact tape whose window starts at `left`:
/// a box at the top of each row, as wide as its text (Space Mono at 10 px)
/// and the swatch or fold key before it.
fn tags(reel: &Reel, rows: &[Row], left: f64) -> Vec<(f64, f64, f64, f64)> {
    rows.iter()
        .enumerate()
        .map(|(place, row)| {
            let chars = match *row {
                | Row::Lane { group, lane } => {
                    let lane = &reel.groups[group].lanes[lane];
                    lane.label().chars().count() + lane.ty.chars().count() + 1
                },
                | Row::Group { at, .. } => reel.groups[at].name.chars().count() + 12,
                | Row::Ladder { ref groups } => reel.groups[groups[0]].name.chars().count() + 14,
            };
            let top = AXIS + px(place) * LANE;
            (left, top, left + px(chars) * LABEL_CHAR + 26.0, top + TAG)
        })
        .collect()
}

/// A slant's label as placed: its text, the left edge and middle of its
/// box, and the box's right edge.
#[derive(Clone, Debug, PartialEq)]
struct Placed {
    text:   String,
    left:   f64,
    right:  f64,
    middle: f64,
}

/// Width of one character of a label (Space Mono at 10 px).
const LABEL_CHAR: f64 = 6.2;
/// Half the height of a label's box as drawn (10 px type, its line box
/// 14 px).
const LABEL_HALF: f64 = 7.0;

/// The slants' labels, newest first: each in the gap between its target
/// lane and a neighbour (the side its value came from first), as close
/// before its arrowhead as it can sit clear of every bus, every slant and
/// every label already placed. A label shows only where it fits wholly
/// inside the tape's visible `window` near its arrowhead, so it never
/// slides along the tape or strays from its slant.
fn place_labels(
    reel: &Reel,
    rows: &[Row],
    source: &str,
    step: f64,
    cursor: usize,
    window: (f64, f64),
    avoid: &[(f64, f64, f64, f64)],
) -> Vec<Placed> {
    if step < 6.0 {
        return Vec::new();
    }
    let Layout {
        objects, folded, ..
    } = layout(reel, rows);
    let slants = slants(reel, &objects, &folded, step);
    let bottom = AXIS + px(rows.len()) * LANE;
    let mut placed: Vec<Placed> = Vec::new();
    for (index, flow) in reel.flows.iter().enumerate().rev() {
        if flow.to.1 > cursor {
            continue;
        }
        let Some(&(to, _)) = objects.get(&flow.to.0) else {
            continue;
        };
        let Some(first) = slants.iter().find(|slant| slant.flow == index) else {
            continue;
        };
        let x1 = first.x1;
        if x1 < window.0 || x1 > window.1 {
            continue;
        }
        let Some(text) = flow.label.and_then(|range| source_text(source, range)) else {
            continue;
        };
        let text = format!("{text} → {}", reel.label_of(flow.to.0).unwrap_or_default());
        let width = px(text.chars().count()) * LABEL_CHAR;
        let above = to - LANE / 2.0;
        let below = to + LANE / 2.0;
        let mut gaps = vec![above, below];
        if first.from > to {
            gaps.reverse();
        }
        gaps.retain(|&gap| gap > AXIS + 1.0 && gap < bottom + 1.0);
        // Before the arrowhead first, then further back, then after it.
        let offsets = (0..8)
            .map(|k| x1 - 4.0 - width - px(k) * 14.0)
            .chain((0..4).map(|k| x1 + 8.0 + px(k) * 14.0));
        let candidates = gaps
            .iter()
            .flat_map(|&middle| offsets.clone().map(move |left| (left, middle)));
        let found = candidates.into_iter().find(|&(left, middle)| {
            let label = Placed {
                text: String::new(),
                left,
                right: left + width,
                middle,
            };
            let rect = (
                left - 2.0,
                middle - LABEL_HALF,
                left + width + 2.0,
                middle + LABEL_HALF,
            );
            left >= window.0 + 2.0
                && left + width <= window.1 - 2.0
                && !slants.iter().any(|slant| crosses(slant, rect))
                && !placed.iter().any(|other| overlaps(other, &label))
                && !avoid.iter().any(|&(left, top, right, bottom)| {
                    rect.0 < right && left < rect.2 && rect.1 < bottom && top < rect.3
                })
        });
        if let Some((left, middle)) = found {
            placed.push(Placed {
                text,
                left,
                right: left + width,
                middle,
            });
        }
    }
    placed
}

fn overlaps(a: &Placed, b: &Placed) -> bool {
    a.left < b.right + 6.0
        && b.left < a.right + 6.0
        && (a.middle - b.middle).abs() < 2.0 * LABEL_HALF
}

/// Whether the reader asked for reduced motion.
fn reduced_motion() -> bool {
    web_sys::window()
        .and_then(|window| {
            window
                .match_media("(prefers-reduced-motion: reduce)")
                .ok()
                .flatten()
        })
        .is_some_and(|query| query.matches())
}

/// Move `scale` from one step width to another over `GLIDE_MS`, easing out,
/// one animation frame at a time, until a newer glide (`generation`) takes
/// over or the deck is gone.
fn glide(
    scale: RwSignal<f64>,
    glides: StoredValue<(u32, f64)>,
    generation: u32,
    (from, to): (f64, f64),
    start: f64,
) {
    request_animation_frame(move || {
        if glides.try_get_value().map(|(current, _)| current) != Some(generation) {
            return;
        }
        let done = ((js_sys::Date::now() - start) / GLIDE_MS).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - done).powi(3);
        _ = scale.try_set(from + (to - from) * eased);
        if done < 1.0 {
            glide(scale, glides, generation, (from, to), start);
        }
    });
}

/// Where each row's drawing sits: the y of each group's call bars (a rung
/// of its ladder, if folded into one) and of each object's lane, and the
/// lane name each object is drawn under.
struct Layout<'a> {
    calls:   Vec<(usize, f64, bool)>,
    objects: HashMap<usize, (f64, &'a str)>,
    /// Objects of folded groups, at their group's row: a slant from one
    /// starts at the call that held it.
    folded:  HashMap<usize, f64>,
}

fn layout<'a>(reel: &'a Reel, rows: &[Row]) -> Layout<'a> {
    let mut calls = Vec::new();
    let mut objects = HashMap::new();
    let mut folded = HashMap::new();
    let mut fold = |at: usize, y: f64| {
        for lane in &reel.groups[at].lanes {
            for &object in &lane.objects {
                folded.insert(object, y);
            }
        }
    };
    for (place, row) in rows.iter().enumerate() {
        let top = AXIS + px(place) * LANE;
        match row {
            | &Row::Group { at, folded, .. } => {
                calls.push((at, top + LANE / 2.0, false));
                if folded {
                    fold(at, top + LANE / 2.0);
                }
            },
            | Row::Ladder { groups } => {
                let rungs = px(groups.len().max(2) - 1);
                for (rung, &at) in groups.iter().enumerate() {
                    let y = top + 10.0 + px(rung) * (LANE - 20.0) / rungs;
                    calls.push((at, y, true));
                    fold(at, y);
                }
            },
            | &Row::Lane { group, lane } => {
                let lane = &reel.groups[group].lanes[lane];
                for &object in &lane.objects {
                    objects.insert(object, (top + LANE / 2.0, lane.name.as_str()));
                }
            },
        }
    }
    Layout {
        calls,
        objects,
        folded,
    }
}

/// The tape itself, as one SVG.
#[expect(clippy::too_many_lines, reason = "One block per layer of the drawing.")]
fn tape_svg(
    reel: Memo<Reel>,
    rows: Memo<Vec<Row>>,
    step: f64,
    cursor: usize,
    (view, compact): (f64, bool),
) -> AnyView {
    reel.with(|reel| {
        rows.with(|rows| {
            let count = reel.steps.len();
            let x = move |index: usize| px(index) * step;
            let room = if compact { ROOM_COMPACT } else { ROOM };
            let width = tape_width(count, step, view, room);
            let height = AXIS + px(rows.len()) * LANE + 8.0;
            let Layout {
                calls: call_rows,
                objects,
                folded,
            } = layout(reel, rows);
            let ahead = move |index: usize| if index > cursor { "is-ahead" } else { "" };
            let head = x(cursor) + step / 2.0;
            // The grid; the ruler above it is drawn apart (`ruler_svg`), so it
            // stays on screen as the deck scrolls down.
            let every = tick_interval(step);
            let ticks = (0..count)
                .filter(|index| index % every == 0)
                .map(|index| view! { <line class="tape-grid" x1=x(index) y1=AXIS - 4.0 x2=x(index) y2=height /> })
                .collect_view();
            let calls = call_rows
                .iter()
                .flat_map(|&(at, y, rung)| {
                    let class = if rung { "tape-call is-rung" } else { "tape-call" };
                    reel.groups[at].calls.iter().map(move |call| {
                        let mut parts = Vec::new();
                        let mut start = call.first;
                        for (from, to, on) in &call.waiting {
                            if *from > start {
                                parts.push(view! { <line class=format!("{class} {}", ahead(start)) x1=x(start) y1=y x2=x(*from) y2=y /> }.into_any());
                            }
                            parts.push(view! { <line class=format!("{class} is-waiting {}", ahead(*from)) x1=x(*from) y1=y x2=x(*to) y2=y /> }.into_any());
                            // Repeated along the wait, so one is in view;
                            // under the rule on a compact tape, where the
                            // group's name rides above it.
                            let text = format!("waiting for {on}");
                            let baseline = if compact { y + 11.0 } else { y - 5.0 };
                            let text_width = px(text.chars().count()) * 6.0 + 12.0;
                            let mut at = x(*from) + 6.0;
                            while !rung && at + text_width <= x(*to) {
                                parts.push(view! {
                                    <text class=format!("tape-waiting {}", ahead(*from)) x=at y=baseline>{text.clone()}</text>
                                }.into_any());
                                at += (text_width + 80.0).max(360.0);
                            }
                            start = *to;
                        }
                        if call.last + 1 > start {
                            parts.push(view! { <line class=format!("{class} {}", ahead(start)) x1=x(start) y1=y x2=x(call.last + 1) y2=y /> }.into_any());
                        }
                        let edge = if rung { 3.0 } else { 6.0 };
                        for at in [call.first, call.last + 1] {
                            if at < count {
                                parts.push(view! { <line class=format!("tape-edge {}", ahead(at)) x1=x(at) y1=y - edge x2=x(at) y2=y + edge /> }.into_any());
                            }
                        }
                        parts.into_iter().collect_view()
                    })
                })
                .collect_view();
            let used = |object: usize| {
                reel.access
                    .get(cursor)
                    .and_then(|access| access.iter().find(|(id, _)| *id == object))
                    .map(|(_, kind)| kind.clone())
            };
            let lanes = rows
                .iter()
                .filter_map(|row| match *row {
                    | Row::Lane { group, lane } => Some(&reel.groups[group].lanes[lane]),
                    | _ => None,
                })
                .map(|lane| {
                    let y = objects.get(&lane.objects[0]).map_or(0.0, |&(y, _)| y);
                    let segments = lane
                        .segments
                        .iter()
                        .map(|segment| {
                            let here = segment.from <= cursor && cursor < segment.to;
                            bus(segment, y, step, cursor, here.then(|| used(segment.object)).flatten(), !compact)
                        })
                        .collect_view();
                    let ends = lane.ends.iter().map(|&end| view! {
                        <line class=format!("tape-end {}", ahead(end)) x1=x(end) y1=y - 8.0 x2=x(end) y2=y + 8.0 />
                    }).collect_view();
                    view! { <g class=palette::class(&lane.name)>{segments}{ends}</g> }
                })
                .collect_view();
            let flows = slants(reel, &objects, &folded, step)
                .into_iter()
                .map(|slant| {
                    view! {
                        <g class=format!("tape-flow {}", ahead(slant.step))>
                            <line x1=slant.x0 y1=slant.y0 x2=slant.x1 y2=slant.y1 marker-end="url(#tape-arrow)" />
                            <circle cx=slant.x0 cy=slant.from r="2.4" />
                        </g>
                    }
                })
                .collect_view();
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
                    {calls}
                    {lanes}
                    {flows}
                    <rect class="tape-blank" x=x(count) y=AXIS - 2.0 width=(width - x(count)).max(0.0) height=height />
                    // A compact tape writes it down the narrow blank.
                    {reel.open.then(|| if compact {
                        view! { <text class="tape-unrun" transform=format!("translate({:.1} {:.1}) rotate(90)", x(count) + 14.0, AXIS + 10.0)>"NOT YET RUN"</text> }.into_any()
                    } else {
                        view! { <text class="tape-unrun" x=x(count) + 10.0 y=AXIS + 16.0>"NOT YET RUN"</text> }.into_any()
                    })}
                    {(count > 0).then(|| view! { <line class="tape-head" x1=head y1=AXIS - 4.0 x2=head y2=height /> })}
                </svg>
            }
            .into_any()
        })
    })
}

/// How wide the tape is drawn: the run, `room` after its last step for
/// "NOT YET RUN" and the flag, and at least the window.
fn tape_width(count: usize, step: f64, view: f64, room: f64) -> f64 {
    let width = px(count) * step + room;
    // A run that misses the window by a sliver gives up that much of the
    // blank room after it rather than scroll a few pixels.
    if width - view <= SLIVER { view } else { width }
}

/// The most of the room after the run a tape gives up to fit its window.
const SLIVER: f64 = 16.0;

/// Steps between numbered ticks: the first round interval that leaves room
/// for a number.
fn tick_interval(step: f64) -> usize {
    [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000]
        .into_iter()
        .find(|&every| px(every) * step >= 44.0)
        .unwrap_or(2000)
}

/// The ruler: step numbers, the clock lane (one pulse per step, taller at
/// a sequence point) and the playhead's flag. It sits at the top of the
/// deck while the lanes scroll under it.
fn ruler_svg(reel: Memo<Reel>, step: f64, cursor: usize, (view, room): (f64, f64)) -> AnyView {
    reel.with(|reel| {
        let count = reel.steps.len();
        let x = move |index: usize| px(index) * step;
        let width = tape_width(count, step, view, room);
        let head = x(cursor) + step / 2.0;
        let number = reel.steps.get(cursor).map_or(0, |step| step.0).to_string();
        let flag = px(number.len()) * 7.0 + 10.0;
        let every = tick_interval(step);
        let ticks = (0..count)
            .filter(|index| index % every == 0)
            .map(|index| {
                // No number under the flag.
                let clear = (x(index) + 3.0 - head).abs() > flag / 2.0 + 22.0;
                view! {
                    <line class="tape-grid" x1=x(index) y1=AXIS - 4.0 x2=x(index) y2=AXIS />
                    {clear.then(|| view! { <text class="tape-number" x=x(index) + 3.0 y="11">{reel.steps[index].0}</text> })}
                }
            })
            .collect_view();
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
        view! {
            <svg class="tape-ruler" width=width height=AXIS viewBox=format!("0 0 {width} {AXIS}") aria-hidden="true">
                <rect class="tape-ruler-ground" width=width height=AXIS />
                {ticks}
                <path class="tape-clock" d=clock />
                {(count > 0).then(|| view! {
                    <line class="tape-head" x1=head y1="14" x2=head y2=AXIS />
                    <rect class="tape-flag" x=head - flag / 2.0 y="0" width=flag height="15" />
                    <text class="tape-flag-number" x=head y="11" text-anchor="middle">{number}</text>
                })}
            </svg>
        }
        .into_any()
    })
}

/// One slant as drawn: from a value read to the store that took it, in
/// tape coordinates, with the step of the store.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Slant {
    flow: usize,
    step: usize,
    from: f64,
    x0:   f64,
    y0:   f64,
    x1:   f64,
    y1:   f64,
}

/// Steps narrower than this draw no slants: a thousand steps at a pixel
/// each would be a fence of arrows.
const SLANT_MIN_STEP: f64 = 4.0;

/// Every slant of the reel whose ends are on screen (a lane, or the row of
/// a folded call), except a value that stays in its own lane, which the
/// lane carries.
fn slants(
    reel: &Reel,
    objects: &HashMap<usize, (f64, &str)>,
    folded: &HashMap<usize, f64>,
    step: f64,
) -> Vec<Slant> {
    if step < SLANT_MIN_STEP {
        return Vec::new();
    }
    let mut slants = Vec::new();
    for (index, flow) in reel.flows.iter().enumerate() {
        let Some(&(to, _)) = objects.get(&flow.to.0) else {
            continue;
        };
        let x1 = px(flow.to.1) * step + 2.0;
        for &(object, at) in &flow.from {
            let Some(from) = objects
                .get(&object)
                .map(|&(y, _)| y)
                .or_else(|| folded.get(&object).copied())
            else {
                continue;
            };
            if (from - to).abs() < 1.0 {
                continue;
            }
            let down = if to > from { 1.0 } else { -1.0 };
            slants.push(Slant {
                flow: index,
                step: flow.to.1,
                from,
                x0: px(at) * step + step / 2.0,
                y0: from + down * 8.0,
                x1,
                y1: to - down * 8.0,
            });
        }
    }
    slants
}

/// Whether the segment of `slant` (and its arrowhead, a few pixels past
/// its end) meets the box `(left, top, right, bottom)`.
fn crosses(slant: &Slant, (left, top, right, bottom): (f64, f64, f64, f64)) -> bool {
    // Liang–Barsky clipping of the segment against the box.
    let (dx, dy) = (slant.x1 - slant.x0, slant.y1 - slant.y0);
    let length = dx.hypot(dy).max(1.0);
    let (dx, dy) = (dx * (length + 6.0) / length, dy * (length + 6.0) / length);
    let mut low = 0.0_f64;
    let mut high = 1.0_f64;
    for (p, q) in [
        (-dx, slant.x0 - left),
        (dx, right - slant.x0),
        (-dy, slant.y0 - top),
        (dy, bottom - slant.y0),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else {
            let t = q / p;
            if p < 0.0 {
                low = low.max(t);
            } else {
                high = high.min(t);
            }
        }
    }
    low <= high
}

/// One bus segment: a hexagon between two transitions, with its value
/// written inside (cut short where it does not fit, and repeated along a
/// long segment, so one is always in view). `used` is what the step at the
/// playhead did to it, if it is the segment there: a store lights it, a
/// read outlines it in the flow's blue, a designation dashes it. A compact
/// tape writes values apart (`pinned_values`), where they stay in view.
fn bus(
    segment: &Segment,
    y: f64,
    step: f64,
    cursor: usize,
    used: Option<String>,
    written: bool,
) -> AnyView {
    let (x0, x1) = (px(segment.from) * step + 1.5, px(segment.to) * step - 1.5);
    let slope = (5.0_f64).min((x1 - x0) / 2.0);
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
    let class = bus_class(segment, cursor);
    let room = ((x1 - x0 - 2.0 * slope - 8.0).min(320.0) / CHAR).floor();
    let text = segment.value.clone().unwrap_or_else(|| "?".to_owned());
    let text = if room >= 3.0 && written {
        shorten(&text, chars_in(room))
    } else {
        String::new()
    };
    let text_width = px(text.chars().count()) * CHAR + 6.0;
    let mut labels = Vec::new();
    let mut at = x0 + slope + 4.0;
    while !text.is_empty() && at + text_width <= x1 - slope + 1.0 {
        labels.push(at);
        at += (text_width + 60.0).max(240.0);
    }
    view! {
        <g class=class data-object=segment.object data-access=used>
            <polygon points=points />
            {labels.into_iter().map(|at| view! { <text x=at y=y + 4.5>{text.clone()}</text> }).collect_view()}
        </g>
    }
    .into_any()
}

/// A bus segment's classes: indeterminate, under the playhead, ahead of it.
fn bus_class(segment: &Segment, cursor: usize) -> String {
    let mut class = String::from("tape-bus");
    if segment.value.is_none() {
        class.push_str(" is-indeterminate");
    }
    if segment.from <= cursor && cursor < segment.to {
        class.push_str(" is-now");
    }
    if segment.from > cursor {
        class.push_str(" is-ahead");
    }
    class
}

/// A value written on a compact tape: its segment's classes and access,
/// where it starts, its baseline and its text.
#[derive(Clone, Debug, PartialEq)]
struct Pinned {
    class:  String,
    access: Option<String>,
    left:   f64,
    y:      f64,
    text:   String,
}

/// The values of a compact tape, one per segment in the visible `window`,
/// each at the start of its segment or, where that has scrolled out of the
/// window, at the window's left edge: a phone's narrow window shows the
/// value of every lane it crosses.
fn pinned_values(
    reel: &Reel,
    rows: &[Row],
    step: f64,
    cursor: usize,
    window: (f64, f64),
) -> Vec<Pinned> {
    let Layout { objects, .. } = layout(reel, rows);
    let mut pinned = Vec::new();
    for row in rows {
        let &Row::Lane { group, lane } = row else {
            continue;
        };
        let lane = &reel.groups[group].lanes[lane];
        let Some(&(y, _)) = lane.objects.first().and_then(|object| objects.get(object)) else {
            continue;
        };
        for segment in &lane.segments {
            let (x0, x1) = (px(segment.from) * step + 1.5, px(segment.to) * step - 1.5);
            if x1 <= window.0 || x0 >= window.1 {
                continue;
            }
            let slope = (5.0_f64).min((x1 - x0) / 2.0);
            let left = (x0 + slope + 4.0).max(window.0 + 6.0);
            let right = (x1 - slope - 2.0).min(window.1 - 4.0);
            let room = ((right - left).min(320.0) / CHAR).floor();
            let value = segment.value.clone().unwrap_or_else(|| "?".to_owned());
            if room < 3.0 && room < px(value.chars().count()) {
                continue;
            }
            let here = segment.from <= cursor && cursor < segment.to;
            pinned.push(Pinned {
                class: bus_class(segment, cursor),
                access: here
                    .then(|| {
                        reel.access.get(cursor).and_then(|access| {
                            access
                                .iter()
                                .find(|(id, _)| *id == segment.object)
                                .map(|(_, kind)| kind.clone())
                        })
                    })
                    .flatten(),
                left,
                y: y + 4.5,
                text: shorten(&value, chars_in(room)),
            });
        }
    }
    pinned
}

/// A whole number of characters, from a measured `room`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "A small, non-negative count: at most 40 characters."
)]
fn chars_in(room: f64) -> usize {
    room.clamp(0.0, 40.0) as usize
}

/// A reel: its hub turns as the playhead moves, and its pack of tape grows
/// (on the left) or shrinks (on the right) with what has played.
#[component]
fn Spool(side: &'static str, fill: Signal<f64>, turn: Signal<f64>) -> impl IntoView {
    view! {
        <div class=format!("spool is-{side}") aria-hidden="true">
            <svg viewBox="-28 -28 56 56" width="92" height="92">
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

/// Call `measure` on the deck's body whenever it or its rows change size:
/// a fold opened or a viewport resized lays the rows out again after the
/// effects that track them have run, so only the layout tells when the
/// rows outgrow the body or fit it again.
fn observe_rows(element: NodeRef<html::Div>, measure: impl Fn(&web_sys::Element) + Copy + 'static) {
    #[cfg(feature = "hydrate")]
    Effect::new(move |observing: Option<bool>| {
        use wasm_bindgen::JsCast;
        if observing == Some(true) {
            return true;
        }
        let Some(node) = element.get() else {
            return false;
        };
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(node)) = element.try_get_untracked() {
                measure(&node);
            }
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref()) {
            observer.observe(&node);
            // The lane names list every row, so they grow and shrink with
            // them.
            if let Some(names) = node.first_element_child() {
                observer.observe(&names);
            }
        }
        closure.forget();
        true
    });
    #[cfg(not(feature = "hydrate"))]
    let _ = (element, measure);
}

/// Keep `width` at the width of the deck's window onto the tape: its body
/// less the lane names pinned at its left; and `full` at the body's whole
/// width.
fn observe_width(element: NodeRef<html::Div>, width: RwSignal<f64>, full: RwSignal<f64>) {
    #[cfg(feature = "hydrate")]
    Effect::new(move |observing: Option<bool>| {
        use wasm_bindgen::JsCast;
        if observing == Some(true) {
            return true;
        }
        let Some(node) = element.get() else {
            return false;
        };
        let measure = |node: &web_sys::HtmlDivElement| {
            let names = node
                .first_element_child()
                .map_or(0.0, |names| names.get_bounding_client_rect().width());
            f64::from(node.client_width()) - names
        };
        full.set(f64::from(node.client_width()));
        width.set(measure(&node));
        // A change of view unmounts the deck, and the observer reports its
        // removal after the signals are gone; it then does nothing.
        let closure = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Some(Some(node)) = element.try_get_untracked() {
                _ = full.try_set(f64::from(node.client_width()));
                _ = width.try_set(measure(&node));
            }
        });
        if let Ok(observer) = web_sys::ResizeObserver::new(closure.as_ref().unchecked_ref()) {
            observer.observe(&node);
            // The names change width on their own where the deck turns
            // compact.
            if let Some(names) = node.first_element_child() {
                observer.observe(&names);
            }
        }
        closure.forget();
        true
    });
    #[cfg(not(feature = "hydrate"))]
    let _ = (element, width, full);
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
        execute(demos::find(key).expect("a lesson").source)
    }

    fn execute(source: &str) -> Vec<Snapshot> {
        let mut machine = Machine::default();
        let snapshot = |answer| match answer {
            | Ok(Answer::Snapshot(snapshot)) => *snapshot,
            | _ => panic!("a snapshot"),
        };
        let source = source.to_owned();
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
        let values: Vec<_> = lane(0, "side")
            .segments
            .iter()
            .map(|segment| segment.value.as_deref())
            .collect();
        assert_eq!(values, [None, Some("3")]);
        assert_eq!(lane(1, "n").segments[0].value.as_deref(), Some("4"));
        assert_eq!(lane(1, "n").ends.len(), 1, "n ends when square returns");
        // main waits while square runs.
        assert_eq!(reel.groups[0].calls[0].waiting[0].2, "square(side + 1)");
        // side flows into n at the call, labelled with the argument; n
        // flows into area through the return, labelled with what returned.
        let side = lane(0, "side").objects[0];
        let n = lane(1, "n").objects[0];
        let area = lane(0, "area").objects[0];
        let source = demos::find("path").unwrap().source;
        let label = |to: usize| {
            let flow = reel
                .flows
                .iter()
                .find(|flow| flow.to.0 == to)
                .expect("a flow");
            (
                flow.from
                    .iter()
                    .map(|&(object, _)| object)
                    .collect::<Vec<_>>(),
                flow.label.and_then(|range| source_text(source, range)),
            )
        };
        assert_eq!(label(n), (vec![side], Some("side + 1".to_owned())));
        assert_eq!(label(area), (vec![n, n], Some("n * n".to_owned())));
    }

    #[test]
    fn a_shadowing_object_gets_a_lane_of_its_own() {
        let history = execute(
            "int main(void) {\n    int x = 1;\n    {\n        int x = 20;\n        x = x + 1;\n    }\n    for \
             (int i = 0; i < 2; i = i + 1) {\n        int x = i;\n    }\n    return x;\n}\n",
        );
        let reel = Reel::record(&history);
        let lanes: Vec<(&str, usize, usize)> = reel.groups[0]
            .lanes
            .iter()
            .map(|lane| (lane.name.as_str(), lane.ordinal, lane.objects.len()))
            .collect();
        // The outer x lives throughout; each inner x takes the second lane
        // in turn, once the one before it has ended.
        assert_eq!(lanes, [("x", 1, 1), ("x", 2, 3), ("i", 1, 1)]);
        for lane in &reel.groups[0].lanes {
            for pair in lane.segments.windows(2) {
                assert!(
                    pair[0].to <= pair[1].from,
                    "one value at a time on {}",
                    lane.key
                );
            }
        }
    }

    #[test]
    fn reads_that_only_find_an_address_do_not_slant() {
        let source =
            "int fill(int *p, int n, int v) {\n    for (int i = 0; i < n; i = i + 1) {\n        \
             p[i] = v * 10 + i;\n    }\n    return n;\n}\n\nint main(void) {\n    int \
             numbers[3];\n    return fill(numbers, 3, 7);\n}\n";
        let history = execute(source);
        let reel = Reel::record(&history);
        let name = |object: usize| {
            reel.groups
                .iter()
                .flat_map(|group| &group.lanes)
                .find(|lane| lane.objects.contains(&object))
                .map(|lane| lane.name.as_str())
        };
        let into_numbers: Vec<&Flow> = reel
            .flows
            .iter()
            .filter(|flow| name(flow.to.0) == Some("numbers"))
            .collect();
        assert_eq!(into_numbers.len(), 3);
        for flow in into_numbers {
            // v and the second i are values; p and the subscript's i only
            // find the element.
            let from: Vec<_> = flow
                .from
                .iter()
                .filter_map(|&(object, _)| name(object))
                .collect();
            assert_eq!(from, ["v", "i"]);
            assert_eq!(
                flow.label
                    .and_then(|range| source_text(source, range))
                    .as_deref(),
                Some("v * 10 + i")
            );
        }
    }

    #[test]
    fn a_deep_recursion_keeps_its_ends_open_and_folds_the_rest_to_a_ladder() {
        let history = execute(
            "int depth(int n) {\n    if (n <= 0) {\n        return 0;\n    }\n    int r = depth(n \
             - 1);\n    return r + 1;\n}\n\nint main(void) {\n    return depth(6);\n}\n",
        );
        let reel = Reel::record(&history);
        // The innermost call: every depth is live.
        let deepest = (0..history.len())
            .max_by_key(|&at| history[at].stack.len())
            .unwrap();
        let rows = rows(&reel, deepest, &[]);
        let open: Vec<usize> = rows
            .iter()
            .filter_map(|row| match *row {
                | Row::Group {
                    at, folded: false, ..
                } if reel.groups[at].name == "depth" => Some(reel.groups[at].depth),
                | _ => None,
            })
            .collect();
        assert_eq!(open, [2, 7, 8], "the outermost call and the two innermost");
        let ladder: Vec<usize> = rows
            .iter()
            .find_map(|row| match row {
                | Row::Ladder { groups } =>
                    Some(groups.iter().map(|&at| reel.groups[at].depth).collect()),
                | _ => None,
            })
            .expect("a ladder");
        assert_eq!(ladder, [3, 4, 5, 6]);
    }

    /// Every label against every bus segment drawn, at each step of `history`.
    fn labels_clear_the_buses(history: &[Snapshot], source: &str) -> usize {
        let reel = Reel::record(history);
        let mut checked = 0;
        for cursor in 0..history.len() {
            let rows = rows(&reel, cursor, &[]);
            let step = 20.0;
            let placed = place_labels(&reel, &rows, source, step, cursor, (0.0, 1.0e9), &[]);
            let Layout {
                objects, folded, ..
            } = layout(&reel, &rows);
            for slant in slants(&reel, &objects, &folded, step) {
                for label in &placed {
                    let rect = (
                        label.left,
                        label.middle - LABEL_HALF,
                        label.right,
                        label.middle + LABEL_HALF,
                    );
                    assert!(
                        !crosses(&slant, rect),
                        "step {cursor}: a slant crosses \"{}\"",
                        label.text
                    );
                }
            }
            for (place, row) in rows.iter().enumerate() {
                let &Row::Lane { group, lane } = row else {
                    continue;
                };
                let y = AXIS + px(place) * LANE + LANE / 2.0;
                for segment in &reel.groups[group].lanes[lane].segments {
                    let (x0, x1) = (px(segment.from) * step, px(segment.to) * step);
                    for label in &placed {
                        let clear = label.right <= x0
                            || x1 <= label.left
                            || label.middle + LABEL_HALF <= y - BUS
                            || y + BUS <= label.middle - LABEL_HALF;
                        assert!(
                            clear,
                            "step {cursor}: \"{}\" covers a segment of {}",
                            label.text, reel.groups[group].lanes[lane].name
                        );
                    }
                }
            }
            checked += placed.len();
        }
        checked
    }

    #[test]
    fn slant_labels_sit_in_the_gaps_between_lanes_clear_of_slants() {
        let mut checked = 0;
        for key in ["path", "recursion", "tour", "pointers"] {
            checked += labels_clear_the_buses(&run(key), demos::find(key).unwrap().source);
        }
        let source = "int main(void) {\n    int a = 5;\n    int b = 7;\n    int c = 0;\n    int d \
                      = 1;\n    c = a + 1;\n    d = a * b + c;\n    b = d - a;\n    a = b + c + \
                      d;\n    return a;\n}\n";
        checked += labels_clear_the_buses(&execute(source), source);
        assert!(checked > 20, "labels were placed: {checked}");
    }

    #[test]
    fn each_recursive_function_folds_into_one_ladder() {
        let history = execute(
            "int sum(int n) {\n    if (n == 0) {\n        return 0;\n    }\n    int s = sum(n - \
             1);\n    return s + n;\n}\n\nint down(int n) {\n    if (n <= 0) {\n        return \
             0;\n    }\n    int r = down(n - 1);\n    return r + 1;\n}\n\nint main(void) {\n    \
             int total = sum(8);\n    int d = down(6);\n    return total + d;\n}\n",
        );
        let reel = Reel::record(&history);
        let rows = rows(&reel, history.len() - 1, &[]);
        let ladders: Vec<(String, usize)> = rows
            .iter()
            .filter_map(|row| match row {
                | Row::Ladder { groups } =>
                    Some((reel.groups[groups[0]].name.clone(), groups.len())),
                | _ => None,
            })
            .collect();
        assert_eq!(ladders, [("sum".to_owned(), 9), ("down".to_owned(), 7)]);
        // main's group and lanes, then one ladder per function.
        assert_eq!(rows.len(), 1 + reel.groups[0].lanes.len() + 2);
    }

    #[test]
    fn a_lane_keeps_one_type() {
        let history = execute(
            "int main(void) {\n    int v = 3;\n    {\n        int x = 1;\n        x = x + v;\n    \
             }\n    {\n        int *x = &v;\n        *x = 9;\n    }\n    return v;\n}\n",
        );
        let reel = Reel::record(&history);
        let lanes: Vec<(String, &str)> = reel.groups[0]
            .lanes
            .iter()
            .map(|lane| (lane.label(), lane.ty.as_str()))
            .collect();
        assert_eq!(
            lanes,
            [
                ("v".to_owned(), "int"),
                ("x".to_owned(), "int"),
                ("x #2".to_owned(), "int *")
            ]
        );
    }

    #[test]
    fn a_label_shows_only_beside_its_visible_arrowhead() {
        let history = run("path");
        let reel = Reel::record(&history);
        let last = history.len() - 1;
        let rows = rows(&reel, last, &[]);
        let source = demos::find("path").unwrap().source;
        let all = place_labels(&reel, &rows, source, 20.0, last, (0.0, 1.0e9), &[]);
        assert_ne!(all.len(), 0);
        // A window that has scrolled past every arrowhead shows no label,
        // rather than one pinned to its edge.
        let past = px(history.len()) * 20.0 + 50.0;
        assert_eq!(
            place_labels(&reel, &rows, source, 20.0, last, (past, past + 400.0), &[]).len(),
            0
        );
    }

    #[test]
    fn crossing_is_segment_against_box() {
        let slant = Slant {
            flow: 0,
            step: 0,
            from: 0.0,
            x0:   0.0,
            y0:   0.0,
            x1:   100.0,
            y1:   100.0,
        };
        assert!(crosses(&slant, (40.0, 40.0, 60.0, 60.0)));
        assert!(!crosses(&slant, (60.0, 0.0, 100.0, 30.0)));
        // The arrowhead reaches a few pixels past the end.
        assert!(crosses(&slant, (102.0, 102.0, 110.0, 110.0)));
    }

    #[test]
    fn long_values_are_cut_short_whole_elements_first() {
        assert_eq!(shorten("{3,1,4,1,5,9,2,6}", 10), "{3,1,4,…}");
        assert_eq!(shorten("2147483647", 10), "2147483647");
        assert_eq!(shorten("→a_rather_long_name", 8), "→a_rath…");
        assert_eq!(fit_slots(10), 16);
        assert_eq!(fit_slots(16), 16);
        assert_eq!(fit_slots(22), 32);
        assert_eq!(fit_slots(40), 64);
    }

    #[test]
    fn a_short_run_fits_its_window_and_the_scale_changes_seldom() {
        for view in [360.0, 390.0, 670.0, 1070.0] {
            let room = if view < COMPACT_BELOW {
                ROOM_COMPACT
            } else {
                ROOM
            };
            let mut scales = Vec::new();
            for count in 1..=64 {
                let step = fitted_step(count, view - room, 1.0);
                // A run that fits at the narrowest fitted step never scrolls.
                if step > FIT_MIN {
                    assert!(
                        tape_width(count, step, view, room) <= view,
                        "{count} steps scroll in {view} px"
                    );
                }
                if scales.last() != Some(&step) {
                    scales.push(step);
                }
            }
            assert!(scales.len() <= 3, "{view} px: {scales:?}");
        }
    }

    #[test]
    fn a_sliver_of_overflow_takes_the_room_instead_of_a_scrollbar() {
        // 50 steps of 14 px and 130 px of room: 830 px.
        assert_eq!(tape_width(50, 14.0, 827.0, ROOM), 827.0);
        assert_eq!(tape_width(50, 14.0, 814.0, ROOM), 814.0);
        assert_eq!(tape_width(50, 14.0, 813.0, ROOM), 830.0);
        assert_eq!(tape_width(5, 14.0, 900.0, ROOM), 900.0);
    }

    #[test]
    fn a_compact_tape_keeps_each_value_in_its_window() {
        let history = run("path");
        let reel = Reel::record(&history);
        let source = demos::find("path").expect("a lesson").source;
        for cursor in 0..history.len() {
            let rows = rows(&reel, cursor, &[]);
            for left in [0.0, 37.0, 120.0, 260.0] {
                let window = (left, left + 300.0);
                for value in pinned_values(&reel, &rows, 18.0, cursor, window) {
                    let right = value.left + px(value.text.chars().count()) * CHAR;
                    assert!(
                        value.left >= window.0 + 6.0 && right <= window.1,
                        "{value:?} outside {window:?}"
                    );
                }
                let avoid = tags(&reel, &rows, left);
                for label in place_labels(&reel, &rows, source, 18.0, cursor, window, &avoid) {
                    assert!(
                        avoid.iter().all(|&(l, t, r, b)| label.right < l
                            || r < label.left
                            || label.middle + LABEL_HALF < t
                            || b < label.middle - LABEL_HALF),
                        "{label:?} over a lane's name"
                    );
                }
            }
        }
        // The lane under the window's left edge still says what it holds.
        let rows = rows(&reel, history.len() - 1, &[]);
        let values = pinned_values(&reel, &rows, 18.0, history.len() - 1, (200.0, 500.0));
        assert!(
            values.iter().any(|value| (value.left - 206.0).abs() < 0.01),
            "{values:?}"
        );
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
        assert_eq!(
            reel.groups[bump].calls.len(),
            3,
            "one group for the loop's three calls"
        );
        let last_call = reel.groups[bump].calls[2].last;
        let after = rows(&reel, last_call + 2, &[]);
        assert!(after.contains(&Row::Group {
            at:     bump,
            folded: true,
            live:   false,
        }));
        let opened = rows(&reel, last_call + 2, &[reel.groups[bump].key.clone()]);
        assert!(
            opened.len() > after.len(),
            "an opened group shows its lanes"
        );
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
        assert_eq!(
            value_text(&pointer(1, false), &objects).as_deref(),
            Some("→values[1]")
        );
        assert_eq!(
            value_text(&pointer(3, true), &objects).as_deref(),
            Some("→values+3")
        );
        assert_eq!(
            value_text(&objects[0].value, &objects).as_deref(),
            Some("{?,?,?}")
        );
    }
}

//! Why one line takes several steps: the steps of the expression the current
//! step belongs to, in the order the machine took them. The machine takes
//! one evaluation per step (designating an object, reading it, applying an
//! operator, storing), and a full expression is done at the sequence point
//! that ends it (C99 §5.1.2.3p2, §6.8p4). A call the expression makes folds
//! into one entry, however many steps the called function takes.
//!
//! The grouping reads only what the engine reported: a step belongs to the
//! expression whose tree it shows, in the same call, and the steps of a call
//! that expression is waiting on belong to it too. Calls are told apart by
//! where they are in the source, so two calls spelled alike stay two.

use leptos::prelude::*;

use crate::snapshot::{
    N1256_URL,
    Range,
    Snapshot,
};

/// The steps of one expression, as far as the run has recorded them.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The expression's source, on one line.
    pub text:     String,
    /// The function whose call evaluates it.
    pub function: String,
    pub parts:    Vec<Part>,
    /// Whether the group is all there: the run has recorded the step after
    /// its last, or ended there, or the engine's next operation lies outside
    /// the expression.
    pub complete: bool,
    /// What the engine says the next step does, when the current step is the
    /// last the run has recorded and the group goes on.
    pub next:     Option<String>,
    /// Where Over, pressed at the current step, stops.
    pub over:     OverStop,
}

/// Where Over stops, as the group can show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverStop {
    /// At the recorded step at this index into the history: the group's
    /// last.
    At(usize),
    /// At the group's last step, which the run has not reached yet.
    End,
    /// The current step is the group's last, so at the last step of the
    /// next expression.
    Next,
}

impl Group {
    /// The current step's place among the group's own steps (from 1), or
    /// `None` while the current step is in a call the expression made.
    pub fn place(&self) -> Option<usize> {
        let current = self.parts.iter().position(Part::current)?;
        matches!(self.parts[current], Part::Step { .. }).then(|| self.own(current + 1))
    }

    /// How many of the group's own steps the run has recorded.
    pub fn total(&self) -> usize {
        self.own(self.parts.len())
    }

    /// How many of the first `count` parts are the expression's own steps.
    fn own(&self, count: usize) -> usize {
        self.parts[..count]
            .iter()
            .filter(|part| matches!(part, Part::Step { .. }))
            .count()
    }

    /// The function the current step runs in a call the expression made.
    pub fn calling(&self) -> Option<&str> {
        self.parts.iter().find_map(|part| match part {
            | Part::Call {
                function,
                current: true,
                ..
            } => Some(function.as_str()),
            | _ => None,
        })
    }
}

/// One entry of a group.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// A step of the expression itself.
    Step {
        /// Index into the history, to jump to.
        index:          usize,
        step:           usize,
        title:          String,
        category:       String,
        sequence_point: bool,
        current:        bool,
        /// Whether the step comes after the current one: the reader went
        /// back.
        after:          bool,
    },
    /// The steps of a call the expression made, from the call to the last
    /// step before it returned.
    Call {
        index:          usize,
        function:       String,
        first:          usize,
        last:           usize,
        /// Whether a sequence point comes before the call, after its
        /// function designator and arguments are evaluated (C99 §6.5.2.2p10).
        sequence_point: bool,
        current:        bool,
        after:          bool,
    },
}

impl Part {
    pub fn current(&self) -> bool {
        match self {
            | Self::Step { current, .. } | Self::Call { current, .. } => *current,
        }
    }
}

/// The expression a step evaluates: the call that evaluates it, and a source
/// range in its full expression, which is the full expression's own range or,
/// at a call's first step, the call's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    frame: usize,
    root:  Range,
}

/// How a step belongs to a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Member {
    /// It evaluates part of the expression.
    Own,
    /// It runs in a call the expression is waiting on.
    InCall,
}

/// The expression `snapshot`'s step evaluated, if it evaluated one: a
/// declaration's creation of its object, a block's entry or exit and a jump
/// stand on their own.
fn key(snapshot: &Snapshot) -> Option<Key> {
    let frame = snapshot.stack.first()?.id;
    let root = snapshot.tree.as_ref()?.nodes.last()?.range?;
    Some(Key { frame, root })
}

/// Whether `snapshot`'s step belongs to the expression `key` names. Full
/// expressions do not overlap, so the one whose range holds `key`'s is it.
pub fn member(snapshot: &Snapshot, key: Key) -> Option<Member> {
    if self::key(snapshot).is_some_and(|own| own.frame == key.frame && within(key.root, own.root)) {
        return Some(Member::Own);
    }
    // A call the expression made is still running: its frame is below the
    // innermost one, waiting on a call inside the expression. The call
    // itself is the first such step.
    snapshot
        .stack
        .iter()
        .skip(1)
        .any(|frame| frame.id == key.frame && frame.call.is_some_and(|call| within(call, key.root)))
        .then_some(Member::InCall)
}

/// The expression the step at `index` belongs to. A call's first step
/// shows the called function, which has no expression yet: it belongs to
/// the caller's, which holds the call the caller waits on. The called
/// function's own steps that evaluate no expression stand alone.
pub fn key_at(history: &[Snapshot], index: usize) -> Option<Key> {
    let snapshot = history.get(index)?;
    key(snapshot).or_else(|| {
        if snapshot.event.kind != "call" {
            return None;
        }
        let caller = snapshot.stack.get(1)?;
        Some(Key {
            frame: caller.id,
            root:  caller.call?,
        })
    })
}

/// Step over's walk: to the last step of the expression the step it starts
/// at belongs to, through any call it makes. At that last step, or at a step
/// that evaluates no expression, it walks to the last step of the next one
/// instead (a step that evaluates none is its own last step).
#[derive(Clone, Copy, Debug)]
pub struct Over {
    finishing: Option<Key>,
    steps:     usize,
}

impl Over {
    /// A walk from the step at `cursor`.
    pub fn new(history: &[Snapshot], cursor: usize) -> Self {
        Self {
            finishing: key_at(history, cursor),
            steps:     0,
        }
    }

    /// Take the step the walk has reached, at `cursor`: `None` to go on,
    /// or the step to stop at.
    pub fn stepped(&mut self, history: &[Snapshot], cursor: usize) -> Option<usize> {
        self.steps += 1;
        let inside = self.finishing.is_some_and(|finishing| {
            history
                .get(cursor)
                .and_then(|snapshot| member(snapshot, finishing))
                .is_some()
        });
        if inside {
            return None;
        }
        if self.steps > 1 {
            // One step past the expression's last: show its last.
            return Some(cursor.saturating_sub(1));
        }
        // The first step left the expression: finish the next one.
        self.finishing = key_at(history, cursor);
        self.finishing.is_none().then_some(cursor)
    }
}

/// Whether `inner` lies within `outer`.
fn within(inner: Range, outer: Range) -> bool {
    let at = |position: crate::snapshot::Position| (position.line, position.column);
    at(inner.start) >= at(outer.start) && at(inner.end) <= at(outer.end)
}

/// The group of the step at `cursor`, with what the history has recorded
/// before and after it; `None` for a step that evaluates no expression.
pub fn group(history: &[Snapshot], cursor: usize, source: &str) -> Option<Group> {
    let mut key = key_at(history, cursor)?;
    let mut first = cursor;
    while first > 0 && member(&history[first - 1], key).is_some() {
        first -= 1;
    }
    let mut last = cursor;
    while last + 1 < history.len() && member(&history[last + 1], key).is_some() {
        last += 1;
    }
    // At a call's first step the key holds only the call; a step of the
    // expression's own holds the whole of it.
    if let Some(own) = history[first..=last]
        .iter()
        .filter(|snapshot| member(snapshot, key) == Some(Member::Own))
        .find_map(self::key)
    {
        key = own;
    }
    let complete =
        last + 1 < history.len() || !history[last].can_step() || ends(&history[last], key);
    let mut parts = Vec::new();
    let mut index = first;
    while index <= last {
        let snapshot = &history[index];
        match member(snapshot, key) {
            | Some(Member::InCall) => {
                let start = index;
                while index < last && member(&history[index + 1], key) == Some(Member::InCall) {
                    index += 1;
                }
                // The function the expression called sits just above its
                // own frame on the stack.
                let function = snapshot
                    .stack
                    .iter()
                    .position(|frame| frame.id == key.frame)
                    .and_then(|place| place.checked_sub(1))
                    .and_then(|place| snapshot.stack.get(place))
                    .map_or_default(|frame| frame.name.clone());
                parts.push(Part::Call {
                    index: start,
                    function,
                    first: snapshot.step,
                    last: history[index].step,
                    sequence_point: snapshot.event.sequence_point,
                    current: (start..=index).contains(&cursor),
                    after: start > cursor,
                });
            },
            | _ => parts.push(Part::Step {
                index,
                step: snapshot.step,
                title: snapshot.event.title.replace('`', ""),
                category: snapshot.event.category.clone(),
                sequence_point: snapshot.event.sequence_point,
                current: index == cursor,
                after: index > cursor,
            }),
        }
        index += 1;
    }
    let next = (!complete && last == cursor)
        .then(|| {
            history[cursor]
                .upcoming
                .as_ref()
                .map(|upcoming| upcoming.text.replace('`', ""))
        })
        .flatten();
    // Over's own walk (`Over::stepped`) finishes the expression, or the
    // next one from its last step.
    let over = match (complete, last == cursor) {
        | (false, _) => OverStop::End,
        | (true, true) => OverStop::Next,
        | (true, false) => OverStop::At(last),
    };
    Some(Group {
        text: source_text(source, key.root),
        function: history[cursor]
            .stack
            .iter()
            .find(|frame| frame.id == key.frame)
            .map_or_default(|frame| frame.name.clone()),
        parts,
        complete,
        next,
        over,
    })
}

/// Whether the expression `key` names is done at `snapshot`'s step: the
/// step is the expression's own, and the next operation lies outside it.
fn ends(snapshot: &Snapshot, key: Key) -> bool {
    member(snapshot, key) == Some(Member::Own)
        && snapshot
            .location
            .is_some_and(|next| !within(next, key.root))
}

/// The source of `range`, its whitespace runs closed up to one space, cut
/// short past 48 characters.
fn source_text(source: &str, range: Range) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let mut text = String::new();
    for line in range.start.line..=range.end.line {
        let Some(units) = lines
            .get(line.wrapping_sub(1))
            .map(|line| line.encode_utf16().collect::<Vec<u16>>())
        else {
            continue;
        };
        // Columns count UTF-16 code units from 1.
        let from = if line == range.start.line {
            range.start.column.saturating_sub(1)
        } else {
            0
        };
        let to = if line == range.end.line {
            range.end.column.saturating_sub(1).min(units.len())
        } else {
            units.len()
        };
        if from < to {
            text.push(' ');
            text.push_str(&String::from_utf16_lossy(&units[from..to]));
        }
    }
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 48 {
        let mut shown: String = text.chars().take(47).collect();
        shown.push('…');
        shown
    } else {
        text
    }
}

/// The current step among the steps of its expression, each a button to
/// jump to, with the sequence points between them marked.
#[component]
pub fn Evaluation(group: Memo<Option<Group>>, on_jump: Callback<usize>) -> impl IntoView {
    let list = NodeRef::<leptos::html::Ol>::new();
    // Beside the tape the steps of a long expression scroll in a window of
    // their own: each step brings its entry into it, once the entries are
    // laid out.
    Effect::new(move || {
        group.track();
        request_animation_frame(move || {
            if let Some(Some(list)) = list.try_get_untracked() {
                reveal_current(&list);
            }
        });
    });
    let heading = move || {
        group.with(|group| {
            group.as_ref().map(|group| {
                let total = group.total();
                let so_far = if group.complete { "" } else { " so far" };
                let count = match (group.place(), group.calling()) {
                    | (Some(at), _) => format!("step {at} of {total}{so_far}"),
                    | (None, Some(function)) => format!("waiting on {function}()"),
                    | (None, None) => format!("{total} steps{so_far}"),
                };
                view! {
                    <span class="evaluation-label">"Evaluating"</span>
                    " "
                    <code class="evaluation-text">{group.text.clone()}</code>
                    " "
                    <span class="evaluation-count">{format!("{count} · in {}()", group.function)}</span>
                }
            })
        })
    };
    let parts = move || {
        group.with(|group| {
            group.as_ref().map(|group| {
                let stop = match group.over {
                    | OverStop::At(index) => Some(index),
                    | OverStop::End | OverStop::Next => None,
                };
                let mut views = group
                    .parts
                    .iter()
                    .map(|part| part_view(part, stop, on_jump))
                    .collect::<Vec<_>>();
                if let Some(next) = &group.next {
                    views.push(
                        view! {
                            <li class="evaluation-next">
                                <span class="evaluation-index">"next"</span>
                                <span class="evaluation-name">{next.clone()}</span>
                            </li>
                        }
                        .into_any(),
                    );
                } else if !group.complete {
                    views.push(
                        view! {
                            <li class="evaluation-next">
                                <span class="evaluation-name">"more to come"</span>
                            </li>
                        }
                        .into_any(),
                    );
                }
                // Where Over stops when the panel has no entry for it yet.
                let beyond = match group.over {
                    | OverStop::At(_) => None,
                    | OverStop::End => Some("runs to its last step"),
                    | OverStop::Next => Some("runs on to the next expression"),
                };
                if let Some(beyond) = beyond {
                    views.push(
                        view! {
                            <li class="evaluation-end">
                                {over_tag()}
                                <span class="evaluation-name">{beyond}</span>
                            </li>
                        }
                        .into_any(),
                    );
                }
                views.collect_view()
            })
        })
    };
    view! {
        <section class="evaluation" aria-labelledby="evaluation-title" hidden=move || group.with(Option::is_none)>
            <h3 class="evaluation-title" id="evaluation-title">{heading}</h3>
            <ol class="evaluation-steps" node_ref=list>{parts}</ol>
            <p class="evaluation-key">
                "One evaluation per step; "<b>"Over"</b>" (O) stops where its tag is. "
                <span class="evaluation-point-key" aria-hidden="true"></span>
                " marks a "
                <a href=format!("{N1256_URL}#page=25") target="_blank" rel="noopener" title="C99 §5.1.2.3p2: at a sequence point every earlier side effect is complete">
                    "sequence point"<span class="sr-only">" (C99 §5.1.2.3p2, opens the standard in a new tab)"</span>
                </a>
                ": every side effect before it is complete. On the tape its clock pulse stands taller."
            </p>
        </section>
    }
}

/// Scroll `list`, where it scrolls, just far enough that the current entry
/// and the one after it (the next step, or what comes next) are in its
/// window, or the current entry alone where both do not fit.
fn reveal_current(list: &web_sys::Element) {
    let (height, shown) = (
        f64::from(list.client_height()),
        f64::from(list.scroll_top()),
    );
    if f64::from(list.scroll_height()) <= height + 1.0 {
        return;
    }
    let Ok(Some(current)) = list.query_selector(":scope > .is-current") else {
        return;
    };
    let origin = list.get_bounding_client_rect().top() - shown;
    let box_of = |element: &web_sys::Element| {
        let rect = element.get_bounding_client_rect();
        (rect.top() - origin, rect.bottom() - origin)
    };
    let (top, mut bottom) = box_of(&current);
    if let Some(after) = current.next_element_sibling() {
        let (_, below) = box_of(&after);
        if below - top <= height - 8.0 {
            bottom = below;
        }
    }
    let target = if top - 4.0 < shown {
        top - 4.0
    } else if bottom + 4.0 > shown + height {
        bottom + 4.0 - height
    } else {
        return;
    };
    let options = web_sys::ScrollToOptions::new();
    options.set_top(target.max(0.0));
    options.set_behavior(web_sys::ScrollBehavior::Instant);
    list.scroll_to_with_scroll_to_options(&options);
}

/// The tag that marks where Over stops: the Over key's arrow and its name.
fn over_tag() -> impl IntoView {
    view! {
        <span class="evaluation-over">
            <svg viewBox="0 0 20 20" width="12" height="12" aria-hidden="true"><path d="M4 13a6 6 0 0 1 11-3M15 6v4h-4" /></svg>
            "Over"
        </span>
    }
}

fn part_view(part: &Part, stop: Option<usize>, on_jump: Callback<usize>) -> AnyView {
    match part.clone() {
        | Part::Step {
            index,
            step,
            title,
            category,
            sequence_point,
            current,
            after,
        } => {
            let over = stop == Some(index);
            let label = format!(
                "Step {step}: {title}{}{}{}",
                if sequence_point {
                    ", then a sequence point"
                } else {
                    ""
                },
                if current { " (this step)" } else { "" },
                if over { " (Over stops here)" } else { "" }
            );
            view! {
                <li class="evaluation-step" data-category=category class:is-current=current class:is-after=after class:is-point=sequence_point class:is-over=over>
                    <button
                        type="button"
                        aria-label=label
                        aria-current=current.then_some("step")
                        title=title.clone()
                        on:click=move |_| on_jump.run(index)
                    >
                        <span class="evaluation-index" aria-hidden="true">{step}</span>
                        <span class="evaluation-name" aria-hidden="true">{title.clone()}</span>
                        {over.then(|| view! { <span class="evaluation-over-at" aria-hidden="true">{over_tag()}</span> })}
                    </button>
                </li>
            }
            .into_any()
        },
        | Part::Call {
            index,
            function,
            first,
            last,
            sequence_point,
            current,
            after,
        } => {
            let count = last + 1 - first;
            let plural = if count == 1 { "" } else { "s" };
            let label = call_label(&function, first, last, sequence_point, current);
            view! {
                <li class="evaluation-call" class:is-current=current class:is-after=after class:is-point=sequence_point>
                    {sequence_point.then(|| view! { <span class="evaluation-point" aria-hidden="true"></span> })}
                    <button type="button" aria-label=label title=format!("Jump to the call to {function}()") on:click=move |_| on_jump.run(index)>
                        <span class="evaluation-index" aria-hidden="true">{first}</span>
                        <span class="evaluation-name" aria-hidden="true">{format!("{function}() runs: {count} step{plural}")}</span>
                    </button>
                </li>
            }
            .into_any()
        },
    }
}

/// What a folded call's entry announces. The sequence point comes after the
/// arguments, before the call: it leads the entry, however many steps the
/// call folds.
fn call_label(
    function: &str,
    first: usize,
    last: usize,
    sequence_point: bool,
    current: bool,
) -> String {
    let steps = if first == last {
        format!("step {first}")
    } else {
        format!("steps {first}–{last}")
    };
    format!(
        "{} to {function}, {steps}{}",
        if sequence_point {
            "A sequence point, then the call"
        } else {
            "The call"
        },
        if current {
            " (the step shown is one of them)"
        } else {
            ""
        }
    )
}

#[cfg(test)]
mod tests;

//! Why one line takes several steps: the steps of the expression the current
//! step belongs to, in the order the machine took them. The machine takes
//! one evaluation per step (designating an object, reading it, applying an
//! operator, storing), and a full expression is done at the sequence point
//! that ends it (C99 §5.1.2.3p2, §6.8p4). A call the expression makes folds
//! into one entry, however many steps the called function takes.
//!
//! The grouping reads only what the engine reported: a step belongs to the
//! expression whose tree it shows, in the same call, and the steps of a call
//! that expression is waiting on belong to it too.

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
    /// Whether the run has recorded the step after the group's last, or
    /// ended there: the group is all there.
    pub complete: bool,
    /// What the engine says the next step does, when the current step is the
    /// last the run has recorded and the group goes on.
    pub next:     Option<String>,
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
        index:    usize,
        function: String,
        first:    usize,
        last:     usize,
        current:  bool,
        after:    bool,
    },
}

impl Part {
    pub fn current(&self) -> bool {
        match self {
            | Self::Step { current, .. } | Self::Call { current, .. } => *current,
        }
    }
}

/// The expression a step evaluates: the call that evaluates it, and its
/// full expression's source range.
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

/// Whether `snapshot`'s step belongs to the expression `key` names.
pub fn member(snapshot: &Snapshot, key: Key) -> Option<Member> {
    if self::key(snapshot) == Some(key) {
        return Some(Member::Own);
    }
    // A call the expression made is still running: its frame is below the
    // innermost one. The call itself is the first such step.
    snapshot
        .stack
        .iter()
        .skip(1)
        .any(|frame| frame.id == key.frame)
        .then_some(Member::InCall)
}

/// The expression the step at `index` belongs to. A call's first step
/// shows the called function, which has no expression yet: it belongs to
/// the caller's, which the step before it evaluated.
pub fn key_at(history: &[Snapshot], index: usize) -> Option<Key> {
    let snapshot = history.get(index)?;
    key(snapshot).or_else(|| {
        let before = key(history.get(index.checked_sub(1)?)?)?;
        (member(snapshot, before) == Some(Member::InCall)).then_some(before)
    })
}

/// The group of the step at `cursor`, with what the history has recorded
/// before and after it; `None` for a step that evaluates no expression.
pub fn group(history: &[Snapshot], cursor: usize, source: &str) -> Option<Group> {
    let key = key_at(history, cursor)?;
    let mut first = cursor;
    while first > 0 && member(&history[first - 1], key).is_some() {
        first -= 1;
    }
    let mut last = cursor;
    while last + 1 < history.len() && member(&history[last + 1], key).is_some() {
        last += 1;
    }
    let complete = last + 1 < history.len() || !history[last].can_step();
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
            history[cursor].effects.first().map(|effect| {
                let effect = effect.strip_prefix("Next: ").unwrap_or(effect);
                effect.replace('`', "")
            })
        })
        .flatten();
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
    })
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
                    <span class="evaluation-count">{format!("{count} · in {}()", group.function)}</span>
                }
            })
        })
    };
    let parts = move || {
        group.with(|group| {
            group.as_ref().map(|group| {
                let mut views = group
                    .parts
                    .iter()
                    .map(|part| part_view(part, on_jump))
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
                views.collect_view()
            })
        })
    };
    view! {
        <section class="evaluation" aria-labelledby="evaluation-title" hidden=move || group.with(Option::is_none)>
            <h3 class="evaluation-title" id="evaluation-title">{heading}</h3>
            <ol class="evaluation-steps">{parts}</ol>
            <p class="evaluation-key">
                "One evaluation per step. "
                <span class="evaluation-point-key" aria-hidden="true"></span>
                " marks a "
                <a href=format!("{N1256_URL}#page=25") target="_blank" rel="noopener" title="C99 §5.1.2.3p2: at a sequence point every earlier side effect is complete">
                    "sequence point"<span class="sr-only">" (C99 §5.1.2.3p2, opens the standard in a new tab)"</span>
                </a>
                ": every side effect before it is complete."
            </p>
        </section>
    }
}

fn part_view(part: &Part, on_jump: Callback<usize>) -> AnyView {
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
            let label = format!(
                "Step {step}: {title}{}{}",
                if sequence_point {
                    ", then a sequence point"
                } else {
                    ""
                },
                if current { " (this step)" } else { "" }
            );
            view! {
                <li class="evaluation-step" data-category=category class:is-current=current class:is-after=after class:is-point=sequence_point>
                    <button
                        type="button"
                        aria-label=label
                        aria-current=current.then_some("step")
                        title=title.clone()
                        on:click=move |_| on_jump.run(index)
                    >
                        <span class="evaluation-index" aria-hidden="true">{step}</span>
                        <span class="evaluation-name" aria-hidden="true">{title.clone()}</span>
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
            current,
            after,
        } => {
            let count = last + 1 - first;
            let plural = if count == 1 { "" } else { "s" };
            let steps = if first == last {
                format!("step {first}")
            } else {
                format!("steps {first}–{last}")
            };
            let label = format!(
                "The call to {function}, {steps}{}",
                if current {
                    " (the step shown is one of them)"
                } else {
                    ""
                }
            );
            view! {
                <li class="evaluation-call" class:is-current=current class:is-after=after>
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

#[cfg(test)]
mod tests;

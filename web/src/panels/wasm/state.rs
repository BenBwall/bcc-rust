//! The WebAssembly machine's state, one card each: the operand stack, the
//! current call's locals, the call stack, the blocks a branch can reach,
//! the globals, and the program's output.

use leptos::prelude::*;

use crate::{
    app::wasm::WasmState,
    wasm::{
        Slot,
        WasmSnapshot,
    },
};

/// A memo of one part of the current snapshot.
fn part<T: Clone + PartialEq + Send + Sync + 'static>(
    wasm: WasmState,
    read: fn(&WasmSnapshot) -> T,
) -> Memo<Option<T>> {
    Memo::new(move |_| wasm.current.with(|current| current.as_ref().map(read)))
}

#[component]
pub fn OperandStack(wasm: WasmState) -> impl IntoView {
    let stack = part(wasm, |snapshot| (snapshot.stack.clone(), snapshot.below));
    view! {
        <section class="card wasm-card stack-card" aria-labelledby="operands-title">
            <header class="card-header">
                <h2 id="operands-title">"Operand stack"</h2>
                <span class="card-subtitle">"top first"</span>
            </header>
            {move || stack.with(|stack| {
                let Some((operands, below)) = stack else {
                    return ().into_any();
                };
                let below = *below;
                view! {
                    {if operands.is_empty() {
                        view! { <p class="empty-state compact">"Empty. Constants and reads push values here; operations pop them."</p> }.into_any()
                    } else {
                        view! {
                            <ol class="operands">
                                {operands.iter().rev().enumerate().map(|(index, operand)| view! {
                                    <li class="operand" class:is-fresh=operand.fresh class:is-top=index == 0>
                                        <span class="value-type" data-type=operand.ty.clone()>{operand.ty.clone()}</span>
                                        <span class="operand-value">{operand.text.clone()}</span>
                                    </li>
                                }).collect_view()}
                            </ol>
                        }.into_any()
                    }}
                    <p class="operands-below" hidden=below == 0>
                        {format!("+ {below} value{} below, waiting in the callers", if below == 1 { "" } else { "s" })}
                    </p>
                }.into_any()
            })}
        </section>
    }
}

#[component]
pub fn Locals(wasm: WasmState) -> impl IntoView {
    let locals = part(wasm, |snapshot| {
        (!snapshot.frames.is_empty()).then(|| snapshot.locals.clone())
    });
    let function = part(wasm, |snapshot| {
        snapshot
            .frames
            .first()
            .map(|frame| frame.name.clone())
            .unwrap_or_default()
    });
    view! {
        <section class="card wasm-card locals-card" aria-labelledby="locals-title">
            <header class="card-header">
                <h2 id="locals-title">"Locals"</h2>
                <span class="card-subtitle">{move || function.get().filter(|name| !name.is_empty()).map(|name| format!("of ${name}"))}</span>
            </header>
            {move || locals.with(|locals| match locals {
                | Some(Some(locals)) if !locals.is_empty() => view! {
                    <table class="slots">
                        <tbody>
                            {locals.iter().map(slot_row).collect_view()}
                        </tbody>
                    </table>
                }.into_any(),
                | Some(Some(_)) => view! { <p class="empty-state compact">"This function has no parameters or locals."</p> }.into_any(),
                | Some(None) => view! { <p class="empty-state compact">"No call is running."</p> }.into_any(),
                | None => ().into_any(),
            })}
        </section>
    }
}

fn slot_row(slot: &Slot) -> impl IntoView {
    let name = slot
        .name
        .as_ref()
        .map_or_else(|| slot.index.to_string(), |name| format!("${name}"));
    view! {
        <tr class="slot" data-touched=slot.touched.clone() title=slot.note.clone().unwrap_or_default()>
            <th scope="row" class="slot-name">{name}</th>
            <td><span class="value-type" data-type=slot.ty.clone()>{slot.ty.clone()}</span></td>
            <td class="slot-value">{slot.text.clone()}</td>
            <td class="slot-role">
                <span class="slot-tag" data-role=slot.role.clone()>{slot.role.clone()}</span>
            </td>
        </tr>
    }
}

#[component]
pub fn CallStack(wasm: WasmState) -> impl IntoView {
    let frames = part(wasm, |snapshot| snapshot.frames.clone());
    view! {
        <section class="card wasm-card calls-card" aria-labelledby="calls-title">
            <header class="card-header">
                <h2 id="calls-title">"Call stack"</h2>
                <span class="card-subtitle">"innermost first"</span>
            </header>
            {move || frames.with(|frames| match frames {
                | Some(frames) if !frames.is_empty() => view! {
                    <ol class="calls">
                        {frames.iter().enumerate().map(|(index, frame)| {
                            let place = match frame.source {
                                | Some(line) => format!("C line {line}"),
                                | None => "no C line".to_owned(),
                            };
                            let waiting = if index == 0 { "running" } else { "waiting for its call" };
                            view! {
                                <li class="call" class:is-active=index == 0>
                                    <span class="call-name">{format!("${}", frame.name)}</span>
                                    <span class="call-place">{format!("{place} · {waiting}")}</span>
                                    {frame.stack_pointer.map(|pointer| view! {
                                        <span class="call-frame" title="The shadow stack pointer when this call began">{format!("sp {pointer}")}</span>
                                    })}
                                </li>
                            }
                        }).collect_view()}
                    </ol>
                }.into_any(),
                | Some(_) => view! { <p class="empty-state compact">"No calls are active: the program has returned to the host."</p> }.into_any(),
                | None => ().into_any(),
            })}
        </section>
    }
}

/// The labels of the current call: what each `br N` would reach.
#[component]
pub fn Blocks(wasm: WasmState) -> impl IntoView {
    let labels = part(wasm, |snapshot| snapshot.labels.clone());
    view! {
        <section class="card wasm-card blocks-card" aria-labelledby="blocks-title">
            <header class="card-header">
                <h2 id="blocks-title">"Branch targets"</h2>
                <span class="card-subtitle">"what br N reaches"</span>
            </header>
            {move || labels.with(|labels| match labels {
                | Some(labels) if !labels.is_empty() => view! {
                    <ol class="labels">
                        {labels.iter().enumerate().map(|(depth, label)| {
                            let (name, goes) = match label.kind.as_str() {
                                | "function" => ("function body".to_owned(), "returns from the function".to_owned()),
                                | "loop" => (format!("loop @{}", label.name), format!("back to its start, line {}", label.start)),
                                | kind => (format!("{kind} @{}", label.name), format!("out, past its end on line {}", label.end)),
                            };
                            view! {
                                <li class="label" data-kind=label.kind.clone()>
                                    <span class="label-depth" title=format!("br {depth}")>{depth}</span>
                                    <span class="label-name">{name}</span>
                                    <span class="label-goes">{goes}</span>
                                </li>
                            }
                        }).collect_view()}
                    </ol>
                }.into_any(),
                | _ => view! { <p class="empty-state compact">"No call is running."</p> }.into_any(),
            })}
        </section>
    }
}

#[component]
pub fn Globals(wasm: WasmState) -> impl IntoView {
    let globals = part(wasm, |snapshot| snapshot.globals.clone());
    view! {
        <section class="card wasm-card globals-card" aria-labelledby="globals-title">
            <header class="card-header">
                <h2 id="globals-title">"Globals"</h2>
                <span class="card-subtitle">"shared by every call"</span>
            </header>
            {move || globals.with(|globals| {
                let Some(globals) = globals else {
                    return ().into_any();
                };
                if globals.is_empty() {
                    return view! { <p class="empty-state compact">"This module has no globals."</p> }.into_any();
                }
                let (mutable, constant): (Vec<&Slot>, Vec<&Slot>) =
                    globals.iter().partition(|global| global.role == "mut");
                let constants = constant.len();
                view! {
                    <table class="slots globals">
                        <tbody>
                            {mutable.into_iter().map(global_row).collect_view()}
                        </tbody>
                    </table>
                    <details class="globals-constants" hidden=constants == 0>
                        <summary>{format!("{constants} constant global{} the linker added", if constants == 1 { "" } else { "s" })}</summary>
                        <table class="slots globals">
                            <tbody>{constant.into_iter().map(global_row).collect_view()}</tbody>
                        </table>
                    </details>
                }.into_any()
            })}
        </section>
    }
}

fn global_row(slot: &Slot) -> impl IntoView {
    let name = slot
        .name
        .as_ref()
        .map_or_else(|| slot.index.to_string(), |name| format!("${name}"));
    view! {
        <tr class="slot" data-touched=slot.touched.clone()>
            <th scope="row" class="slot-name">{name}</th>
            <td><span class="value-type" data-type=slot.ty.clone()>{slot.ty.clone()}</span></td>
            <td class="slot-value">{slot.text.clone()}</td>
            <td class="slot-note">{slot.note.clone().unwrap_or_default()}</td>
        </tr>
    }
}

/// What the program wrote to standard output and standard error.
#[component]
pub fn Console(wasm: WasmState) -> impl IntoView {
    let output = part(wasm, |snapshot| snapshot.output.clone());
    view! {
        <section class="card wasm-card console-card" aria-labelledby="console-title" hidden=move || output.with(|output| output.as_ref().is_none_or(Vec::is_empty))>
            <header class="card-header">
                <h2 id="console-title">"Output"</h2>
                <span class="card-subtitle">"written through WASI's fd_write"</span>
            </header>
            <pre class="console">
                {move || output.get().unwrap_or_default().into_iter().map(|chunk| view! {
                    <span data-stream=chunk.stream>{chunk.text}</span>
                }).collect_view()}
            </pre>
        </section>
    }
}

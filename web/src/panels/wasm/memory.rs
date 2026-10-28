//! Linear memory as rows of bytes: the shadow stack (with each call's
//! frame shaded and `__stack_pointer` marked), the static data, and the
//! bytes the last instruction read or wrote. Addresses are decimal, as the
//! narration and the operand stack show them; bytes are hexadecimal, or
//! grouped into little-endian i32 words.

use leptos::prelude::*;

use crate::{
    app::wasm::WasmState,
    wasm::{
        MemoryView,
        MemoryWindow,
    },
};

#[component]
pub fn LinearMemory(wasm: WasmState) -> impl IntoView {
    let view = Memo::new(move |_| {
        wasm.current
            .with(|current| current.as_ref().map(|current| current.memory.clone()))
    });
    let tab = move || {
        let wanted = wasm.memory_tab.get();
        view.with(|view| {
            let windows = view
                .as_ref()
                .map(|view| view.windows.as_slice())
                .unwrap_or_default();
            // Follow an access as it happens; otherwise keep the choice.
            if windows.iter().any(|window| window.key == wanted) {
                wanted
            } else {
                windows.first().map_or("stack", |window| key(&window.key))
            }
        })
    };
    view! {
        <section class="card wasm-card memory-view" aria-labelledby="linear-memory-title" hidden=move || view.with(|view| view.as_ref().is_none_or(|view| view.pages == 0))>
            <header class="card-header memory-header">
                <h2 id="linear-memory-title">"Linear memory"</h2>
                <span class="card-subtitle">
                    {move || view.with(|view| view.as_ref().map(|view| {
                        format!("{} page{} · {} bytes", view.pages, if view.pages == 1 { "" } else { "s" }, u64::from(view.pages) * 65_536)
                    }))}
                </span>
            </header>
            <div class="memory-controls">
                <div class="opt-toggle" role="tablist" aria-label="Memory region">
                    {move || view.with(|view| {
                        view.as_ref()
                            .map(|view| view.windows.clone())
                            .unwrap_or_default()
                            .into_iter()
                            .map(|window| {
                                let name = key(&window.key);
                                view! {
                                    <button
                                        type="button"
                                        role="tab"
                                        class="opt-option memory-tab"
                                        aria-selected=move || (tab() == name).to_string()
                                        on:click=move |_| wasm.memory_tab.set(name)
                                    >
                                        {window.title}
                                    </button>
                                }
                            })
                            .collect_view()
                    })}
                </div>
                <div class="opt-toggle" role="group" aria-label="Show memory as">
                    <button type="button" class="opt-option" aria-pressed=move || (!wasm.memory_words.get()).to_string() on:click=move |_| wasm.memory_words.set(false)>"bytes"</button>
                    <button type="button" class="opt-option" aria-pressed=move || wasm.memory_words.get().to_string() on:click=move |_| wasm.memory_words.set(true)>"i32"</button>
                </div>
            </div>
            {move || view.with(|view| {
                let view = view.as_ref()?;
                let selected = tab();
                let window = view.windows.iter().find(|window| window.key == selected)?;
                Some(view! {
                    <p class="memory-note">{window.note.clone()}</p>
                    {grid(view, window, wasm.memory_words.get())}
                    {bands(view)}
                })
            })}
        </section>
    }
}

/// A window key as a `'static` tab name.
fn key(key: &str) -> &'static str {
    match key {
        | "data" => "data",
        | "access" => "access",
        | _ => "stack",
    }
}

/// The window's rows: an address, then 16 bytes (or 4 words), then the
/// bytes as text.
fn grid(view: &MemoryView, window: &MemoryWindow, words: bool) -> AnyView {
    let rows = (window.start..).step_by(16).zip(window.bytes.chunks(16)).map(|(address, bytes)| {
        let cells = if words {
            (address..)
                .step_by(4)
                .zip(bytes.chunks(4))
                .map(|(at, word)| {
                    let mut raw = [0; 4];
                    raw[..word.len()].copy_from_slice(word);
                    let value = i32::from_le_bytes(raw);
                    let classes = classes(view, at, 4);
                    view! {
                        <span class=format!("memory-word {classes}") title=format!("{at}: i32 {value}")>{value}</span>
                    }
                    .into_any()
                })
                .collect::<Vec<_>>()
        } else {
            (address..)
                .zip(bytes)
                .map(|(at, byte)| {
                    let classes = classes(view, at, 1);
                    view! {
                        <span class=format!("memory-byte {classes}") title=format!("{at}: {byte} ({byte:#04x})")>{format!("{byte:02x}")}</span>
                    }
                    .into_any()
                })
                .collect::<Vec<_>>()
        };
        let text: String = bytes
            .iter()
            .map(|&byte| {
                if byte.is_ascii_graphic() || byte == b' ' {
                    char::from(byte)
                } else {
                    '·'
                }
            })
            .collect();
        view! {
            <div class="memory-row" class:is-words=words>
                <span class="memory-address" title=format!("{address:#x}")>{address}</span>
                <span class="memory-cells">{cells}</span>
                <span class="memory-text" aria-hidden="true">{text}</span>
            </div>
        }
    });
    view! {
        <div class="memory-grid" role="table" aria-label=format!("{} bytes from address {}", window.bytes.len(), window.start)>
            {rows.collect_view()}
        </div>
    }
    .into_any()
}

/// The classes for the `size` bytes at `address`: the last access, the
/// stack pointer, and which call's frame holds them.
fn classes(view: &MemoryView, address: u32, size: u32) -> String {
    let mut classes = Vec::new();
    let end = u64::from(address) + u64::from(size);
    if let Some(access) = view.access
        && u64::from(address) < access.address + access.size
        && end > access.address
    {
        classes.push(if access.write {
            "is-written"
        } else {
            "is-read"
        });
    }
    if view
        .stack_pointer
        .is_some_and(|pointer| pointer >= address && u64::from(pointer) < end)
    {
        classes.push("is-sp");
    }
    if let Some(index) = view
        .bands
        .iter()
        .position(|band| address >= band.start && address < band.end)
    {
        classes.push(if index == 0 {
            "band-current"
        } else if index % 2 == 1 {
            "band-odd"
        } else {
            "band-even"
        });
    } else if view.stack_top.is_some_and(|top| address < top)
        && view.stack_pointer.is_some_and(|pointer| address >= pointer)
    {
        classes.push("band-even");
    }
    classes.join(" ")
}

/// The legend: which call owns which part of the shadow stack.
fn bands(view: &MemoryView) -> AnyView {
    if view.bands.is_empty() {
        return ().into_any();
    }
    view! {
        <ul class="memory-bands" aria-label="Shadow stack frames, innermost first">
            <li class="band-sp"><i aria-hidden="true"></i>{format!("__stack_pointer = {}", view.stack_pointer.unwrap_or_default())}</li>
            {view.bands.iter().enumerate().map(|(index, band)| {
                let class = if index == 0 { "band-current" } else if index % 2 == 1 { "band-odd" } else { "band-even" };
                view! {
                    <li class=class><i aria-hidden="true"></i>{format!("${}: {}–{} ({} bytes)", band.name, band.start, band.end - 1, band.end - band.start)}</li>
                }
            }).collect_view()}
        </ul>
    }
    .into_any()
}

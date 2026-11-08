//! Figure 3: standard output, as a terminal screen. Everything the program
//! has written so far at the playhead, with the bytes the step wrote lit, so
//! Back and Forward show the output of each step as it was. A byte that is
//! not printable ASCII is shown as its escape sequence, set apart.

use leptos::prelude::*;

use crate::snapshot::Snapshot;

#[component]
pub fn Output(snapshot: Memo<Option<Snapshot>>) -> impl IntoView {
    let shown = Memo::new(move |_| {
        snapshot.with(|snapshot| {
            snapshot
                .as_ref()
                .filter(|snapshot| snapshot.stdout)
                .map(|snapshot| {
                    let split = snapshot.output.len() - snapshot.written.min(snapshot.output.len());
                    (
                        snapshot.output[..split].to_vec(),
                        snapshot.output[split..].to_vec(),
                    )
                })
        })
    });
    let count = move || {
        shown.with(|shown| {
            shown
                .as_ref()
                .map_or(0, |(before, now)| before.len() + now.len())
        })
    };
    view! {
        <figure class="output" aria-labelledby="output-caption" hidden=move || shown.with(Option::is_none)>
            <figcaption class="module-caption" id="output-caption">
                <b>"Figure 3"</b>
                {move || {
                    let count = count();
                    match count {
                        | 0 => " — Standard output".to_owned(),
                        | 1 => " — Standard output, 1 character".to_owned(),
                        | _ => format!(" — Standard output, {count} characters"),
                    }
                }}
            </figcaption>
            <pre class="output-screen" aria-label="What the program has written to standard output">
                {move || shown.with(|shown| match shown {
                    | Some((before, now)) if !before.is_empty() || !now.is_empty() => view! {
                        {text(before)}
                        <mark class="output-new" hidden=now.is_empty()>
                            <span class="sr-only">"Written by this step: "</span>
                            {text(now)}
                        </mark>
                    }.into_any(),
                    | _ => view! {
                        <span class="output-empty">"Nothing has been written yet. printf, puts and putchar write here."</span>
                    }.into_any(),
                })}
            </pre>
        </figure>
    }
}

/// `bytes` as text, with each byte that is not printable ASCII (other than a
/// new-line or a tab) shown as its escape.
fn text(bytes: &[u8]) -> Vec<AnyView> {
    let mut parts = Vec::new();
    let mut run = String::new();
    for &byte in bytes {
        match byte {
            | b'\n' | b'\t' | 0x20..=0x7E => run.push(char::from(byte)),
            | _ => {
                if !run.is_empty() {
                    parts.push(std::mem::take(&mut run).into_any());
                }
                let escape = if byte == 0 {
                    "\\0".to_owned()
                } else {
                    format!("\\x{byte:02x}")
                };
                parts.push(
                    view! {
                        <span class="output-byte" title=format!("byte {byte}, not printable ASCII")>{escape}</span>
                    }
                    .into_any(),
                );
            },
        }
    }
    if !run.is_empty() {
        parts.push(run.into_any());
    }
    parts
}

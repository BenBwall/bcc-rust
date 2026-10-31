//! An encoder knob, after the coloured encoders of a hardware sequencer: a
//! slider that turns. Arrow keys, Page Up and Down, the mouse wheel and a
//! vertical drag each turn it a detent at a time; its pointer shows where it
//! stands between its ends.

use leptos::prelude::*;

#[component]
pub fn Knob(
    /// The engraving under the knob, such as `ZOOM`.
    label: &'static str,
    /// Its colour: `blue`, `ochre` or `grey`.
    colour: &'static str,
    /// Where it stands, from 0 (fully left) to 1.
    position: Signal<f64>,
    /// Its reading, shown under it and announced.
    reading: Signal<String>,
    /// What one detent does: called with -1 or +1 (or ±10 for Page keys).
    on_turn: Callback<i32>,
    #[prop(optional)] disabled: Signal<bool>,
    /// Its accessible name.
    name: &'static str,
) -> impl IntoView {
    let dragging = StoredValue::new(None::<i32>);
    let turn = move |detents: i32| {
        if !disabled.get_untracked() && detents != 0 {
            on_turn.run(detents);
        }
    };
    view! {
        <div class=format!("knob is-{colour}")>
            <div
                class="knob-dial"
                role="slider"
                tabindex="0"
                aria-label=name
                aria-valuemin="0"
                aria-valuemax="100"
                aria-valuenow=move || format!("{:.0}", position.get() * 100.0)
                aria-valuetext=move || reading.get()
                aria-disabled=move || disabled.get().to_string()
                style:--turn=move || format!("{:.0}deg", -135.0 + 270.0 * position.get().clamp(0.0, 1.0))
                on:keydown=move |event| {
                    let detents = match event.key().as_str() {
                        | "ArrowUp" | "ArrowRight" => 1,
                        | "ArrowDown" | "ArrowLeft" => -1,
                        | "PageUp" => 10,
                        | "PageDown" => -10,
                        | _ => return,
                    };
                    // The page's arrow-key shortcuts step the machine; on a
                    // knob the arrows turn it instead.
                    event.prevent_default();
                    event.stop_propagation();
                    turn(detents);
                }
                on:wheel=move |event| {
                    event.prevent_default();
                    turn(if event.delta_y() < 0.0 { 1 } else { -1 });
                }
                on:pointerdown=move |event| {
                    dragging.set_value(Some(event.client_y()));
                    capture(&event);
                }
                on:pointermove=move |event| {
                    if let Some(start) = dragging.get_value() {
                        let detents = (start - event.client_y()) / 12;
                        if detents != 0 {
                            dragging.set_value(Some(start - detents * 12));
                            turn(detents);
                        }
                    }
                }
                on:pointerup=move |_| dragging.set_value(None)
                on:pointercancel=move |_| dragging.set_value(None)
            ></div>
            <span class="knob-label" aria-hidden="true">{label}</span>
            <output class="knob-reading" aria-hidden="true">{move || reading.get()}</output>
        </div>
    }
}

/// Keep a dragged pointer's moves on the element it pressed.
#[cfg(feature = "hydrate")]
pub(super) fn capture(event: &leptos::ev::PointerEvent) {
    use wasm_bindgen::JsCast;
    if let Some(target) = event
        .current_target()
        .and_then(|target| target.dyn_into::<web_sys::Element>().ok())
    {
        _ = target.set_pointer_capture(event.pointer_id());
    }
}

#[cfg(not(feature = "hydrate"))]
pub(super) fn capture(_event: &leptos::ev::PointerEvent) {}

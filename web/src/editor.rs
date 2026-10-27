//! Code views: Monaco (the editor behind VS Code) for the C source and the
//! compiler outputs, with syntax highlighting from the Monarch grammars in
//! `web/editor/src/languages.js`.
//!
//! [`CodeEditor`] renders a highlighted `<pre>` first, so prerendered pages
//! show the code before any script runs. In the browser it then loads the
//! Monaco bundle (from the content-hashed URL the build records in a `<meta>`
//! tag) and mounts an editor over the fallback. Text, language, theme and
//! decorations are signals; Rust decides what to highlight, and the bundle
//! only applies it.

use leptos::{
    html,
    prelude::*,
};
use serde::Serialize;

use crate::highlight;

/// One highlighted range or line. Columns are 1-based UTF-16 columns; a
/// `start_column` of 0 marks whole lines.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decoration {
    pub start_line:   usize,
    pub start_column: usize,
    pub end_line:     usize,
    pub end_column:   usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_class: Option<String>,
    /// A class for the whole line (when `start_column == 0`) or one box
    /// behind the range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub class_name:   Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub glyph_class:  Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hover:        Option<String>,
    /// Text drawn before the range without being part of the code, such as
    /// the mark where a line splice was, styled with `before_class`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before:       Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before_class: Option<String>,
    pub z_index:      i32,
}

impl Decoration {
    /// A whole-line decoration with `class`.
    pub fn line(line: usize, class: &str) -> Self {
        Self {
            start_line: line,
            end_line: line,
            class_name: Some(class.to_owned()),
            ..Self::default()
        }
    }
}

/// A diagnostic squiggle, drawn by Monaco's marker service with its message
/// on hover. Lines are 1-based; columns are 1-based UTF-16 columns, the end
/// just past the range.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub start_line:   usize,
    pub start_column: usize,
    pub end_line:     usize,
    pub end_column:   usize,
    /// `error`, `warning` or `note`.
    pub severity:     String,
    pub message:      String,
}

/// A named set of decorations, replaced as a unit.
pub type Layer = (&'static str, Vec<Decoration>);

/// The `<meta>` name under which the build records the editor bundle's URL.
pub const EDITOR_META: &str = "bcc-editor";

#[component]
pub fn CodeEditor(
    #[prop(into)] text: Signal<String>,
    language: Signal<&'static str>,
    /// `dark` or `light`.
    theme: Signal<&'static str>,
    label: &'static str,
    #[prop(optional)] read_only: bool,
    #[prop(optional)] decorations: Option<Signal<Vec<Layer>>>,
    #[prop(optional)] markers: Option<Signal<Vec<Marker>>>,
    #[prop(optional)] on_change: Option<Callback<String>>,
    #[prop(optional)] on_hover_line: Option<Callback<usize>>,
    #[prop(optional)] on_gutter_click: Option<Callback<usize>>,
    /// A line to scroll into view.
    #[prop(optional)]
    reveal: Option<Signal<Option<usize>>>,
    /// The gutter's label for each line, in place of its number; empty
    /// numbers the lines as usual.
    #[prop(optional)]
    line_numbers: Option<Signal<Vec<String>>>,
) -> impl IntoView {
    let host = NodeRef::<html::Div>::new();
    let ready = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    monaco::mount(monaco::Mount {
        host,
        ready,
        text,
        language,
        theme,
        label,
        read_only,
        decorations,
        markers,
        on_change,
        on_hover_line,
        on_gutter_click,
        reveal,
        line_numbers,
    });
    #[cfg(not(feature = "hydrate"))]
    let _ = (
        theme,
        read_only,
        decorations,
        markers,
        on_change,
        on_hover_line,
        on_gutter_click,
        reveal,
        line_numbers,
    );
    view! {
        <div class="code-editor" class:is-ready=move || ready.get() data-language=move || language.get()>
            <pre class="code-fallback" aria-label=label tabindex="0">
                {move || text.with(|text| highlight::fallback(text, language.get()))}
            </pre>
            <div class="monaco-host" node_ref=host></div>
        </div>
    }
}

#[cfg(feature = "hydrate")]
mod monaco {
    use std::cell::RefCell;

    use js_sys::Promise;
    use leptos::{
        html,
        prelude::*,
        task::spawn_local,
    };
    use serde::Serialize;
    use wasm_bindgen::{
        JsCast,
        JsValue,
        closure::Closure,
        prelude::wasm_bindgen,
    };
    use wasm_bindgen_futures::JsFuture;

    use super::{
        EDITOR_META,
        Layer,
        Marker,
    };

    #[wasm_bindgen(module = "/js/bridge.js")]
    extern "C" {
        #[wasm_bindgen(js_name = loadModule)]
        fn load_module(url: &str) -> Promise;
    }

    #[wasm_bindgen]
    extern "C" {
        /// The bundle's module namespace (`web/editor/src/main.js`).
        type EditorModule;
        #[wasm_bindgen(method, js_name = createEditor)]
        fn create_editor(
            this: &EditorModule,
            host: &web_sys::HtmlElement,
            options: &JsValue,
        ) -> Editor;
        #[wasm_bindgen(method, js_name = setTheme)]
        fn set_theme(this: &EditorModule, theme: &str);

        /// One Monaco editor.
        pub type Editor;
        #[wasm_bindgen(method, js_name = setValue)]
        fn set_value(this: &Editor, text: &str);
        #[wasm_bindgen(method, js_name = setLanguage)]
        fn set_language(this: &Editor, language: &str);
        #[wasm_bindgen(method, js_name = setDecorations)]
        fn set_decorations(this: &Editor, layer: &str, items: &JsValue);
        #[wasm_bindgen(method, js_name = setMarkers)]
        fn set_markers(this: &Editor, items: &JsValue);
        #[wasm_bindgen(method, js_name = revealLine)]
        fn reveal_line(this: &Editor, line: usize);
        #[wasm_bindgen(method, js_name = setLineNumbers)]
        fn set_line_numbers(this: &Editor, labels: &JsValue);
        #[wasm_bindgen(method, js_name = onChange)]
        fn on_change(this: &Editor, callback: &js_sys::Function);
        #[wasm_bindgen(method, js_name = onHoverLine)]
        fn on_hover_line(this: &Editor, callback: &js_sys::Function);
        #[wasm_bindgen(method, js_name = onGutterClick)]
        fn on_gutter_click(this: &Editor, callback: &js_sys::Function);
        #[wasm_bindgen(method)]
        fn dispose(this: &Editor);
    }

    thread_local! {
        static MODULE: RefCell<Option<Promise>> = const { RefCell::new(None) };
    }

    /// The editor bundle, loaded once per page.
    async fn module() -> Result<EditorModule, String> {
        let promise = MODULE.with(|module| {
            module
                .borrow_mut()
                .get_or_insert_with(|| load_module(&bundle_url()))
                .clone()
        });
        JsFuture::from(promise)
            .await
            .map(JsCast::unchecked_into)
            .map_err(|error| format!("{error:?}"))
    }

    fn bundle_url() -> String {
        let document = web_sys::window().and_then(|window| window.document());
        let recorded = document.as_ref().and_then(|document| {
            document
                .query_selector(&format!("meta[name=\"{EDITOR_META}\"]"))
                .ok()
                .flatten()?
                .get_attribute("content")
        });
        let relative = recorded.unwrap_or_else(|| "./editor/main.js".into());
        // A dynamic import resolves against the importing script, so make the
        // URL absolute against the page.
        document
            .and_then(|document| document.base_uri().ok().flatten())
            .and_then(|base| web_sys::Url::new_with_base(&relative, &base).ok())
            .map_or(relative, |url| url.href())
    }

    #[derive(Clone, Copy)]
    pub struct Mount {
        pub host:            NodeRef<html::Div>,
        pub ready:           RwSignal<bool>,
        pub text:            Signal<String>,
        pub language:        Signal<&'static str>,
        pub theme:           Signal<&'static str>,
        pub label:           &'static str,
        pub read_only:       bool,
        pub decorations:     Option<Signal<Vec<Layer>>>,
        pub markers:         Option<Signal<Vec<Marker>>>,
        pub on_change:       Option<Callback<String>>,
        pub on_hover_line:   Option<Callback<usize>>,
        pub on_gutter_click: Option<Callback<usize>>,
        pub reveal:          Option<Signal<Option<usize>>>,
        pub line_numbers:    Option<Signal<Vec<String>>>,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Options<'a> {
        value:      &'a str,
        language:   &'a str,
        read_only:  bool,
        aria_label: &'a str,
        theme:      &'a str,
    }

    /// A mounted editor and the Rust closures its listeners call.
    struct Instance {
        editor:     Editor,
        _listeners: Vec<Closure<dyn FnMut(JsValue)>>,
    }

    impl Instance {
        /// Release the Monaco editor (its model, listeners and layout
        /// observer) before the closures its listeners call are dropped.
        fn dispose(self) {
            self.editor.dispose();
        }
    }

    type Slot = StoredValue<Option<Instance>, LocalStorage>;

    /// Load Monaco into `mount.host` once it exists, then keep the editor in
    /// sync with the signals. The editor is disposed when the component's
    /// owner is cleaned up, as when the workbench switches views.
    pub fn mount(mount: Mount) {
        let editor: Slot = StoredValue::new_local(None);
        on_cleanup(move || {
            if let Some(instance) = editor.try_update_value(Option::take).flatten() {
                instance.dispose();
            }
        });
        let owner = Owner::current();
        Effect::new(move |started: Option<bool>| {
            if started == Some(true) {
                return true;
            }
            let Some(element) = mount.host.get() else {
                return false;
            };
            let owner = owner.clone();
            spawn_local(async move {
                let module = module().await;
                // The view may have unmounted while the bundle loaded; its
                // owner's cleanup disposed the slot, so create nothing.
                let Some(owner) = owner.filter(|_| !editor.is_disposed()) else {
                    return;
                };
                match module {
                    | Ok(module) => owner.with(|| create(mount, editor, module, &element)),
                    | Err(error) => {
                        web_sys::console::warn_1(&format!("Monaco unavailable: {error}").into());
                    },
                }
            });
            true
        });
        sync(mount, editor);
    }

    fn create(mount: Mount, editor: Slot, module: EditorModule, element: &web_sys::HtmlElement) {
        let options = Options {
            value:      &mount.text.get_untracked(),
            language:   mount.language.get_untracked(),
            read_only:  mount.read_only,
            aria_label: mount.label,
            theme:      mount.theme.get_untracked(),
        };
        let Ok(options) = serde_wasm_bindgen::to_value(&options) else {
            return;
        };
        let instance = module.create_editor(element, &options);
        let mut listeners = Vec::new();
        if let Some(callback) = mount.on_change {
            listeners.push(listen(
                &instance,
                Editor::on_change,
                move |value: JsValue| {
                    callback.run(value.as_string().unwrap_or_default());
                },
            ));
        }
        if let Some(callback) = mount.on_hover_line {
            listeners.push(listen(
                &instance,
                Editor::on_hover_line,
                move |line: JsValue| {
                    callback.run(line_number(&line));
                },
            ));
        }
        if let Some(callback) = mount.on_gutter_click {
            listeners.push(listen(
                &instance,
                Editor::on_gutter_click,
                move |line: JsValue| {
                    callback.run(line_number(&line));
                },
            ));
        }
        editor.set_value(Some(Instance {
            editor:     instance,
            _listeners: listeners,
        }));
        mount.ready.set(true);
        Effect::new(move || module.set_theme(mount.theme.get()));
    }

    /// Apply text, language, decorations and scroll requests once ready.
    fn sync(mount: Mount, editor: Slot) {
        let Mount {
            ready,
            text,
            language,
            decorations,
            markers,
            reveal,
            line_numbers,
            ..
        } = mount;
        let with = move |apply: &dyn Fn(&Editor)| {
            editor.try_with_value(|instance| {
                if let Some(instance) = instance {
                    apply(&instance.editor);
                }
            });
        };
        Effect::new(move || {
            let value = text.get();
            if ready.get() {
                with(&|editor| editor.set_value(&value));
            }
        });
        Effect::new(move || {
            let language = language.get();
            if ready.get() {
                with(&|editor| editor.set_language(language));
            }
        });
        if let Some(decorations) = decorations {
            Effect::new(move || {
                let layers = decorations.get();
                if ready.get() {
                    with(&|editor| {
                        for (name, items) in &layers {
                            if let Ok(items) = serde_wasm_bindgen::to_value(items) {
                                editor.set_decorations(name, &items);
                            }
                        }
                    });
                }
            });
        }
        if let Some(markers) = markers {
            Effect::new(move || {
                let markers = markers.get();
                if ready.get()
                    && let Ok(items) = serde_wasm_bindgen::to_value(&markers)
                {
                    with(&|editor| editor.set_markers(&items));
                }
            });
        }
        if let Some(line_numbers) = line_numbers {
            Effect::new(move || {
                let labels = line_numbers.get();
                if ready.get()
                    && let Ok(labels) = serde_wasm_bindgen::to_value(&labels)
                {
                    with(&|editor| editor.set_line_numbers(&labels));
                }
            });
        }
        if let Some(reveal) = reveal {
            Effect::new(move || {
                let line = reveal.get();
                if ready.get()
                    && let Some(line) = line
                {
                    with(&|editor| editor.reveal_line(line));
                }
            });
        }
    }

    /// Register `handler` through `register`, returning the closure, which
    /// must outlive the editor's listener.
    fn listen(
        editor: &Editor,
        register: fn(&Editor, &js_sys::Function),
        handler: impl FnMut(JsValue) + 'static,
    ) -> Closure<dyn FnMut(JsValue)> {
        let closure = Closure::<dyn FnMut(JsValue)>::new(handler);
        register(editor, closure.as_ref().unchecked_ref());
        closure
    }

    fn line_number(value: &JsValue) -> usize {
        value
            .as_f64()
            .filter(|line| line.is_finite() && *line >= 0.0)
            .map_or(0, |line| {
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "Monaco line numbers are small non-negative integers."
                )]
                let line = line as usize;
                line
            })
    }
}

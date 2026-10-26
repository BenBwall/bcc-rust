//! Static Leptos site for the bcc C abstract machine.
//!
//! `app` holds the page and its controls; `engine` proxies the engine Worker
//! over the messages in `protocol`; `source_view` and `panels` render engine
//! snapshots, whose shape `snapshot` describes; `demos` embeds the demo C
//! programs. `machine` runs the bcc abstract machine inside the Worker.
//!
//! Each build uses one feature. `ssr` (`src/bin/prerender.rs`) renders [`App`]
//! to HTML once at build time; `hydrate` (`src/main.rs`) attaches to that
//! markup in the browser; `worker` (`src/bin/engine_worker.rs`) is the engine.

mod app;
mod demos;
mod engine;
#[cfg(feature = "worker")]
pub mod machine;
mod panels;
pub mod protocol;
mod snapshot;
mod source_view;

pub use app::App;

/// The `<meta>` name under which the build records the engine Worker's
/// content-hashed URL.
pub const ENGINE_META: &str = "bcc-engine-worker";

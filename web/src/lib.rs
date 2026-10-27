//! Static Leptos site for the bcc C abstract machine and compiler pipeline.
//!
//! `app` holds the page and its controls; `engine` proxies the engine Worker
//! over the messages in `protocol`; `editor` mounts Monaco for the source and
//! the compiler outputs, with `highlight` colouring the prerendered fallback;
//! `panels` render engine snapshots (whose shape `snapshot` describes) and the
//! compiler pipeline (`pipeline`); `palette` keeps each variable's colour the
//! same in every view; `demos` embeds the lesson programs. `machine` runs the
//! bcc abstract machine inside its Worker, and `explorer` runs bcc's real
//! compiler inside another.
//!
//! Each build uses one feature. `ssr` (`src/bin/prerender.rs`) renders [`App`]
//! to HTML once at build time; `hydrate` (`src/main.rs`) attaches to that
//! markup in the browser; `worker` (`src/bin/engine_worker.rs`) is the
//! abstract machine; `compiler` (`src/bin/compiler_worker.rs`) is the
//! compiler, which the page loads only when it first compiles.

// Leptos views are nested types; the pipeline view's prerendering nests
// deeper than the compiler's default query depth allows.
#![recursion_limit = "256"]

mod app;
mod demos;
mod editor;
mod engine;
#[cfg(feature = "compiler")]
pub mod explorer;
mod highlight;
#[cfg(feature = "worker")]
pub mod machine;
mod palette;
mod panels;
mod pipeline;
pub mod protocol;
mod snapshot;

pub use app::App;
pub use editor::EDITOR_META;

/// The `<meta>` name under which the build records the engine Worker's
/// content-hashed URL.
pub const ENGINE_META: &str = "bcc-engine-worker";

/// The `<meta>` name under which the build records the compiler Worker's
/// content-hashed URL.
pub const COMPILER_META: &str = "bcc-compiler-worker";

/// The file name bcc's in-browser compiler gives the page's source, in token
/// locations and diagnostics.
pub const INPUT_NAME: &str = "<input>";

//! Where the WebAssembly view's modules come from.
//!
//! [`WasmBackEnd`] is the seam: anything that can turn C source into a
//! module, with or without a source-line map. [`Precompiled`] holds the
//! bundled lessons, compiled by clang at `-O0 -g` (`web/wasm/generate.sh`),
//! whose DWARF the stepper reads for line numbers; `bcc.rs` compiles any
//! source live.

use super::WasmModule;
use crate::demos;

/// A source of WebAssembly modules for C programs.
pub trait WasmBackEnd {
    /// Compile `source` to a module.
    ///
    /// # Errors
    /// Why this back end has no module for `source`, for the page to show.
    fn compile(&self, source: &str) -> Result<WasmModule, String>;
}

/// The lessons, precompiled by clang.
pub struct Precompiled;

impl WasmBackEnd for Precompiled {
    fn compile(&self, source: &str) -> Result<WasmModule, String> {
        let demo = demos::matching(source).ok_or_else(|| UNAVAILABLE.to_owned())?;
        let bytes = MODULES
            .iter()
            .find(|(name, _)| *name == demo.name)
            .map(|(_, bytes)| *bytes)
            .ok_or_else(|| UNAVAILABLE.to_owned())?;
        Ok(WasmModule {
            bytes:    bytes.to_vec(),
            lines:    None,
            producer: "clang -O0 -g, precompiled".into(),
            command:  format!(
                "$ clang {LESSON_FLAGS} {} -o {}",
                demo.name,
                demo.name.replace(".c", ".wasm")
            ),
            text:     None,
        })
    }
}

/// The flags `web/wasm/generate.sh` compiles the lessons with; each
/// module records them, with its source, in a `bcc.lesson` custom section.
pub const LESSON_FLAGS: &str =
    "--target=wasm32-unknown-unknown -nostdlib -O0 -g -Wl,--no-entry -Wl,--export-all";

/// The custom section that records how a lesson module was built.
pub const LESSON_SECTION: &str = "bcc.lesson";

/// Why there is no precompiled module: only the lessons have one.
pub const UNAVAILABLE: &str =
    "Only the bundled lessons have a module that clang compiled ahead of time.";

/// Each lesson's module, by its file name in `web/demos/`.
const MODULES: [(&str, &[u8]); 9] = [
    ("objects.c", include_bytes!("../../wasm/objects.wasm")),
    ("pointers.c", include_bytes!("../../wasm/pointers.wasm")),
    ("recursion.c", include_bytes!("../../wasm/recursion.wasm")),
    ("promotion.c", include_bytes!("../../wasm/promotion.wasm")),
    ("overflow.c", include_bytes!("../../wasm/overflow.wasm")),
    ("bounds.c", include_bytes!("../../wasm/bounds.wasm")),
    (
        "indeterminate.c",
        include_bytes!("../../wasm/indeterminate.wasm"),
    ),
    ("macros.c", include_bytes!("../../wasm/macros.wasm")),
    ("path.c", include_bytes!("../../wasm/path.wasm")),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_lesson_has_a_module() {
        for demo in &demos::DEMOS {
            let module = Precompiled.compile(demo.source).unwrap();
            assert!(module.bytes.starts_with(b"\0asm"), "{}", demo.name);
        }
        assert!(Precompiled.compile("int main(void) { return 1; }").is_err());
    }
}

//! Where the WebAssembly view's modules come from.
//!
//! [`WasmBackEnd`] is the seam: anything that can turn C source into a
//! module, with or without a source-line map. [`Precompiled`] is the only
//! implementation until bcc's WebAssembly back end lands: the bundled
//! lessons, compiled by clang at `-O0 -g` (`web/wasm/generate.sh`), whose
//! DWARF the stepper reads for line numbers.

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
                "$ clang --target=wasm32 -nostdlib -O0 -g -Wl,--no-entry -Wl,--export-all {} -o {}",
                demo.name,
                demo.name.replace(".c", ".wasm")
            ),
        })
    }
}

/// Why edited source has no module yet.
pub const UNAVAILABLE: &str = "bcc's WebAssembly back end is not part of this build yet, so the \
                               WebAssembly view can show only the bundled lessons, which clang \
                               compiled ahead of time. Pick a lesson, or step through your edits \
                               on the abstract machine.";

/// Each lesson's module, by its file name in `web/demos/`.
const MODULES: [(&str, &[u8]); 7] = [
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

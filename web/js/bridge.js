// The page's only hand-written glue besides the Monaco bundle: Rust cannot
// express a dynamic import(), so wasm-bindgen imports this function to load
// the editor bundle from its content-hashed URL (see src/editor.rs).
export function loadModule(url) {
  return import(url);
}

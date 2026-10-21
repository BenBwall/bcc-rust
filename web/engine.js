// The only execution boundary. Replace this explicit unavailable result with the
// Rust Wasm contract when its artifact and ABI are available in this checkout.
export async function connectEngine() {
  throw new Error("The Rust WebAssembly build is not installed. No program has executed.");
}

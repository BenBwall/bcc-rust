import { createWasmSession } from "./wasm-session.js";

let session;
self.addEventListener("message", async ({ data }) => {
  const response = { version: 1, id: data?.id };
  try {
    if (data?.version !== 1) throw new Error("Unsupported Worker protocol.");
    if (data.command === "connect") {
      const url = new URL("./pkg/bcc_rust.wasm", import.meta.url);
      const fetched = await fetch(url, { cache: "no-store" });
      if (!fetched.ok) throw new Error(`Rust Wasm artifact not found (${fetched.status}). Run sh web/build.sh from the repository root. No program has executed.`);
      // Buffer instantiation also works with static servers whose MIME type
      // for .wasm is application/octet-stream; no streaming-only dependency.
      const { instance } = await WebAssembly.instantiate(await fetched.arrayBuffer(), {});
      session = createWasmSession(instance);
      response.result = { abiVersion: 1 };
    } else {
      if (!session) throw new Error("Connect the Rust Wasm engine before loading a program.");
      if (data.command === "load") response.result = session.load(data.source);
      else if (data.command === "step") response.result = session.step();
      else throw new Error("Unknown engine command.");
    }
  } catch (error) { response.error = error instanceof Error ? error.message : String(error); response.fatal = error instanceof WebAssembly.RuntimeError; }
  self.postMessage(response);
});

// This proxy owns one Worker. Rust compilation and execution never run on the
// UI thread. A timeout terminates the instance, including a non-yielding call.
export async function connectEngine() {
  if (typeof Worker === "undefined" || typeof WebAssembly === "undefined") throw new Error("This browser needs Web Workers and WebAssembly to execute C.");
  const worker = new Worker(new URL("./engine-worker.js", import.meta.url), { type: "module", name: "bcc-rust-engine" });
  const requests = new Map();
  let sequence = 0;
  let connected = true;

  function dispose(reason = "Rust engine stopped.") {
    connected = false;
    worker.terminate();
    for (const request of requests.values()) { clearTimeout(request.timer); request.reject(new Error(reason)); }
    requests.clear();
  }

  function request(command, payload = {}) {
    if (!connected) return Promise.reject(new Error("Rust engine disconnected. Retry engine to start a fresh instance."));
    const id = ++sequence;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => dispose("Rust engine timed out and was terminated. Retry engine to start a fresh instance."), command === "connect" ? 20000 : 10000);
      requests.set(id, { resolve, reject, timer });
      worker.postMessage({ version: 1, id, command, ...payload });
    });
  }

  worker.addEventListener("message", ({ data }) => {
    if (data?.version !== 1) { dispose("The Rust Worker protocol is incompatible. Rebuild the engine and reload the page."); return; }
    const pending = requests.get(data.id);
    if (!pending) return;
    if (data.fatal) { dispose(`Rust Wasm stopped unexpectedly: ${data.error}. Retry engine to start a fresh instance.`); return; }
    clearTimeout(pending.timer);
    requests.delete(data.id);
    if (data.error) pending.reject(new Error(data.error));
    else pending.resolve(data.result);
  });
  worker.addEventListener("error", event => { event.preventDefault(); dispose(event.message || "The Rust engine Worker could not start. Serve web/ over HTTP and rebuild the engine."); });
  worker.addEventListener("messageerror", () => dispose("Unable to decode the Rust Worker response."));

  try { await request("connect"); }
  catch (error) { dispose(); throw error; }
  return {
    load: source => request("load", { source }),
    step: () => request("step"),
    get connected() { return connected; },
    dispose,
  };
}

import assert from 'node:assert/strict';
import { Worker as NodeWorker } from 'node:worker_threads';
import { connectEngine } from './engine.js';
import { demos } from './demos.js';

// Exercise the actual Worker and proxy modules. This shim supplies browser
// messaging and file-backed fetch; it does not simulate engine responses.
const harness = `
import { parentPort, workerData } from 'node:worker_threads';
import { readFile } from 'node:fs/promises';
globalThis.self = {
  addEventListener(type, callback) {
    if (type === 'message') parentPort.on('message', data => callback({ data }));
  },
  postMessage(data) { parentPort.postMessage(data); }
};
globalThis.fetch = async url => {
  try {
    if (workerData.missing) return new Response('missing', { status: 404 });
    return new Response(await readFile(url));
  }
  catch (error) {
    if (error.code === 'ENOENT') return new Response('missing', { status: 404 });
    throw error;
  }
};
await import(workerData.url);
`;
globalThis.Worker = class extends EventTarget {
  constructor(url) {
    super();
    this.worker = new NodeWorker(new URL(`data:text/javascript,${encodeURIComponent(harness)}`), { workerData: { url: url.href, missing: process.argv.includes('--missing') } });
    this.worker.on('message', data => this.dispatchEvent(new MessageEvent('message', { data })));
    this.worker.on('error', error => {
      const event = new Event('error', { cancelable: true });
      Object.defineProperty(event, 'message', { value: error.message });
      this.dispatchEvent(event);
    });
  }
  postMessage(data) { this.worker.postMessage(data); }
  terminate() { void this.worker.terminate(); }
};

if (process.argv.includes('--missing')) {
  await assert.rejects(connectEngine(), /artifact not found \(404\)/);
  console.log('PASS: missing Wasm reaches the proxy as an explicit setup error');
} else {
  const engine = await connectEngine();
  try {
    let state = await engine.load(demos.tour.source);
    assert.equal(state.status, 'ready');
    let count = 0;
    while (['ready', 'running'].includes(state.status)) {
      assert(count++ < 2200);
      state = await engine.step();
    }
    assert.equal(state.status, 'finished', JSON.stringify(state.diagnostic));
    assert.equal(state.returnValue.text, '15');
    state = await engine.load('#include <stdio.h>\nint main(void) { return 0; }');
    assert.equal(state.status, 'error');
    assert.equal(engine.connected, true);
    state = await engine.load('int main(void) { return 42; }');
    while (['ready', 'running'].includes(state.status)) state = await engine.step();
    assert.equal(state.returnValue.text, '42');
    const pending = engine.step();
    engine.dispose();
    await assert.rejects(pending, /stopped/);
    await assert.rejects(engine.step(), /disconnected/);
    console.log('PASS: real Worker compile/step, diagnostics, reload and cancellation');
  } finally { engine.dispose(); }
}

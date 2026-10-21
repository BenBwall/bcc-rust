// Raw ABI v1: Rust owns execution and response JSON. This module only copies
// bytes, validates the contract, and adapts observations for presentation.
export function createWasmSession(instance) {
  const api = instance.exports;
  const exports = ["bcc_abi_version", "bcc_alloc", "bcc_free", "bcc_compile", "bcc_step", "bcc_back", "bcc_reset", "bcc_state", "bcc_response_ptr", "bcc_response_len"];
  if (!(api.memory instanceof WebAssembly.Memory) || exports.some(name => typeof api[name] !== "function")) throw new Error("The Wasm artifact does not expose the bcc ABI. Rebuild with sh web/build.sh.");
  if (api.bcc_abi_version() !== 1) throw new Error("Unsupported bcc Wasm ABI version. Rebuild the engine and reload the page.");
  const encoder = new TextEncoder();
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let sourceBytes = new Uint8Array();

  function response(code) {
    // Always create a fresh view: Rust allocation can grow linear memory and
    // detach every previous memory.buffer view. Decode before any next call.
    const raw = JSON.parse(decoder.decode(new Uint8Array(api.memory.buffer, api.bcc_response_ptr(), api.bcc_response_len())));
    if (code !== 0 || raw.error) throw new Error(raw.error || `Rust Wasm dispatch failed (${code}).`);
    return normalizeState(raw, sourceBytes);
  }

  return {
    load(source) {
      if (typeof source !== "string") throw new Error("Expected C source text.");
      sourceBytes = encoder.encode(source);
      if (sourceBytes.length > 32768) throw new Error("The bounded Rust engine accepts at most 32 KiB of source.");
      const ptr = sourceBytes.length ? api.bcc_alloc(sourceBytes.length) : 0;
      if (sourceBytes.length && !ptr) throw new Error("Rust could not allocate source storage.");
      try {
        if (sourceBytes.length) new Uint8Array(api.memory.buffer, ptr, sourceBytes.length).set(sourceBytes);
        return response(api.bcc_compile(ptr, sourceBytes.length));
      } finally { if (ptr) api.bcc_free(ptr, sourceBytes.length); }
    },
    step: () => response(api.bcc_step()),
    back: () => response(api.bcc_back()),
    reset: () => response(api.bcc_reset()),
    state: () => response(api.bcc_state()),
  };
}

function normalizeState(raw, sourceBytes) {
  const statuses = ["ready", "running", "finished", "undefined_behavior", "error", "limit"];
  if (raw.schema_version !== 1 || raw.engine !== "bounded-c-subset" || !statuses.includes(raw.status) || !Number.isSafeInteger(raw.step) || !Array.isArray(raw.objects) || !Array.isArray(raw.frames) || !Array.isArray(raw.pending_effects) || !Array.isArray(raw.diagnostics)) throw new Error("The Rust snapshot schema is incompatible. Rebuild the engine and reload the page.");
  const frames = new Map(raw.frames.map(frame => [frame.id, frame.function]));
  const objects = new Map(raw.objects.map(object => [object.id, object]));
  const fault = raw.diagnostics[0];
  return {
    step: raw.step,
    status: raw.status,
    location: sourceRange(fault?.source_range ?? raw.source_range, sourceBytes),
    event: raw.event || (raw.status === "ready" ? "Program loaded. No operations have executed." : ""),
    stack: [...raw.frames].reverse().map(frame => ({ depth: frame.depth, name: frame.function, label: `activation #${frame.id}` })),
    objects: raw.objects.map(object => ({ id: object.id, name: object.name, type: object.type, storage: object.storage_duration, alive: object.lifetime === "alive", scope: `${frames.get(object.frame_id) || "ended activation"} · frame #${object.frame_id}`, value: normalizeValue(object.value, objects) })),
    effects: raw.pending_effects,
    sequenceExplanation: "Stores commit atomically. Pending effects describe the next store; this prototype does not model deferred or unsequenced writes. Operand order is left to right.",
    diagnostic: fault ? {
      kind: fault.kind,
      title: ({ undefined_behavior: "Undefined behavior detected", indeterminate: "Indeterminate value read", unsupported: "Outside the supported C subset", syntax: "Source could not be compiled", limit: "Engine resource limit reached" })[fault.kind] || "Execution stopped",
      message: fault.message,
      citation: fault.citation || (fault.kind === "limit" ? "An engine resource limit is a prototype restriction, not undefined behavior." : "This diagnostic describes a prototype restriction; it does not establish undefined behavior."),
      detail: raw.diagnostics.slice(1).map(item => item.message).join("\n"),
    } : undefined,
    returnValue: raw.return_value == null ? undefined : normalizeValue(raw.return_value, objects),
  };
}

function normalizeValue(value, objects) {
  if (!value) throw new Error("The Rust snapshot contains no value for an object.");
  if (value.kind === "indeterminate") return { kind: "indeterminate" };
  if (value.kind === "array") return { kind: "array", elements: value.elements.map(item => normalizeValue(item, objects)) };
  if (value.kind === "pointer") {
    const target = objects.get(value.target);
    const length = target?.value?.kind === "array" ? target.value.elements.length : target ? 1 : undefined;
    return { kind: "pointer", target: value.target, offset: value.offset, onePast: value.offset === length, ended: target?.lifetime === "ended", text: value.target == null ? "null pointer" : undefined };
  }
  if (value.kind === "integer") return { kind: "integer", text: String(value.value) };
  throw new Error(`Unsupported Rust value kind: ${String(value.kind)}`);
}

function sourceRange(range, sourceBytes) {
  if (!range || range.end <= range.start) return undefined;
  const position = offset => {
    const prefix = new TextDecoder().decode(sourceBytes.subarray(0, offset));
    const lines = prefix.split("\n");
    return { line: lines.length, column: lines.at(-1).length + 1 };
  };
  return { start: position(range.start), end: position(range.end) };
}

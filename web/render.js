// Render engine data as text and DOM nodes. Program text never becomes HTML.
export function renderSnapshot(snapshot, source, targetLine) {
  renderSource(source, snapshot?.location, targetLine);
  renderStack(snapshot?.stack ?? []);
  renderObjects(snapshot?.objects ?? []);
  renderEffects(snapshot);
  renderDiagnostic(snapshot?.diagnostic);
  element("step-count").textContent = snapshot ? String(snapshot.step) : "—";
  const status = element("machine-status");
  status.textContent = snapshot ? snapshot.diagnostic?.kind === "indeterminate" ? "Indeterminate read" : statusLabel(snapshot.status) : "Not loaded";
  status.dataset.status = snapshot?.status ?? "idle";
  const location = snapshot?.location?.start;
  element("source-location").textContent = location ? `Ln ${location.line}, Col ${location.column}` : "No current evaluation";
}

export function renderSource(source, range, targetLine) {
  const container = element("code-view");
  const oldScroll = { top: container.scrollTop, left: container.scrollLeft };
  const fragment = document.createDocumentFragment();
  const tokens = syntaxTokens(source);
  let lineOffset = 0;
  for (const [index, text] of source.split("\n").entries()) {
    const line = index + 1;
    const row = node("div", "code-line");
    row.dataset.line = String(line);
    const active = range && line >= range.start.line && line <= range.end.line && !(line === range.end.line && range.end.column === 1 && line !== range.start.line);
    if (active) { row.classList.add("is-current"); row.setAttribute("aria-current", "step"); }
    if (line === targetLine) row.classList.add("is-target");
    const number = node("button", "line-number", String(line));
    number.type = "button";
    number.dataset.line = String(line);
    number.setAttribute("aria-label", `Set run target to line ${line}`);
    number.setAttribute("aria-pressed", String(line === targetLine));
    const code = node("span", "line-code");
    const start = active ? Math.max(0, line === range.start.line ? range.start.column - 1 : 0) : -1;
    const end = active ? Math.min(text.length, line === range.end.line ? range.end.column - 1 : text.length) : -1;
    const boundaries = new Set([0, text.length]);
    if (active) { boundaries.add(Math.min(start, text.length)); boundaries.add(Math.max(0, end)); }
    const lineTokens = tokens.filter(token => token.end > lineOffset && token.start < lineOffset + text.length);
    for (const token of lineTokens) { boundaries.add(Math.max(0, token.start - lineOffset)); boundaries.add(Math.min(text.length, token.end - lineOffset)); }
    const points = [...boundaries].sort((a, b) => a - b);
    for (let part = 0; part + 1 < points.length; part += 1) {
      const from = points[part];
      const to = points[part + 1];
      const token = lineTokens.find(item => item.start <= lineOffset + from && item.end > lineOffset + from);
      const segment = node(active && from >= start && to <= end ? "mark" : "span", token ? `syntax-${token.kind}` : "", text.slice(from, to));
      code.append(segment);
    }
    if (!text.length) code.textContent = " ";
    row.append(number, code);
    fragment.append(row);
    lineOffset += text.length + 1;
  }
  container.replaceChildren(fragment);
  container.scrollTop = oldScroll.top;
  container.scrollLeft = oldScroll.left;
}

export function renderDiagnostic(diagnostic) {
  element("diagnostic").hidden = !diagnostic;
  if (!diagnostic) return;
  element("diagnostic-kind").textContent = ({ undefined_behavior: "UNDEFINED BEHAVIOR · EXECUTION STOPPED", indeterminate: "INDETERMINATE VALUE · POLICY STOP", unsupported: "OUTSIDE THE SUPPORTED SUBSET", syntax: "COMPILATION DIAGNOSTIC", limit: "RESOURCE LIMIT · EXECUTION STOPPED" })[diagnostic.kind] || "PROGRAM DIAGNOSTIC";
  element("diagnostic-title").textContent = diagnostic.title || "Execution stopped";
  element("diagnostic-message").textContent = diagnostic.message || "The engine stopped execution.";
  element("diagnostic-citation").textContent = diagnostic.citation || "No standard citation was supplied by the engine.";
  element("diagnostic-detail").hidden = !diagnostic.detail;
  element("diagnostic-detail").textContent = diagnostic.detail || "";
}

export function element(id) { return document.getElementById(id); }

function renderStack(frames) {
  element("stack-count").textContent = `${frames.length} ${frames.length === 1 ? "frame" : "frames"}`;
  const list = element("call-stack");
  if (!frames.length) { list.replaceChildren(node("li", "empty-state", "No active function calls.")); return; }
  list.replaceChildren(...frames.map((frame, index) => {
    const row = node("li", `stack-frame${index === 0 ? " is-active" : ""}`);
    row.append(node("span", "frame-depth", `#${frame.depth ?? index}`), node("span", "frame-name", `${frame.name}()`), node("span", "frame-meta", frame.location ? `line ${frame.location.line}` : frame.label || ""));
    if (index === 0) row.setAttribute("aria-current", "true");
    return row;
  }));
}

function renderObjects(objects) {
  element("object-count").textContent = `${objects.length} ${objects.length === 1 ? "object" : "objects"}`;
  if (!objects.length) { element("objects").replaceChildren(node("p", "empty-state", "No objects in this state.")); return; }
  const names = new Map(objects.map(object => [String(object.id), object.name]));
  element("objects").replaceChildren(...objects.map(object => {
    const card = node("article", `object-card${object.alive === false ? " is-ended" : ""}`);
    card.id = objectAnchor(object.id);
    card.tabIndex = -1;
    const title = node("div", "object-title");
    title.append(node("h3", "", object.name || "(unnamed)"), node("span", "object-id", `#${object.id}`), node("span", "object-type", object.type || ""));
    const details = node("div", "object-details");
    details.append(node("span", "", `${object.storage || "unspecified"} storage`), node("span", "object-lifetime", object.alive === false ? "Lifetime ended" : object.alive === true ? "Alive" : "Lifetime not reported"));
    if (object.scope) details.append(node("span", "", object.scope));
    const value = node("div", "object-value");
    appendValue(value, object.value, names);
    card.append(title, details, value);
    return card;
  }));
}

function appendValue(container, value, names) {
  if (!value) { container.append(node("span", "muted", "Value not reported")); return; }
  if (value.kind === "indeterminate") { container.append(node("span", "indeterminate", "? indeterminate")); return; }
  if (value.kind === "array") {
    value.elements.forEach((item, index) => { const cell = node("span", "array-item"); cell.append(node("span", "array-index", `[${index}]`)); appendValue(cell, item, names); container.append(cell); });
    return;
  }
  if (value.kind === "pointer" && value.target != null) {
    const offset = value.offset ? ` + ${value.offset}` : "";
    const target = names.get(String(value.target));
    const label = `→ #${value.target}${target ? ` ${target}` : ""}${offset}${value.onePast ? " (one past)" : ""}${value.ended ? " (lifetime ended)" : ""}`;
    if (target) { const link = node("a", "pointer-link", label); link.href = `#${objectAnchor(value.target)}`; container.append(link); }
    else container.append(node("span", "pointer-link", `${label} · not in snapshot`));
    return;
  }
  container.append(node("span", "", value.text ?? "Value not reported"));
}

function renderEffects(snapshot) {
  element("event-description").textContent = snapshot?.event || (snapshot ? "Ready for the next engine evaluation." : "Load the Rust engine to begin stepping.");
  element("sequence-state").textContent = snapshot ? "Atomic stores · prototype" : "Awaiting engine";
  element("sequence-explanation").textContent = snapshot?.sequenceExplanation || "Load a program to inspect the prototype’s sequencing model.";
  element("return-value").hidden = !snapshot?.returnValue;
  element("return-value").textContent = snapshot?.returnValue ? `main returned ${snapshot.returnValue.text ?? snapshot.returnValue.kind}` : "";
  const effects = snapshot?.effects ?? [];
  element("effects-count").textContent = String(effects.length);
  element("effects-list").replaceChildren(...(effects.length ? effects.map(effect => node("li", "", effect)) : [node("li", "muted", snapshot ? "No pending effects." : "No execution state loaded.")]));
}

function syntaxTokens(source) {
  const expression = /\/\/[^\n]*|\/\*[\s\S]*?\*\/|"(?:\\.|[^"\\])*"|'(?:\\.|[^'\\])*'|\b(?:int|void|char|short|long|unsigned|signed|const|volatile|static|extern|auto|register|return|for|while|do|if|else|break|continue|sizeof|struct|union|enum|typedef|switch|case|default)\b|\b(?:0[xX][\da-fA-F]+|\d+(?:\.\d+)?)[uUlLfF]*\b/g;
  return [...source.matchAll(expression)].map(match => ({ start: match.index, end: match.index + match[0].length, kind: match[0].startsWith("/") ? "comment" : /^["']/.test(match[0]) ? "string" : /^\d/.test(match[0]) ? "number" : "keyword" }));
}

function node(tag, className, text) { const result = document.createElement(tag); if (className) result.className = className; if (text != null) result.textContent = String(text); return result; }
function objectAnchor(id) { return `object-${encodeURIComponent(String(id))}`; }
function statusLabel(status) { return ({ ready: "Ready", running: "Paused", finished: "Finished", undefined_behavior: "Undefined behavior", error: "Stopped", limit: "Resource limit" })[status] || status; }

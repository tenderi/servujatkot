// Applies re-rendered overlay HTML pushed by the server. Only nodes and
// attributes that differ are touched, so images aren't reloaded and a slot's
// pop-in animation runs once, when its hero arrives. The fragment's shape is
// fixed (same slots every time), so matching children by position is enough.
// EventSource reconnects on its own if servujatkot restarts.

function morph(from, to) {
  if (from.nodeType !== to.nodeType || from.nodeName !== to.nodeName) {
    from.replaceWith(to.cloneNode(true));
    return;
  }
  if (from.nodeType === Node.TEXT_NODE) {
    if (from.nodeValue !== to.nodeValue) from.nodeValue = to.nodeValue;
    return;
  }
  if (from.nodeType !== Node.ELEMENT_NODE) return;
  for (const a of [...from.attributes]) {
    if (!to.hasAttribute(a.name)) from.removeAttribute(a.name);
  }
  for (const a of [...to.attributes]) {
    if (from.getAttribute(a.name) !== a.value) from.setAttribute(a.name, a.value);
  }
  const old = [...from.childNodes];
  const next = [...to.childNodes];
  next.forEach((n, i) => (i < old.length ? morph(old[i], n) : from.appendChild(n.cloneNode(true))));
  old.slice(next.length).forEach((n) => n.remove());
}

function apply(root, html) {
  const t = document.createElement("template");
  t.innerHTML = html;
  const next = [...t.content.childNodes];
  const old = [...root.childNodes];
  next.forEach((n, i) => (i < old.length ? morph(old[i], n) : root.appendChild(n)));
  old.slice(next.length).forEach((n) => n.remove());
}

const root = document.getElementById("root");
if (root && root.dataset.events) {
  new EventSource(root.dataset.events).onmessage = (e) => apply(root, e.data);
}

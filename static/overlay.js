// Swaps in re-rendered overlay HTML pushed by the server. EventSource
// reconnects on its own if servujatkot restarts.
const root = document.getElementById("root");
new EventSource(root.dataset.events).onmessage = (e) => {
  root.innerHTML = e.data;
};

const { activateFonts, request } = require("./client.js");
const host = require("./host.js");

const status = document.getElementById("status");
const auto = document.getElementById("auto");
auto.checked = localStorage.getItem("typeshelf.auto") !== "off";
auto.addEventListener("change", () => localStorage.setItem("typeshelf.auto", auto.checked ? "on" : "off"));

async function run() {
  try {
    const fonts = host.documentFonts();
    const r = await activateFonts(fonts);
    const parts = [`Activated ${r.activated} font${r.activated === 1 ? "" : "s"}`];
    if (r.alreadyActive) parts.push(`${r.alreadyActive} already active`);
    if (r.missing.length) parts.push(`not in your library: ${r.missing.join(", ")}`);
    status.textContent = parts.join("\n");
  } catch (e) {
    status.textContent = e.message;
  }
}

document.getElementById("now").addEventListener("click", run);
host.onDocumentOpened(() => {
  if (auto.checked) run();
});
request({ type: "ping" })
  .then((r) => (status.textContent = `Connected to typeshelf ${r.version}`))
  .catch((e) => (status.textContent = e.message));

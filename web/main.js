const status = document.querySelector("#status");
const seedInput = document.querySelector("#seed");
const input = document.querySelector("#input");
const output = document.querySelector("#output");
const renderButton = document.querySelector("#render");

let wasm = null;

async function loadWasm() {
  if (wasm) return wasm;
  const module = await import("../pkg/ffx_ctb_rust.js");
  await module.default();
  wasm = module;
  status.textContent = "WASM loaded";
  return wasm;
}

renderButton.addEventListener("click", async () => {
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const rendered = JSON.parse(module.render_ctb_json(seed, input.value));
    output.textContent = JSON.stringify(rendered, null, 2);
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
});

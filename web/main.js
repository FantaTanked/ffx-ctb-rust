const status = document.querySelector("#status");
const appShell = document.querySelector(".app-shell");
const modeTabs = [...document.querySelectorAll(".mode-tab")];
const modePanels = [...document.querySelectorAll("[data-mode-panel]")];
const seedInput = document.querySelector("#seed");
const input = document.querySelector("#input");
const output = document.querySelector("#output");
const inputFind = document.querySelector("#inputFind");
const inputFindNext = document.querySelector("#inputFindNext");
const outputFind = document.querySelector("#outputFind");
const outputFindNext = document.querySelector("#outputFindNext");
const sampleButton = document.querySelector("#sample");
const openInputButton = document.querySelector("#openInput");
const saveInputButton = document.querySelector("#saveInput");
const saveOutputButton = document.querySelector("#saveOutput");
const fileInput = document.querySelector("#fileInput");
const party = document.querySelector("#party");
const chocobo = document.querySelector("#chocobo");
const tanker = document.querySelector("#tanker");
const prevEncounterButton = document.querySelector("#prevEncounter");
const nextEncounterButton = document.querySelector("#nextEncounter");
const encounterSelect = document.querySelector("#encounterSelect");
const currentEncounter = document.querySelector("#currentEncounter");
const trackerPanes = {
  drops: {
    input: document.querySelector("#dropsInput"),
    output: document.querySelector("#dropsOutput"),
    open: document.querySelector("#openDrops"),
    save: document.querySelector("#saveDrops"),
    load: document.querySelector("#loadDrops"),
    noEncounters: document.querySelector("#searchNoEncounters"),
  },
  encounters: {
    input: document.querySelector("#encountersTrackerInput"),
    output: document.querySelector("#encountersTrackerOutput"),
    open: document.querySelector("#openEncountersTracker"),
    saveCsv: document.querySelector("#saveEncountersCsv"),
    load: document.querySelector("#loadEncountersTracker"),
    sliders: document.querySelector("#encounterSliders"),
    sliderData: [],
  },
};

const APP_BUILD_ID = "ctb-tracker-render-20260501-266";
const WORKSPACE_STORAGE_KEY = "ffxCtbRustWorkspace.v1";
const AUTO_RENDER_DELAY_MS = 450;
let lastRendered = null;
let lastRenderedInput = null;
let tankerPatternValue = "awsdn-";
let activeModes = new Set(["ctb"]);
let ctbRenderTimer = null;
let cursorUiTimer = null;
let ctbRenderRevision = 0;
let pendingFileTarget = "ctb";
let outputFindLineIndex = -1;

let wasm = null;

async function loadWasm() {
  if (wasm) return wasm;
  const module = await import(`../pkg/ffx_ctb_rust.js?v=${APP_BUILD_ID}`);
  await module.default(`../pkg/ffx_ctb_rust_bg.wasm?v=${APP_BUILD_ID}`);
  wasm = module;
  status.textContent = "WASM loaded";
  return wasm;
}

sampleButton.addEventListener("click", loadSample);

async function loadSample() {
  try {
    const module = await loadWasm();
    const sample = JSON.parse(module.sample_json());
    seedInput.value = sample.seed;
    input.value = sample.input || "";
    moveCursorToFirstEncounter();
    saveWorkspaceCache();
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

input.addEventListener("keyup", updateCursorUi);
input.addEventListener("click", updateCursorUi);
input.addEventListener("input", () => {
  saveWorkspaceCache();
  scheduleCursorUi();
  scheduleCtbRender();
});
inputFind.addEventListener("input", updateInputFindState);
inputFind.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    findNextInputMatch();
  }
});
inputFindNext.addEventListener("click", findNextInputMatch);
outputFind.addEventListener("input", updateOutputFindState);
outputFind.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    findNextOutputMatch();
  }
});
outputFindNext.addEventListener("click", findNextOutputMatch);
seedInput.addEventListener("input", () => {
  saveWorkspaceCache();
  scheduleVisibleRenders();
});

openInputButton.addEventListener("click", () => openTextFileFor("ctb"));
fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;
  await loadTextFileIntoTarget(pendingFileTarget, await file.text());
  fileInput.value = "";
});
saveInputButton.addEventListener("click", () => downloadText("ctb_actions_input.txt", input.value));
saveOutputButton.addEventListener("click", () => downloadText("ctb_output.txt", output.textContent || ""));
prevEncounterButton.addEventListener("click", () => jumpRelativeEncounter(-1));
nextEncounterButton.addEventListener("click", () => jumpRelativeEncounter(1));
encounterSelect.addEventListener("change", () => {
  const list = encounterList();
  const position = Number.parseInt(encounterSelect.value, 10);
  if (Number.isFinite(position) && list[position]) jumpToEncounter(list[position], position);
});
Object.entries(trackerPanes).forEach(([tracker, pane]) => {
  pane.open?.addEventListener("click", () => openTextFileFor(tracker));
  pane.save?.addEventListener("click", () => saveTrackerInput(tracker));
  pane.saveCsv?.addEventListener("click", () => saveEncountersCsv());
  pane.load.addEventListener("click", () => loadTrackerDefault(tracker));
  pane.input.addEventListener("input", () => {
    saveWorkspaceCache();
    scheduleTrackerRender(tracker);
  });
});
trackerPanes.drops.noEncounters?.addEventListener("click", searchNoEncountersRoutes);
modeTabs.forEach((tab) => {
  tab.addEventListener("click", () => toggleMode(tab.dataset.mode || "ctb"));
});
setupResizeHandles();
updateVisibleModes();

function toggleMode(mode) {
  if (activeModes.has(mode)) {
    activeModes.delete(mode);
  } else {
    activeModes.add(mode);
  }
  if (!activeModes.size) activeModes.add(mode);
  updateVisibleModes();
  saveWorkspaceCache();
  scheduleVisibleRenders();
}

function updateVisibleModes() {
  const visiblePanels = [];
  modePanels.forEach((panel) => {
    const visible = activeModes.has(panel.dataset.modePanel);
    panel.classList.toggle("is-visible", visible);
    panel.classList.remove("visible-index-1", "visible-index-2", "visible-index-3");
    if (visible) visiblePanels.push(panel);
  });
  visiblePanels.forEach((panel, index) => {
    panel.classList.add(`visible-index-${index + 1}`);
  });
  appShell.dataset.visibleCount = String(visiblePanels.length || 1);
  appShell.dataset.ctbVisible = activeModes.has("ctb") ? "true" : "false";
  appShell.dataset.trackerPair = activeModes.has("drops") && activeModes.has("encounters") ? "true" : "false";
  modeTabs.forEach((tab) => {
    const active = activeModes.has(tab.dataset.mode);
    tab.classList.toggle("is-active", active);
    tab.setAttribute("aria-pressed", active ? "true" : "false");
  });
}

function scheduleVisibleRenders() {
  if (activeModes.has("ctb")) scheduleCtbRender();
  if (activeModes.has("drops")) scheduleTrackerRender("drops");
  if (activeModes.has("encounters")) scheduleTrackerRender("encounters");
}

function scheduleCtbRender() {
  ctbRenderRevision += 1;
  const revision = ctbRenderRevision;
  clearTimeout(ctbRenderTimer);
  ctbRenderTimer = setTimeout(async () => {
    if (revision !== ctbRenderRevision) return;
    await renderCurrentInput();
  }, AUTO_RENDER_DELAY_MS);
}

function scheduleCursorUi() {
  clearTimeout(cursorUiTimer);
  cursorUiTimer = setTimeout(updateCursorUi, 120);
}

function scheduleTrackerRender(tracker) {
  const pane = trackerPanes[tracker];
  pane.renderRevision = (pane.renderRevision || 0) + 1;
  const revision = pane.renderRevision;
  clearTimeout(pane.renderTimer);
  pane.renderTimer = setTimeout(async () => {
    if (revision !== pane.renderRevision) return;
    await renderTracker(tracker);
  }, AUTO_RENDER_DELAY_MS);
}

async function renderCurrentInput() {
  ctbRenderRevision += 1;
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const rawRendered = lastRenderedInput === null
      ? module.render_ctb_json(seed, input.value)
      : module.render_ctb_diff_json(seed, input.value, lastRenderedInput);
    const rendered = JSON.parse(rawRendered);
    lastRendered = rendered;
    lastRenderedInput = input.value;
    renderOutputText(output, rendered.output || "", "ctb");
    const partyPayload = renderParty(module, seed);
    renderChocoboTools(module, seed, rendered, partyPayload);
    renderTankerTools(module, rendered);
    updateEncounterControls(rendered.encounters || []);
    status.textContent = rendered.message;
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function loadTrackerDefault(tracker) {
  const pane = trackerPanes[tracker];
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const payload = JSON.parse(module.tracker_default_json(tracker, seed));
    pane.input.value = payload.input || "";
    renderOutputText(pane.output, "", tracker);
    if (tracker === "encounters") {
      pane.sliderData = Array.isArray(payload.sliders) ? payload.sliders : [];
      renderEncounterSliderControls(pane);
    }
    saveWorkspaceCache();
    await renderTracker(tracker);
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function renderTracker(tracker) {
  const pane = trackerPanes[tracker];
  pane.renderRevision = (pane.renderRevision || 0) + 1;
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const payload = JSON.parse(module.tracker_render_json(tracker, seed, pane.input.value));
    pane.lastRenderedInput = pane.input.value;
    renderOutputText(pane.output, payload.output || "", tracker);
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function searchNoEncountersRoutes() {
  const pane = trackerPanes.drops;
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const startLine = textareaCursorLine(pane.input);
    const encountersPane = trackerPanes.encounters;
    const encountersOutput = encountersPane.lastRenderedInput === encountersPane.input.value
      ? encountersPane.output.textContent || null
      : null;
    const payload = JSON.parse(module.no_encounters_routes_json(
      seed,
      pane.input.value,
      startLine,
      encountersPane.input.value || null,
      encountersOutput,
    ));
    if (typeof payload.edited_input === "string" && payload.edited_input !== pane.input.value) {
      pane.input.value = payload.edited_input;
      saveWorkspaceCache();
    }
    renderOutputText(pane.output, payload.output || "", "drops");
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

function textareaCursorLine(textarea) {
  return textarea.value.slice(0, textarea.selectionStart || 0).split("\n").length;
}

function updateInputFindState() {
  const query = inputFind.value;
  inputFindNext.disabled = !query || !input.value.toLowerCase().includes(query.toLowerCase());
}

function findNextInputMatch() {
  const query = inputFind.value;
  if (!query) {
    updateInputFindState();
    return;
  }
  const haystack = input.value.toLowerCase();
  const needle = query.toLowerCase();
  let index = haystack.indexOf(needle, input.selectionEnd || 0);
  if (index < 0) index = haystack.indexOf(needle);
  inputFindNext.disabled = index < 0;
  if (index < 0) return;
  input.focus();
  input.setSelectionRange(index, index + query.length);
  scrollTextareaOffsetIntoView(input, index);
  scheduleCursorUi();
}

function scrollTextareaOffsetIntoView(textarea, offset) {
  const lineNumber = textarea.value.slice(0, offset).split("\n").length;
  const style = window.getComputedStyle(textarea);
  const fontSize = Number.parseFloat(style.fontSize) || 13;
  const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.4;
  const targetTop = Math.max(0, (lineNumber - 2) * lineHeight);
  if (targetTop < textarea.scrollTop || targetTop > textarea.scrollTop + textarea.clientHeight - lineHeight * 2) {
    textarea.scrollTop = targetTop;
  }
}

function updateOutputFindState(options = {}) {
  const query = outputFind.value;
  const lines = outputRawLines();
  const hasMatch = Boolean(query) && lines.some((line) => line.toLowerCase().includes(query.toLowerCase()));
  outputFindNext.disabled = !hasMatch;
  if (!hasMatch) {
    outputFindLineIndex = -1;
    clearActiveOutputFindLine();
    return;
  }
  if (options.preserveLine && outputFindLineIndex >= 0) {
    setActiveOutputFindLine(outputFindLineIndex, { scroll: false });
  }
}

function findNextOutputMatch() {
  const query = outputFind.value;
  if (!query) {
    updateOutputFindState();
    return;
  }
  const lines = outputRawLines();
  const needle = query.toLowerCase();
  const start = outputFindLineIndex + 1;
  let index = lines.findIndex((line, lineIndex) => lineIndex >= start && line.toLowerCase().includes(needle));
  if (index < 0) index = lines.findIndex((line) => line.toLowerCase().includes(needle));
  outputFindNext.disabled = index < 0;
  if (index < 0) return;
  setActiveOutputFindLine(index, { scroll: true });
}

function outputRawLines() {
  return (output.dataset.rawText || output.textContent || "").split(/\r?\n/);
}

function clearActiveOutputFindLine() {
  output.querySelector(".output-search-active")?.classList.remove("output-search-active");
}

function setActiveOutputFindLine(index, options = {}) {
  clearActiveOutputFindLine();
  outputFindLineIndex = index;
  const line = output.querySelector(`[data-output-line="${index}"]`);
  if (!line) return;
  line.classList.add("output-search-active");
  if (options.scroll) {
    output.scrollTop = Math.max(0, line.offsetTop - output.clientHeight * 0.25);
  }
}

function openTextFileFor(target) {
  pendingFileTarget = target;
  fileInput.click();
}

async function loadTextFileIntoTarget(target, text) {
  if (target === "drops" || target === "encounters") {
    const pane = trackerPanes[target];
    if (target === "encounters") {
      const loadedCsv = await loadEncountersCsvIfPresent(text);
      if (!loadedCsv) {
        pane.input.value = text;
        await hydrateEncounterSlidersFromDefaults();
        syncEncounterSliderControlsToInput(pane);
      }
    } else {
      pane.input.value = text;
    }
    saveWorkspaceCache();
    await renderTracker(target);
    return;
  }

  input.value = text;
  moveCursorToFirstEncounter();
  saveWorkspaceCache();
  await renderCurrentInput();
}

function saveTrackerInput(tracker) {
  const pane = trackerPanes[tracker];
  const filename = tracker === "encounters" ? "encounters_input.txt" : "drops_input.txt";
  downloadText(filename, pane.input.value);
}

async function loadEncountersCsvIfPresent(text) {
  if (!looksLikeEncountersCsv(text)) return false;
  const pane = trackerPanes.encounters;
  const sliders = parseEncounterSliderCsv(text);
  if (!sliders.length) return false;
  pane.sliderData = sliders;
  renderEncounterSliderControls(pane);
  pane.input.value = buildEncountersInputFromControls(pane);
  return true;
}

function looksLikeEncountersCsv(text) {
  const firstDataLine = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line && !line.startsWith("#"));
  if (!firstDataLine) return false;
  const fields = parseCsvLine(firstDataLine);
  return fields.length >= 6 && fields[1] && fields[3] !== undefined && fields[4] !== undefined && fields[5] !== undefined;
}

function parseEncounterSliderCsv(text) {
  return text.split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line && !line.startsWith("#"))
    .map((line, index) => {
      const fields = parseCsvLine(line);
      const name = fields[0] || "";
      const label = fields[2] || name;
      return {
        index,
        name,
        initiative: /^true$/i.test(fields[1] || ""),
        label,
        min: parseIntegerField(fields[3], 0),
        default: parseIntegerField(fields[4], 0),
        max: parseIntegerField(fields[5], 0),
      };
    })
    .filter((slider) => slider.name);
}

function parseCsvLine(line) {
  const fields = [];
  let field = "";
  let quoted = false;
  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (character === '"') {
      if (quoted && line[index + 1] === '"') {
        field += '"';
        index += 1;
      } else {
        quoted = !quoted;
      }
    } else if (character === "," && !quoted) {
      fields.push(field.trim());
      field = "";
    } else {
      field += character;
    }
  }
  fields.push(field.trim());
  return fields;
}

function parseIntegerField(value, fallback) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function saveEncountersCsv() {
  const pane = trackerPanes.encounters;
  if (!pane.sliderData.length) return;
  const counts = currentEncounterSliderCounts(pane);
  const lines = ["#name or zone,initiative (true or false),label (optional),min,default,max"];
  pane.sliderData.forEach((slider) => {
    const value = counts.get(slider.index) ?? slider.default;
    lines.push([
      csvEscape(slider.name),
      slider.initiative ? "true" : "false",
      slider.label === slider.name ? "" : csvEscape(slider.label),
      slider.min,
      value,
      slider.max,
    ].join(","));
  });
  downloadText("encounters_notes.csv", `${lines.join("\n")}\n`, "text/csv;charset=utf-8");
}

function currentEncounterSliderCounts(pane) {
  const counts = new Map();
  pane.sliders.querySelectorAll("input[type='range']").forEach((slider) => {
    counts.set(Number.parseInt(slider.dataset.index, 10), Number.parseInt(slider.value, 10));
  });
  return counts;
}

function csvEscape(value) {
  const text = String(value ?? "");
  return /[",\r\n]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text;
}

const ALLY_OUTPUT_ACTORS = new Set([
  "tidus", "yuna", "auron", "kimahri", "wakka", "lulu", "rikku",
  "valefor", "ifrit", "ixion", "shiva", "bahamut", "yojimbo",
  "cindy", "sandy", "mindy", "unknown",
]);

const NAMED_ENEMY_OUTPUT_ACTORS = new Set(["anima", "seymour"]);

function renderOutputText(target, text, kind) {
  target.dataset.rawText = text;
  if (!text) {
    target.textContent = "";
    if (target === output) updateOutputFindState();
    return;
  }
  target.innerHTML = text
    .split("\n")
    .map((line, index) => formatOutputLine(line, kind, index))
    .join("\n");
  if (target === output) updateOutputFindState({ preserveLine: true });
}

function formatOutputLine(line, kind, index) {
  const className = outputLineClass(line, kind);
  const highlighted = highlightOutputTokens(escapeHtml(line));
  return `<span class="${className}" data-output-line="${index}">${highlighted}</span>`;
}

function outputLineClass(line, kind) {
  const trimmed = line.trim();
  const lower = trimmed.toLowerCase();
  const firstToken = lower.split(/\s+/, 1)[0] || "";

  if (!trimmed) return "output-line output-blank";
  if (/^=+$/.test(trimmed)) return "output-line output-separator";
  if (lower.startsWith("error:")) return "output-line output-error";
  if (lower.startsWith("warning:")) return "output-line output-warning";
  if (/^#=+.*=+$/.test(trimmed)) return "output-line output-section";
  if (isEncounterOutputLine(trimmed, kind)) return "output-line output-encounter";
  if (kind === "ctb") {
    if (lower.startsWith("# party rolls:")) return "output-line output-party-roll-line";
    if (
      lower.startsWith("# enemy rolls:")
      || /^m\d+\b/.test(lower)
      || lower.startsWith("spawn ")
      || NAMED_ENEMY_OUTPUT_ACTORS.has(firstToken)
    ) {
      return "output-line output-enemy";
    }
    if (ALLY_OUTPUT_ACTORS.has(firstToken)) return "output-line output-ally";
  }
  if (lower.startsWith("#")) return "output-line output-comment";
  if (kind === "ctb" && isDropsResultLine(lower)) return "output-line output-drop";
  if (lower.startsWith("no encounters search:") || lower.startsWith("route status:")) {
    return "output-line output-search";
  }
  return "output-line";
}

function isEncounterOutputLine(trimmed, kind) {
  if (/^(?:#\s*)?(?:random encounter:|simulated encounter:|multizone encounter:|encounter:)\s*\d+/i.test(trimmed)) {
    return true;
  }
  if (kind !== "encounters") return false;
  return /^\d+\s*\|/.test(trimmed) || /^\d+\s+\d+\s+\d+\s*\|/.test(trimmed);
}

function isDropsResultLine(lower) {
  return lower.includes("drop") || lower.includes("steal") || lower.includes("equipment") || lower.includes("sphere");
}

function highlightOutputTokens(html) {
  if (html.startsWith("# party rolls:")) {
    return highlightRollComment(html, "party");
  }
  if (html.startsWith("# enemy rolls:")) {
    return highlightRollComment(html, "enemy");
  }
  return html
    .replace(/\(Crit\)/g, '<strong class="output-crit">(Crit)</strong>')
    .replace(/\[CRIT\]/g, '<strong class="output-crit">[CRIT]</strong>')
    .replace(/\[(?:[^\]\n]+)\]/g, (token) => (
      token === "[CRIT]" ? token : `<span class="output-status-token">${token}</span>`
    ))
    .replace(/\bNo Encounters\b/g, '<strong class="output-nea">No Encounters</strong>');
}

function highlightRollComment(html, actorSide) {
  const prefix = actorSide === "party" ? "# party rolls: " : "# enemy rolls: ";
  const prefixClass = actorSide === "party" ? "output-party-roll-prefix" : "output-enemy-roll-prefix";
  const targetClass = actorSide === "party" ? "output-party-roll-target" : "output-enemy-roll-target";
  const tailClass = actorSide === "party" ? "output-enemy-health" : "output-player-health";
  return `<span class="${prefixClass}">${prefix}</span>` + html.slice(prefix.length).split(" | ").map((part) => {
    const match = part.match(/^([^:\n]+: )(\[[^\]\n]+\]\s+[^-\n]+?)(\s+-&gt;\s+.*)$/);
    if (!match) return highlightOutputTokens(part);
    return `<span class="${targetClass}">${match[1]}</span>`
      + `<span class="output-damage-roll">${highlightDamageRollTokens(match[2])}</span>`
      + `<span class="${tailClass}">${match[3]}</span>`;
  }).join(" | ");
}

function highlightDamageRollTokens(html) {
  return html
    .replace(/\(Crit\)/g, '<strong class="output-crit">(Crit)</strong>')
    .replace(/\[CRIT\]/g, '<strong class="output-crit">[CRIT]</strong>');
}

function escapeHtml(text) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function renderEncounterSliderControls(pane) {
  const sliders = pane.sliderData || [];
  if (!sliders.length) {
    pane.sliders.replaceChildren();
    return;
  }
  const controls = sliders
    .filter((slider) => slider.min !== slider.max)
    .map((slider) => {
      const label = document.createElement("label");
      label.className = "encounter-slider";
      const name = document.createElement("span");
      name.textContent = slider.label || slider.name;
      const value = document.createElement("output");
      value.textContent = String(slider.default);
      const range = document.createElement("input");
      range.type = "range";
      range.min = String(slider.min);
      range.max = String(slider.max);
      range.value = String(slider.default);
      range.dataset.index = String(slider.index);
      range.addEventListener("input", () => {
        value.textContent = range.value;
        pane.input.value = buildEncountersInputFromControls(pane);
        saveWorkspaceCache();
        scheduleTrackerRender("encounters");
      });
      label.replaceChildren(name, range, value);
      return label;
    });
  pane.sliders.replaceChildren(...controls);
}

async function hydrateEncounterSlidersFromDefaults() {
  const pane = trackerPanes.encounters;
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const payload = JSON.parse(module.tracker_default_json("encounters", seed));
    pane.sliderData = Array.isArray(payload.sliders) ? payload.sliders : [];
    renderEncounterSliderControls(pane);
    syncEncounterSliderControlsToInput(pane);
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

function syncEncounterSliderControlsToInput(pane) {
  const lines = pane.input.value.split(/\r?\n/).map((line) => line.trim().toLowerCase());
  const counts = new Map();
  pane.sliderData.forEach((slider) => {
    const expected = encounterInputLine(slider.name).toLowerCase();
    const count = lines.filter((line) => line === expected).length;
    counts.set(slider.index, Math.min(Math.max(count, slider.min), slider.max));
  });
  pane.sliders.querySelectorAll("input[type='range']").forEach((range) => {
    const index = Number.parseInt(range.dataset.index, 10);
    const value = counts.get(index);
    if (value === undefined) return;
    range.value = String(value);
    const output = range.closest(".encounter-slider")?.querySelector("output");
    if (output) output.textContent = String(value);
  });
}

function buildEncountersInputFromControls(pane) {
  const counts = new Map(
    [...pane.sliders.querySelectorAll("input[type='range']")]
      .map((slider) => [Number.parseInt(slider.dataset.index, 10), Number.parseInt(slider.value, 10)])
  );
  const lines = ["/nopadding", "/usage"];
  let initiativeEquipped = false;
  pane.sliderData.forEach((slider) => {
    const count = counts.has(slider.index) ? counts.get(slider.index) : slider.default;
    if (slider.initiative && !initiativeEquipped) {
      lines.push("weapon tidus 1 initiative");
      initiativeEquipped = true;
    } else if (!slider.initiative && initiativeEquipped) {
      lines.push("weapon tidus 1");
      initiativeEquipped = false;
    }
    const encounterLine = encounterInputLine(slider.name);
    if (slider.min !== slider.max) {
      lines.push("", `#    ${slider.label || slider.name}`);
    }
    for (let index = 0; index < count; index += 1) {
      lines.push(encounterLine);
    }
    for (let index = count; index < slider.max; index += 1) {
      lines.push(`# ${encounterLine}`);
    }
  });
  return `${lines.join("\n")}\n`;
}

function encounterInputLine(name) {
  return `encounter ${name.includes(" ") ? "multizone " : ""}${name}`;
}

async function updatePartyAtCursor() {
  if (!wasm) return;
  const seed = Number.parseInt(seedInput.value, 10) >>> 0;
  const partyPayload = renderParty(wasm, seed);
  if (lastRendered) {
    const encounters = encounterList();
    const currentRendered = { ...lastRendered, encounters };
    renderChocoboTools(wasm, seed, currentRendered, partyPayload);
    renderTankerTools(wasm, currentRendered);
    updateEncounterControls(encounters);
  }
}

async function updateCursorUi() {
  await updatePartyAtCursor();
  updateEncounterControls(encounterList());
}

function renderParty(module, seed) {
  const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
  const payload = JSON.parse(module.party_json(seed, input.value, cursorLine));
  const partyText = payload.party.map((character) => character.name).join(", ") || "None";
  const reserveText = payload.reserves.map((character) => character.name).join(", ") || "None";
  party.textContent = `Party at line ${cursorLine}: ${partyText} | Reserves: ${reserveText}`;
  return payload;
}

function renderChocoboTools(module, seed, rendered, partyPayload) {
  const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
  const encounter = currentEncounterAtLine(rendered.encounters || [], cursorLine);
  if (!encounter || encounter.name !== "chocobo_eater") {
    chocobo.replaceChildren();
    return;
  }

  const buttons = [];
  partyPayload.party.forEach((_, index) => {
    buttons.push(chocoboButton(`Attack Slot ${index + 1}`, () => applyChocoboAction(module, seed, "attack_slot", index)));
  });
  buttons.push(chocoboButton("Generic Attack", () => applyChocoboAction(module, seed, "generic_attack", null)));
  buttons.push(chocoboButton("Fists Of Fury", () => applyChocoboAction(module, seed, "fists_of_fury", null)));
  buttons.push(chocoboButton("Thwack", () => applyChocoboAction(module, seed, "thwack", null)));
  const swapControls = buildChocoboSwapControls(module, seed, partyPayload);
  chocobo.replaceChildren(...buttons, swapControls);
}

function renderTankerTools(module, rendered) {
  const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
  const encounter = currentEncounterAtLine(rendered.encounters || [], cursorLine);
  if (!encounter || !["tanker", "tros", "garuda_1", "garuda_2", "lancet_tutorial"].includes(encounter.name)) {
    tanker.replaceChildren();
    return;
  }

  if (encounter.name === "garuda_2") {
    const label = document.createElement("span");
    label.textContent = "Garuda 2 Attack";
    const attackButton = chocoboButton("Attack", () => applyGaruda2Attack(module, "attack"));
    const sonicButton = chocoboButton("Sonic Boom", () => applyGaruda2Attack(module, "sonic_boom"));
    const nothingButton = chocoboButton("Does Nothing", () => applyGaruda2Attack(module, "does_nothing"));
    tanker.replaceChildren(label, attackButton, sonicButton, nothingButton);
    return;
  }

  if (encounter.name === "lancet_tutorial") {
    const label = document.createElement("span");
    label.textContent = "Lancet Tutorial";
    const beforeButton = chocoboButton("Before Lancet", () => applyLancetTutorialTiming(module, "before"));
    const afterButton = chocoboButton("After Lancet", () => applyLancetTutorialTiming(module, "after"));
    tanker.replaceChildren(label, beforeButton, afterButton);
    return;
  }

  if (encounter.name === "garuda_1") {
    const label = document.createElement("span");
    label.textContent = "Garuda 1 Attacks";
    const selects = Array.from({ length: 5 }, (_, index) => {
      const select = document.createElement("select");
      select.setAttribute("aria-label", `Garuda 1 attack ${index + 1}`);
      [["attack", "Attack"], ["sonic_boom", "Sonic Boom"]].forEach(([value, text]) => {
        const option = document.createElement("option");
        option.value = value;
        option.textContent = text;
        select.append(option);
      });
      return select;
    });
    const button = chocoboButton("Overwrite", () => applyGaruda1Attacks(module, selects.map((select) => select.value)));
    tanker.replaceChildren(label, ...selects, button);
    return;
  }

  if (encounter.name === "tros") {
    const label = document.createElement("span");
    label.textContent = "Tros First Attack";
    const attackButton = chocoboButton("Attack", () => applyTrosAttack(module, "attack"));
    const tentaclesButton = chocoboButton("Tentacles", () => applyTrosAttack(module, "tentacles"));
    tanker.replaceChildren(label, attackButton, tentaclesButton);
    return;
  }

  const label = document.createElement("span");
  label.textContent = "Tanker Pattern";
  const patternInput = document.createElement("input");
  patternInput.type = "text";
  patternInput.value = tankerPatternValue;
  patternInput.placeholder = "awsdn-";
  patternInput.addEventListener("input", () => {
    tankerPatternValue = patternInput.value;
  });
  patternInput.addEventListener("keydown", (event) => {
    if (event.key === "Enter") applyTankerPattern(module, patternInput.value);
  });
  const button = chocoboButton("Overwrite", () => applyTankerPattern(module, patternInput.value));
  tanker.replaceChildren(label, patternInput, button);
}

function chocoboButton(label, onClick) {
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = label;
  button.addEventListener("click", onClick);
  return button;
}

function buildChocoboSwapControls(module, seed, partyPayload) {
  const group = document.createElement("div");
  group.className = "swap-controls";
  if (!partyPayload.party.length || !partyPayload.reserves.length) return group;

  const slotSelect = document.createElement("select");
  partyPayload.party.forEach((character, index) => {
    const option = document.createElement("option");
    option.value = String(index);
    option.textContent = `${index + 1}. ${character.name}`;
    slotSelect.append(option);
  });

  const replacementSelect = document.createElement("select");
  partyPayload.reserves.forEach((character) => {
    const option = document.createElement("option");
    option.value = character.input_name;
    option.textContent = character.name;
    replacementSelect.append(option);
  });

  const swapButton = chocoboButton("Swap", () => (
    applyChocoboSwap(module, seed, Number.parseInt(slotSelect.value, 10), replacementSelect.value)
  ));
  group.replaceChildren(slotSelect, replacementSelect, swapButton);
  return group;
}

function currentEncounterAtLine(encounterList, cursorLine) {
  for (let index = encounterList.length - 1; index >= 0; index -= 1) {
    const encounter = encounterList[index];
    if (cursorLine >= encounter.start_line && cursorLine <= encounter.end_line) return encounter;
  }
  return null;
}

function encounterList() {
  return lastRenderedInput === input.value && lastRendered?.encounters?.length
    ? lastRendered.encounters
    : scanEncounters(input.value);
}

function scanEncounters(text) {
  const lines = text.split(/\r?\n/);
  const scanned = [];
  let currentName = null;
  let currentStart = null;
  let index = 0;
  let inBlockComment = false;
  lines.forEach((line, zeroIndex) => {
    const stripped = line.trim();
    if (stripped.startsWith("/*")) {
      if (!stripped.endsWith("*/")) inBlockComment = true;
      return;
    }
    if (inBlockComment) {
      if (stripped.endsWith("*/")) inBlockComment = false;
      return;
    }
    if (!stripped.toLowerCase().startsWith("encounter ")) return;
    const words = stripped.split(/\s+/);
    const lineNumber = zeroIndex + 1;
    if (currentName !== null && currentStart !== null) {
      scanned.push({ index, name: currentName, start_line: currentStart, end_line: lineNumber - 1 });
    }
    currentName = words[1] || "unknown";
    currentStart = lineNumber;
    index += 1;
  });
  if (currentName !== null && currentStart !== null) {
    scanned.push({ index, name: currentName, start_line: currentStart, end_line: Math.max(lines.length, currentStart) });
  }
  return scanned;
}

function updateEncounterControls(list) {
  const cursor = cursorLine();
  const current = currentEncounterAtLine(list, cursor);
  const currentPosition = currentEncounterPosition(list, current);
  currentEncounter.textContent = current && currentPosition >= 0 ? `${currentPosition + 1}. ${current.name}` : "None";
  encounterSelect.replaceChildren(
    ...list.map((encounter, position) => {
      const option = document.createElement("option");
      option.value = String(position);
      option.textContent = `${position + 1}. ${encounter.name}`;
      return option;
    })
  );
  if (currentPosition >= 0) encounterSelect.value = String(currentPosition);
  prevEncounterButton.disabled = !list.length;
  nextEncounterButton.disabled = !list.length;
}

function cursorLine() {
  return input.value.slice(0, input.selectionStart).split(/\r?\n/).length;
}

function jumpRelativeEncounter(delta) {
  const list = encounterList();
  if (!list.length) return;
  const current = currentEncounterAtLine(list, cursorLine());
  const currentPosition = currentEncounterPosition(list, current);
  const nextPosition = Math.min(Math.max(currentPosition + delta, 0), list.length - 1);
  jumpToEncounter(list[nextPosition], nextPosition);
}

function currentEncounterPosition(list, current) {
  if (!current) return -1;
  return list.findIndex((encounter) => encounter.start_line === current.start_line);
}

function jumpToEncounter(encounter, position) {
  jumpToLine(encounter.start_line);
  scrollOutputToEncounterPosition(position);
}

function jumpToLine(lineNumber) {
  const offset = lineStartOffset(input.value, lineNumber);
  input.focus();
  input.selectionStart = input.selectionEnd = offset;
  scrollInputLineToTop(lineNumber);
  updateCursorUi();
}

function moveCursorToFirstEncounter() {
  const firstEncounter = scanEncounters(input.value)[0];
  const lineNumber = firstEncounter?.start_line || 1;
  const offset = lineStartOffset(input.value, lineNumber);
  input.selectionStart = input.selectionEnd = offset;
  scrollInputLineToTop(lineNumber);
}

function scrollInputLineToTop(lineNumber) {
  const style = window.getComputedStyle(input);
  const fontSize = Number.parseFloat(style.fontSize) || 13;
  const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.4;
  input.scrollTop = Math.max(0, (lineNumber - 1) * lineHeight);
}

function scrollOutputToEncounterPosition(encounterPosition) {
  const lines = (output.textContent || "").split(/\r?\n/);
  let seen = -1;
  const outputLineIndex = lines.findIndex((line) => {
    if (!/^(?:#\s*)?(?:Random Encounter:|Simulated Encounter:|Multizone encounter:|Encounter:)\s*\d+/i.test(line)) {
      return false;
    }
    seen += 1;
    return seen === encounterPosition;
  });
  if (outputLineIndex < 0) return;
  const style = window.getComputedStyle(output);
  const fontSize = Number.parseFloat(style.fontSize) || 13;
  const lineHeight = Number.parseFloat(style.lineHeight) || fontSize * 1.4;
  output.scrollTop = Math.max(0, outputLineIndex * lineHeight - lineHeight - 10);
}

function lineStartOffset(text, lineNumber) {
  if (lineNumber <= 1) return 0;
  let offset = 0;
  for (let line = 1; line < lineNumber; line += 1) {
    const next = text.indexOf("\n", offset);
    if (next === -1) return text.length;
    offset = next + 1;
  }
  return offset;
}

async function applyChocoboAction(module, seed, actionKind, slotIndex) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.chocobo_action_json(seed, input.value, cursorLine, actionKind, slotIndex ?? undefined));
    insertAtLine(payload.insert_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyChocoboSwap(module, seed, slotIndex, replacement) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.chocobo_swap_json(seed, input.value, cursorLine, slotIndex, replacement));
    insertAtLine(payload.insert_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyTankerPattern(module, pattern) {
  try {
    tankerPatternValue = pattern;
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.tanker_pattern_json(input.value, cursorLine, pattern));
    replaceLineRange(payload.start_line, payload.end_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyTrosAttack(module, attack) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.tros_attack_json(input.value, cursorLine, attack));
    replaceLineRange(payload.start_line, payload.end_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyGaruda1Attacks(module, attacks) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.garuda1_attacks_json(input.value, cursorLine, attacks.join(",")));
    replaceLineRange(payload.start_line, payload.end_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyGaruda2Attack(module, attack) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const payload = JSON.parse(module.garuda2_attack_json(seed, input.value, cursorLine, attack));
    if (payload.lines.length || payload.end_line >= payload.start_line) {
      replaceLineRange(payload.start_line, payload.end_line, payload.lines.join("\n"));
      await renderCurrentInput();
    }
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

async function applyLancetTutorialTiming(module, timing) {
  try {
    const cursorLine = input.value.slice(0, input.selectionStart).split("\n").length;
    const payload = JSON.parse(module.lancet_tutorial_timing_json(input.value, cursorLine, timing));
    replaceLineRange(payload.start_line, payload.end_line, payload.lines.join("\n"));
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

function insertAtLine(lineNumber, text) {
  const lines = input.value.split("\n");
  const insertion = text.endsWith("\n") ? text : `${text}\n`;
  const offset = lines.slice(0, Math.max(lineNumber - 1, 0)).join("\n").length;
  const adjustedOffset = lineNumber > 1 ? offset + 1 : 0;
  input.value = `${input.value.slice(0, adjustedOffset)}${insertion}${input.value.slice(adjustedOffset)}`;
  input.focus();
  input.selectionStart = input.selectionEnd = adjustedOffset + insertion.length;
}

function replaceLineRange(startLine, endLine, text) {
  const insertion = !text || text.endsWith("\n") ? text : `${text}\n`;
  const startOffset = lineStartOffset(input.value, startLine);
  const endOffset = endLine >= startLine
    ? lineStartOffset(input.value, endLine + 1)
    : startOffset;
  input.value = `${input.value.slice(0, startOffset)}${insertion}${input.value.slice(endOffset)}`;
  input.focus();
  input.selectionStart = input.selectionEnd = startOffset + insertion.length;
}

function setupResizeHandles() {
  document.querySelectorAll(".main-resize").forEach((handle) => {
    handle.addEventListener("pointerdown", (event) => startSplitResize(event, handle.closest(".workspace"), "--input-column", "--input-row"));
  });
  document.querySelectorAll(".tracker-resize").forEach((handle) => {
    handle.addEventListener("pointerdown", (event) => startSplitResize(event, handle.closest(".tracker-body"), "--tracker-input-column", "--input-row"));
  });
}

function startSplitResize(event, container, columnVar, rowVar) {
  if (!container) return;
  event.preventDefault();
  const handle = event.currentTarget;
  handle.setPointerCapture?.(event.pointerId);
  const onMove = (moveEvent) => {
    const rect = container.getBoundingClientRect();
    const stacked = window.matchMedia("(max-width: 920px)").matches;
    if (stacked) {
      const percent = clamp(((moveEvent.clientY - rect.top) / rect.height) * 100, 24, 76);
      container.style.setProperty(rowVar, `${percent.toFixed(1)}%`);
    } else {
      const percent = clamp(((moveEvent.clientX - rect.left) / rect.width) * 100, 22, 72);
      container.style.setProperty(columnVar, `${percent.toFixed(1)}%`);
    }
    saveWorkspaceCache();
  };
  const onUp = () => {
    handle.releasePointerCapture?.(event.pointerId);
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", onUp);
  };
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", onUp);
  onMove(event);
}

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max);
}

function downloadText(filename, text, mimeType = "text/plain;charset=utf-8") {
  const blob = new Blob([text], { type: mimeType });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}

function saveWorkspaceCache() {
  try {
    localStorage.setItem(WORKSPACE_STORAGE_KEY, JSON.stringify({
      seed: seedInput.value,
      ctbInput: input.value,
      dropsInput: trackerPanes.drops.input.value,
      encountersInput: trackerPanes.encounters.input.value,
      layout: currentLayoutState(),
    }));
  } catch {
    // Storage can be unavailable in private/file contexts; keep the editor usable.
  }
}

function restoreWorkspaceCache() {
  try {
    const cached = JSON.parse(localStorage.getItem(WORKSPACE_STORAGE_KEY) || "null");
    if (!cached || typeof cached !== "object") return false;
    seedInput.value = cached.seed || seedInput.value;
    input.value = typeof cached.ctbInput === "string" ? cached.ctbInput : input.value;
    trackerPanes.drops.input.value = typeof cached.dropsInput === "string" ? cached.dropsInput : "";
    trackerPanes.encounters.input.value = typeof cached.encountersInput === "string" ? cached.encountersInput : "";
    restoreLayoutState(cached.layout);
    moveCursorToFirstEncounter();
    return Boolean(input.value || trackerPanes.drops.input.value || trackerPanes.encounters.input.value);
  } catch {
    return false;
  }
}

function currentLayoutState() {
  const workspace = document.querySelector(".workspace");
  const trackerBodies = Object.fromEntries(
    Object.entries(trackerPanes).map(([name, pane]) => {
      const body = pane.input.closest(".tracker-body");
      return [name, {
        trackerInputColumn: body?.style.getPropertyValue("--tracker-input-column") || "",
        inputRow: body?.style.getPropertyValue("--input-row") || "",
      }];
    })
  );
  return {
    activeModes: [...activeModes],
    workspace: {
      inputColumn: workspace?.style.getPropertyValue("--input-column") || "",
      inputRow: workspace?.style.getPropertyValue("--input-row") || "",
    },
    trackerBodies,
  };
}

function restoreLayoutState(layout) {
  if (!layout || typeof layout !== "object") return;
  const restoredModes = Array.isArray(layout.activeModes)
    ? layout.activeModes.filter((mode) => ["ctb", "drops", "encounters"].includes(mode))
    : [];
  if (restoredModes.length) activeModes = new Set(restoredModes);
  const workspace = document.querySelector(".workspace");
  if (workspace && layout.workspace) {
    setOptionalStyleProperty(workspace, "--input-column", layout.workspace.inputColumn);
    setOptionalStyleProperty(workspace, "--input-row", layout.workspace.inputRow);
  }
  for (const [name, pane] of Object.entries(trackerPanes)) {
    const body = pane.input.closest(".tracker-body");
    const bodyState = layout.trackerBodies?.[name];
    if (!body || !bodyState) continue;
    setOptionalStyleProperty(body, "--tracker-input-column", bodyState.trackerInputColumn);
    setOptionalStyleProperty(body, "--input-row", bodyState.inputRow);
  }
  updateVisibleModes();
}

function setOptionalStyleProperty(element, property, value) {
  if (typeof value === "string" && value) element.style.setProperty(property, value);
}

async function initializeWorkspace() {
  if (restoreWorkspaceCache()) {
    if (trackerPanes.encounters.input.value) await hydrateEncounterSlidersFromDefaults();
    await renderCurrentInput();
    if (trackerPanes.drops.input.value) await renderTracker("drops");
    if (trackerPanes.encounters.input.value) await renderTracker("encounters");
    return;
  }
  await loadSample();
}

initializeWorkspace();

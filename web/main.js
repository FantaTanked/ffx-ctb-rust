const status = document.querySelector("#status");
const appShell = document.querySelector(".app-shell");
const modeTabs = [...document.querySelectorAll(".mode-tab")];
const seedInput = document.querySelector("#seed");
const input = document.querySelector("#input");
const output = document.querySelector("#output");
const renderButton = document.querySelector("#render");
const sampleButton = document.querySelector("#sample");
const openInputButton = document.querySelector("#openInput");
const saveInputButton = document.querySelector("#saveInput");
const saveOutputButton = document.querySelector("#saveOutput");
const fileInput = document.querySelector("#fileInput");
const summary = document.querySelector("#summary");
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
    summary: document.querySelector("#dropsSummary"),
    load: document.querySelector("#loadDrops"),
    render: document.querySelector("#renderDrops"),
    noEncounters: document.querySelector("#searchNoEncounters"),
  },
  encounters: {
    input: document.querySelector("#encountersTrackerInput"),
    output: document.querySelector("#encountersTrackerOutput"),
    summary: document.querySelector("#encountersTrackerSummary"),
    load: document.querySelector("#loadEncountersTracker"),
    render: document.querySelector("#renderEncountersTracker"),
    sliders: document.querySelector("#encounterSliders"),
    sliderData: [],
  },
};

const APP_BUILD_ID = "ctb-tracker-render-20260430-224";
let lastRendered = null;
let lastRenderedInput = null;
let tankerPatternValue = "awsdn-";

let wasm = null;

async function loadWasm() {
  if (wasm) return wasm;
  const module = await import(`../pkg/ffx_ctb_rust.js?v=${APP_BUILD_ID}`);
  await module.default();
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
    await renderCurrentInput();
  } catch (error) {
    status.textContent = error?.message || String(error);
  }
}

renderButton.addEventListener("click", async () => {
  await renderCurrentInput();
});

input.addEventListener("keyup", updateCursorUi);
input.addEventListener("click", updateCursorUi);
input.addEventListener("input", updateCursorUi);

openInputButton.addEventListener("click", () => fileInput.click());
fileInput.addEventListener("change", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;
  input.value = await file.text();
  input.selectionStart = input.selectionEnd = 0;
  fileInput.value = "";
  await renderCurrentInput();
});
saveInputButton.addEventListener("click", () => downloadText("ctb_actions_input.txt", input.value));
saveOutputButton.addEventListener("click", () => downloadText("ctb_output.txt", output.textContent || ""));
prevEncounterButton.addEventListener("click", () => jumpRelativeEncounter(-1));
nextEncounterButton.addEventListener("click", () => jumpRelativeEncounter(1));
encounterSelect.addEventListener("change", () => {
  const encounter = encounterList().find((item) => String(item.index) === encounterSelect.value);
  if (encounter) jumpToLine(encounter.start_line);
});
Object.entries(trackerPanes).forEach(([tracker, pane]) => {
  pane.load.addEventListener("click", () => loadTrackerDefault(tracker));
  pane.render.addEventListener("click", () => renderTracker(tracker));
});
trackerPanes.drops.noEncounters?.addEventListener("click", searchNoEncountersRoutes);
modeTabs.forEach((tab) => {
  tab.addEventListener("click", () => setMode(tab.dataset.mode || "ctb"));
});

function setMode(mode) {
  appShell.dataset.mode = mode;
  modeTabs.forEach((tab) => {
    tab.classList.toggle("is-active", tab.dataset.mode === mode);
  });
}

async function renderCurrentInput() {
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const started = performance.now();
    const rawRendered = lastRenderedInput === null
      ? module.render_ctb_json(seed, input.value)
      : module.render_ctb_diff_json(seed, input.value, lastRenderedInput);
    const rendered = JSON.parse(rawRendered);
    const durationSeconds = rendered.duration_seconds || (performance.now() - started) / 1000;
    lastRendered = rendered;
    lastRenderedInput = input.value;
    output.textContent = rendered.output || "";
    summary.textContent = `${rendered.prepared_line_count} prepared lines | from line ${rendered.changed_line || 1} | ${rendered.encounters.length} encounters | ${rendered.unsupported_count} unsupported | ${durationSeconds.toFixed(3)}s`;
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
    pane.output.textContent = "";
    pane.summary.textContent = `${payload.input_filename} loaded`;
    if (tracker === "encounters") {
      pane.sliderData = Array.isArray(payload.sliders) ? payload.sliders : [];
      renderEncounterSliderControls(pane);
    }
    await renderTracker(tracker);
  } catch (error) {
    pane.summary.textContent = error?.message || String(error);
  }
}

async function renderTracker(tracker) {
  const pane = trackerPanes[tracker];
  try {
    const module = await loadWasm();
    const seed = Number.parseInt(seedInput.value, 10) >>> 0;
    const started = performance.now();
    const payload = JSON.parse(module.tracker_render_json(tracker, seed, pane.input.value));
    const durationSeconds = payload.duration_seconds || (performance.now() - started) / 1000;
    pane.lastRenderedInput = pane.input.value;
    pane.output.textContent = payload.output || "";
    pane.summary.textContent = `${payload.output_filename} | rendered | ${durationSeconds.toFixed(3)}s`;
  } catch (error) {
    pane.summary.textContent = error?.message || String(error);
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
    }
    pane.output.textContent = payload.output || "";
    pane.summary.textContent = `No Encounters search | line ${startLine}`;
  } catch (error) {
    pane.summary.textContent = error?.message || String(error);
  }
}

function textareaCursorLine(textarea) {
  return textarea.value.slice(0, textarea.selectionStart || 0).split("\n").length;
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
      });
      label.replaceChildren(name, range, value);
      return label;
    });
  pane.sliders.replaceChildren(...controls);
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
  currentEncounter.textContent = current ? `${current.index}. ${current.name}` : "None";
  encounterSelect.replaceChildren(
    ...list.map((encounter) => {
      const option = document.createElement("option");
      option.value = String(encounter.index);
      option.textContent = `${encounter.index}. ${encounter.name}`;
      return option;
    })
  );
  if (current) encounterSelect.value = String(current.index);
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
  const currentPosition = current ? list.findIndex((encounter) => encounter.index === current.index) : 0;
  const nextPosition = Math.min(Math.max(currentPosition + delta, 0), list.length - 1);
  jumpToLine(list[nextPosition].start_line);
}

function jumpToLine(lineNumber) {
  const offset = lineStartOffset(input.value, lineNumber);
  input.focus();
  input.selectionStart = input.selectionEnd = offset;
  updateCursorUi();
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

function downloadText(filename, text) {
  const blob = new Blob([text], { type: "text/plain;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = filename;
  link.click();
  URL.revokeObjectURL(url);
}

loadSample();

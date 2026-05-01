pub mod api;
pub mod battle;
pub mod ctb;
pub mod data;
pub mod encounter;
pub mod model;
pub mod parser;
pub mod rng;
pub mod script;
pub mod simulator;

pub use rng::FfxRngTracker;

#[cfg(feature = "wasm")]
use wasm_bindgen::prelude::*;

#[cfg(feature = "wasm")]
#[wasm_bindgen(start)]
pub fn init_wasm() {
    console_error_panic_hook::set_once();
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn render_ctb_json(seed: u32, input: &str) -> Result<String, JsValue> {
    api::render_ctb_json(seed, input).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn render_ctb_diff_json(
    seed: u32,
    input: &str,
    previous_input: &str,
) -> Result<String, JsValue> {
    api::render_ctb_diff_json(seed, input, previous_input)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn sample_json() -> Result<String, JsValue> {
    api::sample_json().map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn party_json(seed: u32, input: &str, cursor_line: usize) -> Result<String, JsValue> {
    api::party_json(seed, input, cursor_line).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn chocobo_action_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    action_kind: &str,
    slot_index: Option<usize>,
) -> Result<String, JsValue> {
    api::chocobo_action_json(seed, input, cursor_line, action_kind, slot_index)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn chocobo_swap_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    slot_index: usize,
    replacement: &str,
) -> Result<String, JsValue> {
    api::chocobo_swap_json(seed, input, cursor_line, slot_index, replacement)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn tanker_pattern_json(
    input: &str,
    cursor_line: usize,
    pattern: &str,
) -> Result<String, JsValue> {
    api::tanker_pattern_json(input, cursor_line, pattern)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn tros_attack_json(input: &str, cursor_line: usize, attack: &str) -> Result<String, JsValue> {
    api::tros_attack_json(input, cursor_line, attack)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn garuda1_attacks_json(
    input: &str,
    cursor_line: usize,
    attacks: &str,
) -> Result<String, JsValue> {
    api::garuda1_attacks_json(input, cursor_line, attacks)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn garuda2_attack_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    attack: &str,
) -> Result<String, JsValue> {
    api::garuda2_attack_json(seed, input, cursor_line, attack)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn lancet_tutorial_timing_json(
    input: &str,
    cursor_line: usize,
    timing: &str,
) -> Result<String, JsValue> {
    api::lancet_tutorial_timing_json(input, cursor_line, timing)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn tracker_default_json(tracker: &str, seed: u32) -> Result<String, JsValue> {
    api::tracker_default_json(tracker, seed).map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn tracker_render_json(tracker: &str, seed: u32, input: &str) -> Result<String, JsValue> {
    api::tracker_render_json(tracker, seed, input)
        .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn no_encounters_routes_json(
    seed: u32,
    input: &str,
    start_line: usize,
    encounters_input: Option<String>,
    encounters_output: Option<String>,
) -> Result<String, JsValue> {
    api::no_encounters_routes_json(
        seed,
        input,
        start_line,
        encounters_input.as_deref(),
        encounters_output.as_deref(),
    )
    .map_err(|error| JsValue::from_str(&error.to_string()))
}

#[cfg(feature = "wasm")]
#[wasm_bindgen]
pub fn rng_preview_json(seed: u32, index: usize, count: usize) -> Result<String, JsValue> {
    api::rng_preview_json(seed, index, count).map_err(|error| JsValue::from_str(&error.to_string()))
}

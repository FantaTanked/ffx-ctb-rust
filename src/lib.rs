pub mod api;
pub mod battle;
pub mod ctb;
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
pub fn rng_preview_json(seed: u32, index: usize, count: usize) -> Result<String, JsValue> {
    api::rng_preview_json(seed, index, count).map_err(|error| JsValue::from_str(&error.to_string()))
}

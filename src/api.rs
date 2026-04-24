use serde::Serialize;
use thiserror::Error;

use crate::ctb;
use crate::rng::FfxRngTracker;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("rng index must be between 0 and 67")]
    InvalidRngIndex,
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
struct RngPreviewResponse {
    seed: u32,
    index: usize,
    values: Vec<u32>,
}

pub fn rng_preview_json(seed: u32, index: usize, count: usize) -> Result<String, ApiError> {
    if index >= 68 {
        return Err(ApiError::InvalidRngIndex);
    }
    let mut tracker = FfxRngTracker::new(seed);
    let values = (0..count).map(|_| tracker.advance_rng(index)).collect();
    Ok(serde_json::to_string(&RngPreviewResponse {
        seed,
        index,
        values,
    })?)
}

pub fn render_ctb_json(seed: u32, input: &str) -> Result<String, ApiError> {
    Ok(serde_json::to_string(&ctb::render_ctb(seed, input))?)
}

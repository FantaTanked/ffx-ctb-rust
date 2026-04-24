use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RenderResponse {
    pub seed: u32,
    pub output: String,
    pub duration_seconds: f64,
    pub implemented: bool,
    pub message: String,
}

pub fn render_ctb(seed: u32, input: &str) -> RenderResponse {
    let _line_count = input.lines().count();
    RenderResponse {
        seed,
        output: String::new(),
        duration_seconds: 0.0,
        implemented: false,
        message: "Rust CTB renderer is scaffolded; port parser/game-state/events next.".to_string(),
    }
}

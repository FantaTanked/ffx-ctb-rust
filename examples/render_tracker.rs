use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let tracker = args.next().unwrap_or_else(|| "drops".to_string());
    let seed = args
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(3096296922);
    let input_path = args.next().map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(
            "../ctb-live-editor-pages/search_outputs/3096296922/seed_3096296922_search_drops.txt",
        )
    });
    let input = fs::read_to_string(&input_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", input_path.display()));
    let response = ffx_ctb_rust::api::tracker_render_json(&tracker, seed, &input)
        .unwrap_or_else(|error| panic!("failed to render {tracker} tracker: {error}"));
    let payload: Value =
        serde_json::from_str(&response).expect("tracker render response should be valid JSON");
    if let Some(output) = payload.get("output").and_then(Value::as_str) {
        print!("{output}");
    }
}

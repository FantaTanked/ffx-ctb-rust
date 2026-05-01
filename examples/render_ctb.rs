use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let seed = args
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(3096296922);
    let input_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixtures/ctb_actions_input.txt"));
    let input = fs::read_to_string(&input_path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", input_path.display()));
    let response = ffx_ctb_rust::ctb::render_ctb(seed, &input);
    print!("{}", response.output.trim_end());
    println!();
}

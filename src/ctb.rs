use serde::Serialize;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::script::prepare_action_lines;
use crate::simulator;

#[derive(Debug, Clone, Serialize)]
pub struct RenderResponse {
    pub seed: u32,
    pub output: String,
    pub changed_line: usize,
    pub checkpoint_line: usize,
    pub duration_seconds: f64,
    pub implemented: bool,
    pub parity_complete: bool,
    pub unsupported_count: usize,
    pub message: String,
    pub prepared_line_count: usize,
    pub encounters: Vec<EncounterBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EncounterBlock {
    pub index: usize,
    pub name: String,
    pub start_line: usize,
    pub end_line: usize,
}

pub fn render_ctb(seed: u32, input: &str) -> RenderResponse {
    render_ctb_with_previous(seed, input, None)
}

pub fn render_ctb_with_previous(
    seed: u32,
    input: &str,
    previous_input: Option<&str>,
) -> RenderResponse {
    let started = render_timer_start();
    let prepared = prepare_action_lines(input);
    let changed_line = previous_input
        .map(|previous_input| first_changed_prepared_line(previous_input, &prepared.lines))
        .unwrap_or(1);
    let encounters = scan_encounters_from_text(input);
    let simulated = simulator::SimulationState::new(seed)
        .with_editor_echoes()
        .run_lines(&prepared.lines);
    let implemented = simulated.unsupported_count == 0;
    RenderResponse {
        seed,
        output: simulated.text,
        changed_line,
        checkpoint_line: 1,
        duration_seconds: render_duration_seconds(started),
        implemented,
        parity_complete: false,
        unsupported_count: simulated.unsupported_count,
        message: render_message(simulated.unsupported_count),
        prepared_line_count: prepared.lines.len(),
        encounters,
    }
}

fn first_changed_prepared_line(previous_input: &str, prepared_lines: &[String]) -> usize {
    let previous = prepare_action_lines(previous_input);
    let max_len = previous.lines.len().max(prepared_lines.len());
    for index in 0..max_len {
        if previous.lines.get(index) != prepared_lines.get(index) {
            return index + 1;
        }
    }
    1
}

#[cfg(not(target_arch = "wasm32"))]
fn render_timer_start() -> Instant {
    Instant::now()
}

#[cfg(target_arch = "wasm32")]
fn render_timer_start() {}

#[cfg(not(target_arch = "wasm32"))]
fn render_duration_seconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

#[cfg(target_arch = "wasm32")]
fn render_duration_seconds(_: ()) -> f64 {
    0.0
}

pub fn scan_encounters(lines: &[String]) -> Vec<EncounterBlock> {
    let mut encounters = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_start_line: Option<usize> = None;
    let mut encounter_index = 0;
    let mut in_block_comment = false;

    for (zero_index, raw_line) in lines.iter().enumerate() {
        let line_number = zero_index + 1;
        let stripped = raw_line.trim();
        if stripped.starts_with("/*") {
            let ends_on_same_line = stripped.ends_with("*/");
            if !ends_on_same_line {
                in_block_comment = true;
            }
            continue;
        }
        if in_block_comment {
            if stripped.ends_with("*/") {
                in_block_comment = false;
            }
            continue;
        }
        if !stripped.to_ascii_lowercase().starts_with("encounter ") {
            continue;
        }
        if let (Some(name), Some(start_line)) = (current_name.take(), current_start_line) {
            encounters.push(EncounterBlock {
                index: encounter_index,
                name,
                start_line,
                end_line: line_number - 1,
            });
        }
        let name = stripped
            .split_whitespace()
            .nth(1)
            .unwrap_or("unknown")
            .to_string();
        encounter_index += 1;
        current_name = Some(name);
        current_start_line = Some(line_number);
    }

    if let (Some(name), Some(start_line)) = (current_name, current_start_line) {
        encounters.push(EncounterBlock {
            index: encounter_index,
            name,
            start_line,
            end_line: lines.len().max(start_line),
        });
    }

    encounters
}

pub fn scan_encounters_from_text(input: &str) -> Vec<EncounterBlock> {
    let lines = input
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect::<Vec<_>>();
    scan_encounters(&lines)
}

fn render_message(unsupported_count: usize) -> String {
    if unsupported_count == 0 {
        "Rust CTB renderer handled all parsed commands in this input with the current shallow simulation layer; full Python parity is still in progress.".to_string()
    } else {
        format!(
            "Rust CTB renderer is partially ported; {unsupported_count} command(s) still need event-specific logic."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        render_ctb, render_ctb_with_previous, scan_encounters, scan_encounters_from_text,
        EncounterBlock,
    };

    #[test]
    fn scans_encounter_blocks_like_web_python() {
        let lines = vec![
            "# heading".to_string(),
            "encounter tanker".to_string(),
            "tidus attack m1".to_string(),
            "encounter multizone ruins".to_string(),
            "m1 attack".to_string(),
            "encounter".to_string(),
            "  encounter sahagins".to_string(),
        ];
        assert_eq!(
            scan_encounters(&lines),
            vec![
                EncounterBlock {
                    index: 1,
                    name: "tanker".to_string(),
                    start_line: 2,
                    end_line: 3,
                },
                EncounterBlock {
                    index: 2,
                    name: "multizone".to_string(),
                    start_line: 4,
                    end_line: 6,
                },
                EncounterBlock {
                    index: 3,
                    name: "sahagins".to_string(),
                    start_line: 7,
                    end_line: 7,
                },
            ]
        );
    }

    #[test]
    fn render_response_uses_prepared_lines() {
        let response = render_ctb(
            3096296922,
            "encounter tanker\nstatus atb\n/repeat 2 1\nencounter ammes",
        );
        assert_eq!(response.prepared_line_count, 6);
        assert_eq!(response.encounters.len(), 2);
        assert_eq!(response.encounters[1].start_line, 4);
        assert!(response.output.contains("Encounter:   1 | Tanker"));
        assert!(response.implemented);
        assert!(!response.parity_complete);
        assert_eq!(response.unsupported_count, 0);
        assert_eq!(response.changed_line, 1);
        assert_eq!(response.checkpoint_line, 1);
    }

    #[test]
    fn render_response_treats_parser_errors_as_handled_commands() {
        let response = render_ctb(3096296922, "definitely unknown");

        assert!(response.output.contains("Error: Impossible to parse"));
        assert!(response.implemented);
        assert_eq!(response.unsupported_count, 0);
        assert!(!response.parity_complete);
    }

    #[test]
    fn scans_raw_input_encounters_for_web_editor_line_numbers() {
        let encounters = scan_encounters_from_text("encounter a\n/repeat 2 1\nencounter b");
        assert_eq!(encounters[1].start_line, 3);
    }

    #[test]
    fn scans_raw_input_encounters_preserves_textarea_trailing_blank_line() {
        let encounters = scan_encounters_from_text("encounter tanker\r\nstatus atb\r\n");
        assert_eq!(encounters[0].start_line, 1);
        assert_eq!(encounters[0].end_line, 3);
    }

    #[test]
    fn scans_encounters_skip_block_commented_rows_like_python_web() {
        let encounters = scan_encounters_from_text(
            "/*\nencounter tanker\n*/\nencounter chocobo_eater\n/* encounter tanker */\nencounter ammes\n",
        );

        assert_eq!(
            encounters,
            vec![
                EncounterBlock {
                    index: 1,
                    name: "chocobo_eater".to_string(),
                    start_line: 4,
                    end_line: 5,
                },
                EncounterBlock {
                    index: 2,
                    name: "ammes".to_string(),
                    start_line: 6,
                    end_line: 7,
                },
            ]
        );
    }

    #[test]
    fn scans_encounters_end_block_comments_on_trailing_marker() {
        let encounters =
            scan_encounters_from_text("/*\nencounter tanker\nnote */\nencounter chocobo_eater\n");

        assert_eq!(
            encounters,
            vec![EncounterBlock {
                index: 1,
                name: "chocobo_eater".to_string(),
                start_line: 4,
                end_line: 5,
            }]
        );
    }

    #[test]
    fn scans_encounters_keep_block_comment_open_until_trailing_marker_like_python() {
        let encounters = scan_encounters_from_text(
            "/* note */ trailing text\nencounter tanker\nstill ignored */\nencounter chocobo_eater\n",
        );

        assert_eq!(
            encounters,
            vec![EncounterBlock {
                index: 1,
                name: "chocobo_eater".to_string(),
                start_line: 4,
                end_line: 5,
            }]
        );
    }

    #[test]
    fn render_response_reports_first_changed_prepared_line_like_python_web() {
        let previous = "encounter tanker\ntidus attack m1\n";
        let current = "encounter tanker\ntidus cheer\n";

        let response = render_ctb_with_previous(3096296922, current, Some(previous));

        assert_eq!(response.changed_line, 2);
        assert_eq!(response.checkpoint_line, 1);
    }

    #[test]
    fn render_response_compares_changed_line_after_macro_expansion() {
        let previous = "/repeat 2 1\ntidus cheer\n";
        let current = "/repeat 2 1\nauron cheer\n";

        let response = render_ctb_with_previous(3096296922, current, Some(previous));

        assert_eq!(response.changed_line, 2);
    }
}

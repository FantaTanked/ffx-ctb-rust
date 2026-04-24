use serde::Serialize;
use std::time::Instant;

use crate::parser::{parse_raw_action_line, ParsedCommand};
use crate::script::prepare_action_lines;
use crate::simulator;

#[derive(Debug, Clone, Serialize)]
pub struct RenderResponse {
    pub seed: u32,
    pub output: String,
    pub duration_seconds: f64,
    pub implemented: bool,
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
    let started = Instant::now();
    let prepared = prepare_action_lines(input);
    let encounters = scan_encounters(&prepared.lines);
    let simulated = simulator::simulate(seed, &prepared.lines);
    let implemented = simulated.unsupported_count == 0;
    RenderResponse {
        seed,
        output: simulated.text,
        duration_seconds: started.elapsed().as_secs_f64(),
        implemented,
        message: render_message(simulated.unsupported_count),
        prepared_line_count: prepared.lines.len(),
        encounters,
    }
}

pub fn scan_encounters(lines: &[String]) -> Vec<EncounterBlock> {
    let mut encounters = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_start_line: Option<usize> = None;
    let mut encounter_index = 0;

    for (zero_index, raw_line) in lines.iter().enumerate() {
        let line_number = zero_index + 1;
        let ParsedCommand::Encounter { name, .. } = parse_raw_action_line(raw_line) else {
            continue;
        };
        if let (Some(name), Some(start_line)) = (current_name.take(), current_start_line) {
            encounters.push(EncounterBlock {
                index: encounter_index,
                name,
                start_line,
                end_line: line_number - 1,
            });
        }
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

fn render_message(unsupported_count: usize) -> String {
    if unsupported_count == 0 {
        "Rust CTB renderer handled all parsed commands in this input.".to_string()
    } else {
        format!(
            "Rust CTB renderer is partially ported; {unsupported_count} command(s) still need event-specific logic."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{render_ctb, scan_encounters, EncounterBlock};

    #[test]
    fn scans_encounter_blocks_like_web_python() {
        let lines = vec![
            "# heading".to_string(),
            "encounter tanker".to_string(),
            "tidus attack m1".to_string(),
            "encounter multizone ruins".to_string(),
            "m1 attack".to_string(),
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
                    name: "ruins".to_string(),
                    start_line: 4,
                    end_line: 5,
                },
            ]
        );
    }

    #[test]
    fn render_response_uses_prepared_lines() {
        let response = render_ctb(3096296922, "encounter a\nx\n/repeat 2 1\nencounter b");
        assert_eq!(response.prepared_line_count, 6);
        assert_eq!(response.encounters.len(), 2);
        assert!(response.output.contains("Encounter:   1 | a"));
        assert!(!response.implemented);
    }
}

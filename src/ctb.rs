use serde::Serialize;

use crate::parser::{parse_raw_action_line, ParsedCommand};
use crate::script::prepare_action_lines;

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
    let prepared = prepare_action_lines(input);
    let encounters = scan_encounters(&prepared.lines);
    RenderResponse {
        seed,
        output: render_scaffold_output(&encounters),
        duration_seconds: 0.0,
        implemented: false,
        message: "Rust CTB renderer is scaffolded through input preparation and encounter scanning; port game-state/events next.".to_string(),
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

fn render_scaffold_output(encounters: &[EncounterBlock]) -> String {
    let mut lines = vec![
        "# Rust CTB renderer scaffold".to_string(),
        "# Encounter scanning is active; action simulation is not ported yet.".to_string(),
    ];
    for encounter in encounters {
        lines.push(format!(
            "# Encounter {}: {} lines {}-{}",
            encounter.index, encounter.name, encounter.start_line, encounter.end_line
        ));
    }
    lines.push(String::new());
    lines.join("\n")
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
        assert!(response.output.contains("Encounter 1: a"));
    }
}

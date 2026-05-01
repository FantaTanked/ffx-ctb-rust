use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
use thiserror::Error;

use crate::battle::ActorId;
use crate::ctb;
use crate::data::{self, EquipmentKind, ItemDrop, MonsterItemDropInfo};
use crate::model::{Character, MonsterSlot};
use crate::parser::{parse_raw_action_line, ParsedCommand};
use crate::rng::FfxRngTracker;
use crate::script::{prepare_action_lines, prepare_action_lines_before_raw_line};
use crate::simulator::SimulationState;

pub const DEFAULT_SEED: u32 = 3_096_296_922;
const DEFAULT_INPUT: &str = include_str!("../fixtures/ctb_actions_input.txt");
const DROPS_NOTES: &str = include_str!("../data/notes/anypercent/drops_notes.txt");
const ENCOUNTERS_NOTES: &str = include_str!("../data/notes/anypercent/encounters_notes.csv");
const DROPS_USAGE_TEXT: &str = r####"# /*
# Commands:
#    "#" is used to ignore a single line
#    "###" is used to start and end ignored blocks
#    "/usage" shows this text
#    "///" will hide everything above it from the output
#    "/nopadding" disables automatic padding of the output
#    "/macro [macro name]" will be replaced with the corresponding macro
#    "/repeat (# of times) (# of lines)" repeats previous lines the specified amount of times
# Events:
#     party [characters initials]
#     kill [monster name] [killer] (characters initials) (overkill/ok)
#     bribe [monster name] [user] (characters initials)
#     steal [monster name] (successful steals)
#     death (character)
#     ap (character) ((+/-)amount)
#     inventory [show/get/buy/use/sell/switch/autosort] [...]
#     inventory show (equipment/gil)
#     inventory [get/buy/use/sell] [item] [amount]
#     inventory [get/use] gil [amount]
#     inventory [get/buy/sell] equipment [equip type] [character] [slots] (abilities)
#     inventory sell equipment [equipment slot]
#     inventory switch [slot 1] [slot 2]
#     inventory autosort
# */"####;
const ENCOUNTERS_USAGE_TEXT: &str = r####"# /*
# Commands:
#    "#" is used to ignore a single line
#    "###" is used to start and end ignored blocks
#    "/usage" shows this text
#    "///" will hide everything above it from the output
#    "/nopadding" disables automatic padding of the output
#    "/macro [macro name]" will be replaced with the corresponding macro
#    "/repeat (# of times) (# of lines)" repeats previous lines the specified amount of times
# Events:
#     encounter (preemp/ambush/simulated/name/zone)
#     equip [equip type] [character] [# of slots] (abilities)
#     encounters_count [total/random/zone name] [(+/-)amount]
# */"####;
const DROPS_KILL_USAGE: &str =
    "Usage: kill [monster name] [killer] (characters initials) (overkill/ok)";
const DROPS_BRIBE_USAGE: &str = "Usage: bribe [monster name] [user] (characters initials)";
const DROPS_STEAL_USAGE: &str = "Usage: steal [monster name] (successful steals)";
const DROPS_CHARACTER_VALUES: &str = "tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown";

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("rng index must be between 0 and 67")]
    InvalidRngIndex,
    #[error("{0}")]
    BadRequest(String),
    #[error("internal CTB render panic: {0}")]
    Panic(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Serialize)]
struct RngPreviewResponse {
    seed: u32,
    index: usize,
    values: Vec<u32>,
}

#[derive(Debug, Serialize)]
struct SampleResponse {
    seed: u32,
    input: &'static str,
}

#[derive(Debug, Serialize)]
struct CharacterResponse {
    name: String,
    input_name: String,
}

#[derive(Debug, Serialize)]
struct PartyResponse {
    party: Vec<CharacterResponse>,
    reserves: Vec<CharacterResponse>,
}

#[derive(Debug, Serialize)]
struct ChocoboActionResponse {
    insert_line: usize,
    lines: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TankerPatternResponse {
    start_line: usize,
    end_line: usize,
    lines: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TrackerDefaultResponse {
    tracker: String,
    seed: u32,
    input: String,
    input_filename: &'static str,
    output_filename: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    sliders: Option<Vec<EncounterSliderResponse>>,
}

#[derive(Debug, Serialize)]
struct TrackerRenderResponse {
    tracker: String,
    output: String,
    duration_seconds: f64,
    output_filename: &'static str,
}

#[derive(Debug, Serialize)]
struct NoEncountersRoutesResponse {
    output: String,
    edited_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactFutureEncounterRow {
    encounter_options: Vec<Vec<String>>,
    random: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExactFutureEncounterOutput {
    rows: Vec<ExactFutureEncounterRow>,
    total_counts: HashMap<String, usize>,
    section_maxima: HashMap<String, HashMap<String, usize>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FutureEncounterSearchView {
    branches: Vec<ExactFutureEncounterOutput>,
    total_counts: HashMap<String, usize>,
    section_maxima: HashMap<String, HashMap<String, usize>>,
    rows_len: usize,
}

impl FutureEncounterSearchView {
    fn from_branch(branch: ExactFutureEncounterOutput) -> Self {
        Self::from_branches(vec![branch])
    }

    fn from_branches(branches: Vec<ExactFutureEncounterOutput>) -> Self {
        let mut total_counts: HashMap<String, usize> = HashMap::new();
        let mut section_maxima: HashMap<String, HashMap<String, usize>> = HashMap::new();
        let mut rows_len = 0;
        for branch in &branches {
            rows_len = rows_len.max(branch.rows.len());
            for (monster, count) in &branch.total_counts {
                let slot = total_counts.entry(monster.clone()).or_default();
                *slot = (*slot).max(*count);
            }
            for (section_name, counts) in &branch.section_maxima {
                let maxima = section_maxima.entry(section_name.clone()).or_default();
                for (monster, count) in counts {
                    let slot = maxima.entry(monster.clone()).or_default();
                    *slot = (*slot).max(*count);
                }
            }
        }
        Self {
            branches,
            total_counts,
            section_maxima,
            rows_len,
        }
    }
}

#[derive(Debug, Serialize)]
struct EncounterSliderResponse {
    index: usize,
    name: String,
    label: String,
    min: i32,
    default: i32,
    max: i32,
    initiative: bool,
}

#[derive(Debug, Clone, Copy)]
struct DropsApState {
    total_ap: i32,
    starting_s_lv: i32,
}

#[derive(Debug, Clone)]
struct DropsInventory {
    items: Vec<Option<String>>,
    quantities: Vec<i32>,
}

#[derive(Debug, Clone)]
struct DropsEquipment {
    owner: Character,
    kind: EquipmentKind,
    slots: u8,
    abilities: Vec<String>,
    sell_value: i32,
    guaranteed: bool,
    for_killer: bool,
    ability_rolls: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DropsEquipmentParseError {
    EquipmentType,
    Character,
    Slots,
    Ability,
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
    render_ctb_json_with_previous(seed, input, None)
}

pub fn render_ctb_diff_json(
    seed: u32,
    input: &str,
    previous_input: &str,
) -> Result<String, ApiError> {
    render_ctb_json_with_previous(seed, input, Some(previous_input))
}

fn render_ctb_json_with_previous(
    seed: u32,
    input: &str,
    previous_input: Option<&str>,
) -> Result<String, ApiError> {
    let mut response = catch_unwind(AssertUnwindSafe(|| {
        ctb::render_ctb_with_previous(seed, input, previous_input)
    }))
    .map_err(|panic| ApiError::Panic(panic_message(panic)))?;
    response.output = normalize_web_render_output(&response.output);
    Ok(serde_json::to_string(&response)?)
}

fn normalize_web_render_output(output: &str) -> String {
    format!("{}\n", output.trim_end())
}

pub fn sample_json() -> Result<String, ApiError> {
    Ok(serde_json::to_string(&SampleResponse {
        seed: DEFAULT_SEED,
        input: DEFAULT_INPUT,
    })?)
}

pub fn party_json(seed: u32, input: &str, cursor_line: usize) -> Result<String, ApiError> {
    let mut state = SimulationState::new(seed);
    state.run_until_prepared_line(input, cursor_line);
    let party = state.party().iter().copied().collect::<Vec<_>>();
    let reserves = party_swap_choices()
        .into_iter()
        .filter(|character| !party.contains(character))
        .collect::<Vec<_>>();
    Ok(serde_json::to_string(&PartyResponse {
        party: party.into_iter().take(3).map(character_response).collect(),
        reserves: reserves.into_iter().map(character_response).collect(),
    })?)
}

pub fn chocobo_action_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    action_kind: &str,
    slot_index: Option<usize>,
) -> Result<String, ApiError> {
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "chocobo_eater")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the chocobo_eater encounter first.".into())
        })?;
    let insert_line = get_chocobo_effective_insert_line(input, &encounter, cursor_line);
    let mut cursor_state = simulate_to_chocobo_cursor_state(seed, input, insert_line);
    if !cursor_state.state.has_monster_party() {
        return Err(ApiError::BadRequest(
            "Could not determine the active Chocobo Eater encounter state at this cursor.".into(),
        ));
    }

    let mut generated = Vec::new();
    for _ in 0..50 {
        let actor = cursor_state.state.next_actor().ok_or_else(|| {
            ApiError::BadRequest("Could not find the next actor in CTB order.".into())
        })?;
        match actor {
            ActorId::Monster(MonsterSlot(1)) | ActorId::Monster(_) => {
                generated.push(build_chocobo_enemy_action_line(
                    action_kind,
                    slot_index,
                    &cursor_state.state,
                )?);
                break;
            }
            ActorId::Character(character) => {
                let action_line = build_chocobo_character_filler_line(
                    &cursor_state.state,
                    character,
                    action_kind,
                    slot_index,
                )?;
                cursor_state.apply_line(&action_line);
                generated.push(action_line);
            }
        }
    }
    if generated.last().is_none_or(|line| !line.starts_with("m1 ")) {
        return Err(ApiError::BadRequest(
            "Stopped after 50 steps without reaching the Chocobo Eater turn.".into(),
        ));
    }
    Ok(serde_json::to_string(&ChocoboActionResponse {
        insert_line,
        lines: generated,
    })?)
}

pub fn chocobo_swap_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    slot_index: usize,
    replacement: &str,
) -> Result<String, ApiError> {
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "chocobo_eater")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the chocobo_eater encounter first.".into())
        })?;
    let insert_line = get_chocobo_swap_insert_line(&encounter, cursor_line);
    let replacement = replacement.parse::<Character>().map_err(|_| {
        ApiError::BadRequest(format!(
            "Unknown party member '{replacement}' for Chocobo swap."
        ))
    })?;
    let state = simulate_to_chocobo_cursor_state(seed, input, insert_line).state;
    if !state.has_monster_party() {
        return Err(ApiError::BadRequest(
            "Could not determine the active Chocobo Eater encounter state at this cursor.".into(),
        ));
    }
    let mut party = state.party().to_vec();
    if slot_index >= party.len() {
        return Err(ApiError::BadRequest(
            "That party slot is not currently available.".into(),
        ));
    }
    if party.contains(&replacement) {
        return Err(ApiError::BadRequest(format!(
            "{} is already in the active party.",
            replacement.display_name()
        )));
    }
    if !party_swap_choices().contains(&replacement) {
        return Err(ApiError::BadRequest(format!(
            "{} cannot be swapped into the Chocobo Eater party.",
            replacement.display_name()
        )));
    }
    party[slot_index] = replacement;
    Ok(serde_json::to_string(&ChocoboActionResponse {
        insert_line,
        lines: vec![format!("party {}", party_to_initials(&party))],
    })?)
}

pub fn tanker_pattern_json(
    input: &str,
    cursor_line: usize,
    pattern: &str,
) -> Result<String, ApiError> {
    let raw_pattern = pattern
        .split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase();
    if !(2..=14).contains(&raw_pattern.len())
        || raw_pattern
            .chars()
            .any(|token| !matches!(token, 'a' | 'w' | 's' | 'd' | 'n' | '-'))
    {
        return Err(ApiError::BadRequest(
            "Use 2 to 14 letters with only a, w, s, d, n, or -.".into(),
        ));
    }
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "tanker")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the tanker encounter first.".into())
        })?;
    let lines = build_tanker_lines(&raw_pattern);
    let input_lines = input.lines().collect::<Vec<_>>();
    let tanker_slots = ["m5", "m7", "m6", "m2", "m3", "m4", "m8"];
    let mut matching_lines = Vec::new();
    let end_line = encounter.end_line.min(input_lines.len());
    let mut in_block_comment = false;
    for line_number in encounter.start_line..=end_line {
        let Some(line) = input_lines.get(line_number.saturating_sub(1)) else {
            continue;
        };
        if !route_line_is_active(line, &mut in_block_comment) {
            continue;
        }
        let token = line
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if tanker_slots.contains(&token.as_str()) {
            matching_lines.push(line_number);
        }
    }
    let (start_line, end_line) = matching_lines
        .first()
        .zip(matching_lines.last())
        .map(|(start, end)| (*start, *end))
        .unwrap_or((cursor_line, cursor_line.saturating_sub(1)));

    Ok(serde_json::to_string(&TankerPatternResponse {
        start_line,
        end_line,
        lines,
    })?)
}

pub fn tros_attack_json(input: &str, cursor_line: usize, attack: &str) -> Result<String, ApiError> {
    let attack = attack.trim().to_ascii_lowercase();
    if !matches!(attack.as_str(), "attack" | "tentacles") {
        return Err(ApiError::BadRequest(
            "Use attack or tentacles for the Tros first attack.".into(),
        ));
    }
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "tros")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the tros encounter first.".into())
        })?;
    let input_lines = input.lines().collect::<Vec<_>>();
    let end_line = encounter.end_line.min(input_lines.len());
    let mut in_block_comment = false;
    let matching_line = (encounter.start_line..=end_line).find(|line_number| {
        input_lines
            .get(line_number.saturating_sub(1))
            .is_some_and(|line| {
                route_line_is_active(line, &mut in_block_comment)
                    && matches!(
                        line.trim().to_ascii_lowercase().as_str(),
                        "m1 attack" | "m1 tentacles"
                    )
            })
    });
    let (start_line, end_line) = matching_line
        .map(|line| (line, line))
        .unwrap_or((cursor_line, cursor_line.saturating_sub(1)));

    Ok(serde_json::to_string(&TankerPatternResponse {
        start_line,
        end_line,
        lines: vec![format!("m1 {attack}")],
    })?)
}

pub fn garuda1_attacks_json(
    input: &str,
    cursor_line: usize,
    attacks: &str,
) -> Result<String, ApiError> {
    let attacks = attacks
        .split(',')
        .map(|attack| attack.trim().to_ascii_lowercase())
        .filter(|attack| !attack.is_empty())
        .collect::<Vec<_>>();
    if attacks.len() != 5
        || attacks
            .iter()
            .any(|attack| !matches!(attack.as_str(), "attack" | "sonic_boom"))
    {
        return Err(ApiError::BadRequest(
            "Use exactly 5 Garuda 1 attacks: attack or sonic_boom.".into(),
        ));
    }
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "garuda_1")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the garuda_1 encounter first.".into())
        })?;
    let replacement_lines = attacks
        .iter()
        .map(|attack| format!("m1 {attack}"))
        .collect::<Vec<_>>();
    replace_matching_helper_lines(input, cursor_line, &encounter, &replacement_lines, |line| {
        matches!(
            line,
            "m1 attack" | "m1 sonic_boom" | "#m1 attack" | "#m1 sonic_boom"
        )
    })
}

pub fn lancet_tutorial_timing_json(
    input: &str,
    cursor_line: usize,
    timing: &str,
) -> Result<String, ApiError> {
    let timing = timing.trim().to_ascii_lowercase();
    if !matches!(timing.as_str(), "before" | "after") {
        return Err(ApiError::BadRequest(
            "Use before or after for the Lancet Tutorial timing.".into(),
        ));
    }
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "lancet_tutorial")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the lancet_tutorial encounter first.".into())
        })?;
    let input_lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(input_lines.len());
    let mut in_block_comment = false;
    let mut before_line = None;
    let mut after_line = None;
    for line_number in encounter.start_line..=encounter_end {
        let Some(line) = input_lines.get(line_number.saturating_sub(1)) else {
            continue;
        };
        if !route_line_is_active(line, &mut in_block_comment) {
            continue;
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "m1 seed_cannon_kimahri" | "#m1 seed_cannon_kimahri" => before_line = Some(line_number),
            "m1 seed_cannon" | "#m1 seed_cannon" => after_line = Some(line_number),
            _ => {}
        }
    }
    let (Some(before_line), Some(after_line)) = (before_line, after_line) else {
        return Err(ApiError::BadRequest(
            "Could not find both Ragora attack lines in the current encounter.".into(),
        ));
    };
    let mut output = input_lines
        .get(encounter.start_line.saturating_sub(1)..encounter_end)
        .unwrap_or_default()
        .iter()
        .map(|line| (*line).to_string())
        .collect::<Vec<_>>();
    let before_index = before_line - encounter.start_line;
    let after_index = after_line - encounter.start_line;
    if timing == "before" {
        output[before_index] = "m1 seed_cannon_kimahri".to_string();
        output[after_index] = "#m1 seed_cannon".to_string();
    } else {
        output[before_index] = "#m1 seed_cannon_kimahri".to_string();
        output[after_index] = "m1 seed_cannon".to_string();
    }

    Ok(serde_json::to_string(&TankerPatternResponse {
        start_line: encounter.start_line,
        end_line: encounter_end,
        lines: output,
    })?)
}

pub fn garuda2_attack_json(
    seed: u32,
    input: &str,
    cursor_line: usize,
    attack: &str,
) -> Result<String, ApiError> {
    let attack = attack.trim().to_ascii_lowercase();
    if !matches!(attack.as_str(), "attack" | "sonic_boom" | "does_nothing") {
        return Err(ApiError::BadRequest(
            "Use attack, sonic_boom, or does_nothing for Garuda 2.".into(),
        ));
    }
    let encounter = current_encounter_at_line(input, cursor_line)
        .filter(|encounter| encounter.name == "garuda_2")
        .ok_or_else(|| {
            ApiError::BadRequest("Move the cursor into the garuda_2 encounter first.".into())
        })?;
    let replacement_lines = if attack == "does_nothing" {
        Vec::new()
    } else {
        vec![format!("m1 {attack}")]
    };
    let payload = replace_matching_helper_lines(
        input,
        cursor_line,
        &encounter,
        &replacement_lines,
        |line| {
            matches!(
                line,
                "m1 attack" | "m1 sonic_boom" | "#m1 attack" | "#m1 sonic_boom"
            )
        },
    )?;
    let response: TankerPatternResponse = serde_json::from_str(&payload)?;
    if response.lines.is_empty() || response.end_line >= response.start_line {
        return Ok(payload);
    }
    let insert_line = garuda2_insert_line(seed, input, &encounter);
    Ok(serde_json::to_string(&TankerPatternResponse {
        start_line: insert_line,
        end_line: insert_line.saturating_sub(1),
        lines: response.lines,
    })?)
}

fn replace_matching_helper_lines<F>(
    input: &str,
    cursor_line: usize,
    encounter: &ctb::EncounterBlock,
    replacement_lines: &[String],
    mut predicate: F,
) -> Result<String, ApiError>
where
    F: FnMut(&str) -> bool,
{
    let input_lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(input_lines.len());
    let mut in_block_comment = false;
    let mut matching_lines = Vec::new();
    for line_number in encounter.start_line..=encounter_end {
        let Some(line) = input_lines.get(line_number.saturating_sub(1)) else {
            continue;
        };
        if !route_line_is_active(line, &mut in_block_comment) {
            continue;
        }
        if predicate(&line.trim().to_ascii_lowercase()) {
            matching_lines.push(line_number);
        }
    }
    if matching_lines.is_empty() {
        return Ok(serde_json::to_string(&TankerPatternResponse {
            start_line: cursor_line,
            end_line: cursor_line.saturating_sub(1),
            lines: replacement_lines.to_vec(),
        })?);
    }

    let encounter_lines = input_lines
        .get(encounter.start_line.saturating_sub(1)..encounter_end)
        .unwrap_or_default();
    let first_relative = matching_lines[0] - encounter.start_line;
    let mut output = Vec::new();
    let mut replacement_index = 0;
    for (index, line) in encounter_lines.iter().enumerate() {
        let line_number = encounter.start_line + index;
        if matching_lines.contains(&line_number) {
            if let Some(replacement) = replacement_lines.get(replacement_index) {
                output.push(replacement.clone());
                replacement_index += 1;
            }
        } else {
            output.push((*line).to_string());
        }
    }
    let remaining = replacement_lines
        .iter()
        .skip(replacement_index)
        .cloned()
        .collect::<Vec<_>>();
    if !remaining.is_empty() {
        let insertion_index = (first_relative + replacement_lines.len()).min(output.len());
        output.splice(insertion_index..insertion_index, remaining);
    }

    Ok(serde_json::to_string(&TankerPatternResponse {
        start_line: encounter.start_line,
        end_line: encounter_end,
        lines: output,
    })?)
}

fn garuda2_insert_line(seed: u32, input: &str, encounter: &ctb::EncounterBlock) -> usize {
    find_insert_line_from_opening_icv(seed, input, encounter, "M1")
        .or_else(|| {
            find_first_active_encounter_line(input, encounter, |line| line == "lulu escape")
        })
        .or_else(|| {
            find_first_active_encounter_line(input, encounter, |line| line == "yuna escape")
                .map(|line| line + 1)
        })
        .unwrap_or(encounter.end_line + 1)
}

fn find_insert_line_from_opening_icv(
    seed: u32,
    input: &str,
    encounter: &ctb::EncounterBlock,
    monster_token: &str,
) -> Option<usize> {
    let tokens = opening_icv_tokens(seed, input, encounter.index);
    if !tokens.iter().any(|token| token == monster_token) {
        return None;
    }
    let mut party_actions_before_monster = 0;
    for token in tokens {
        if token == monster_token {
            break;
        }
        if !token.starts_with('M') {
            party_actions_before_monster += 1;
        }
    }

    let input_lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(input_lines.len());
    let mut seen_party_actions = 0;
    let mut in_block_comment = false;
    for line_number in encounter.start_line.saturating_add(1)..=encounter_end {
        let Some(line) = input_lines.get(line_number.saturating_sub(1)) else {
            continue;
        };
        if !route_line_is_active(line, &mut in_block_comment) {
            continue;
        }
        let stripped = line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let Some(token) = stripped.split_whitespace().next() else {
            continue;
        };
        if token.parse::<Character>().is_err() {
            continue;
        }
        if seen_party_actions == party_actions_before_monster {
            return Some(line_number);
        }
        seen_party_actions += 1;
    }
    Some(encounter_end + 1)
}

fn opening_icv_tokens(seed: u32, input: &str, encounter_index: usize) -> Vec<String> {
    let prepared = prepare_action_lines(input);
    let mut state = SimulationState::new(seed);
    let mut encounter_ordinal = 0;
    let mut in_block_comment = false;
    for raw_line in prepared.lines {
        if !route_line_is_active(&raw_line, &mut in_block_comment) {
            continue;
        }
        let stripped = raw_line.trim();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        let is_encounter = matches!(
            parse_raw_action_line(&raw_line),
            ParsedCommand::Encounter { .. }
        );
        let rendered = state.execute_raw_line(&raw_line);
        if is_encounter {
            encounter_ordinal += 1;
            if encounter_ordinal == encounter_index {
                return ctb_tokens_from_rendered_encounter(&rendered);
            }
        }
    }
    Vec::new()
}

fn ctb_tokens_from_rendered_encounter(rendered: &str) -> Vec<String> {
    let Some(summary) = rendered.rsplit(" | ").next() else {
        return Vec::new();
    };
    summary
        .split_whitespace()
        .filter_map(|token| token.split_once('[').map(|(name, _)| name.to_string()))
        .collect()
}

fn find_first_active_encounter_line<F>(
    input: &str,
    encounter: &ctb::EncounterBlock,
    mut predicate: F,
) -> Option<usize>
where
    F: FnMut(&str) -> bool,
{
    let input_lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(input_lines.len());
    let mut in_block_comment = false;
    for line_number in encounter.start_line..=encounter_end {
        let line = input_lines.get(line_number.saturating_sub(1))?;
        if !route_line_is_active(line, &mut in_block_comment) {
            continue;
        }
        if predicate(&line.trim().to_ascii_lowercase()) {
            return Some(line_number);
        }
    }
    None
}

fn build_tanker_lines(pattern: &str) -> Vec<String> {
    let slot_order = [
        "m5", "m7", "m6", "m2", "m3", "m4", "m8", "m5", "m7", "m6", "m2", "m3", "m4", "m8",
    ];
    let mut actions = vec!["does_nothing"; slot_order.len()];
    let tokens = pattern.chars().collect::<Vec<_>>();
    let mut position = 0;
    let mut index = 0;
    while index < tokens.len() && position < slot_order.len() {
        match tokens[index] {
            'w' | 's' => {
                actions[position] = "wings_flicker";
                let paired_position = position + 7;
                if paired_position < slot_order.len() {
                    actions[paired_position] = "spines";
                    position = paired_position + 1;
                } else {
                    position += 1;
                }
                if tokens[index] == 'w' && index + 1 < tokens.len() && tokens[index + 1] == 's' {
                    index += 1;
                }
            }
            token => {
                actions[position] = map_tanker_action_token(token);
                position += 1;
            }
        }
        index += 1;
    }

    let mut lines = Vec::new();
    for block_index in 0..2 {
        let start = block_index * 7;
        let end = start + 7;
        for index in start..end {
            lines.push(format!("{} {}", slot_order[index], actions[index]));
        }
        if block_index == 0 {
            lines.push(String::new());
        }
    }
    lines
}

fn map_tanker_action_token(token: char) -> &'static str {
    match token {
        'a' => "attack",
        'w' => "wings_flicker",
        's' => "spines",
        'd' | 'n' | '-' => "does_nothing",
        _ => "does_nothing",
    }
}

pub fn tracker_default_json(tracker: &str, seed: u32) -> Result<String, ApiError> {
    let response = match tracker {
        "drops" => TrackerDefaultResponse {
            tracker: tracker.to_string(),
            seed,
            input: DROPS_NOTES.to_string(),
            input_filename: "drops_notes.txt",
            output_filename: "drops_output.txt",
            sliders: None,
        },
        "encounters" => TrackerDefaultResponse {
            tracker: tracker.to_string(),
            seed,
            input: build_encounters_default_input(),
            input_filename: "encounters_notes.csv",
            output_filename: "encounters_output.txt",
            sliders: Some(parse_encounter_sliders()),
        },
        _ => {
            return Err(ApiError::BadRequest(format!(
                "Unknown tracker type '{tracker}'."
            )))
        }
    };
    Ok(serde_json::to_string(&response)?)
}

pub fn tracker_render_json(tracker: &str, seed: u32, input: &str) -> Result<String, ApiError> {
    let started = tracker_timer_start();
    let response = match tracker {
        "encounters" => TrackerRenderResponse {
            tracker: tracker.to_string(),
            output: render_encounters_tracker(seed, input),
            duration_seconds: tracker_duration_seconds(started),
            output_filename: "encounters_output.txt",
        },
        "drops" => TrackerRenderResponse {
            tracker: tracker.to_string(),
            output: render_drops_tracker(seed, input),
            duration_seconds: tracker_duration_seconds(started),
            output_filename: "drops_output.txt",
        },
        _ => {
            return Err(ApiError::BadRequest(format!(
                "Unknown tracker type '{tracker}'."
            )))
        }
    };
    Ok(serde_json::to_string(&response)?)
}

pub fn no_encounters_routes_json(
    seed: u32,
    input: &str,
    start_line: usize,
    encounters_input: Option<&str>,
    encounters_output: Option<&str>,
) -> Result<String, ApiError> {
    let (output, edited_input) = build_no_encounters_routes_output(
        seed,
        input,
        start_line,
        encounters_input,
        encounters_output,
    );
    Ok(serde_json::to_string(&NoEncountersRoutesResponse {
        output,
        edited_input,
    })?)
}

fn render_encounters_tracker(seed: u32, input: &str) -> String {
    let render_input = protect_tracker_block_comment_repeats(input);
    let rendered = ctb::render_ctb(seed, &render_input);
    let padding = !tracker_has_active_nopadding(input);
    let output = edit_encounters_tracker_output(&rendered.output, padding);
    if output.is_empty() {
        output
    } else {
        format!("{}\n", output.trim_end())
    }
}

impl DropsInventory {
    fn new() -> Self {
        let capacity = data::item_names_in_order().len();
        let mut inventory = Self {
            items: vec![None; capacity],
            quantities: vec![0; capacity],
        };
        inventory.add("Potion", 10);
        inventory.add("Phoenix Down", 3);
        inventory
    }

    fn add(&mut self, item: &str, amount: i32) {
        if amount <= 0 {
            return;
        }
        if let Some(index) = self.index_of(item) {
            self.quantities[index] += amount;
            return;
        }
        if let Some(index) = self.items.iter().position(Option::is_none) {
            self.items[index] = Some(item.to_string());
            self.quantities[index] = amount;
        }
    }

    fn remove(&mut self, item: &str, amount: i32) -> Result<(), String> {
        let Some(index) = self.index_of(item) else {
            return Err(format!("{item} is not in the inventory"));
        };
        if amount > self.quantities[index] {
            return Err(format!("Not enough {item} in inventory"));
        }
        self.quantities[index] -= amount;
        if self.quantities[index] == 0 {
            self.items[index] = None;
        }
        Ok(())
    }

    fn switch(&mut self, first: usize, second: usize) -> Result<(String, String), String> {
        if first >= self.items.len() || second >= self.items.len() {
            return Err(format!(
                "Inventory slot needs to be between 1 and {}",
                self.items.len()
            ));
        }
        self.items.swap(first, second);
        self.quantities.swap(first, second);
        Ok((self.item_label(first), self.item_label(second)))
    }

    fn autosort(&mut self) {
        let mut sorted_items = Vec::new();
        let mut sorted_quantities = Vec::new();
        for item in data::item_names_in_order() {
            if let Some(index) = self.index_of(item) {
                sorted_items.push(self.items[index].clone());
                sorted_quantities.push(self.quantities[index]);
            }
        }
        let empty = self.items.len() - sorted_items.len();
        sorted_items.extend((0..empty).map(|_| None));
        sorted_quantities.extend((0..empty).map(|_| 0));
        self.items = sorted_items;
        self.quantities = sorted_quantities;
    }

    fn to_tracker_string(&self) -> String {
        let mut left_padding = 0;
        let mut right_padding = 0;
        for (index, item) in self.items.iter().enumerate() {
            let label = item
                .as_ref()
                .map(|item| format!("{item} {}", self.quantities[index]))
                .unwrap_or_else(|| "None 0".to_string());
            if index % 2 == 0 {
                left_padding = left_padding.max(label.len());
            } else {
                right_padding = right_padding.max(label.len());
            }
        }
        let mut rows = Vec::new();
        for index in (0..self.items.len()).step_by(2) {
            let left = self.slot_text(index, left_padding);
            let right = self.slot_text(index + 1, right_padding);
            rows.push(format!("| {left} | {right} |"));
        }
        let empty_row = format!("| {:left_padding$} | {:right_padding$} |", "-", "-");
        while rows.last().is_some_and(|row| row == &empty_row) {
            rows.pop();
        }
        let divider = format!(
            "+-{}-+-{}-+",
            "-".repeat(left_padding),
            "-".repeat(right_padding)
        );
        format!("{divider}\n{}\n{divider}", rows.join("\n"))
    }

    fn index_of(&self, item: &str) -> Option<usize> {
        self.items
            .iter()
            .position(|stored| stored.as_deref() == Some(item))
    }

    fn item_label(&self, index: usize) -> String {
        self.items
            .get(index)
            .and_then(|item| item.clone())
            .unwrap_or_else(|| "None".to_string())
    }

    fn slot_text(&self, index: usize, width: usize) -> String {
        let Some(item) = self.items.get(index).and_then(|item| item.as_ref()) else {
            return format!("{:width$}", "-");
        };
        let quantity_width = width.saturating_sub(item.len());
        format!("{item}{:quantity_width$}", self.quantities[index])
    }
}

fn render_drops_tracker(seed: u32, input: &str) -> String {
    let input = strip_generated_drops_preamble(input);
    let mut rng = FfxRngTracker::new(seed);
    let mut party = vec![Character::Tidus, Character::Auron];
    let mut ap_state = drops_initial_ap_state();
    let mut inventory = DropsInventory::new();
    let mut equipment_inventory = Vec::new();
    let mut equipment_drops = 0;
    let mut gil = 300;
    let mut lines = Vec::new();
    let mut multiline_comment = false;
    let padding = !tracker_has_active_nopadding(input);
    for raw_line in expand_tracker_repeats(input) {
        let stripped = raw_line.trim();
        if stripped.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            let line = raw_line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
            lines.push(format!("# {line}"));
            if stripped.ends_with("*/") {
                multiline_comment = false;
            }
            continue;
        }
        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            ensure_tracker_blank_line(&mut lines);
            continue;
        }
        if raw_line.starts_with('#') {
            lines.push(raw_line.to_string());
            continue;
        }
        if raw_line.starts_with('/') {
            if raw_line == "/usage" || raw_line.starts_with("/usage ") {
                lines.extend(DROPS_USAGE_TEXT.lines().map(ToOwned::to_owned));
            } else if raw_line == "/macro" || raw_line.starts_with("/macro ") {
                lines.push("Error: Possible macros are ".to_string());
            } else if raw_line == "/nopadding" {
                lines.push(format!("Command: {raw_line}"));
            } else {
                lines.push(format!("Command: {raw_line}"));
            }
            continue;
        }
        let normalized_line = trimmed.to_ascii_lowercase();
        let mut words = normalized_line.split_whitespace().collect::<Vec<_>>();
        if words
            .first()
            .is_some_and(|monster| data::monster_stats(monster).is_some())
        {
            words.insert(0, "kill");
        }
        let Some(rendered) = render_drops_line(
            &mut rng,
            &mut party,
            &mut ap_state,
            &mut inventory,
            &mut equipment_inventory,
            &mut equipment_drops,
            &mut gil,
            &words,
        ) else {
            lines.push(format!("Error: Impossible to parse \"{raw_line}\""));
            continue;
        };
        lines.push(rendered);
    }
    let raw_output = lines.join("\n");
    let visible_output = hide_tracker_output_before_marker(&raw_output);
    let output = if padding {
        let visible_lines = visible_output
            .lines()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        pad_drops_tracker_output(&visible_lines)
    } else {
        visible_output
    }
    .replace("Drops: ", "");
    if output.trim().is_empty() {
        String::new()
    } else {
        format!("{}\n", output.trim_end())
    }
}

fn strip_generated_drops_preamble(input: &str) -> &str {
    if input.lines().next().is_some_and(|line| {
        let normalized = line.trim().to_ascii_lowercase();
        normalized == "drop search result" || normalized == "drop analysis result"
    }) {
        if let Some((_, route)) = input.split_once("Resolved drop route") {
            return route.trim_start_matches(['\r', '\n']);
        }
    }
    input
}

fn build_no_encounters_routes_output(
    seed: u32,
    input: &str,
    start_line: usize,
    encounters_input: Option<&str>,
    encounters_output: Option<&str>,
) -> (String, String) {
    let input_lines = input.lines().collect::<Vec<_>>();
    if input_lines.is_empty() {
        return (
            "No Encounters search: input is empty.".to_string(),
            String::new(),
        );
    }
    let start_line = start_line.clamp(1, input_lines.len());
    let first_ghost_line = drops_search_first_ghost_line(&input_lines, start_line);
    if first_ghost_line.is_none() {
        return (
            "No Encounters search: no Ghost line was found below the cursor.".to_string(),
            input.to_string(),
        );
    }
    let start_section_name = drops_search_start_section_name(&input_lines, start_line);
    let future_from_output = encounters_output
        .filter(|output| !output.trim().is_empty())
        .and_then(|output| parse_exact_future_encounter_output(output, &start_section_name));
    if encounters_output.is_some_and(|output| !output.trim().is_empty()) {
        if future_from_output.is_none() && encounters_input.is_none() {
            return (
                "No Encounters search: the Encounters tracker output could not be parsed. Refresh the Encounters tracker and try again.".to_string(),
                input.to_string(),
            );
        }
    }
    let mut future_from_input = None;
    if let Some(encounters_input) = encounters_input.filter(|input| !input.trim().is_empty()) {
        if future_from_output.is_none() {
            future_from_input =
                parse_exact_future_encounter_output(encounters_input, &start_section_name);
            if future_from_input.is_none() {
                if !encounters_input_has_exact_future_rows(encounters_input) {
                    return (
                        "No Encounters search: the Encounters tracker input could not be parsed. Refresh the Encounters tracker and try again.".to_string(),
                        input.to_string(),
                    );
                }
                let rendered = render_encounters_tracker(seed, encounters_input);
                future_from_input =
                    parse_exact_future_encounter_output(&rendered, &start_section_name);
            }
            if future_from_input.is_none() {
                return (
                    "No Encounters search: the Encounters tracker input could not be parsed. Refresh the Encounters tracker and try again.".to_string(),
                    input.to_string(),
                );
            }
        }
    }
    let future_source = if future_from_output.is_some() {
        "exact-encounters-output"
    } else if future_from_input.is_some() {
        "exact-encounters-input"
    } else {
        "notes-fallback"
    };
    let future_from_notes;
    let future_from_exact;
    let future = if let Some(future) = future_from_output.as_ref().or(future_from_input.as_ref()) {
        future_from_exact = FutureEncounterSearchView::from_branch(future.clone());
        Some(&future_from_exact)
    } else {
        let extra_budget = if start_section_name.is_empty() { 0 } else { 3 };
        future_from_notes =
            build_notes_fallback_future_view(seed, &start_section_name, extra_budget);
        future_from_notes.as_ref()
    };
    let future_rows = future.map(|future| future.rows_len);
    let future_sections_debug = future
        .map(format_future_section_maxima_debug)
        .unwrap_or_else(|| "Future random sections: none.".to_string());
    let preview = build_no_encounters_preview(
        seed,
        input,
        start_line,
        first_ghost_line,
        future_source,
        &future_sections_debug,
        future_rows,
        future,
    );
    (
        preview.output,
        preview.edited_input.unwrap_or_else(|| input.to_string()),
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoEncountersPreview {
    output: String,
    edited_input: Option<String>,
}

fn build_no_encounters_preview(
    seed: u32,
    edited_input: &str,
    start_line: usize,
    first_ghost_line: Option<usize>,
    future_source: &str,
    future_sections_debug: &str,
    future_rows: Option<usize>,
    future: Option<&FutureEncounterSearchView>,
) -> NoEncountersPreview {
    let exact_future_validation = future_source != "notes-fallback";
    let mut search = collect_no_encounters_results(
        seed,
        edited_input,
        start_line,
        future,
        exact_future_validation,
    );
    let deep_search = NoEncountersResultSearch::empty();
    let mut synth_search = NoEncountersResultSearch::empty();
    let best_random_penalty = search
        .results
        .iter()
        .map(|result| result.random_penalty)
        .min();
    let should_run_synth_search = future
        .filter(|future| !future.section_maxima.is_empty())
        .is_some()
        && (best_random_penalty.is_none() || best_random_penalty.unwrap_or_default() > 0);
    if should_run_synth_search && search.results.is_empty() {
        synth_search = collect_synthetic_no_encounters_results(
            seed,
            edited_input,
            start_line,
            future,
            exact_future_validation,
        );
        search.results = merge_no_encounters_results(&search.results, &synth_search.results)
            .into_iter()
            .take(10)
            .collect();
    }
    let mut mode_flags = vec![future_source.to_string()];
    if deep_search.tested_routes > 0 {
        mode_flags.push("deep-search".to_string());
    }
    if synth_search.tested_routes > 0 {
        mode_flags.push("synthetic-search".to_string());
    }
    if should_run_synth_search {
        mode_flags.push("prefer-guaranteed".to_string());
    }
    if search.search_truncated || deep_search.search_truncated {
        mode_flags.push("budgeted-search".to_string());
    }
    search.results.sort_by(no_encounters_result_sort_key);
    search.results.truncate(10);

    let mut lines = vec!["No Encounters search:".to_string()];
    if deep_search.tested_routes > 0 || synth_search.tested_routes > 0 {
        let mut tested = format!(
            "Cursor line {start_line}, first Ghost line {}, tested {} default route{} + {} deep route{}",
            first_ghost_line.unwrap_or_default(),
            search.tested_routes,
            if search.tested_routes == 1 { "" } else { "s" },
            deep_search.tested_routes,
            if deep_search.tested_routes == 1 { "" } else { "s" },
        );
        if synth_search.tested_routes > 0 {
            tested.push_str(&format!(
                " + {} synthetic route{}",
                synth_search.tested_routes,
                if synth_search.tested_routes == 1 {
                    ""
                } else {
                    "s"
                }
            ));
        }
        tested.push('.');
        lines.push(tested);
    } else {
        push_no_encounters_context_line(
            &mut lines,
            start_line,
            first_ghost_line,
            Some(search.tested_routes),
        );
    }
    lines.push(format!("Search mode: {}.", mode_flags.join(", ")));
    lines.push(future_sections_debug.to_string());
    if search.search_truncated {
        lines.push(format!(
            "Large-window best-effort search: default pass explored {} of {} optional subsets, prioritizing actions closest to Ghost.",
            search.tested_option_sets, search.total_option_sets
        ));
    }
    if let Some(best_random_penalty) = search
        .results
        .iter()
        .map(|result| result.random_penalty)
        .min()
    {
        if best_random_penalty == 0 {
            lines.push("Guaranteed-only route(s) found before optional random routes.".to_string());
        } else {
            lines.push(format!(
                "No guaranteed-only route found; best result uses {best_random_penalty} random encounter row{}.",
                if best_random_penalty == 1 { "" } else { "s" }
            ));
        }
    }
    if deep_search.tested_routes > 0 {
        lines.push(
            "Deep search was used after the default path found no valid first-Ghost NEA route."
                .to_string(),
        );
    }
    if synth_search.tested_routes > 0 {
        lines.push("Synthetic future-action search was used after the default search found no valid route.".to_string());
    }
    if search.results.is_empty() {
        lines.push(
            "No route found that gives a No Encounters armor from the first Ghost.".to_string(),
        );
    } else {
        for (index, result) in search.results.iter().take(10).enumerate() {
            lines.push(format!(
                "{}. add {} action{} | random rows: {} | {}",
                index + 1,
                result.added_count,
                if result.added_count == 1 { "" } else { "s" },
                result.random_penalty,
                result.equipment_text
            ));
            if result.added_lines.is_empty() {
                lines.push("   No extra actions needed".to_string());
            } else {
                for line in &result.added_lines {
                    lines.push(format!("   {line}"));
                }
            }
        }
    }
    if let Some(rows) = future_rows {
        lines.push(format!(
            "Future encounters parsed: {rows} row(s) through Ghost."
        ));
    }
    NoEncountersPreview {
        output: lines.join("\n"),
        edited_input: search
            .results
            .first()
            .map(|result| result.route_input.clone()),
    }
}

fn push_no_encounters_context_line(
    lines: &mut Vec<String>,
    start_line: usize,
    first_ghost_line: Option<usize>,
    tested_routes: Option<usize>,
) {
    let Some(first_ghost_line) = first_ghost_line else {
        return;
    };
    let Some(tested_routes) = tested_routes else {
        lines.push(format!(
            "Cursor line {start_line}, first Ghost line {first_ghost_line}."
        ));
        return;
    };
    lines.push(format!(
        "Cursor line {start_line}, first Ghost line {first_ghost_line}, tested {tested_routes} route(s)."
    ));
}

fn find_no_encounters_ghost_drop(rendered_drops: &str) -> Option<String> {
    rendered_drops
        .lines()
        .find(|line| {
            line.trim_start().starts_with("Ghost")
                && line.contains('|')
                && line.contains("Equipment #")
                && line.contains("No Encounters")
        })
        .map(|line| {
            line.split_once("Equipment #")
                .map(|(_, equipment)| format!("Equipment #{}", equipment.trim()))
                .unwrap_or_else(|| line.trim().to_string())
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
struct NoEncountersGhostRouteCandidate {
    ghost_kills: usize,
    pre_ghost_deaths: usize,
    repeat_line: String,
    uncommented_line: Option<(usize, String)>,
    drop: String,
    route_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
struct NoEncountersGhostRouteSearch {
    candidate: Option<NoEncountersGhostRouteCandidate>,
    candidates_tested: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct DropsSearchRouteLine {
    source_line: usize,
    raw_line: String,
    normalized_line: String,
    section_name: String,
    optional: bool,
    is_ghost: bool,
    command: String,
    monster_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoEncountersRouteResult {
    added_count: usize,
    random_penalty: usize,
    route_len: usize,
    tested_index: usize,
    equipment_text: String,
    added_lines: Vec<String>,
    route_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NoEncountersResultSearch {
    results: Vec<NoEncountersRouteResult>,
    tested_routes: usize,
    tested_option_sets: usize,
    total_option_sets: usize,
    search_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SearchSynthesisFamily {
    anchor_index: usize,
    route_lines: Vec<DropsSearchRouteLine>,
    description_lines: Vec<String>,
    max_uses: usize,
}

impl NoEncountersResultSearch {
    fn empty() -> Self {
        Self {
            results: Vec::new(),
            tested_routes: 0,
            tested_option_sets: 0,
            total_option_sets: 0,
            search_truncated: false,
        }
    }
}

fn no_encounters_result_sort_key(
    left: &NoEncountersRouteResult,
    right: &NoEncountersRouteResult,
) -> std::cmp::Ordering {
    let left_score = no_encounters_added_line_score(&left.added_lines);
    let right_score = no_encounters_added_line_score(&right.added_lines);
    (
        left.random_penalty,
        left.added_count,
        left.route_len,
        std::cmp::Reverse(left_score),
        left.tested_index,
    )
        .cmp(&(
            right.random_penalty,
            right.added_count,
            right.route_len,
            std::cmp::Reverse(right_score),
            right.tested_index,
        ))
}

fn no_encounters_added_line_score(lines: &[String]) -> usize {
    lines
        .iter()
        .filter_map(|line| {
            line.replace(':', " ")
                .split_whitespace()
                .find_map(|word| word.parse::<usize>().ok())
        })
        .sum()
}

fn merge_no_encounters_results(
    left: &[NoEncountersRouteResult],
    right: &[NoEncountersRouteResult],
) -> Vec<NoEncountersRouteResult> {
    let mut merged: HashMap<(String, Vec<String>), NoEncountersRouteResult> = HashMap::new();
    for result in left.iter().chain(right) {
        let key = (result.equipment_text.clone(), result.added_lines.clone());
        let replace = merged
            .get(&key)
            .is_none_or(|current| no_encounters_result_sort_key(result, current).is_lt());
        if replace {
            merged.insert(key, result.clone());
        }
    }
    let mut results = merged.into_values().collect::<Vec<_>>();
    results.sort_by(no_encounters_result_sort_key);
    results
}

fn collect_no_encounters_results(
    seed: u32,
    edited_input: &str,
    start_line: usize,
    future: Option<&FutureEncounterSearchView>,
    exact_future_validation: bool,
) -> NoEncountersResultSearch {
    let lines = edited_input
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let (edited_prefix, pre_ghost_lines, ghost_lines) = drops_search_window(&lines, start_line);
    if ghost_lines.is_empty() {
        return NoEncountersResultSearch::empty();
    }
    let active_ghost_line = ghost_lines.iter().find(|line| !line.optional);
    let optional_lines = pre_ghost_lines
        .iter()
        .filter(|line| line.optional && !is_nea_excluded_optional(&line.normalized_line))
        .filter(|line| route_line_matches_future_counts(line, future))
        .cloned()
        .collect::<Vec<_>>();
    let linked_optionals = linked_drops_optionals(&pre_ghost_lines);
    let paired_lines = linked_optionals
        .values()
        .flat_map(|lines| lines.iter().cloned())
        .collect::<HashSet<_>>();
    let mut filtered_optionals = optional_lines
        .into_iter()
        .filter(|line| !paired_lines.contains(line))
        .collect::<Vec<_>>();
    filtered_optionals.reverse();

    let total_option_sets = if filtered_optionals.len() >= usize::BITS as usize {
        usize::MAX
    } else {
        1usize << filtered_optionals.len()
    };
    let subset_budget = 20_000usize;
    let mut tested_option_sets = 0usize;
    let mut tested_routes = 0usize;
    let mut results = Vec::new();
    let mut seen_routes = HashSet::new();
    let mut stop = false;

    for optional_count in 0..=filtered_optionals.len() {
        let combinations = optional_index_combinations(
            filtered_optionals.len(),
            optional_count,
            subset_budget - tested_option_sets,
        );
        for indexes in combinations {
            if tested_option_sets >= subset_budget {
                stop = true;
                break;
            }
            tested_option_sets += 1;
            let mut chosen = HashSet::new();
            for index in indexes {
                let optional = filtered_optionals[index].clone();
                chosen.insert(optional.clone());
                if let Some(linked) = linked_optionals.get(&optional) {
                    chosen.extend(linked.iter().cloned());
                }
            }
            let route_prefix = pre_ghost_lines
                .iter()
                .filter(|line| !line.optional || chosen.contains(*line))
                .cloned()
                .collect::<Vec<_>>();
            for ghost_line in &ghost_lines {
                let mut route_lines = route_prefix.clone();
                route_lines.push(ghost_line.clone());
                let route_key = no_encounters_future_path_key(&route_lines);
                if !seen_routes.insert(route_key) {
                    continue;
                }
                if !route_matches_future_counts(&route_lines, future) {
                    continue;
                }
                tested_routes += 1;
                if exact_future_validation && !route_matches_exact_future_rows(&route_lines, future)
                {
                    continue;
                }
                let route_text = drops_route_text(&edited_prefix, &route_lines);
                let Some(drop) =
                    find_no_encounters_ghost_drop(&render_drops_tracker(seed, &route_text))
                else {
                    continue;
                };
                let added_lines = route_prefix
                    .iter()
                    .filter(|line| line.optional)
                    .map(|line| format!("line {}: {}", line.source_line, line.raw_line))
                    .chain(ghost_line_change_description(active_ghost_line, ghost_line).into_iter())
                    .collect::<Vec<_>>();
                let route_input = apply_route_lines_to_original(&lines, &route_lines);
                results.push(NoEncountersRouteResult {
                    added_count: added_lines.len(),
                    random_penalty: future
                        .and_then(|future| {
                            let route_only = route_lines_text(&route_lines);
                            count_route_future_random_rows(&route_only, future)
                        })
                        .unwrap_or_default(),
                    route_len: route_lines.len(),
                    tested_index: tested_routes,
                    equipment_text: drop,
                    added_lines,
                    route_input,
                });
            }
        }
        if stop {
            break;
        }
    }

    results.sort_by(no_encounters_result_sort_key);
    results.truncate(10);
    NoEncountersResultSearch {
        results,
        tested_routes,
        tested_option_sets,
        total_option_sets,
        search_truncated: total_option_sets > subset_budget,
    }
}

fn optional_index_combinations(len: usize, count: usize, limit: usize) -> Vec<Vec<usize>> {
    fn build(
        len: usize,
        count: usize,
        start: usize,
        current: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
        limit: usize,
    ) {
        if out.len() >= limit {
            return;
        }
        if current.len() == count {
            out.push(current.clone());
            return;
        }
        let remaining = count - current.len();
        if len.saturating_sub(start) < remaining {
            return;
        }
        for index in start..=len - remaining {
            current.push(index);
            build(len, count, index + 1, current, out, limit);
            current.pop();
            if out.len() >= limit {
                break;
            }
        }
    }
    let mut out = Vec::new();
    build(len, count, 0, &mut Vec::new(), &mut out, limit);
    out
}

fn ghost_line_change_description(
    active_ghost_line: Option<&DropsSearchRouteLine>,
    ghost_line: &DropsSearchRouteLine,
) -> Option<String> {
    if active_ghost_line.is_none_or(|active| active.normalized_line != ghost_line.normalized_line) {
        Some(format!(
            "line {}: change ghost to {}",
            ghost_line.source_line, ghost_line.raw_line
        ))
    } else {
        None
    }
}

fn apply_route_lines_to_original(
    original_lines: &[String],
    route_lines: &[DropsSearchRouteLine],
) -> String {
    let mut output = original_lines.to_vec();
    for line in route_lines.iter().filter(|line| line.optional) {
        let index = line.source_line.saturating_sub(1);
        if let Some(slot) = output.get_mut(index) {
            *slot = line.raw_line.clone();
        }
    }
    output.join("\n")
}

fn route_line_matches_future_counts(
    line: &DropsSearchRouteLine,
    future: Option<&FutureEncounterSearchView>,
) -> bool {
    let Some(monster) = line.monster_name.as_ref() else {
        return true;
    };
    let Some(future) = future else {
        return true;
    };
    future_monster_count(&future.total_counts, monster) > 0
}

fn route_matches_future_counts(
    route_lines: &[DropsSearchRouteLine],
    future: Option<&FutureEncounterSearchView>,
) -> bool {
    let Some(future) = future else {
        return true;
    };
    let mut counts: HashMap<String, usize> = HashMap::new();
    for line in route_lines {
        if let Some(monster) = &line.monster_name {
            *counts.entry(monster.clone()).or_default() += 1;
        }
    }
    future.branches.iter().any(|branch| {
        counts
            .iter()
            .all(|(monster, count)| future_monster_count(&branch.total_counts, monster) >= *count)
    })
}

fn future_monster_count(counts: &HashMap<String, usize>, monster: &str) -> usize {
    counts.get(monster).copied().unwrap_or_else(|| {
        let target_family = monster_family(monster);
        counts
            .iter()
            .filter_map(|(name, count)| (monster_family(name) == target_family).then_some(*count))
            .max()
            .unwrap_or_default()
    })
}

fn route_matches_exact_future_rows(
    route_lines: &[DropsSearchRouteLine],
    future: Option<&FutureEncounterSearchView>,
) -> bool {
    let Some(future) = future else {
        return true;
    };
    let route_input = route_lines
        .iter()
        .map(|line| line.normalized_line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    future
        .branches
        .iter()
        .any(|branch| count_route_exact_future_random_rows(&route_input, branch).is_some())
}

fn route_lines_text(route_lines: &[DropsSearchRouteLine]) -> String {
    route_lines
        .iter()
        .map(|line| line.normalized_line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

fn no_encounters_future_path_key(route_lines: &[DropsSearchRouteLine]) -> Vec<String> {
    let last_index = route_lines.len().saturating_sub(1);
    route_lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            if index == last_index && line.monster_name.as_deref() == Some("ghost") {
                "kill ghost".to_string()
            } else {
                line.normalized_line.clone()
            }
        })
        .collect()
}

fn collect_synthetic_no_encounters_results(
    seed: u32,
    edited_input: &str,
    start_line: usize,
    future: Option<&FutureEncounterSearchView>,
    exact_future_validation: bool,
) -> NoEncountersResultSearch {
    let Some(future) = future else {
        return NoEncountersResultSearch::empty();
    };
    let lines = edited_input
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let (edited_prefix, pre_ghost_lines, ghost_lines) = drops_search_window(&lines, start_line);
    let synth_families = no_encounters_synthesis_families(&pre_ghost_lines, &future.section_maxima);
    if synth_families.is_empty() || ghost_lines.is_empty() {
        return NoEncountersResultSearch::empty();
    }
    let active_ghost_line = ghost_lines.iter().find(|line| !line.optional);
    let combos = no_encounters_synth_combos(&synth_families, 20_000);
    let mut results = Vec::new();
    let mut tested_routes = 0;
    let mut seen_routes = HashSet::new();
    let mut max_added_count_in_results = None;

    for counts in combos {
        let synth_added_count = counts
            .iter()
            .zip(&synth_families)
            .map(|(count, family)| count * family.route_lines.len())
            .sum::<usize>();
        if max_added_count_in_results.is_some_and(|max| synth_added_count > max) {
            break;
        }
        let (route_prefix, synth_descriptions) =
            build_synth_route_prefix(&pre_ghost_lines, &synth_families, &counts);
        for ghost_line in &ghost_lines {
            let mut route_lines = route_prefix.clone();
            route_lines.push(ghost_line.clone());
            let route_key = no_encounters_future_path_key(&route_lines);
            if !seen_routes.insert(route_key) {
                continue;
            }
            if !route_matches_future_counts(&route_lines, Some(future)) {
                continue;
            }
            tested_routes += 1;
            if exact_future_validation
                && !route_matches_exact_future_rows(&route_lines, Some(future))
            {
                continue;
            }
            let route_text = drops_route_text(&edited_prefix, &route_lines);
            let Some(drop) =
                find_no_encounters_ghost_drop(&render_drops_tracker(seed, &route_text))
            else {
                continue;
            };
            let mut added_lines = synth_descriptions.clone();
            if let Some(description) = ghost_line_change_description(active_ghost_line, ghost_line)
            {
                added_lines.push(description);
            }
            results.push(NoEncountersRouteResult {
                added_count: added_lines.len(),
                random_penalty: count_route_future_random_rows(&route_text, future)
                    .or_else(|| {
                        let route_only = route_lines_text(&route_lines);
                        count_route_future_random_rows(&route_only, future)
                    })
                    .unwrap_or_default(),
                route_len: route_lines.len(),
                tested_index: tested_routes,
                equipment_text: drop,
                added_lines,
                route_input: apply_route_lines_to_original(&lines, &route_lines),
            });
            if results.len() >= 10 {
                results.sort_by(no_encounters_result_sort_key);
                results.truncate(10);
                max_added_count_in_results = results.last().map(|result| result.added_count);
            }
        }
    }

    results.sort_by(no_encounters_result_sort_key);
    results.truncate(10);
    NoEncountersResultSearch {
        results,
        tested_routes,
        tested_option_sets: 0,
        total_option_sets: 0,
        search_truncated: false,
    }
}

fn no_encounters_synthesis_families(
    pre_ghost_lines: &[DropsSearchRouteLine],
    future_section_maxima: &HashMap<String, HashMap<String, usize>>,
) -> Vec<SearchSynthesisFamily> {
    let near_sections = last_random_sections_near_ghost(pre_ghost_lines, future_section_maxima, 2);
    let mut families = Vec::new();
    let mut seen = HashSet::new();
    for section_name in &near_sections {
        let Some((start_index, end_index)) = section_bounds(pre_ghost_lines, section_name) else {
            continue;
        };
        let Some(section_maxima) = future_section_maxima.get(section_name) else {
            continue;
        };
        for index in start_index..end_index {
            let line = &pre_ghost_lines[index];
            let Some(monster_name) = line.monster_name.as_ref() else {
                continue;
            };
            if !line.optional
                || future_monster_count(section_maxima, monster_name) == 0
                || is_nea_excluded_optional(&line.normalized_line)
            {
                continue;
            }
            let setup_line = immediate_setup_line(pre_ghost_lines, index, true);
            let followup_line = immediate_followup_line(pre_ghost_lines, index, true);
            let (anchor_index, route_lines, description_lines, max_uses) =
                if line.command == "steal" {
                    if let Some(followup_line) = followup_line {
                        (
                            index,
                            vec![line.clone(), followup_line.clone()],
                            vec![
                                format!("line {}: {}", line.source_line, line.raw_line),
                                format!(
                                    "line {}: {}",
                                    followup_line.source_line, followup_line.raw_line
                                ),
                            ],
                            1,
                        )
                    } else if monster_requires_immediate_steal_setup(Some(monster_name)) {
                        continue;
                    } else {
                        (
                            index,
                            vec![line.clone()],
                            vec![format!("line {}: {}", line.source_line, line.raw_line)],
                            1,
                        )
                    }
                } else if let Some(setup_line) = setup_line {
                    (
                        index - 1,
                        vec![setup_line.clone(), line.clone()],
                        vec![
                            format!("line {}: {}", setup_line.source_line, setup_line.raw_line),
                            format!("line {}: {}", line.source_line, line.raw_line),
                        ],
                        future_monster_count(section_maxima, monster_name).min(3),
                    )
                } else if monster_requires_immediate_steal_setup(Some(monster_name)) {
                    continue;
                } else {
                    (
                        index,
                        vec![line.clone()],
                        vec![format!("line {}: {}", line.source_line, line.raw_line)],
                        future_monster_count(section_maxima, monster_name).min(3),
                    )
                };
            if max_uses == 0 {
                continue;
            }
            let key = (
                anchor_index,
                route_lines
                    .iter()
                    .map(|line| line.normalized_line.clone())
                    .collect::<Vec<_>>(),
                max_uses,
            );
            if seen.insert(key) {
                families.push(SearchSynthesisFamily {
                    anchor_index,
                    route_lines,
                    description_lines,
                    max_uses,
                });
            }
        }

        let boss_anchor_index =
            section_boss_anchor_index(pre_ghost_lines, section_name, section_maxima);
        let repeat_end_index = boss_anchor_index.unwrap_or(end_index);
        let repeat_candidate = (start_index..repeat_end_index)
            .filter_map(|index| {
                let line = &pre_ghost_lines[index];
                line.monster_name
                    .as_ref()
                    .filter(|monster| {
                        !line.optional && future_monster_count(section_maxima, monster) > 0
                    })
                    .map(|_| (index, line))
            })
            .last();
        if let Some((index, line)) = repeat_candidate {
            if let Some(monster) = line.monster_name.as_ref() {
                let setup_line = immediate_setup_line(pre_ghost_lines, index, false);
                if !(setup_line.is_none() && monster_requires_immediate_steal_setup(Some(monster)))
                {
                    let max_uses = section_maxima
                        .get(monster)
                        .copied()
                        .unwrap_or_else(|| future_monster_count(section_maxima, monster))
                        .saturating_sub(1)
                        .min(2);
                    if max_uses > 0 {
                        let (route_lines, description_lines) = if let Some(setup_line) = setup_line
                        {
                            (
                                vec![setup_line.clone(), line.clone()],
                                vec![
                                    format!(
                                        "after line {}: {}",
                                        line.source_line, setup_line.raw_line
                                    ),
                                    format!("after line {}: {}", line.source_line, line.raw_line),
                                ],
                            )
                        } else {
                            (
                                vec![line.clone()],
                                vec![format!(
                                    "after line {}: {}",
                                    line.source_line, line.raw_line
                                )],
                            )
                        };
                        families.push(SearchSynthesisFamily {
                            anchor_index: index + 1,
                            route_lines,
                            description_lines,
                            max_uses,
                        });
                    }
                }
            }
        }
        if Some(section_name) != near_sections.last() {
            continue;
        }
        if let Some(boss_anchor_index) = boss_anchor_index {
            let boss_line = &pre_ghost_lines[boss_anchor_index];
            families.push(SearchSynthesisFamily {
                anchor_index: boss_anchor_index,
                route_lines: vec![synthetic_route_line(
                    "death ???",
                    section_name,
                    boss_line.source_line,
                )],
                description_lines: vec![format!(
                    "before line {}: death ???",
                    boss_line.source_line
                )],
                max_uses: 3,
            });

            let party_string = party_string_before_index(pre_ghost_lines, boss_anchor_index);
            if party_string.contains('r') {
                let existing_monsters = pre_ghost_lines[start_index..end_index]
                    .iter()
                    .filter_map(|line| line.monster_name.as_ref().map(|name| monster_family(name)))
                    .collect::<HashSet<_>>();
                let mut missing_monsters = section_maxima
                    .iter()
                    .filter_map(|(monster_name, count)| {
                        (*count >= 2 && !existing_monsters.contains(&monster_family(monster_name)))
                            .then_some(monster_name.clone())
                    })
                    .collect::<Vec<_>>();
                missing_monsters.sort_by(|left, right| {
                    let left_has_digit = left.chars().any(|char| char.is_ascii_digit());
                    let right_has_digit = right.chars().any(|char| char.is_ascii_digit());
                    (!left_has_digit)
                        .cmp(&(!right_has_digit))
                        .then_with(|| {
                            section_maxima
                                .get(right)
                                .copied()
                                .unwrap_or_default()
                                .cmp(&section_maxima.get(left).copied().unwrap_or_default())
                        })
                        .then_with(|| left.cmp(right))
                });
                if let Some(monster_name) = missing_monsters.first() {
                    families.push(SearchSynthesisFamily {
                        anchor_index: boss_anchor_index,
                        route_lines: vec![
                            synthetic_route_line(
                                &format!("steal {monster_name}"),
                                section_name,
                                boss_line.source_line,
                            ),
                            synthetic_route_line(
                                &format!("{monster_name} rikku"),
                                section_name,
                                boss_line.source_line,
                            ),
                        ],
                        description_lines: vec![
                            format!(
                                "before line {}: steal {monster_name}",
                                boss_line.source_line
                            ),
                            format!(
                                "before line {}: {monster_name} rikku",
                                boss_line.source_line
                            ),
                        ],
                        max_uses: 1,
                    });
                }
            }
        }
    }
    families
}

fn party_string_before_index(lines: &[DropsSearchRouteLine], anchor_index: usize) -> String {
    let mut party_string = String::new();
    for line in lines.iter().take(anchor_index) {
        if line.command != "party" {
            continue;
        }
        if let Some((_, party)) = line.normalized_line.split_once(char::is_whitespace) {
            party_string = party.trim().to_string();
        }
    }
    party_string
}

fn last_random_sections_near_ghost(
    pre_ghost_lines: &[DropsSearchRouteLine],
    future_section_maxima: &HashMap<String, HashMap<String, usize>>,
    limit: usize,
) -> Vec<String> {
    let mut sections = Vec::new();
    let mut seen = HashSet::new();
    for line in pre_ghost_lines {
        if line.section_name.is_empty()
            || !future_section_maxima.contains_key(&line.section_name)
            || !seen.insert(line.section_name.clone())
        {
            continue;
        }
        sections.push(line.section_name.clone());
    }
    sections
        .into_iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn section_bounds(lines: &[DropsSearchRouteLine], section_name: &str) -> Option<(usize, usize)> {
    let indices = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.section_name == section_name).then_some(index))
        .collect::<Vec<_>>();
    Some((*indices.first()?, indices.last()? + 1))
}

fn section_boss_anchor_index(
    lines: &[DropsSearchRouteLine],
    section_name: &str,
    section_maxima: &HashMap<String, usize>,
) -> Option<usize> {
    let (start, end) = section_bounds(lines, section_name)?;
    (start..end).find(|index| {
        let line = &lines[*index];
        !line.optional
            && line
                .monster_name
                .as_ref()
                .is_some_and(|monster| future_monster_count(section_maxima, monster) == 0)
    })
}

fn immediate_setup_line(
    lines: &[DropsSearchRouteLine],
    index: usize,
    optional_only: bool,
) -> Option<&DropsSearchRouteLine> {
    if index == 0 {
        return None;
    }
    let line = &lines[index];
    if line.command != "kill"
        || !monster_requires_immediate_steal_setup(line.monster_name.as_deref())
    {
        return None;
    }
    let setup = &lines[index - 1];
    if optional_only && !setup.optional {
        return None;
    }
    (setup.section_name == line.section_name
        && setup.command == "steal"
        && setup.monster_name == line.monster_name)
        .then_some(setup)
}

fn immediate_followup_line(
    lines: &[DropsSearchRouteLine],
    index: usize,
    optional_only: bool,
) -> Option<&DropsSearchRouteLine> {
    let line = &lines[index];
    if line.command != "steal"
        || !monster_requires_immediate_steal_setup(line.monster_name.as_deref())
    {
        return None;
    }
    let followup = lines.get(index + 1)?;
    if optional_only && !followup.optional {
        return None;
    }
    (followup.section_name == line.section_name
        && followup.command == "kill"
        && followup.monster_name == line.monster_name)
        .then_some(followup)
}

fn monster_requires_immediate_steal_setup(monster_name: Option<&str>) -> bool {
    monster_name.is_some_and(|name| monster_family(name) == "mech_scouter")
}

fn synthetic_route_line(
    normalized_line: &str,
    section_name: &str,
    source_line: usize,
) -> DropsSearchRouteLine {
    let command = normalized_line
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    DropsSearchRouteLine {
        source_line,
        raw_line: normalized_line.to_string(),
        normalized_line: normalize_drops_search_line(normalized_line),
        section_name: section_name.to_string(),
        optional: false,
        is_ghost: normalized_line == "kill ghost",
        command,
        monster_name: drops_search_monster_name(normalized_line).map(str::to_string),
    }
}

fn no_encounters_synth_combos(
    families: &[SearchSynthesisFamily],
    max_combos: usize,
) -> Vec<Vec<usize>> {
    fn build(
        families: &[SearchSynthesisFamily],
        index: usize,
        max_cost: usize,
        current_cost: usize,
        current: &mut Vec<usize>,
        out: &mut Vec<Vec<usize>>,
        max_combos: usize,
    ) {
        if out.len() >= max_combos {
            return;
        }
        if index == families.len() {
            if current_cost == max_cost {
                out.push(current.clone());
            }
            return;
        }
        let action_cost = families[index].route_lines.len();
        for count in 0..=families[index].max_uses {
            let next_cost = current_cost + count * action_cost;
            if next_cost > max_cost {
                break;
            }
            current.push(count);
            build(
                families,
                index + 1,
                max_cost,
                next_cost,
                current,
                out,
                max_combos,
            );
            current.pop();
            if out.len() >= max_combos {
                break;
            }
        }
    }
    let mut out = Vec::new();
    let max_possible_cost = families
        .iter()
        .map(|family| family.max_uses * family.route_lines.len())
        .sum::<usize>();
    for max_cost in 0..=max_possible_cost {
        build(
            families,
            0,
            max_cost,
            0,
            &mut Vec::new(),
            &mut out,
            max_combos,
        );
        if out.len() >= max_combos {
            break;
        }
    }
    out
}

fn build_synth_route_prefix(
    pre_ghost_lines: &[DropsSearchRouteLine],
    families: &[SearchSynthesisFamily],
    counts: &[usize],
) -> (Vec<DropsSearchRouteLine>, Vec<String>) {
    let mut insertions: HashMap<usize, Vec<DropsSearchRouteLine>> = HashMap::new();
    let mut descriptions = Vec::new();
    for (family, count) in families.iter().zip(counts) {
        for _ in 0..*count {
            insertions
                .entry(family.anchor_index)
                .or_default()
                .extend(family.route_lines.clone());
            descriptions.extend(family.description_lines.clone());
        }
    }
    let mut route_prefix = Vec::new();
    for (index, line) in pre_ghost_lines.iter().enumerate() {
        route_prefix.extend(insertions.remove(&index).unwrap_or_default());
        if !line.optional {
            route_prefix.push(line.clone());
        }
    }
    route_prefix.extend(
        insertions
            .remove(&pre_ghost_lines.len())
            .unwrap_or_default(),
    );
    (route_prefix, descriptions)
}

#[cfg(test)]
fn synthesize_no_encounters_ghost_route(
    seed: u32,
    edited_input: &str,
    start_line: usize,
    max_ghost_kills: usize,
    max_pre_ghost_deaths: usize,
) -> NoEncountersGhostRouteSearch {
    let lines = edited_input
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let Some(ghost_index) = find_first_ghost_route_line(&lines, start_line) else {
        return NoEncountersGhostRouteSearch {
            candidate: None,
            candidates_tested: 0,
        };
    };
    let subset_search = search_no_encounters_optional_subsets(seed, &lines, start_line);
    if subset_search.candidate.is_some() {
        return subset_search;
    }
    let repeat_index = find_active_repeat_after_ghost(&lines, ghost_index);
    let mut candidates_tested = subset_search.candidates_tested;
    for pre_ghost_deaths in 0..=max_pre_ghost_deaths {
        for ghost_kills in 1..=max_ghost_kills.max(1) {
            candidates_tested += 1;
            let repeat_line = if ghost_kills <= 1 {
                "# /repeat 0".to_string()
            } else {
                format!("/repeat {}", ghost_kills - 1)
            };
            let mut candidate_lines = lines.clone();
            match repeat_index {
                Some(index) if ghost_kills <= 1 => {
                    candidate_lines.remove(index);
                }
                Some(index) => {
                    candidate_lines[index] = repeat_line.clone();
                }
                None if ghost_kills > 1 => {
                    candidate_lines.insert(ghost_index + 1, repeat_line.clone());
                }
                None => {}
            }
            if pre_ghost_deaths > 0 {
                let death_lines = (0..pre_ghost_deaths)
                    .map(|_| "death ???".to_string())
                    .collect::<Vec<_>>();
                candidate_lines.splice(ghost_index..ghost_index, death_lines);
            }
            let route_input = candidate_lines.join("\n");
            let route_search_input =
                first_ghost_search_window_input_from_lines(&candidate_lines, start_line)
                    .unwrap_or_else(|| route_input.clone());
            let rendered = render_drops_tracker(seed, &route_search_input);
            if let Some(drop) = find_no_encounters_ghost_drop(&rendered) {
                return NoEncountersGhostRouteSearch {
                    candidate: Some(NoEncountersGhostRouteCandidate {
                        ghost_kills,
                        pre_ghost_deaths,
                        repeat_line,
                        uncommented_line: None,
                        drop,
                        route_input,
                    }),
                    candidates_tested,
                };
            }
        }
    }
    NoEncountersGhostRouteSearch {
        candidate: None,
        candidates_tested,
    }
}

#[cfg(test)]
fn search_no_encounters_optional_subsets(
    seed: u32,
    lines: &[String],
    start_line: usize,
) -> NoEncountersGhostRouteSearch {
    let (edited_prefix, pre_ghost_lines, ghost_lines) = drops_search_window(lines, start_line);
    if ghost_lines.is_empty() {
        return NoEncountersGhostRouteSearch {
            candidate: None,
            candidates_tested: 0,
        };
    }
    let active_ghost_line = ghost_lines
        .iter()
        .find(|line| !line.optional)
        .or_else(|| ghost_lines.first());
    let active_ghost_kills = active_ghost_line
        .map(|ghost| {
            find_active_repeat_after_ghost(lines, ghost.source_line.saturating_sub(1))
                .and_then(|index| lines.get(index))
                .and_then(|line| parse_repeat_count(line))
                .map(|repeats| repeats + 1)
                .unwrap_or(1)
        })
        .unwrap_or(1);
    let active_repeat_line = active_ghost_line
        .and_then(|ghost| {
            find_active_repeat_after_ghost(lines, ghost.source_line.saturating_sub(1))
        })
        .and_then(|index| lines.get(index))
        .cloned()
        .unwrap_or_else(|| "# /repeat 0".to_string());
    let optional_lines = pre_ghost_lines
        .iter()
        .filter(|line| line.optional && !is_nea_excluded_optional(&line.normalized_line))
        .cloned()
        .collect::<Vec<_>>();
    let linked_optionals = linked_drops_optionals(&pre_ghost_lines);
    let paired_death_lines = linked_optionals
        .values()
        .flat_map(|lines| lines.iter().cloned())
        .collect::<std::collections::HashSet<_>>();
    let filtered_optionals = optional_lines
        .into_iter()
        .filter(|line| !paired_death_lines.contains(line))
        .rev()
        .take(15)
        .collect::<Vec<_>>();
    let option_count = filtered_optionals.len();
    let subset_budget = 20_000usize;
    let mut tested_routes = 0;
    let mut tested_subsets = 0usize;
    let mut seen_routes = std::collections::HashSet::new();

    for subset_size in 0..=option_count {
        let mut mask = 0usize;
        let max_mask = 1usize.checked_shl(option_count as u32).unwrap_or(0);
        while mask < max_mask {
            if mask.count_ones() as usize != subset_size {
                mask += 1;
                continue;
            }
            if tested_subsets >= subset_budget {
                return NoEncountersGhostRouteSearch {
                    candidate: None,
                    candidates_tested: tested_routes,
                };
            }
            tested_subsets += 1;
            let mut chosen = std::collections::HashSet::new();
            for (index, optional) in filtered_optionals.iter().enumerate() {
                if (mask & (1usize << index)) != 0 {
                    chosen.insert(optional.clone());
                    if let Some(linked) = linked_optionals.get(optional) {
                        chosen.extend(linked.iter().cloned());
                    }
                }
            }
            let route_prefix = pre_ghost_lines
                .iter()
                .filter(|line| !line.optional || chosen.contains(*line))
                .cloned()
                .collect::<Vec<_>>();
            for ghost_line in &ghost_lines {
                let mut route_lines = route_prefix.clone();
                route_lines.push(ghost_line.clone());
                let route_key = route_lines
                    .iter()
                    .map(|line| line.normalized_line.clone())
                    .collect::<Vec<_>>();
                if !seen_routes.insert(route_key) {
                    continue;
                }
                tested_routes += 1;
                let route_text = drops_route_text(&edited_prefix, &route_lines);
                let rendered = render_drops_tracker(seed, &route_text);
                let Some(drop) = find_no_encounters_ghost_drop(&rendered) else {
                    continue;
                };
                let mut candidate_lines = lines.to_vec();
                for line in route_lines.iter().filter(|line| line.optional) {
                    let index = line.source_line.saturating_sub(1);
                    if let Some(slot) = candidate_lines.get_mut(index) {
                        *slot = line.raw_line.clone();
                    }
                }
                if let Some(active_ghost) = active_ghost_line {
                    if ghost_line.normalized_line != active_ghost.normalized_line {
                        let index = active_ghost.source_line.saturating_sub(1);
                        if let Some(slot) = candidate_lines.get_mut(index) {
                            *slot = ghost_line.raw_line.clone();
                        }
                    }
                }
                let first_added = route_lines
                    .iter()
                    .find(|line| line.optional)
                    .map(|line| (line.source_line, line.normalized_line.clone()));
                return NoEncountersGhostRouteSearch {
                    candidate: Some(NoEncountersGhostRouteCandidate {
                        ghost_kills: active_ghost_kills,
                        pre_ghost_deaths: 0,
                        repeat_line: active_repeat_line,
                        uncommented_line: first_added,
                        drop,
                        route_input: candidate_lines.join("\n"),
                    }),
                    candidates_tested: tested_routes,
                };
            }
            mask += 1;
        }
    }
    NoEncountersGhostRouteSearch {
        candidate: None,
        candidates_tested: tested_routes,
    }
}

fn drops_search_window(
    lines: &[String],
    start_line: usize,
) -> (String, Vec<DropsSearchRouteLine>, Vec<DropsSearchRouteLine>) {
    let prefix_lines = strip_search_block_comments(&lines[..start_line.saturating_sub(1)]);
    let edited_prefix = edit_drops_tracker_input(&prefix_lines.join("\n"));
    let mut pre_ghost_lines = Vec::new();
    let mut ghost_lines = Vec::new();
    let mut ghost_started = false;
    let mut in_block_comment = false;
    let mut current_section_name = String::new();

    for line in lines.iter().take(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if let Some(section_name) = drops_search_section_name(line) {
            current_section_name = section_name;
        }
    }
    in_block_comment = false;

    for (index, line) in lines.iter().enumerate().skip(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if let Some(section_name) = drops_search_section_name(line) {
            current_section_name = section_name;
            continue;
        }
        let Some(route_line) = drops_search_route_line(line, index + 1, &current_section_name)
        else {
            continue;
        };
        if route_line.is_ghost {
            ghost_started = true;
            ghost_lines.push(route_line);
            continue;
        }
        if ghost_started {
            break;
        }
        pre_ghost_lines.push(route_line);
    }
    (edited_prefix, pre_ghost_lines, ghost_lines)
}

fn strip_search_block_comments(lines: &[String]) -> Vec<String> {
    let mut filtered = Vec::new();
    let mut in_block_comment = false;
    for line in lines {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        filtered.push(line.clone());
    }
    filtered
}

fn drops_search_route_line(
    line: &str,
    source_line: usize,
    section_name: &str,
) -> Option<DropsSearchRouteLine> {
    let normalized_line = normalize_drops_search_line(line);
    if normalized_line.is_empty() {
        return None;
    }
    let words = normalized_line.split_whitespace().collect::<Vec<_>>();
    let command = words.first().copied()?.to_string();
    if !matches!(
        command.as_str(),
        "kill" | "steal" | "death" | "bribe" | "party" | "roll" | "ap" | "inventory"
    ) {
        return None;
    }
    let optional = line.trim_start().starts_with('#');
    let is_ghost = words.len() >= 2 && words[0] == "kill" && words[1] == "ghost";
    let monster_name = drops_search_monster_name(&normalized_line).map(str::to_string);
    Some(DropsSearchRouteLine {
        source_line,
        raw_line: line.trim().trim_start_matches('#').trim().to_string(),
        normalized_line,
        section_name: section_name.to_string(),
        optional,
        is_ghost,
        command,
        monster_name,
    })
}

fn drops_route_text(prefix: &str, route_lines: &[DropsSearchRouteLine]) -> String {
    let suffix = route_lines
        .iter()
        .map(|line| line.normalized_line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    if prefix.trim().is_empty() {
        suffix
    } else if suffix.trim().is_empty() {
        prefix.to_string()
    } else {
        format!("{}\n{}", prefix.trim_end(), suffix)
    }
}

fn is_nea_excluded_optional(line: &str) -> bool {
    matches!(
        line,
        "steal biran_ronso" | "kill yenke_ronso kimahri" | "kill biran_ronso kimahri"
    )
}

fn linked_drops_optionals(
    pre_ghost_lines: &[DropsSearchRouteLine],
) -> std::collections::HashMap<DropsSearchRouteLine, Vec<DropsSearchRouteLine>> {
    let mut linked = std::collections::HashMap::new();
    for pair in pre_ghost_lines.windows(2) {
        let line = &pair[0];
        let next = &pair[1];
        if line.normalized_line == "steal evrae 1"
            && next.optional
            && next.normalized_line.starts_with("death ")
        {
            linked.insert(line.clone(), vec![next.clone()]);
            continue;
        }
        if line.optional
            && next.optional
            && line.normalized_line.split_whitespace().next() == Some("steal")
            && next.normalized_line.split_whitespace().next() == Some("kill")
            && drops_search_monster_name(&line.normalized_line)
                == drops_search_monster_name(&next.normalized_line)
        {
            linked.insert(next.clone(), vec![line.clone()]);
            if drops_search_monster_name(&line.normalized_line)
                .is_some_and(|name| monster_family(name) == "mech_scouter")
            {
                linked.insert(line.clone(), vec![next.clone()]);
            }
        }
    }
    linked
}

fn drops_search_monster_name(line: &str) -> Option<&str> {
    let words = line.split_whitespace().collect::<Vec<_>>();
    match words.as_slice() {
        ["kill" | "steal" | "bribe", monster, ..] => Some(*monster),
        _ => None,
    }
}

#[cfg(test)]
fn parse_repeat_count(line: &str) -> Option<usize> {
    let mut words = line.split_whitespace();
    match (words.next(), words.next()) {
        (Some("/repeat"), Some(count)) => count.parse::<usize>().ok(),
        (Some("/repeat"), None) => Some(1),
        _ => None,
    }
}

fn count_route_exact_future_random_rows(
    route_input: &str,
    future: &ExactFutureEncounterOutput,
) -> Option<usize> {
    let route_monsters = route_kill_monsters(route_input);
    if route_monsters.is_empty() {
        return Some(0);
    }
    let mut next_row_index = 0;
    let mut current_remaining: HashMap<String, usize> = HashMap::new();
    let mut current_is_ghost = false;
    let mut random_rows = 0;

    for monster in route_monsters {
        loop {
            if let Some(resolved) = resolve_remaining_monster(&monster, &current_remaining) {
                if let Some(count) = current_remaining.get_mut(&resolved) {
                    *count -= 1;
                    if *count <= 0 {
                        current_remaining.remove(&resolved);
                    }
                }
                break;
            }
            if current_is_ghost {
                return None;
            }
            let row = future.rows.get(next_row_index)?;
            next_row_index += 1;
            current_remaining = encounter_options_maxima(&row.encounter_options);
            current_is_ghost = current_remaining.contains_key("ghost");
            if row.random {
                random_rows += 1;
            }
        }
    }

    Some(random_rows)
}

fn count_route_future_random_rows(
    route_input: &str,
    future: &FutureEncounterSearchView,
) -> Option<usize> {
    future
        .branches
        .iter()
        .filter_map(|branch| count_route_exact_future_random_rows(route_input, branch))
        .min()
}

fn route_kill_monsters(route_input: &str) -> Vec<String> {
    let mut monsters = Vec::new();
    for line in expand_tracker_repeats(route_input) {
        let normalized = normalize_drops_search_line(&line);
        let words = normalized.split_whitespace().collect::<Vec<_>>();
        if words.len() >= 2 && words[0] == "kill" {
            monsters.push(words[1].to_string());
            if words[1] == "ghost" {
                break;
            }
        }
    }
    monsters
}

fn resolve_remaining_monster(
    monster_name: &str,
    remaining_monsters: &HashMap<String, usize>,
) -> Option<String> {
    if remaining_monsters
        .get(monster_name)
        .copied()
        .unwrap_or_default()
        > 0
    {
        return Some(monster_name.to_string());
    }
    let target_family = monster_family(monster_name);
    let mut family_match = None;
    for (name, count) in remaining_monsters {
        if *count == 0 || monster_family(name) != target_family {
            continue;
        }
        if family_match.is_some() {
            return None;
        }
        family_match = Some(name.clone());
    }
    family_match
}

fn monster_family(monster_name: &str) -> &str {
    monster_name
        .trim_end_matches(|character: char| character.is_ascii_digit())
        .trim_end_matches('_')
}

#[cfg(test)]
fn first_ghost_search_window_input(edited_input: &str, start_line: usize) -> Option<String> {
    let lines = edited_input
        .lines()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    first_ghost_search_window_input_from_lines(&lines, start_line)
}

#[cfg(test)]
fn first_ghost_search_window_input_from_lines(
    lines: &[String],
    start_line: usize,
) -> Option<String> {
    let ghost_index = find_first_ghost_window_line(lines, start_line)?;
    let end_index = find_active_repeat_after_ghost(lines, ghost_index).unwrap_or(ghost_index);
    Some(
        lines
            .iter()
            .take(end_index + 1)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[cfg(test)]
fn find_first_ghost_window_line(lines: &[String], start_line: usize) -> Option<usize> {
    let mut in_block_comment = false;
    for (index, line) in lines.iter().enumerate().skip(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if drops_search_route_line_is_ghost(line) {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
fn find_first_ghost_route_line(lines: &[String], start_line: usize) -> Option<usize> {
    let mut in_block_comment = false;
    for (index, line) in lines.iter().enumerate().skip(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if line.trim_start().starts_with('#') {
            continue;
        }
        if drops_search_route_line_is_ghost(line) {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
fn find_active_repeat_after_ghost(lines: &[String], ghost_index: usize) -> Option<usize> {
    let mut in_block_comment = false;
    for (index, line) in lines.iter().enumerate().skip(ghost_index + 1) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed == "/repeat" || trimmed.starts_with("/repeat ") {
            return Some(index);
        }
        return None;
    }
    None
}

fn format_future_section_maxima_debug(future: &FutureEncounterSearchView) -> String {
    let mut section_parts = future
        .section_maxima
        .iter()
        .filter_map(|(section_name, maxima)| {
            let mut monster_parts = maxima
                .iter()
                .map(|(monster_name, count)| (monster_name.as_str(), *count))
                .collect::<Vec<_>>();
            monster_parts
                .sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(right.0)));
            let monster_parts = monster_parts
                .into_iter()
                .take(4)
                .map(|(monster_name, count)| format!("{monster_name}x{count}"))
                .collect::<Vec<_>>();
            if monster_parts.is_empty() {
                None
            } else {
                Some(format!("{}: {}", section_name, monster_parts.join(", ")))
            }
        })
        .collect::<Vec<_>>();
    section_parts.sort();
    if section_parts.is_empty() {
        "Future random sections: none.".to_string()
    } else {
        format!("Future random sections: {}.", section_parts.join(" | "))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EncounterSearchNote {
    name: String,
    label: String,
    default_count: usize,
    max_count: usize,
    section_name: String,
}

fn build_notes_fallback_future_view(
    seed: u32,
    start_section_name: &str,
    extra_budget: usize,
) -> Option<FutureEncounterSearchView> {
    let notes = future_encounter_search_notes(start_section_name)?;
    let branches = future_encounter_count_branches(&notes, extra_budget);
    let mut futures = Vec::new();
    for counts in branches {
        if let Some(future) = render_future_note_branch(seed, start_section_name, &notes, &counts) {
            futures.push(future);
        }
    }
    if futures.is_empty() {
        None
    } else {
        Some(FutureEncounterSearchView::from_branches(futures))
    }
}

fn future_encounter_search_notes(start_section_name: &str) -> Option<Vec<EncounterSearchNote>> {
    let mut notes = Vec::new();
    let mut current_section_name = String::new();
    let mut started = start_section_name.is_empty();
    for line in ENCOUNTERS_NOTES.lines().skip(1) {
        let parts = line.split(',').map(str::trim).collect::<Vec<_>>();
        if parts.len() < 6 || parts[0].is_empty() {
            continue;
        }
        let note_name = parts[0];
        let label = parts[2];
        current_section_name = encounter_note_section_name(label, note_name, &current_section_name);
        if !started {
            if current_section_name != start_section_name {
                continue;
            }
            started = true;
        }
        notes.push(EncounterSearchNote {
            name: note_name.to_string(),
            label: label.to_string(),
            default_count: parts[4].parse().ok()?,
            max_count: parts[5].parse().ok()?,
            section_name: current_section_name.clone(),
        });
    }
    if started {
        Some(notes)
    } else {
        None
    }
}

fn future_encounter_count_branches(
    notes: &[EncounterSearchNote],
    extra_budget: usize,
) -> Vec<Vec<usize>> {
    let random_note_indexes = notes
        .iter()
        .enumerate()
        .filter_map(|(index, note)| {
            (is_random_encounter_note_name(&note.name) && note.max_count > note.default_count)
                .then_some(index)
        })
        .collect::<Vec<_>>();
    let mut counts = notes
        .iter()
        .map(|note| note.default_count)
        .collect::<Vec<_>>();
    let mut branches = Vec::new();
    fn build(
        notes: &[EncounterSearchNote],
        random_note_indexes: &[usize],
        position: usize,
        remaining_extra: usize,
        counts: &mut [usize],
        branches: &mut Vec<Vec<usize>>,
    ) {
        if position >= random_note_indexes.len() {
            branches.push(counts.to_vec());
            return;
        }
        let note_index = random_note_indexes[position];
        let note = &notes[note_index];
        let max_extra = (note.max_count - note.default_count).min(remaining_extra);
        for extra in 0..=max_extra {
            counts[note_index] = note.default_count + extra;
            build(
                notes,
                random_note_indexes,
                position + 1,
                remaining_extra - extra,
                counts,
                branches,
            );
        }
        counts[note_index] = note.default_count;
    }
    build(
        notes,
        &random_note_indexes,
        0,
        extra_budget,
        &mut counts,
        &mut branches,
    );
    let mut seen = HashSet::new();
    branches
        .into_iter()
        .filter(|branch| seen.insert(branch.clone()))
        .take(1024)
        .collect()
}

fn render_future_note_branch(
    seed: u32,
    start_section_name: &str,
    notes: &[EncounterSearchNote],
    counts: &[usize],
) -> Option<ExactFutureEncounterOutput> {
    let mut input_lines = vec!["/nopadding".to_string()];
    for (note, count) in notes.iter().zip(counts) {
        if !note.label.is_empty() {
            input_lines.push(format!("# {}:", note.label));
        }
        for _ in 0..*count {
            input_lines.push(format!("encounter {}", note.name));
        }
        if note.name == "cave_white_zone cave_green_zone" {
            input_lines.push("# Ghost search stop".to_string());
        }
    }
    let rendered = render_encounters_tracker(seed, &input_lines.join("\n"));
    let mut future = parse_exact_future_encounter_output(&rendered, start_section_name)?;
    future.section_maxima = future_note_branch_random_section_maxima(&future.rows, notes, counts);
    Some(future)
}

fn is_random_encounter_note_name(name: &str) -> bool {
    if data::random_zone_stats(name).is_some() {
        return true;
    }
    let words = name.split_whitespace().collect::<Vec<_>>();
    !words.is_empty()
        && words
            .iter()
            .all(|zone| data::random_zone_stats(zone).is_some())
}

fn future_note_branch_random_section_maxima(
    rows: &[ExactFutureEncounterRow],
    notes: &[EncounterSearchNote],
    counts: &[usize],
) -> HashMap<String, HashMap<String, usize>> {
    let mut section_maxima: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut row_index = 0usize;
    for (note, count) in notes.iter().zip(counts) {
        for _ in 0..*count {
            let Some(row) = rows.get(row_index) else {
                return section_maxima;
            };
            row_index += 1;
            if is_random_encounter_note_name(&note.name) {
                let maxima = encounter_options_maxima(&row.encounter_options);
                add_counts(
                    section_maxima.entry(note.section_name.clone()).or_default(),
                    &maxima,
                );
            }
            if row
                .encounter_options
                .iter()
                .any(|monsters| monsters.iter().any(|monster| monster == "ghost"))
            {
                return section_maxima;
            }
        }
    }
    section_maxima
}

fn parse_exact_future_encounter_output(
    encounters_output: &str,
    start_section_name: &str,
) -> Option<ExactFutureEncounterOutput> {
    let mut rows = Vec::new();
    let mut total_counts = HashMap::new();
    let mut section_maxima: HashMap<String, HashMap<String, usize>> = HashMap::new();
    let mut current_section_name = String::new();
    let mut started = start_section_name.is_empty();
    let mut in_block_comment = false;

    for line in encounters_output.lines() {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if stripped.is_empty() || stripped.chars().all(|character| character == '=') {
            continue;
        }
        if stripped.starts_with('#') {
            let label = stripped
                .trim_start_matches('#')
                .trim()
                .trim_end_matches(':')
                .trim();
            current_section_name = encounter_note_section_name(label, "", &current_section_name);
            continue;
        }
        if !line.contains('|') {
            continue;
        }

        let parts = line.split('|').map(str::trim).collect::<Vec<_>>();
        let mut label = "";
        let row_prefix_random = encounter_output_row_prefix_random(parts[0]);
        let first_part = strip_encounter_output_row_prefix(parts[0]);
        let encounter_parts = if looks_like_encounter_output_metadata(first_part) {
            if parts
                .get(1)
                .is_some_and(|part| !parse_output_formation_monsters(part).is_empty())
            {
                &parts[1..]
            } else if parts.len() >= 3 {
                label = parts[1];
                &parts[2..]
            } else {
                continue;
            }
        } else {
            label = parts[0];
            &parts[1..]
        };

        let mut row_section_name = current_section_name.clone();
        if !label.is_empty() {
            let note_name = normalize_output_label_to_note_name(label);
            row_section_name = if row_prefix_random.is_some() {
                raw_encounter_output_label_section_name(
                    label,
                    &current_section_name,
                    start_section_name,
                )
            } else {
                encounter_note_section_name("", &note_name, &current_section_name)
            };
            current_section_name.clone_from(&row_section_name);
        }
        if !started {
            if row_section_name != start_section_name {
                continue;
            }
            started = true;
        }

        let encounter_options = encounter_parts
            .iter()
            .map(|part| parse_output_formation_monsters(part))
            .filter(|monsters| !monsters.is_empty())
            .collect::<Vec<_>>();
        if encounter_options.is_empty() {
            continue;
        }

        let row_is_random = row_prefix_random.unwrap_or(label.is_empty());
        let maxima = encounter_options_maxima(&encounter_options);
        add_counts(&mut total_counts, &maxima);
        if row_is_random {
            add_counts(section_maxima.entry(row_section_name).or_default(), &maxima);
        }
        let has_ghost = encounter_options
            .iter()
            .any(|monsters| monsters.iter().any(|monster| monster == "ghost"));
        rows.push(ExactFutureEncounterRow {
            encounter_options,
            random: row_is_random,
        });
        if has_ghost {
            break;
        }
    }

    (started && !rows.is_empty()).then_some(ExactFutureEncounterOutput {
        rows,
        total_counts,
        section_maxima,
    })
}

fn raw_encounter_output_label_section_name(
    label: &str,
    current_section_name: &str,
    start_section_name: &str,
) -> String {
    let section_names = label
        .split('/')
        .map(|part| encounter_note_section_name(part.trim(), "", current_section_name))
        .filter(|section_name| !section_name.is_empty())
        .collect::<Vec<_>>();
    if !start_section_name.is_empty()
        && section_names
            .iter()
            .any(|section_name| section_name == start_section_name)
    {
        start_section_name.to_string()
    } else {
        section_names
            .into_iter()
            .next()
            .unwrap_or_else(|| current_section_name.to_string())
    }
}

fn strip_encounter_output_row_prefix(text: &str) -> &str {
    for prefix in [
        "Random Encounter:",
        "Multizone encounter:",
        "Simulated Encounter:",
        "Encounter:",
    ] {
        if let Some(stripped) = text.trim().strip_prefix(prefix) {
            return stripped.trim();
        }
    }
    text
}

fn encounter_output_row_prefix_random(text: &str) -> Option<bool> {
    let trimmed = text.trim();
    if trimmed.starts_with("Random Encounter:") || trimmed.starts_with("Multizone encounter:") {
        Some(true)
    } else if trimmed.starts_with("Encounter:") || trimmed.starts_with("Simulated Encounter:") {
        Some(false)
    } else {
        None
    }
}

fn looks_like_encounter_output_metadata(text: &str) -> bool {
    let words = text.split_whitespace().collect::<Vec<_>>();
    !words.is_empty()
        && words
            .iter()
            .all(|word| word.chars().all(|c| c.is_ascii_digit()))
}

fn parse_output_formation_monsters(formation_text: &str) -> Vec<String> {
    let mut stripped = formation_text.trim();
    if stripped.is_empty() || stripped == "-" {
        return Vec::new();
    }
    for suffix in [" Ambush", " Preemptive", " Normal"] {
        if let Some(without_suffix) = stripped.strip_suffix(suffix) {
            stripped = without_suffix;
            break;
        }
    }
    let mut monsters = Vec::new();
    for monster_name in stripped.split(',').map(str::trim) {
        if monster_name.is_empty() {
            continue;
        }
        let normalized = monster_name
            .to_ascii_lowercase()
            .replace(' ', "_")
            .replace('#', "_");
        let Some(monster) =
            data::monster_stats(monster_name).or_else(|| data::monster_stats(&normalized))
        else {
            return Vec::new();
        };
        monsters.push(monster.key);
    }
    monsters
}

fn encounter_options_maxima(encounter_options: &[Vec<String>]) -> HashMap<String, usize> {
    let mut maxima: HashMap<String, usize> = HashMap::new();
    for monsters in encounter_options {
        let mut current_counts = HashMap::new();
        for monster in monsters {
            *current_counts.entry(monster.clone()).or_default() += 1;
        }
        for (monster, count) in current_counts {
            let maximum = maxima.entry(monster).or_default();
            *maximum = (*maximum).max(count);
        }
    }
    maxima
}

fn add_counts(target: &mut HashMap<String, usize>, counts: &HashMap<String, usize>) {
    for (monster, count) in counts {
        *target.entry(monster.clone()).or_default() += count;
    }
}

fn normalize_output_label_to_note_name(label: &str) -> String {
    label
        .trim()
        .to_ascii_lowercase()
        .replace(' ', "_")
        .replace('#', "_")
}

fn encounter_note_section_name(label: &str, note_name: &str, current_section_name: &str) -> String {
    let label = label.trim();
    if label.starts_with("Underwater Ruins") {
        "Underwater Ruins"
    } else if label.starts_with("Besaid") {
        "Besaid"
    } else if label.starts_with("Kilika") {
        "Kilika"
    } else if label.starts_with("Mi'ihen") {
        "Mi'ihen"
    } else if label.starts_with("Old Road") {
        "Old Road"
    } else if label.starts_with("Clasko Skip Screen") {
        "Clasko Skip Screen"
    } else if label.starts_with("Djose") {
        "Djose"
    } else if label.starts_with("Moonflow") {
        "Moonflow"
    } else if label.starts_with("Thunder Plains") {
        "Thunder Plains"
    } else if label.starts_with("Macalania") {
        "Macalania Woods"
    } else if label.starts_with("Lake Macalania") {
        "Lake Macalania"
    } else if label.starts_with("Crevasse") {
        "Crevasse"
    } else if label.starts_with("Bikanel") || label.starts_with("Sandragora") {
        "Bikanel"
    } else if label.starts_with("Home") {
        "Home"
    } else if label.starts_with("Airship") || label.starts_with("Bevelle") {
        "Airship"
    } else if label.starts_with("Via Purifico (Maze)")
        || label.starts_with("Via Purifico (Corridor)")
    {
        "Via Purifico"
    } else if label.starts_with("Via Purifico (Underwater)") {
        "Underwater"
    } else if label.starts_with("Highbridge") {
        "Highbridge"
    } else if label.starts_with("Calm Lands") || label.starts_with("Biran & Yenke (pre Cave)") {
        "Calm Lands"
    } else if label.starts_with("Cave") {
        "Cavern of the Stolen Fayth"
    } else if label.starts_with("Biran & Yenke") || label.starts_with("Gagazet") {
        "Gagazet"
    } else if label.starts_with("Zanarkand") {
        "Zanarkand"
    } else if label.starts_with("Inside Sin") {
        "Sin"
    } else if !label.is_empty() {
        label
    } else {
        match note_name {
            "evrae" => "Airship",
            "bevelle_guards_1" | "bevelle_guards_2" | "bevelle_guards_3" | "bevelle_guards_4"
            | "bevelle_guards_5" => "Bevelle",
            "evrae_altana" => "Underwater",
            "seymour_natus" => "Highbridge",
            "defender_x" => "Calm Lands",
            "biran_&_yenke" | "seymour_flux" | "sanctuary_keeper" => "Gagazet",
            "spectral_keeper" | "yunalesca" => "Zanarkand",
            "left_fin" | "right_fin" | "sin_core" | "overdrive_sin" | "seymour_omnis" => "Sin",
            _ => current_section_name,
        }
    }
    .to_string()
}

fn encounters_input_has_exact_future_rows(encounters_input: &str) -> bool {
    let mut in_block_comment = false;
    for line in encounters_input.lines() {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if stripped.is_empty() || stripped.starts_with('#') || stripped == "///" {
            continue;
        }
        let ParsedCommand::Encounter {
            name,
            multizone,
            zones,
        } = parse_raw_action_line(stripped)
        else {
            continue;
        };
        if multizone {
            if !zones.is_empty()
                && zones
                    .iter()
                    .all(|zone| data::random_zone_stats(zone).is_some())
            {
                return true;
            }
        } else if data::boss_or_simulated_formation(&name).is_some()
            || data::random_zone_stats(&name).is_some()
        {
            return true;
        }
    }
    false
}

fn edit_drops_tracker_input(input: &str) -> String {
    input
        .lines()
        .map(|line| {
            let mut words = line.split_whitespace();
            let Some(first) = words.next() else {
                return line.to_string();
            };
            if data::monster_stats(first).is_some() {
                format!("kill {line}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn drops_search_first_ghost_line(input_lines: &[&str], start_line: usize) -> Option<usize> {
    let mut in_block_comment = false;
    for (index, line) in input_lines
        .iter()
        .enumerate()
        .skip(start_line.saturating_sub(1))
    {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if drops_search_route_line_is_ghost(line) {
            return Some(index + 1);
        }
    }
    None
}

fn drops_search_start_section_name(input_lines: &[&str], start_line: usize) -> String {
    let mut current_section_name = String::new();
    let mut first_route_section_name = None;
    let mut in_block_comment = false;
    for line in input_lines.iter().take(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if let Some(section_name) = drops_search_section_name(line) {
            current_section_name = section_name;
        }
    }
    in_block_comment = false;
    for line in input_lines.iter().skip(start_line.saturating_sub(1)) {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            if !stripped.ends_with("*/") {
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
        if let Some(section_name) = drops_search_section_name(line) {
            current_section_name = section_name;
            continue;
        }
        if drops_search_route_line_is_candidate(line) {
            first_route_section_name.get_or_insert_with(|| current_section_name.clone());
        }
        if drops_search_route_line_is_ghost(line) {
            break;
        }
    }
    first_route_section_name.unwrap_or(current_section_name)
}

fn drops_search_section_name(line: &str) -> Option<String> {
    let stripped = line.trim();
    if stripped.starts_with("# -- ") && stripped.ends_with(" --") {
        Some(stripped[5..stripped.len() - 3].trim().to_string())
    } else {
        None
    }
}

fn drops_search_route_line_is_candidate(line: &str) -> bool {
    let normalized = normalize_drops_search_line(line);
    let Some(command) = normalized.split_whitespace().next() else {
        return false;
    };
    matches!(
        command,
        "kill" | "steal" | "death" | "bribe" | "party" | "roll" | "ap" | "inventory"
    )
}

fn drops_search_route_line_is_ghost(line: &str) -> bool {
    let normalized = normalize_drops_search_line(line);
    let words = normalized.split_whitespace().collect::<Vec<_>>();
    words.len() >= 2 && words[0] == "kill" && words[1] == "ghost"
}

fn normalize_drops_search_line(line: &str) -> String {
    let mut stripped = line.trim();
    while let Some(rest) = stripped.strip_prefix('#') {
        stripped = rest.trim();
    }
    edit_drops_tracker_input(stripped)
        .trim()
        .to_ascii_lowercase()
}

fn expand_tracker_repeats(input: &str) -> Vec<String> {
    let mut lines = input.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut index = 0;
    let mut multiline_comment = false;
    while index < lines.len() {
        let line = lines[index].clone();
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            multiline_comment = true;
        }
        let should_expand =
            !multiline_comment && (line == "/repeat" || line.starts_with("/repeat "));
        if should_expand {
            let rest = line.split_whitespace().skip(1).collect::<Vec<_>>();
            let times = rest
                .first()
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 5000);
            let n_of_lines = rest
                .get(1)
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(1)
                .clamp(1, 5000 / times);
            let repeatable = index.min(n_of_lines);
            let mut insertions = Vec::new();
            for _ in 0..times {
                let start = index - repeatable;
                insertions.extend(lines[start..index].iter().cloned());
            }
            lines.splice(index + 1..index + 1, insertions);
        }
        if multiline_comment && stripped.ends_with("*/") {
            multiline_comment = false;
        }
        index += 1;
    }
    lines
}

fn tracker_has_active_nopadding(input: &str) -> bool {
    let mut multiline_comment = false;
    for raw_line in input.lines() {
        let stripped = raw_line.trim();
        if stripped.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            if stripped.ends_with("*/") {
                multiline_comment = false;
            }
            continue;
        }
        if raw_line == "/nopadding" {
            return true;
        }
    }
    false
}

fn protect_tracker_block_comment_repeats(input: &str) -> String {
    let mut multiline_comment = false;
    let mut lines = Vec::new();
    for raw_line in input.lines() {
        let stripped = raw_line.trim();
        if stripped.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment && (raw_line == "/repeat" || raw_line.starts_with("/repeat ")) {
            lines.push(format!(
                "__ctb_tracker_block_comment_repeat__{}",
                &raw_line[7..]
            ));
        } else {
            lines.push(raw_line.to_string());
        }
        if multiline_comment && stripped.ends_with("*/") {
            multiline_comment = false;
        }
    }
    lines.join("\n")
}

fn render_drops_line(
    rng: &mut FfxRngTracker,
    party: &mut Vec<Character>,
    ap_state: &mut HashMap<Character, DropsApState>,
    inventory: &mut DropsInventory,
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    equipment_drops: &mut usize,
    gil: &mut i32,
    words: &[&str],
) -> Option<String> {
    match words.first().copied()?.to_ascii_lowercase().as_str() {
        "party" => {
            let Some(initials) = words.get(1).copied() else {
                return Some("Error: Usage: party [characters initials]".to_string());
            };
            Some(render_drops_party_line(party, initials))
        }
        "death" => {
            let character = words
                .get(1)
                .and_then(|value| value.parse::<Character>().ok())
                .unwrap_or(Character::Unknown);
            for _ in 0..3 {
                rng.advance_rng(10);
            }
            Some(format!("Character death: {}", character.display_name()))
        }
        "roll" | "advance" | "waste" => {
            let Some(raw_index_token) = words.get(1).copied() else {
                return Some("Error: rng needs to be an integer".to_string());
            };
            let index_token = raw_index_token
                .strip_prefix("rng")
                .unwrap_or(raw_index_token);
            let Ok(index) = index_token.parse::<i32>() else {
                return Some("Error: rng needs to be an integer".to_string());
            };
            let amount = match words.get(2) {
                Some(value) => match value.parse::<i32>() {
                    Ok(amount) => amount,
                    Err(_) => return Some("Error: rng needs to be an integer".to_string()),
                },
                None => 1,
            };
            if amount < 0 {
                return Some("Error: amount needs to be an greater or equal to 0".to_string());
            }
            if amount > 200 {
                return Some("Error: Can't advance rng more than 200 times".to_string());
            }
            if !(0..68).contains(&index) {
                return Some(format!("Error: Can't advance rng index {index}"));
            }
            for _ in 0..amount {
                rng.advance_rng(index as usize);
            }
            Some(format!("Advanced rng{index} {amount} times"))
        }
        "ap" => {
            let character = match words.get(1) {
                Some(value) => match value.parse::<Character>() {
                    Ok(character) => Some(character),
                    Err(_) => {
                        return Some(format!(
                            "Error: character can only be one of these values: {DROPS_CHARACTER_VALUES}"
                        ))
                    }
                },
                None => None,
            };
            let amount = words
                .get(2)
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(0);
            Some(render_drops_ap_line(ap_state, character, amount))
        }
        "inventory" => Some(render_drops_inventory_line(
            inventory,
            equipment_inventory,
            gil,
            &words[1..],
        )),
        "steal" => {
            let Some(monster_key) = words.get(1).copied() else {
                return Some(format!("Error: {DROPS_STEAL_USAGE}"));
            };
            if data::monster_stats(monster_key).is_none() {
                return Some(format!("Error: No monster named \"{monster_key}\""));
            }
            let successful_steals = match words.get(2) {
                Some(value) => match value.parse::<i32>() {
                    Ok(value) if value >= 0 => value as u32,
                    Ok(_) => {
                        return Some(
                            "Error: successful steals must be greater or equal to 0".to_string(),
                        )
                    }
                    Err(_) => {
                        return Some("Error: successful steals must be an integer".to_string())
                    }
                },
                None => 0,
            };
            Some(render_steal_line(
                rng,
                inventory,
                monster_key,
                successful_steals,
            ))
        }
        "kill" => {
            let (Some(monster_key), Some(killer)) = (words.get(1).copied(), words.get(2).copied())
            else {
                return Some(format!("Error: {DROPS_KILL_USAGE}"));
            };
            if data::monster_stats(monster_key).is_none() {
                return Some(format!("Error: No monster named \"{monster_key}\""));
            }
            if killer.parse::<Character>().is_err() {
                return Some(format!(
                    "Error: killer can only be one of these values: {DROPS_CHARACTER_VALUES}"
                ));
            }
            let ap_characters = words.get(3).copied().unwrap_or_default();
            let overkill = words
                .get(4)
                .is_some_and(|token| is_drops_overkill_token(token));
            Some(render_kill_line(
                rng,
                ap_state,
                inventory,
                equipment_inventory,
                equipment_drops,
                gil,
                party,
                monster_key,
                killer,
                ap_characters,
                overkill,
            ))
        }
        "bribe" => {
            let (Some(monster_key), Some(user)) = (words.get(1).copied(), words.get(2).copied())
            else {
                return Some(format!("Error: {DROPS_BRIBE_USAGE}"));
            };
            if data::monster_stats(monster_key).is_none() {
                return Some(format!("Error: No monster named \"{monster_key}\""));
            }
            if user.parse::<Character>().is_err() {
                return Some(format!(
                    "Error: user can only be one of these values: {DROPS_CHARACTER_VALUES}"
                ));
            }
            let ap_characters = words.get(3).copied().unwrap_or_default();
            Some(render_bribe_drop_line(
                rng,
                ap_state,
                inventory,
                equipment_inventory,
                equipment_drops,
                gil,
                party,
                monster_key,
                user,
                ap_characters,
            ))
        }
        command => {
            if data::monster_stats(command).is_some() {
                let Some(killer) = words.get(1).copied() else {
                    return Some(format!("Error: {DROPS_KILL_USAGE}"));
                };
                if killer.parse::<Character>().is_err() {
                    return Some(format!(
                        "Error: killer can only be one of these values: {DROPS_CHARACTER_VALUES}"
                    ));
                }
                let ap_characters = words.get(2).copied().unwrap_or_default();
                let overkill = words
                    .get(3)
                    .is_some_and(|token| is_drops_overkill_token(token));
                Some(render_kill_line(
                    rng,
                    ap_state,
                    inventory,
                    equipment_inventory,
                    equipment_drops,
                    gil,
                    party,
                    command,
                    killer,
                    ap_characters,
                    overkill,
                ))
            } else {
                None
            }
        }
    }
}

fn render_drops_party_line(party: &mut Vec<Character>, initials: &str) -> String {
    let old_party = format_drops_party(party);
    let mut new_party = Vec::new();
    for initial in initials.chars() {
        let Some(character) = Character::from_party_initial(initial) else {
            continue;
        };
        if !new_party.contains(&character) {
            new_party.push(character);
        }
    }
    if new_party.is_empty() {
        return format!("Error: no characters initials in \"{initials}\"");
    }
    *party = new_party;
    format!("Party: {old_party} -> {}", format_drops_party(party))
}

fn is_drops_overkill_token(value: &str) -> bool {
    value.eq_ignore_ascii_case("overkill") || value.eq_ignore_ascii_case("ok")
}

fn format_drops_party(party: &[Character]) -> String {
    party
        .iter()
        .map(|character| character.display_name())
        .collect::<Vec<_>>()
        .join(", ")
}

fn drops_initial_ap_state() -> HashMap<Character, DropsApState> {
    drops_ap_characters()
        .into_iter()
        .filter_map(|character| {
            data::character_stats(character).map(|stats| {
                (
                    character,
                    DropsApState {
                        total_ap: 0,
                        starting_s_lv: stats.starting_s_lv,
                    },
                )
            })
        })
        .collect()
}

fn render_drops_ap_line(
    ap_state: &mut HashMap<Character, DropsApState>,
    character: Option<Character>,
    amount: i32,
) -> String {
    let characters = character
        .map(|character| vec![character])
        .unwrap_or_else(drops_ap_characters);
    let mut lines = Vec::new();
    for character in characters {
        let state = ap_state.entry(character).or_insert_with(|| DropsApState {
            total_ap: 0,
            starting_s_lv: data::character_stats(character)
                .map(|stats| stats.starting_s_lv)
                .unwrap_or_default(),
        });
        if amount != 0 {
            state.total_ap += amount;
        }
        let s_lv = total_ap_to_s_lv(state.total_ap, state.starting_s_lv);
        let next_s_lv_ap = s_lv_to_total_ap(s_lv + 1, state.starting_s_lv) - state.total_ap;
        let added = if amount != 0 {
            format!(" (added {amount} AP)")
        } else {
            String::new()
        };
        lines.push(format!(
            "{}: {s_lv} S. Lv ({} AP Total, {next_s_lv_ap} for next level){added}",
            character.display_name(),
            state.total_ap,
        ));
    }
    lines.join("\n")
}

fn drops_ap_characters() -> Vec<Character> {
    vec![
        Character::Tidus,
        Character::Yuna,
        Character::Auron,
        Character::Kimahri,
        Character::Wakka,
        Character::Lulu,
        Character::Rikku,
    ]
}

fn drops_characters_from_initials(initials: &str) -> Vec<Character> {
    let mut characters = Vec::new();
    for initial in initials.chars() {
        let Some(character) = Character::from_party_initial(initial) else {
            continue;
        };
        if !characters.contains(&character) && drops_ap_characters().contains(&character) {
            characters.push(character);
        }
    }
    characters
}

fn credit_drops_ap(
    ap_state: &mut HashMap<Character, DropsApState>,
    characters: &[Character],
    amount: i32,
) {
    for character in characters {
        let state = ap_state.entry(*character).or_insert_with(|| DropsApState {
            total_ap: 0,
            starting_s_lv: data::character_stats(*character)
                .map(|stats| stats.starting_s_lv)
                .unwrap_or_default(),
        });
        state.total_ap += amount;
    }
}

fn drops_character_initials(characters: &[Character]) -> String {
    characters
        .iter()
        .filter_map(|character| character.input_name().chars().next())
        .map(|initial| initial.to_ascii_uppercase())
        .collect()
}

fn render_drops_inventory_line(
    inventory: &mut DropsInventory,
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    gil: &mut i32,
    words: &[&str],
) -> String {
    match words {
        ["show", "equipment", ..] => render_drops_equipment_inventory(equipment_inventory),
        ["show", ..]
            if words.get(1).copied() != Some("gil")
                && words.get(1).copied() != Some("equipment") =>
        {
            inventory.to_tracker_string()
        }
        ["show", "gil", ..] => format!("Gil: {gil}\n"),
        ["get", "gil", amount, ..] => match amount.parse::<i32>() {
            Ok(amount) if amount > 0 => {
                *gil += amount;
                format!("Added {amount} Gil ({gil} Gil total)")
            }
            Ok(_) => "Error: Gil amount needs to be more than 0".to_string(),
            Err(_) => "Error: Gil amount needs to be an integer".to_string(),
        },
        ["use", "gil", amount, ..] => match amount.parse::<i32>() {
            Ok(amount) if amount > 0 && amount <= *gil => {
                *gil -= amount;
                format!("Used {amount} Gil ({gil} Gil total)")
            }
            Ok(amount) if amount > 0 => {
                format!("Error: Not enough gil (need {} more)", amount - *gil)
            }
            Ok(_) => "Error: Gil amount needs to be more than 0".to_string(),
            Err(_) => "Error: Gil amount needs to be an integer".to_string(),
        },
        [command @ ("get" | "use"), "gil", ..] => {
            format!("Error: Usage: inventory {command} gil [amount]")
        }
        ["sell", "equipment", slot, ..] if slot.chars().all(|char| char.is_ascii_digit()) => {
            render_drops_equipment_inventory_sell_slot(equipment_inventory, gil, slot)
        }
        ["get" | "buy", "equipment", kind, character, slots, abilities @ ..] => {
            render_drops_equipment_inventory_get_or_buy(
                equipment_inventory,
                gil,
                words[0],
                kind,
                character,
                slots,
                abilities,
            )
        }
        ["sell", "equipment", kind, character, slots, abilities @ ..] => {
            render_drops_equipment_inventory_sell(gil, kind, character, slots, abilities)
        }
        [command @ ("get" | "buy"), "equipment", ..] => {
            format!(
                "Error: Usage: inventory {command} equipment [equip type] [character] [slots] (abilities)"
            )
        }
        ["sell", "equipment", "weapon" | "armor", ..] => {
            "Error: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)"
                .to_string()
        }
        ["sell", "equipment", ..] => {
            "Error: Usage: inventory sell equipment [equipment slot]".to_string()
        }
        ["get" | "buy" | "use" | "sell", item_name, amount, ..] => {
            render_drops_item_inventory_command(inventory, gil, words[0], item_name, amount)
        }
        [command @ ("get" | "buy" | "use" | "sell"), ..] => {
            format!("Error: Usage: inventory {command} [item] [amount]")
        }
        ["switch", first, second, ..] => render_drops_inventory_switch(inventory, first, second),
        ["switch", ..] => "Error: Usage: inventory switch [slot 1] [slot 2]".to_string(),
        ["autosort", ..] => {
            inventory.autosort();
            "Autosorted inventory".to_string()
        }
        _ => "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]".to_string(),
    }
}

fn render_drops_item_inventory_command(
    inventory: &mut DropsInventory,
    gil: &mut i32,
    command: &str,
    item_name: &str,
    amount: &str,
) -> String {
    let Some(item) = data::item_name_by_key(item_name) else {
        return format!(
            "Error: item can only be one of these values: {}",
            drops_item_values()
        );
    };
    let Ok(amount) = amount.parse::<i32>() else {
        return "Error: Amount needs to be an integer".to_string();
    };
    if amount < 1 {
        return "Error: Amount needs to be more than 0".to_string();
    }
    match command {
        "get" => {
            inventory.add(item, amount);
            format!("Added {item} x{amount} to inventory")
        }
        "buy" => {
            let price = data::item_price(item).unwrap_or_default() * amount;
            if price > *gil {
                return format!("Error: Not enough gil (need {} more)", price - *gil);
            }
            *gil -= price;
            inventory.add(item, amount);
            format!("Bought {item} x{amount} for {price} gil")
        }
        "use" => match inventory.remove(item, amount) {
            Ok(()) => format!("Used {item} x{amount}"),
            Err(error) => format!("Error: {error}"),
        },
        "sell" => match inventory.remove(item, amount) {
            Ok(()) => {
                let price = (data::item_price(item).unwrap_or_default() / 4).max(1) * amount;
                *gil += price;
                format!("Sold {item} x{amount} for {price} gil")
            }
            Err(error) => format!("Error: {error}"),
        },
        _ => "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]".to_string(),
    }
}

fn drops_item_values() -> String {
    data::item_names_in_order()
        .iter()
        .map(|item| {
            item.to_ascii_lowercase()
                .replace(' ', "_")
                .replace(['(', ')', '\''], "")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn drops_autoability_values() -> String {
    data::autoability_names_in_order()
        .iter()
        .map(|ability| {
            ability
                .to_ascii_lowercase()
                .replace(' ', "_")
                .replace(['(', ')', '\''], "")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn render_drops_inventory_switch(
    inventory: &mut DropsInventory,
    first: &str,
    second: &str,
) -> String {
    let Ok(first_index) = first.parse::<i32>() else {
        return "Error: Inventory slot needs to be an integer".to_string();
    };
    let Ok(second_index) = second.parse::<i32>() else {
        return "Error: Inventory slot needs to be an integer".to_string();
    };
    let max_slot = inventory.items.len() as i32;
    if first_index < 1 || second_index < 1 || first_index > max_slot || second_index > max_slot {
        return format!(
            "Error: Inventory slot needs to be between 1 and {}",
            inventory.items.len()
        );
    }
    let first_index = first_index as usize;
    let second_index = second_index as usize;
    match inventory.switch(first_index - 1, second_index - 1) {
        Ok((first_item, second_item)) => format!(
            "Switched {second_item} (slot {first_index}) for {first_item} (slot {second_index})"
        ),
        Err(error) => format!("Error: {error}"),
    }
}

fn render_drops_equipment_inventory(equipment_inventory: &[Option<DropsEquipment>]) -> String {
    let mut lines = equipment_inventory
        .iter()
        .enumerate()
        .map(|(index, equipment)| {
            let equipment = equipment
                .as_ref()
                .map(format_drops_equipment)
                .unwrap_or_else(|| "None".to_string());
            format!("#{} {equipment}", index + 1)
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        lines.push("Empty".to_string());
    }
    format!("Equipment: {}", lines.join("\n           "))
}

fn render_drops_equipment_inventory_sell_slot(
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    gil: &mut i32,
    slot: &str,
) -> String {
    let Ok(slot) = slot.parse::<usize>() else {
        return "Error: Equipment slot needs to be an integer".to_string();
    };
    if equipment_inventory.is_empty() {
        return "Error: Equipment inventory is empty".to_string();
    }
    if slot == 0 || slot > equipment_inventory.len() {
        return format!(
            "Error: Equipment slot needs to be between 1 and {}",
            equipment_inventory.len()
        );
    }
    let Some(equipment) = equipment_inventory[slot - 1].take() else {
        return format!("Error: Slot {slot} is empty");
    };
    *gil += equipment.sell_value;
    while equipment_inventory.last().is_some_and(Option::is_none) {
        equipment_inventory.pop();
    }
    format!("Sold {}", format_drops_equipment(&equipment))
}

fn render_drops_equipment_inventory_get_or_buy(
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    gil: &mut i32,
    command: &str,
    kind: &str,
    character: &str,
    slots: &str,
    abilities: &[&str],
) -> String {
    let equipment = match parse_drops_equipment(kind, character, slots, abilities) {
        Ok(equipment) => equipment,
        Err(error) => {
            return render_drops_equipment_parse_error(
                error,
                &format!(
                "Usage: inventory {command} equipment [equip type] [character] [slots] (abilities)"
            ),
            )
        }
    };
    if command == "buy" {
        let gil_value = drops_equipment_gil_value(equipment.slots, &equipment.abilities);
        if gil_value > *gil {
            return format!("Error: Not enough gil (need {} more)", gil_value - *gil);
        }
        *gil -= gil_value;
        add_drops_equipment_inventory(equipment_inventory, equipment.clone());
        format!(
            "Bought {} for {gil_value} gil",
            format_drops_equipment(&equipment)
        )
    } else {
        add_drops_equipment_inventory(equipment_inventory, equipment.clone());
        format!("Added {}", format_drops_equipment(&equipment))
    }
}

fn render_drops_equipment_inventory_sell(
    gil: &mut i32,
    kind: &str,
    character: &str,
    slots: &str,
    abilities: &[&str],
) -> String {
    let equipment = match parse_drops_equipment(kind, character, slots, abilities) {
        Ok(equipment) => equipment,
        Err(error) => {
            return render_drops_equipment_parse_error(
                error,
                "Usage: inventory sell equipment [equip type] [character] [slots] (abilities)",
            )
        }
    };
    *gil += equipment.sell_value;
    format!("Sold {}", format_drops_equipment(&equipment))
}

fn render_drops_equipment_parse_error(error: DropsEquipmentParseError, _usage: &str) -> String {
    match error {
        DropsEquipmentParseError::EquipmentType => {
            "Error: equipment type can only be one of these values: weapon, armor".to_string()
        }
        DropsEquipmentParseError::Character => {
            format!("Error: character can only be one of these values: {DROPS_CHARACTER_VALUES}")
        }
        DropsEquipmentParseError::Slots => "Error: Slots must be between 0 and 4".to_string(),
        DropsEquipmentParseError::Ability => {
            format!(
                "Error: ability can only be one of these values: {}",
                drops_autoability_values()
            )
        }
    }
}

fn s_lv_to_ap(s_lv: i32) -> i32 {
    let ap = 5 * (s_lv + 1) + (s_lv.pow(3) / 50);
    ap.min(22_000)
}

fn s_lv_to_total_ap(s_lv: i32, starting_s_lv: i32) -> i32 {
    (0..s_lv)
        .map(|level| s_lv_to_ap(level + starting_s_lv))
        .sum()
}

fn total_ap_to_s_lv(total_ap: i32, starting_s_lv: i32) -> i32 {
    let mut ap = 0;
    let mut s_lv = 0;
    loop {
        if total_ap < ap {
            return s_lv - 1;
        }
        ap += s_lv_to_ap(s_lv + starting_s_lv);
        s_lv += 1;
    }
}

fn render_steal_line(
    rng: &mut FfxRngTracker,
    inventory: &mut DropsInventory,
    monster_key: &str,
    successful_steals: u32,
) -> String {
    let Some(monster) = data::monster_stats(monster_key) else {
        return format!("Steal: {monster_key} | Unknown monster");
    };
    let steal_roll = rng.advance_rng(10) % 255;
    let divisor = 2_u32.saturating_pow(successful_steals.min(31));
    let steal_chance = u32::from(monster.steal.base_chance) / divisor;
    let item = if steal_chance > steal_roll {
        let rarity_roll = rng.advance_rng(11) & 255;
        if rarity_roll < 32 {
            monster.steal.rare
        } else {
            monster.steal.common
        }
    } else {
        None
    };
    if let Some(item) = item.as_ref() {
        inventory.add(&item.item, i32::from(item.quantity));
    }
    format!(
        "Steal: {} | {}",
        monster.display_name,
        item.as_ref()
            .map(format_item_drop)
            .unwrap_or_else(|| "Failed".to_string())
    )
}

fn render_kill_line(
    rng: &mut FfxRngTracker,
    ap_state: &mut HashMap<Character, DropsApState>,
    inventory: &mut DropsInventory,
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    equipment_drops: &mut usize,
    gil: &mut i32,
    party: &[Character],
    monster_key: &str,
    killer: &str,
    ap_characters: &str,
    overkill: bool,
) -> String {
    let Some(monster) = data::monster_stats(monster_key) else {
        return format!("{} | Unknown monster", monster_display_name(monster_key));
    };
    let item_1 = roll_monster_item_drop(rng, &monster.item_1, overkill);
    let item_2 = roll_monster_item_drop(rng, &monster.item_2, overkill);
    for item in item_1.iter().chain(item_2.iter()) {
        inventory.add(&item.item, i32::from(item.quantity));
    }
    let equipment = roll_drops_equipment(rng, party, &monster, killer);
    if let Some(equipment) = equipment.as_ref() {
        *equipment_drops += 1;
        add_drops_equipment_inventory(equipment_inventory, equipment.clone());
    }
    let mut rendered = format!("Drops: {} | ", monster.display_name);
    match (item_1.as_ref(), item_2.as_ref()) {
        (Some(first), Some(second)) => rendered.push_str(&format!(
            "{}, {}",
            format_item_drop(first),
            format_item_drop(second)
        )),
        (Some(first), None) => rendered.push_str(&format_item_drop(first)),
        (None, Some(second)) => rendered.push_str(&format_item_drop(second)),
        (None, None) => rendered.push('-'),
    }
    let ap = if overkill {
        monster.overkill_ap
    } else {
        monster.normal_ap
    };
    *gil += monster.gil;
    rendered.push_str(&format!(" | {ap} AP"));
    let credited_characters = drops_characters_from_initials(ap_characters);
    if !credited_characters.is_empty() {
        credit_drops_ap(ap_state, &credited_characters, ap);
        rendered.push_str(" to ");
        rendered.push_str(&drops_character_initials(&credited_characters));
    }
    if overkill {
        rendered.push_str(" (OK)");
    }
    if let Some(equipment) = equipment {
        rendered.push_str(&format!(
            " | Equipment #{} {}",
            *equipment_drops,
            format_drops_equipment_drop(&equipment)
        ));
    }
    rendered
}

fn render_bribe_drop_line(
    rng: &mut FfxRngTracker,
    ap_state: &mut HashMap<Character, DropsApState>,
    inventory: &mut DropsInventory,
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    equipment_drops: &mut usize,
    gil: &mut i32,
    party: &[Character],
    monster_key: &str,
    user: &str,
    ap_characters: &str,
) -> String {
    let Some(monster) = data::monster_stats(monster_key) else {
        return format!("{} | Unknown monster", monster_display_name(monster_key));
    };
    let equipment = roll_drops_equipment(rng, party, &monster, user);
    if let Some(equipment) = equipment.as_ref() {
        *equipment_drops += 1;
        add_drops_equipment_inventory(equipment_inventory, equipment.clone());
    }
    let mut rendered = format!("Drops: {} | ", monster.display_name);
    if let Some(item) = monster.bribe.as_ref() {
        inventory.add(&item.item, i32::from(item.quantity));
        rendered.push_str(&format_item_drop(item));
    } else {
        rendered.push('-');
    }
    *gil += monster.gil;
    rendered.push_str(&format!(" | {} AP", monster.normal_ap));
    let credited_characters = drops_characters_from_initials(ap_characters);
    if !credited_characters.is_empty() {
        credit_drops_ap(ap_state, &credited_characters, monster.normal_ap);
        rendered.push_str(" to ");
        rendered.push_str(&drops_character_initials(&credited_characters));
    }
    if let Some(equipment) = equipment {
        rendered.push_str(&format!(
            " | Equipment #{} {}",
            *equipment_drops,
            format_drops_equipment_drop(&equipment)
        ));
    }
    rendered
}

fn roll_drops_equipment(
    rng: &mut FfxRngTracker,
    party: &[Character],
    monster: &data::MonsterStats,
    killer: &str,
) -> Option<DropsEquipment> {
    let equipment_roll = rng.advance_rng(10) % 255;
    if u32::from(monster.equipment.drop_chance) <= equipment_roll {
        return None;
    }
    let mut possible_owners = party
        .iter()
        .copied()
        .filter(|character| drops_ap_characters().contains(character))
        .collect::<Vec<_>>();
    if possible_owners.is_empty() {
        possible_owners.push(Character::Tidus);
    }
    let owner_roll = rng.advance_rng(12);
    let killer_bonus_chance = 3_u32;
    let killer_is_owner = owner_roll % (possible_owners.len() as u32 + killer_bonus_chance)
        >= possible_owners.len() as u32;
    let killer = killer.parse::<Character>().ok();
    if let Some(killer) = killer.filter(|character| drops_ap_characters().contains(character)) {
        for _ in 0..killer_bonus_chance {
            possible_owners.push(killer);
        }
    }
    let owner = possible_owners[owner_roll as usize % possible_owners.len()];
    let kind = if rng.advance_rng(12) & 1 == 0 {
        EquipmentKind::Weapon
    } else {
        EquipmentKind::Armor
    };
    let slots = monster
        .equipment
        .slots_range
        .get((rng.advance_rng(12) & 7) as usize)
        .copied()
        .unwrap_or(1);
    let max_ability_rolls = monster
        .equipment
        .max_ability_rolls_range
        .get((rng.advance_rng(12) & 7) as usize)
        .copied()
        .unwrap_or_default();
    let ability_list = monster
        .equipment
        .ability_lists
        .get(&kind)
        .and_then(|owners| owners.get(&owner))
        .cloned()
        .unwrap_or_else(|| vec![None; 8]);
    let mut abilities = Vec::new();
    if let Some(Some(forced)) = ability_list.first() {
        abilities.push(forced.clone());
    }
    let mut ability_rolls = 0;
    for _ in 0..max_ability_rolls {
        if abilities.len() >= slots as usize {
            break;
        }
        ability_rolls += 1;
        let ability_index = (rng.advance_rng(13) % 7 + 1) as usize;
        let Some(Some(ability)) = ability_list.get(ability_index) else {
            continue;
        };
        if !abilities.contains(ability) {
            abilities.push(ability.clone());
        }
    }
    Some(DropsEquipment {
        owner,
        kind,
        slots,
        sell_value: drops_equipment_sell_value(slots, &abilities),
        abilities,
        guaranteed: monster.equipment.drop_chance == 255,
        for_killer: killer_is_owner,
        ability_rolls: Some(ability_rolls),
    })
}

fn parse_drops_equipment(
    kind: &str,
    character: &str,
    slots: &str,
    abilities: &[&str],
) -> Result<DropsEquipment, DropsEquipmentParseError> {
    let kind = match kind {
        "weapon" => EquipmentKind::Weapon,
        "armor" => EquipmentKind::Armor,
        _ => return Err(DropsEquipmentParseError::EquipmentType),
    };
    let owner = character
        .parse::<Character>()
        .map_err(|_| DropsEquipmentParseError::Character)?;
    let mut slots = slots
        .parse::<u8>()
        .map_err(|_| DropsEquipmentParseError::Slots)?;
    if slots > 4 {
        return Err(DropsEquipmentParseError::Slots);
    }
    let mut parsed_abilities = Vec::new();
    for ability in abilities {
        if parsed_abilities.len() >= 4 {
            break;
        }
        let ability = parse_drops_ability_name(ability).ok_or(DropsEquipmentParseError::Ability)?;
        if !parsed_abilities.contains(&ability) {
            parsed_abilities.push(ability);
        }
    }
    slots = slots.max(parsed_abilities.len() as u8);
    Ok(DropsEquipment {
        owner,
        kind,
        slots,
        sell_value: drops_equipment_sell_value(slots, &parsed_abilities),
        abilities: parsed_abilities,
        guaranteed: false,
        for_killer: false,
        ability_rolls: None,
    })
}

fn parse_drops_ability_name(value: &str) -> Option<String> {
    data::autoability_names_in_order()
        .into_iter()
        .find(|ability| drops_autoability_value_key(ability) == value)
        .map(normalize_drops_ability_name)
}

fn drops_autoability_value_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace(' ', "_")
        .replace(['(', ')', '\''], "")
}

fn normalize_drops_ability_name(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace("->", "to")
        .replace([' ', '-'], "_")
        .replace(['(', ')', '\''], "")
}

fn add_drops_equipment_inventory(
    equipment_inventory: &mut Vec<Option<DropsEquipment>>,
    equipment: DropsEquipment,
) {
    if let Some(index) = equipment_inventory.iter().position(Option::is_none) {
        equipment_inventory[index] = Some(equipment);
    } else {
        equipment_inventory.push(Some(equipment));
    }
}

fn format_drops_equipment(equipment: &DropsEquipment) -> String {
    let name = drops_equipment_name(equipment);
    let mut abilities = equipment
        .abilities
        .iter()
        .map(|ability| {
            data::autoability_name_by_key(ability)
                .unwrap_or(ability)
                .to_string()
        })
        .collect::<Vec<_>>();
    while abilities.len() < equipment.slots as usize {
        abilities.push("-".to_string());
    }
    let mut rendered = format!(
        "{name} ({}) [{}][{} gil]",
        equipment.owner.display_name(),
        abilities.join(", "),
        equipment.sell_value
    );
    if equipment.guaranteed {
        rendered.push_str("(guaranteed)");
    }
    if equipment.for_killer {
        rendered.push_str("(for killer)");
    }
    rendered
}

fn format_drops_equipment_drop(equipment: &DropsEquipment) -> String {
    let mut rendered = format_drops_equipment(equipment);
    if let Some(ability_rolls) = equipment.ability_rolls {
        rendered.push_str(&format!(
            " ({} ability roll{})",
            ability_rolls,
            if ability_rolls == 1 { "" } else { "s" }
        ));
    }
    rendered
}

fn drops_equipment_name(equipment: &DropsEquipment) -> String {
    match (equipment.kind, equipment.owner) {
        (EquipmentKind::Weapon, Character::Seymour) => return "Seymour Staff".to_string(),
        (EquipmentKind::Armor, Character::Seymour) => return "Seymour Armor".to_string(),
        (EquipmentKind::Weapon, owner)
            if !matches!(
                owner,
                Character::Tidus
                    | Character::Yuna
                    | Character::Auron
                    | Character::Kimahri
                    | Character::Wakka
                    | Character::Lulu
                    | Character::Rikku
            ) =>
        {
            return format!("{}'s weapon", owner.display_name());
        }
        (EquipmentKind::Armor, owner)
            if !matches!(
                owner,
                Character::Tidus
                    | Character::Yuna
                    | Character::Auron
                    | Character::Kimahri
                    | Character::Wakka
                    | Character::Lulu
                    | Character::Rikku
            ) =>
        {
            return format!("{}'s armor", owner.display_name());
        }
        _ => {}
    }
    let index = match equipment.kind {
        EquipmentKind::Weapon => drops_weapon_name_index(&equipment.abilities, equipment.slots),
        EquipmentKind::Armor => drops_armor_name_index(&equipment.abilities, equipment.slots),
    };
    data::equipment_name(equipment.kind, equipment.owner, index).unwrap_or_else(|| {
        match equipment.kind {
            EquipmentKind::Weapon => "Weapon",
            EquipmentKind::Armor => "Armor",
        }
        .to_string()
    })
}

fn drops_weapon_name_index(abilities: &[String], slots: u8) -> usize {
    let elemental_strikes = drops_count_abilities(
        abilities,
        &["firestrike", "icestrike", "lightningstrike", "waterstrike"],
    );
    let status_strikes = drops_count_abilities(
        abilities,
        &[
            "deathstrike",
            "zombiestrike",
            "stonestrike",
            "poisonstrike",
            "sleepstrike",
            "silencestrike",
            "darkstrike",
            "slowstrike",
        ],
    );
    let status_touches = drops_count_abilities(
        abilities,
        &[
            "deathtouch",
            "zombietouch",
            "stonetouch",
            "poisontouch",
            "sleeptouch",
            "silencetouch",
            "darktouch",
            "slowtouch",
        ],
    );
    let strength_bonuses = drops_count_abilities(
        abilities,
        &[
            "strength_+3%",
            "strength_+5%",
            "strength_+10%",
            "strength_+20%",
        ],
    );
    let magic_bonuses = drops_count_abilities(
        abilities,
        &["magic_+3%", "magic_+5%", "magic_+10%", "magic_+20%"],
    );
    let counter = drops_has_ability(abilities, "counterattack")
        || drops_has_ability(abilities, "evade_&_counter");

    if drops_has_ability(abilities, "capture") {
        2
    } else if elemental_strikes == 4 {
        3
    } else if drops_has_ability(abilities, "break_damage_limit") {
        4
    } else if drops_has_all(
        abilities,
        &["triple_overdrive", "triple_ap", "overdrive_to_ap"],
    ) {
        5
    } else if drops_has_all(abilities, &["triple_overdrive", "overdrive_to_ap"]) {
        6
    } else if drops_has_all(abilities, &["double_overdrive", "double_ap"]) {
        7
    } else if drops_has_ability(abilities, "triple_overdrive") {
        8
    } else if drops_has_ability(abilities, "double_overdrive") {
        9
    } else if drops_has_ability(abilities, "triple_ap") {
        10
    } else if drops_has_ability(abilities, "double_ap") {
        11
    } else if drops_has_ability(abilities, "overdrive_to_ap") {
        12
    } else if drops_has_ability(abilities, "sos_overdrive") {
        13
    } else if drops_has_ability(abilities, "one_mp_cost") {
        14
    } else if status_strikes == 4 {
        15
    } else if strength_bonuses == 4 {
        16
    } else if magic_bonuses == 4 {
        17
    } else if drops_has_ability(abilities, "magic_booster") && magic_bonuses == 3 {
        18
    } else if drops_has_ability(abilities, "half_mp_cost") {
        19
    } else if drops_has_ability(abilities, "gillionaire") {
        20
    } else if elemental_strikes == 3 {
        21
    } else if status_strikes == 3 {
        22
    } else if drops_has_ability(abilities, "magic_counter") && counter {
        23
    } else if counter {
        24
    } else if drops_has_ability(abilities, "magic_counter") {
        25
    } else if drops_has_ability(abilities, "magic_booster") {
        26
    } else if drops_has_ability(abilities, "alchemy") {
        27
    } else if drops_has_ability(abilities, "first_strike") {
        28
    } else if drops_has_ability(abilities, "initiative") {
        29
    } else if drops_has_ability(abilities, "deathstrike") {
        30
    } else if drops_has_ability(abilities, "slowstrike") {
        31
    } else if drops_has_ability(abilities, "stonestrike") {
        32
    } else if drops_has_ability(abilities, "poisonstrike") {
        33
    } else if drops_has_ability(abilities, "sleepstrike") {
        34
    } else if drops_has_ability(abilities, "silencestrike") {
        35
    } else if drops_has_ability(abilities, "darkstrike") {
        36
    } else if strength_bonuses == 3 {
        37
    } else if magic_bonuses == 3 {
        38
    } else if elemental_strikes == 2 {
        39
    } else if status_touches >= 2 {
        40
    } else if drops_has_ability(abilities, "deathtouch") {
        41
    } else if drops_has_ability(abilities, "slowtouch") {
        42
    } else if drops_has_ability(abilities, "stonetouch") {
        43
    } else if drops_has_ability(abilities, "poisontouch") {
        44
    } else if drops_has_ability(abilities, "sleeptouch") {
        45
    } else if drops_has_ability(abilities, "silencetouch") {
        46
    } else if drops_has_ability(abilities, "darktouch") {
        47
    } else if drops_has_ability(abilities, "sensor") {
        48
    } else if drops_has_ability(abilities, "firestrike") {
        49
    } else if drops_has_ability(abilities, "icestrike") {
        50
    } else if drops_has_ability(abilities, "lightningstrike") {
        51
    } else if drops_has_ability(abilities, "waterstrike") {
        52
    } else if drops_has_ability(abilities, "distill_power") {
        53
    } else if drops_has_ability(abilities, "distill_mana") {
        54
    } else if drops_has_ability(abilities, "distill_speed") {
        55
    } else if drops_has_ability(abilities, "distill_ability") {
        56
    } else if slots == 4 {
        57
    } else if strength_bonuses >= 1 && magic_bonuses >= 1 {
        58
    } else if slots == 2 || slots == 3 {
        59
    } else if drops_has_any(abilities, &["magic_+10%", "magic_+20%"]) {
        60
    } else if drops_has_any(abilities, &["strength_+10%", "strength_+20%"]) {
        61
    } else if drops_has_ability(abilities, "magic_+5%") {
        62
    } else if drops_has_ability(abilities, "magic_+3%") {
        63
    } else if drops_has_ability(abilities, "strength_+5%") {
        64
    } else if drops_has_ability(abilities, "strength_+3%") {
        65
    } else if drops_has_ability(abilities, "piercing") {
        66
    } else {
        67
    }
}

fn drops_armor_name_index(abilities: &[String], slots: u8) -> usize {
    let elemental_eaters = drops_count_abilities(
        abilities,
        &["fire_eater", "ice_eater", "lightning_eater", "water_eater"],
    );
    let elemental_proofs = drops_count_abilities(
        abilities,
        &["fireproof", "iceproof", "lightningproof", "waterproof"],
    );
    let status_proofs = drops_count_abilities(
        abilities,
        &[
            "deathproof",
            "zombieproof",
            "stoneproof",
            "poisonproof",
            "sleepproof",
            "silenceproof",
            "darkproof",
            "slowproof",
            "confuseproof",
            "berserkproof",
            "curseproof",
        ],
    );
    let defense_bonuses = drops_count_abilities(
        abilities,
        &["defense_+3%", "defense_+5%", "defense_+10%", "defense_+20%"],
    );
    let magic_def_bonuses = drops_count_abilities(
        abilities,
        &[
            "magic_def_+3%",
            "magic_def_+5%",
            "magic_def_+10%",
            "magic_def_+20%",
        ],
    );
    let hp_bonuses = drops_count_abilities(abilities, &["hp_+5%", "hp_+10%", "hp_+20%", "hp_+30%"]);
    let mp_bonuses = drops_count_abilities(abilities, &["mp_+5%", "mp_+10%", "mp_+20%", "mp_+30%"]);
    let auto_statuses = drops_count_abilities(
        abilities,
        &[
            "auto_shell",
            "auto_protect",
            "auto_haste",
            "auto_regen",
            "auto_reflect",
        ],
    );
    let elemental_sos_auto_statuses = drops_count_abilities(
        abilities,
        &[
            "sos_nultide",
            "sos_nulfrost",
            "sos_nulshock",
            "sos_nulblaze",
        ],
    );
    let status_soses = drops_count_abilities(
        abilities,
        &[
            "sos_shell",
            "sos_protect",
            "sos_haste",
            "sos_regen",
            "sos_reflect",
        ],
    );

    if drops_has_all(abilities, &["break_hp_limit", "break_mp_limit"]) {
        0
    } else if drops_has_ability(abilities, "ribbon") {
        1
    } else if drops_has_ability(abilities, "break_hp_limit") {
        2
    } else if drops_has_ability(abilities, "break_mp_limit") {
        3
    } else if elemental_eaters == 4 {
        4
    } else if elemental_proofs == 4 {
        5
    } else if drops_has_all(
        abilities,
        &["auto_shell", "auto_protect", "auto_reflect", "auto_regen"],
    ) {
        6
    } else if drops_has_all(abilities, &["auto_potion", "auto_med", "auto_phoenix"]) {
        7
    } else if drops_has_all(abilities, &["auto_potion", "auto_med"]) {
        8
    } else if status_proofs == 4 {
        9
    } else if defense_bonuses == 4 {
        10
    } else if magic_def_bonuses == 4 {
        11
    } else if hp_bonuses == 4 {
        12
    } else if mp_bonuses == 4 {
        13
    } else if drops_has_ability(abilities, "master_thief") {
        14
    } else if drops_has_ability(abilities, "pickpocket") {
        15
    } else if drops_has_all(abilities, &["hp_stroll", "mp_stroll"]) {
        16
    } else if auto_statuses == 3 {
        17
    } else if elemental_eaters == 3 {
        18
    } else if drops_has_ability(abilities, "hp_stroll") {
        19
    } else if drops_has_ability(abilities, "mp_stroll") {
        20
    } else if drops_has_ability(abilities, "auto_phoenix") {
        21
    } else if drops_has_ability(abilities, "auto_med") {
        22
    } else if elemental_sos_auto_statuses == 4 {
        23
    } else if status_soses == 4 {
        24
    } else if status_proofs == 3 {
        25
    } else if drops_has_ability(abilities, "no_encounters") {
        26
    } else if drops_has_ability(abilities, "auto_potion") {
        27
    } else if elemental_proofs == 3 {
        28
    } else if status_soses == 3 {
        29
    } else if auto_statuses == 2 {
        30
    } else if elemental_sos_auto_statuses == 2 {
        31
    } else if drops_has_any(abilities, &["auto_regen", "sos_regen"]) {
        32
    } else if drops_has_any(abilities, &["auto_haste", "sos_haste"]) {
        33
    } else if drops_has_any(abilities, &["auto_reflect", "sos_reflect"]) {
        34
    } else if drops_has_any(abilities, &["auto_shell", "sos_shell"]) {
        35
    } else if drops_has_any(abilities, &["auto_protect", "sos_protect"]) {
        36
    } else if defense_bonuses == 3 {
        37
    } else if magic_def_bonuses == 3 {
        38
    } else if hp_bonuses == 3 {
        39
    } else if mp_bonuses == 3 {
        40
    } else if elemental_eaters + elemental_proofs >= 2 {
        41
    } else if status_proofs == 2 {
        42
    } else if drops_has_ability(abilities, "fire_eater") {
        43
    } else if drops_has_ability(abilities, "ice_eater") {
        44
    } else if drops_has_ability(abilities, "lightning_eater") {
        45
    } else if drops_has_ability(abilities, "water_eater") {
        46
    } else if drops_has_ability(abilities, "curseproof") {
        47
    } else if drops_has_any(abilities, &["confuse_ward", "confuseproof"]) {
        48
    } else if drops_has_any(abilities, &["berserk_ward", "berserkproof"]) {
        49
    } else if drops_has_any(abilities, &["slow_ward", "slowproof"]) {
        50
    } else if drops_has_any(abilities, &["death_ward", "deathproof"]) {
        51
    } else if drops_has_any(abilities, &["zombie_ward", "zombieproof"]) {
        52
    } else if drops_has_any(abilities, &["stone_ward", "stoneproof"]) {
        53
    } else if drops_has_any(abilities, &["poison_ward", "poisonproof"]) {
        54
    } else if drops_has_any(abilities, &["sleep_ward", "sleepproof"]) {
        55
    } else if drops_has_any(abilities, &["silence_ward", "silenceproof"]) {
        56
    } else if drops_has_any(abilities, &["dark_ward", "darkproof"]) {
        57
    } else if drops_has_any(abilities, &["fire_ward", "fireproof"]) {
        58
    } else if drops_has_any(abilities, &["ice_ward", "iceproof"]) {
        59
    } else if drops_has_any(abilities, &["lightning_ward", "lightningproof"]) {
        60
    } else if drops_has_any(abilities, &["water_ward", "waterproof"]) {
        61
    } else if drops_has_ability(abilities, "sos_nultide") {
        62
    } else if drops_has_ability(abilities, "sos_nulblaze") {
        63
    } else if drops_has_ability(abilities, "sos_nulshock") {
        64
    } else if drops_has_ability(abilities, "sos_nulfrost") {
        65
    } else if hp_bonuses == 2 && mp_bonuses == 2 {
        66
    } else if slots == 4 {
        67
    } else if defense_bonuses >= 1 && magic_def_bonuses >= 1 {
        68
    } else if defense_bonuses == 2 {
        69
    } else if magic_def_bonuses == 2 {
        70
    } else if hp_bonuses == 2 {
        71
    } else if mp_bonuses == 2 {
        72
    } else if drops_has_any(abilities, &["defense_+10%", "defense_+20%"]) {
        73
    } else if drops_has_any(abilities, &["magic_def_+10%", "magic_def_+20%"]) {
        74
    } else if drops_has_any(abilities, &["mp_+20%", "mp_+30%"]) {
        75
    } else if drops_has_any(abilities, &["hp_+20%", "hp_+30%"]) {
        76
    } else if slots == 3 {
        77
    } else if drops_has_any(abilities, &["defense_+3%", "defense_+5%"]) {
        78
    } else if drops_has_any(abilities, &["magic_def_+3%", "magic_def_+5%"]) {
        79
    } else if drops_has_any(abilities, &["mp_+5%", "mp_+10%"]) {
        80
    } else if drops_has_any(abilities, &["hp_+5%", "hp_+10%"]) {
        81
    } else if slots == 2 {
        82
    } else {
        83
    }
}

fn drops_has_ability(abilities: &[String], name: &str) -> bool {
    abilities.iter().any(|ability| ability == name)
}

fn drops_has_any(abilities: &[String], names: &[&str]) -> bool {
    names.iter().any(|name| drops_has_ability(abilities, name))
}

fn drops_has_all(abilities: &[String], names: &[&str]) -> bool {
    names.iter().all(|name| drops_has_ability(abilities, name))
}

fn drops_count_abilities(abilities: &[String], names: &[&str]) -> usize {
    abilities
        .iter()
        .filter(|ability| names.iter().any(|name| *ability == name))
        .count()
}

fn drops_equipment_gil_value(slots: u8, abilities: &[String]) -> i32 {
    let empty_slots = slots.saturating_sub(abilities.len() as u8) as usize;
    let base_gil_value = abilities
        .iter()
        .filter_map(|ability| data::autoability_gil_value(ability))
        .sum::<i32>();
    let slot_modifier = match slots {
        0 | 1 => 1.0,
        2 => 1.5,
        3 => 3.0,
        _ => 5.0,
    };
    let empty_modifier = match empty_slots {
        0 | 1 => 1.0,
        2 => 1.5,
        3 => 3.0,
        _ => 400.0,
    };
    ((50 + base_gil_value) as f64 * slot_modifier * empty_modifier) as i32
}

fn drops_equipment_sell_value(slots: u8, abilities: &[String]) -> i32 {
    drops_equipment_gil_value(slots, abilities) / 4
}

fn roll_monster_item_drop(
    rng: &mut FfxRngTracker,
    drop_info: &MonsterItemDropInfo,
    overkill: bool,
) -> Option<ItemDrop> {
    let drop_roll = rng.advance_rng(10) % 255;
    if u32::from(drop_info.drop_chance) <= drop_roll {
        return None;
    }
    let rarity_roll = rng.advance_rng(11) & 255;
    match (overkill, rarity_roll < 32) {
        (true, true) => drop_info.overkill_rare.clone(),
        (true, false) => drop_info.overkill_common.clone(),
        (false, true) => drop_info.normal_rare.clone(),
        (false, false) => drop_info.normal_common.clone(),
    }
}

fn format_item_drop(drop: &ItemDrop) -> String {
    let mut output = format!("{} x{}", drop.item, drop.quantity);
    if drop.rare {
        output.push_str(" (rare)");
    }
    output
}

fn monster_display_name(monster_key: &str) -> String {
    if let Some(monster) = data::monster_stats(monster_key) {
        return monster.display_name;
    }
    monster_key
        .split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn ensure_tracker_blank_line(lines: &mut Vec<String>) {
    if lines.last().is_some_and(|line| line.is_empty()) {
        return;
    }
    lines.push(String::new());
}

fn hide_tracker_output_before_marker(output: &str) -> String {
    let Some(tail) = output
        .rsplit("Command: ///")
        .next()
        .filter(|_| output.contains("Command: ///"))
    else {
        return output.to_string();
    };
    tail.split_once('\n')
        .map_or(tail, |(_, rest)| rest)
        .to_string()
}

fn pad_drops_tracker_output(lines: &[String]) -> String {
    let mut split_lines = Vec::new();
    for line in lines {
        if let Some((event_name, rest)) = line.split_once(':') {
            if rest.is_empty() {
                split_lines.push((line.clone(), Vec::new()));
            } else {
                split_lines.push((
                    event_name.to_string(),
                    rest.split('|').map(ToOwned::to_owned).collect::<Vec<_>>(),
                ));
            }
        } else {
            split_lines.push((line.clone(), Vec::new()));
        }
    }
    let mut paddings: std::collections::HashMap<String, std::collections::HashMap<usize, usize>> =
        std::collections::HashMap::new();
    for (event_name, parts) in &split_lines {
        if parts.is_empty() {
            continue;
        }
        let entry = paddings.entry(event_name.clone()).or_default();
        for (index, part) in parts.iter().enumerate() {
            let width = entry.entry(index).or_default();
            *width = (*width).max(part.len());
        }
    }
    if paddings.contains_key("Steal") && paddings.contains_key("Drops") {
        let steal_width = paddings
            .get("Steal")
            .and_then(|entry| entry.get(&0))
            .copied()
            .unwrap_or_default();
        let drops_width = paddings
            .get("Drops")
            .and_then(|entry| entry.get(&0))
            .copied()
            .unwrap_or_default();
        let width = steal_width.max(drops_width);
        paddings
            .entry("Steal".to_string())
            .or_default()
            .insert(0, width);
        paddings
            .entry("Drops".to_string())
            .or_default()
            .insert(0, width + 7);
    }

    split_lines
        .into_iter()
        .map(|(event_name, parts)| {
            if parts.is_empty() {
                return event_name;
            }
            let Some(widths) = paddings.get(&event_name) else {
                return format!("{event_name}:{}", parts.join("|"));
            };
            let padded = parts
                .into_iter()
                .enumerate()
                .map(|(index, part)| {
                    let width = widths.get(&index).copied().unwrap_or(part.len());
                    format!("{part:width$}")
                })
                .collect::<Vec<_>>();
            format!("{event_name}:{}", padded.join("|"))
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn edit_encounters_tracker_output(output: &str, padding: bool) -> String {
    let output = if let Some((_, tail)) = output.rsplit_once("Command: ///") {
        tail.split_once('\n').map_or(tail, |(_, rest)| rest)
    } else {
        output
    };

    let mut lines = Vec::new();
    let mut multiline_comment = false;
    for raw_line in output.lines() {
        let stripped = raw_line.trim();
        if stripped.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            if is_commented_encounters_tracker_command_row(raw_line) {
                if stripped.ends_with("*/") {
                    multiline_comment = false;
                }
                continue;
            }
            let line = raw_line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
            lines.push(format!("# {line}"));
            if stripped.ends_with("*/") {
                multiline_comment = false;
            }
            continue;
        }
        if raw_line == "///" {
            lines.clear();
            continue;
        }
        if raw_line == "/usage" || raw_line.starts_with("/usage ") {
            lines.extend(ENCOUNTERS_USAGE_TEXT.lines().map(ToOwned::to_owned));
            continue;
        }
        if raw_line == "/macro" || raw_line.starts_with("/macro ") {
            lines.push("Error: Possible macros are ".to_string());
            continue;
        }
        if raw_line.starts_with('/') {
            continue;
        }
        if raw_line.starts_with("Equipment:") || raw_line.starts_with("Command:") {
            continue;
        }
        if raw_line.starts_with("encounter ") {
            continue;
        }
        if is_commented_encounters_tracker_command_row(raw_line) {
            continue;
        }
        let commented = raw_line.strip_prefix("# ");
        let summary_line = commented
            .filter(|line| is_encounters_tracker_summary_row(line) || line.starts_with("====="))
            .unwrap_or(raw_line);
        if summary_line.starts_with("=====") {
            continue;
        }
        let mut line = summary_line.to_string();
        if line.starts_with("Random Encounter:") || line.starts_with("Simulated Encounter:") {
            let mut parts = line.split('|').map(str::to_string).collect::<Vec<_>>();
            if parts.len() > 2 {
                if parts.len() > 4 {
                    parts[0].push(' ');
                }
                parts.pop();
                parts.remove(1);
                line = parts.join("|");
            }
        } else if line.starts_with("Encounter:") || line.starts_with("Multizone encounter:") {
            let mut parts = line.split('|').map(str::to_string).collect::<Vec<_>>();
            if parts.len() > 2 {
                parts.pop();
                line = parts.join("|");
            }
        }
        line = line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
        if !line.trim().is_empty() {
            lines.push(line);
        }
    }

    if lines.is_empty() {
        String::new()
    } else {
        let mut output = lines.join("\n");
        output = output
            .replace("Multizone encounter:", "Encounter:")
            .replace("Random Encounter:", "Encounter:")
            .replace("Simulated Encounter:", "Encounter:")
            .replace(" Normal", "")
            .replace("| -", "");
        if padding {
            output = pad_drops_tracker_output(
                &output.lines().map(ToOwned::to_owned).collect::<Vec<_>>(),
            );
        }
        output = output.replace("Encounter: ", "");
        let mut lines = output
            .lines()
            .map(ToOwned::to_owned)
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>();
        let spacer = "=".repeat(lines.iter().map(|line| line.len()).max().unwrap_or(0));
        let mut index = 1;
        while index < lines.len() {
            if lines[index].starts_with("#    ") {
                lines.insert(index, spacer.clone());
                index += 1;
            }
            index += 1;
        }
        format!("{}\n", lines.join("\n"))
    }
}

fn is_encounters_tracker_summary_row(line: &str) -> bool {
    line.starts_with("Random Encounter:")
        || line.starts_with("Simulated Encounter:")
        || line.starts_with("Encounter:")
        || line.starts_with("Multizone encounter:")
}

fn is_commented_encounters_tracker_command_row(line: &str) -> bool {
    let mut text = line.trim_start();
    let mut had_comment_marker = false;
    while let Some(rest) = text.strip_prefix('#') {
        had_comment_marker = true;
        text = rest.trim_start();
    }
    if let Some(rest) = text.strip_prefix("/*") {
        had_comment_marker = true;
        text = rest.trim_start();
    }
    if let Some(rest) = text.strip_suffix("*/") {
        had_comment_marker = true;
        text = rest.trim_end();
    }
    had_comment_marker && text.starts_with("encounter ")
}

#[cfg(not(target_arch = "wasm32"))]
fn tracker_timer_start() -> Instant {
    Instant::now()
}

#[cfg(target_arch = "wasm32")]
fn tracker_timer_start() {}

#[cfg(not(target_arch = "wasm32"))]
fn tracker_duration_seconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64()
}

#[cfg(target_arch = "wasm32")]
fn tracker_duration_seconds(_: ()) -> f64 {
    0.0
}

fn character_response(character: Character) -> CharacterResponse {
    CharacterResponse {
        name: character.display_name().to_string(),
        input_name: character.input_name().to_string(),
    }
}

fn party_swap_choices() -> [Character; 7] {
    [
        Character::Tidus,
        Character::Yuna,
        Character::Auron,
        Character::Kimahri,
        Character::Wakka,
        Character::Lulu,
        Character::Rikku,
    ]
}

fn current_encounter_at_line(input: &str, cursor_line: usize) -> Option<ctb::EncounterBlock> {
    let encounters = ctb::scan_encounters_from_text(input);
    for encounter in encounters.iter().rev() {
        if cursor_line >= encounter.start_line && cursor_line <= encounter.end_line {
            return Some(encounter.clone());
        }
    }
    None
}

fn route_line_is_active(raw_line: &str, in_block_comment: &mut bool) -> bool {
    let stripped = raw_line.trim();
    if stripped.starts_with("/*") {
        if !stripped.ends_with("*/") {
            *in_block_comment = true;
        }
        return false;
    }
    if *in_block_comment {
        if stripped.ends_with("*/") {
            *in_block_comment = false;
        }
        return false;
    }
    true
}

struct ChocoboCursorState {
    state: SimulationState,
    shadow_ctbs: Option<HashMap<Character, i32>>,
}

impl ChocoboCursorState {
    fn new(seed: u32) -> Self {
        Self {
            state: SimulationState::new(seed),
            shadow_ctbs: None,
        }
    }

    fn apply_line(&mut self, line: &str) {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            return;
        }
        let command = parse_raw_action_line(line);
        let party_before_turn = self.state.party().to_vec();
        let is_party_change = matches!(command, ParsedCommand::Party { .. });
        let ticks_shadow_ctb = matches!(
            command,
            ParsedCommand::CharacterAction { .. }
                | ParsedCommand::MonsterAction { .. }
                | ParsedCommand::YojimboTurn { .. }
                | ParsedCommand::MagusTurn { .. }
        );
        self.state.execute_raw_line(line);
        if ticks_shadow_ctb {
            self.apply_shadow_turn(&party_before_turn);
        }
        if !self.state.has_living_monsters() {
            self.shadow_ctbs = None;
            return;
        }
        if is_party_change {
            self.ensure_shadow_members();
            self.apply_shadow_party_change();
        }
    }

    fn apply_shadow_turn(&mut self, party_before_turn: &[Character]) {
        let Some(shadow_ctbs) = self.shadow_ctbs.as_mut() else {
            return;
        };
        let delta = self.state.ctb_since_last_action();
        for character in party_swap_choices() {
            if party_before_turn.contains(&character) {
                if let Some(ctb) = self.state.character_ctb(character) {
                    shadow_ctbs.insert(character, ctb);
                }
            } else if let Some(ctb) = shadow_ctbs.get_mut(&character) {
                *ctb = (*ctb - delta).max(0);
            }
        }
    }

    fn ensure_shadow_members(&mut self) {
        let shadow_ctbs = self.shadow_ctbs.get_or_insert_with(HashMap::new);
        for character in self.state.party() {
            if shadow_ctbs.contains_key(character) {
                continue;
            }
            if let Some(ctb) = self.state.character_ctb(*character) {
                let ctb = if ctb == 0 {
                    self.state
                        .character_shadow_ctb_fallback(*character)
                        .unwrap_or(ctb)
                } else {
                    ctb
                };
                shadow_ctbs.insert(*character, ctb);
            }
        }
    }

    fn apply_shadow_party_change(&mut self) {
        let Some(shadow_ctbs) = self.shadow_ctbs.as_mut() else {
            return;
        };
        let party = self.state.party().to_vec();
        for character in &party {
            if let Some(ctb) = shadow_ctbs.get(character).copied() {
                self.state.set_character_ctb(*character, ctb);
            }
        }
        for character in party {
            if let Some(ctb) = self.state.character_ctb(character) {
                shadow_ctbs.insert(character, ctb);
            }
        }
    }
}

fn simulate_to_chocobo_cursor_state(
    seed: u32,
    input: &str,
    cursor_line: usize,
) -> ChocoboCursorState {
    let prepared = prepare_action_lines(input);
    let prepared_prefix = prepare_action_lines_before_raw_line(input, cursor_line);
    let encounter_parties = infer_chocobo_encounter_parties(&prepared.lines);
    let encounter_party_swaps = infer_chocobo_encounter_party_swaps(&prepared.lines);
    let mut cursor_state = ChocoboCursorState::new(seed);
    let mut current_party_plan: Option<Vec<Character>> = None;
    let mut multiline_comment = false;
    for (absolute_index, raw_line) in prepared_prefix.lines.into_iter().enumerate() {
        if !route_line_is_active(&raw_line, &mut multiline_comment) {
            continue;
        }
        if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
            continue;
        }
        let command = parse_raw_action_line(&raw_line);
        if matches!(command, ParsedCommand::Encounter { .. }) {
            if encounter_party_swaps.contains(&absolute_index) {
                current_party_plan = Some(cursor_state.state.party().to_vec());
            } else if let Some(planned_party) = encounter_parties.get(&absolute_index) {
                cursor_state.state.sync_party_if_needed(planned_party);
                current_party_plan = Some(planned_party.clone());
            } else {
                current_party_plan = None;
            }
            cursor_state.shadow_ctbs = None;
        }
        if let (ParsedCommand::CharacterAction { actor, .. }, Some(planned_party)) =
            (&command, current_party_plan.as_deref())
        {
            if !cursor_state.state.party().contains(actor) {
                cursor_state.state.sync_party_if_needed(planned_party);
            }
        }
        cursor_state.apply_line(&raw_line);
        if matches!(command, ParsedCommand::Party { .. }) {
            current_party_plan = Some(cursor_state.state.party().to_vec());
        }
    }
    cursor_state
}

fn infer_chocobo_encounter_parties(lines: &[String]) -> HashMap<usize, Vec<Character>> {
    let mut encounter_parties = HashMap::new();
    let mut current_encounter_index = None;
    let mut current_party = Vec::new();
    let mut in_block_comment = false;
    for (index, raw_line) in lines.iter().enumerate() {
        if !route_line_is_active(raw_line, &mut in_block_comment) {
            continue;
        }
        if raw_line.trim().is_empty() || raw_line.starts_with('#') {
            continue;
        }
        match parse_raw_action_line(raw_line) {
            ParsedCommand::Encounter { .. } => {
                if let Some(encounter_index) = current_encounter_index {
                    if !current_party.is_empty() {
                        encounter_parties.insert(encounter_index, current_party.clone());
                    }
                }
                current_encounter_index = Some(index);
                current_party.clear();
            }
            ParsedCommand::CharacterAction { actor, .. } => {
                if !current_party.contains(&actor) {
                    current_party.push(actor);
                }
            }
            _ => {}
        }
    }
    if let Some(encounter_index) = current_encounter_index {
        if !current_party.is_empty() {
            encounter_parties.insert(encounter_index, current_party);
        }
    }
    encounter_parties
}

fn infer_chocobo_encounter_party_swaps(lines: &[String]) -> HashSet<usize> {
    let mut encounter_party_swaps = HashSet::new();
    let mut current_encounter_index = None;
    let mut in_block_comment = false;
    for (index, raw_line) in lines.iter().enumerate() {
        if !route_line_is_active(raw_line, &mut in_block_comment) {
            continue;
        }
        if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
            continue;
        }
        match parse_raw_action_line(raw_line) {
            ParsedCommand::Encounter { .. } => current_encounter_index = Some(index),
            ParsedCommand::Party { .. } => {
                if let Some(encounter_index) = current_encounter_index {
                    encounter_party_swaps.insert(encounter_index);
                }
            }
            _ => {}
        }
    }
    encounter_party_swaps
}

fn get_chocobo_effective_insert_line(
    input: &str,
    encounter: &ctb::EncounterBlock,
    cursor_line: usize,
) -> usize {
    let lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(lines.len());
    let mut in_block_comment = false;
    let haste_line = (encounter.start_line..=encounter_end).find(|line_number| {
        lines
            .get(line_number.saturating_sub(1))
            .is_some_and(|line| {
                route_line_is_active(line, &mut in_block_comment)
                    && line
                        .trim()
                        .eq_ignore_ascii_case("tidus haste chocobo_eater")
            })
    });
    haste_line.map_or(cursor_line, |line| cursor_line.max(line + 1))
}

fn get_chocobo_swap_insert_line(encounter: &ctb::EncounterBlock, cursor_line: usize) -> usize {
    cursor_line.max(encounter.start_line + 1)
}

fn build_chocobo_enemy_action_line(
    action_kind: &str,
    slot_index: Option<usize>,
    state: &SimulationState,
) -> Result<String, ApiError> {
    match action_kind {
        "attack_slot" => {
            let slot_index = slot_index.ok_or_else(|| {
                ApiError::BadRequest("That party slot is not currently available.".into())
            })?;
            let target = state.party().get(slot_index).ok_or_else(|| {
                ApiError::BadRequest("That party slot is not currently available.".into())
            })?;
            Ok(format!("m1 attack {}", target.input_name()))
        }
        "generic_attack" => Ok("m1 attack".to_string()),
        "fists_of_fury" => Ok("m1 fists_of_fury".to_string()),
        "thwack" => Ok("m1 thwack".to_string()),
        _ => Err(ApiError::BadRequest(format!(
            "Unsupported Chocobo Eater action '{action_kind}'."
        ))),
    }
}

fn build_chocobo_character_filler_line(
    state: &SimulationState,
    character: Character,
    action_kind: &str,
    slot_index: Option<usize>,
) -> Result<String, ApiError> {
    if let Some(replacement) = chocobo_ko_replacement(state, character, action_kind, slot_index)? {
        let mut party = state.party().to_vec();
        if let Some(slot) = party.iter().position(|candidate| *candidate == character) {
            party[slot] = replacement;
            return Ok(format!("party {}", party_to_initials(&party)));
        }
    }
    Ok(format!("{} defend", character.input_name()))
}

fn chocobo_ko_replacement(
    state: &SimulationState,
    character: Character,
    action_kind: &str,
    slot_index: Option<usize>,
) -> Result<Option<Character>, ApiError> {
    if action_kind != "attack_slot" {
        return Ok(None);
    }
    let Some(target) = slot_index.and_then(|slot| state.party().get(slot)).copied() else {
        return Ok(None);
    };
    if target != character {
        return Ok(None);
    }
    let enemy_line = build_chocobo_enemy_action_line(action_kind, slot_index, state)?;
    let before_hp = state.character_hp(character).unwrap_or(0);
    let mut projected = state.clone();
    projected.execute_raw_line(&enemy_line);
    let after_hp = projected.character_hp(character).unwrap_or(0);
    if before_hp <= 0 || after_hp > 0 {
        return Ok(None);
    }
    Ok(choose_chocobo_swap_replacement(state))
}

fn choose_chocobo_swap_replacement(state: &SimulationState) -> Option<Character> {
    let party = state.party();
    party_swap_choices()
        .into_iter()
        .enumerate()
        .filter(|(_, character)| !party.contains(character))
        .filter_map(|(index, character)| {
            if !state.character_is_alive(character) {
                return None;
            }
            let hp = state.character_hp(character)?;
            Some((character, hp, index))
        })
        .max_by_key(|(_, hp, index)| (*hp, std::cmp::Reverse(*index)))
        .map(|(character, _, _)| character)
}

fn party_to_initials(party: &[Character]) -> String {
    party
        .iter()
        .map(|character| character.input_name().chars().next().unwrap_or('u'))
        .collect()
}

fn parse_encounter_sliders() -> Vec<EncounterSliderResponse> {
    ENCOUNTERS_NOTES
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .enumerate()
        .filter_map(|(index, line)| {
            let fields = line.split(',').map(str::trim).collect::<Vec<_>>();
            let name = fields.first()?.to_string();
            let initiative = fields
                .get(1)
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            let label = fields
                .get(2)
                .filter(|value| !value.is_empty())
                .map(|value| (*value).to_string())
                .unwrap_or_else(|| name.clone());
            Some(EncounterSliderResponse {
                index,
                name,
                label,
                min: parse_i32_field(&fields, 3, 0),
                default: parse_i32_field(&fields, 4, 0),
                max: parse_i32_field(&fields, 5, 0),
                initiative,
            })
        })
        .collect()
}

fn build_encounters_default_input() -> String {
    let mut input = String::from("/nopadding\n/usage\n");
    let mut initiative_equipped = false;
    for slider in parse_encounter_sliders() {
        if slider.initiative && !initiative_equipped {
            input.push_str("weapon tidus 1 initiative\n");
            initiative_equipped = true;
        } else if !slider.initiative && initiative_equipped {
            input.push_str("weapon tidus 1\n");
            initiative_equipped = false;
        }
        let line = encounter_input_line(&slider.name);
        if slider.min == slider.max {
            for _ in 0..slider.default.max(0) {
                input.push_str(&line);
                input.push('\n');
            }
            continue;
        }
        input.push_str("\n#    ");
        input.push_str(&slider.label);
        input.push('\n');
        for _ in 0..slider.default.max(0) {
            input.push_str(&line);
            input.push('\n');
        }
        for _ in slider.default.max(0)..slider.max.max(slider.default).max(0) {
            input.push_str("# ");
            input.push_str(&line);
            input.push('\n');
        }
    }
    input.trim_end_matches('\n').to_string()
}

fn encounter_input_line(name: &str) -> String {
    if name.contains(' ') {
        format!("encounter multizone {name}")
    } else {
        format!("encounter {name}")
    }
}

fn parse_i32_field(fields: &[&str], index: usize, default: i32) -> i32 {
    fields
        .get(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = panic.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::Value;

    use crate::rng::FfxRngTracker;
    use crate::simulator::SimulationState;

    use super::{
        chocobo_action_json, chocobo_swap_json, choose_chocobo_swap_replacement,
        drops_autoability_values, drops_item_values, drops_search_first_ghost_line,
        drops_search_window, edit_encounters_tracker_output, find_active_repeat_after_ghost,
        find_first_ghost_route_line, find_no_encounters_ghost_drop,
        first_ghost_search_window_input, garuda1_attacks_json, garuda2_attack_json,
        lancet_tutorial_timing_json, no_encounters_routes_json, no_encounters_synthesis_families,
        parse_exact_future_encounter_output, party_json, render_ctb_diff_json, render_ctb_json,
        render_drops_tracker, render_steal_line, sample_json, simulate_to_chocobo_cursor_state,
        synthesize_no_encounters_ghost_route, tanker_pattern_json, tracker_default_json,
        tracker_render_json, tros_attack_json, ChocoboCursorState, DropsInventory, DEFAULT_INPUT,
        DEFAULT_SEED, DROPS_NOTES,
    };

    #[test]
    fn sample_payload_matches_python_web_default_shape() {
        let payload: Value = serde_json::from_str(&sample_json().unwrap()).unwrap();
        assert_eq!(payload["seed"], DEFAULT_SEED);
        assert!(payload["input"]
            .as_str()
            .is_some_and(|input| input.contains("encounter tanker")));
    }

    #[test]
    fn render_ctb_payload_matches_python_trailing_newline() {
        let payload: Value =
            serde_json::from_str(&render_ctb_json(DEFAULT_SEED, "encounter tanker\n").unwrap())
                .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Tanker"));
        assert!(output.ends_with('\n'));
        assert!(!output.ends_with("\n\n"));

        let empty_payload: Value =
            serde_json::from_str(&render_ctb_json(DEFAULT_SEED, "").unwrap()).unwrap();
        assert_eq!(empty_payload["output"], "\n");
    }

    #[test]
    fn render_ctb_payload_reports_changed_line_from_previous_input() {
        let payload: Value = serde_json::from_str(
            &render_ctb_diff_json(
                DEFAULT_SEED,
                "encounter tanker\ntidus cheer\n",
                "encounter tanker\ntidus attack m1\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(payload["changed_line"], 2);
        assert_eq!(payload["checkpoint_line"], 1);
    }

    #[test]
    fn party_payload_reports_party_and_reserves_at_cursor() {
        let payload: Value =
            serde_json::from_str(&party_json(DEFAULT_SEED, "party tay\nparty twl\n", 2).unwrap())
                .unwrap();
        let party = payload["party"].as_array().unwrap();
        let reserves = payload["reserves"].as_array().unwrap();
        assert_eq!(party[0]["input_name"], "tidus");
        assert_eq!(party[1]["input_name"], "auron");
        assert_eq!(party[2]["input_name"], "yuna");
        assert!(reserves
            .iter()
            .any(|character| character["input_name"] == "wakka"));
    }

    #[test]
    fn party_payload_replays_macros_before_raw_cursor_line() {
        let payload: Value = serde_json::from_str(
            &party_json(
                DEFAULT_SEED,
                "/macro moonflow grid\nparty tw\nstatus atb\n",
                3,
            )
            .unwrap(),
        )
        .unwrap();
        let party = payload["party"].as_array().unwrap();
        assert_eq!(party[0]["input_name"], "tidus");
        assert_eq!(party[1]["input_name"], "wakka");
    }

    #[test]
    fn party_payload_uses_raw_cursor_line_after_default_macros() {
        let payload: Value =
            serde_json::from_str(&party_json(DEFAULT_SEED, DEFAULT_INPUT, 624).unwrap()).unwrap();
        let party = payload["party"].as_array().unwrap();
        assert_eq!(
            party
                .iter()
                .map(|member| member["input_name"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["tidus", "auron", "wakka"]
        );
    }

    #[test]
    fn chocobo_action_payload_generates_insert_lines_inside_encounter() {
        let payload: Value = serde_json::from_str(
            &chocobo_action_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\nstatus atb\n",
                2,
                "generic_attack",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 2);
        let lines = payload["lines"].as_array().unwrap();
        assert!(!lines.is_empty());
        assert!(lines
            .last()
            .and_then(|line| line.as_str())
            .is_some_and(|line| line.starts_with("m1 ")));
    }

    #[test]
    fn chocobo_action_payload_ignores_block_commented_haste_insert_line() {
        let payload: Value = serde_json::from_str(
            &chocobo_action_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\n/*\ntidus haste chocobo_eater\n*/\nstatus atb\n",
                2,
                "generic_attack",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 2);
    }

    #[test]
    fn chocobo_action_payload_accepts_trailing_blank_line_inside_last_encounter() {
        let payload: Value = serde_json::from_str(
            &chocobo_action_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\nstatus atb\n",
                3,
                "generic_attack",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 3);
    }

    #[test]
    fn chocobo_action_payload_uses_raw_cursor_line_after_default_macros() {
        let payload: Value = serde_json::from_str(
            &chocobo_action_json(DEFAULT_SEED, DEFAULT_INPUT, 624, "generic_attack", None).unwrap(),
        )
        .unwrap();
        let lines = payload["lines"].as_array().unwrap();
        assert!(lines
            .last()
            .and_then(|line| line.as_str())
            .is_some_and(|line| line.starts_with("m1 ")));
    }

    #[test]
    fn chocobo_action_payload_rejects_other_encounters() {
        let error = chocobo_action_json(
            DEFAULT_SEED,
            "encounter tanker\nstatus atb\n",
            2,
            "generic_attack",
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("chocobo_eater encounter"));
    }

    #[test]
    fn chocobo_action_payload_rejects_cursor_before_first_encounter() {
        let error = chocobo_action_json(
            DEFAULT_SEED,
            "# route heading\nencounter chocobo_eater\nstatus atb\n",
            1,
            "generic_attack",
            None,
        )
        .unwrap_err();
        assert!(error.to_string().contains("chocobo_eater encounter"));
    }

    #[test]
    fn chocobo_action_payload_requires_active_encounter_state_like_python() {
        let error = chocobo_action_json(
            DEFAULT_SEED,
            "encounter chocobo_eater\nstatus atb\n",
            1,
            "generic_attack",
            None,
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("active Chocobo Eater encounter state"));
    }

    #[test]
    fn tanker_pattern_payload_builds_python_lines() {
        let payload: Value = serde_json::from_str(
            &tanker_pattern_json("encounter tanker\nstatus atb\n", 2, "awsdn-").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 2);
        assert_eq!(payload["end_line"], 1);
        let lines = payload["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            lines,
            vec![
                "m5 attack",
                "m7 wings_flicker",
                "m6 does_nothing",
                "m2 does_nothing",
                "m3 does_nothing",
                "m4 does_nothing",
                "m8 does_nothing",
                "",
                "m5 does_nothing",
                "m7 spines",
                "m6 does_nothing",
                "m2 does_nothing",
                "m3 does_nothing",
                "m4 does_nothing",
                "m8 does_nothing",
            ]
        );
    }

    #[test]
    fn tanker_pattern_payload_rejects_invalid_pattern() {
        let error = tanker_pattern_json("encounter tanker\n", 1, "a?").unwrap_err();
        assert!(error
            .to_string()
            .contains("Use 2 to 14 letters with only a, w, s, d, n, or -."));

        let error = tanker_pattern_json("encounter tanker\n", 1, "a").unwrap_err();
        assert!(error
            .to_string()
            .contains("Use 2 to 14 letters with only a, w, s, d, n, or -."));
    }

    #[test]
    fn tanker_pattern_payload_rejects_other_encounters() {
        let error =
            tanker_pattern_json("encounter chocobo_eater\nstatus atb\n", 2, "aa").unwrap_err();
        assert!(error.to_string().contains("tanker encounter"));
    }

    #[test]
    fn tanker_pattern_payload_rejects_cursor_before_first_encounter() {
        let error = tanker_pattern_json("# route heading\nencounter tanker\nstatus atb\n", 1, "aa")
            .unwrap_err();
        assert!(error.to_string().contains("tanker encounter"));
    }

    #[test]
    fn tanker_pattern_payload_replaces_existing_tanker_slots() {
        let payload: Value = serde_json::from_str(
            &tanker_pattern_json(
                "encounter tanker\n# note\nm5 attack\nm7 attack\nstatus atb\nm8 attack\n",
                4,
                "aa",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 3);
        assert_eq!(payload["end_line"], 6);
    }

    #[test]
    fn tros_attack_payload_replaces_existing_first_attack_like_python_frontend() {
        let payload: Value = serde_json::from_str(
            &tros_attack_json("encounter tros\nstatus atb\nm1 attack\n", 3, "tentacles").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 3);
        assert_eq!(payload["end_line"], 3);
        assert_eq!(payload["lines"], serde_json::json!(["m1 tentacles"]));
    }

    #[test]
    fn tros_attack_payload_inserts_at_cursor_when_missing_like_python_frontend() {
        let payload: Value = serde_json::from_str(
            &tros_attack_json("encounter tros\nstatus atb\n", 2, "attack").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 2);
        assert_eq!(payload["end_line"], 1);
        assert_eq!(payload["lines"], serde_json::json!(["m1 attack"]));
    }

    #[test]
    fn tros_attack_payload_ignores_block_commented_first_attack() {
        let payload: Value = serde_json::from_str(
            &tros_attack_json(
                "encounter tros\n/*\nm1 tentacles\n*/\nstatus atb\n",
                5,
                "attack",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 5);
        assert_eq!(payload["end_line"], 4);
    }

    #[test]
    fn tros_attack_payload_rejects_invalid_context_or_attack() {
        let error = tros_attack_json("encounter tanker\nstatus atb\n", 2, "attack").unwrap_err();
        assert!(error.to_string().contains("tros encounter"));

        let error = tros_attack_json("encounter tros\n", 1, "sonic_boom").unwrap_err();
        assert!(error
            .to_string()
            .contains("Use attack or tentacles for the Tros first attack."));
    }

    #[test]
    fn garuda1_attacks_payload_replaces_matching_rows_and_preserves_notes() {
        let payload: Value = serde_json::from_str(
            &garuda1_attacks_json(
                "encounter garuda_1\n# note\nm1 attack\nm1 sonic_boom\n#m1 attack\n",
                3,
                "attack,sonic_boom,attack,sonic_boom,attack",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 1);
        assert_eq!(payload["end_line"], 5);
        assert_eq!(
            payload["lines"],
            serde_json::json!([
                "encounter garuda_1",
                "# note",
                "m1 attack",
                "m1 sonic_boom",
                "m1 attack",
                "m1 sonic_boom",
                "m1 attack"
            ])
        );
    }

    #[test]
    fn garuda1_attacks_payload_inserts_at_cursor_when_missing() {
        let payload: Value = serde_json::from_str(
            &garuda1_attacks_json(
                "encounter garuda_1\nstatus atb\n",
                2,
                "attack,attack,sonic_boom,attack,sonic_boom",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 2);
        assert_eq!(payload["end_line"], 1);
        assert_eq!(
            payload["lines"],
            serde_json::json!([
                "m1 attack",
                "m1 attack",
                "m1 sonic_boom",
                "m1 attack",
                "m1 sonic_boom"
            ])
        );
    }

    #[test]
    fn garuda1_attacks_payload_ignores_block_commented_rows() {
        let payload: Value = serde_json::from_str(
            &garuda1_attacks_json(
                "encounter garuda_1\n/*\nm1 attack\n*/\nstatus atb\n",
                5,
                "attack,attack,attack,attack,attack",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 5);
        assert_eq!(payload["end_line"], 4);
    }

    #[test]
    fn garuda1_attacks_payload_rejects_invalid_context_or_actions() {
        let error = garuda1_attacks_json(
            "encounter garuda_2\nstatus atb\n",
            2,
            "attack,attack,attack,attack,attack",
        )
        .unwrap_err();
        assert!(error.to_string().contains("garuda_1 encounter"));

        let error = garuda1_attacks_json("encounter garuda_1\n", 1, "attack,attack,sonic_boom")
            .unwrap_err();
        assert!(error.to_string().contains("Use exactly 5 Garuda 1 attacks"));
    }

    #[test]
    fn garuda2_attack_payload_replaces_matching_row() {
        let payload: Value = serde_json::from_str(
            &garuda2_attack_json(
                DEFAULT_SEED,
                "encounter garuda_2\n# note\nm1 attack\n",
                3,
                "sonic_boom",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 1);
        assert_eq!(payload["end_line"], 3);
        assert_eq!(
            payload["lines"],
            serde_json::json!(["encounter garuda_2", "# note", "m1 sonic_boom"])
        );
    }

    #[test]
    fn garuda2_attack_payload_removes_matching_row_for_does_nothing() {
        let payload: Value = serde_json::from_str(
            &garuda2_attack_json(
                DEFAULT_SEED,
                "encounter garuda_2\nm1 sonic_boom\nstatus atb\n",
                2,
                "does_nothing",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["lines"],
            serde_json::json!(["encounter garuda_2", "status atb"])
        );
    }

    #[test]
    fn garuda2_attack_payload_noops_does_nothing_when_no_row_exists() {
        let payload: Value = serde_json::from_str(
            &garuda2_attack_json(
                DEFAULT_SEED,
                "encounter garuda_2\nstatus atb\n",
                2,
                "does_nothing",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 2);
        assert_eq!(payload["end_line"], 1);
        assert_eq!(payload["lines"], serde_json::json!([]));
    }

    #[test]
    fn garuda2_attack_payload_inserts_after_opening_party_turns_like_python_frontend() {
        let payload: Value = serde_json::from_str(
            &garuda2_attack_json(
                DEFAULT_SEED,
                "party tyl\n\nencounter garuda_2\ntidus escape\nyuna escape\nlulu escape\n",
                5,
                "attack",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 7);
        assert_eq!(payload["end_line"], 6);
        assert_eq!(payload["lines"], serde_json::json!(["m1 attack"]));
    }

    #[test]
    fn garuda2_attack_payload_falls_back_to_end_when_no_opening_cue_exists() {
        let payload: Value = serde_json::from_str(
            &garuda2_attack_json(
                DEFAULT_SEED,
                "encounter garuda_2\nstatus atb\n",
                2,
                "sonic_boom",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 3);
        assert_eq!(payload["end_line"], 2);
        assert_eq!(payload["lines"], serde_json::json!(["m1 sonic_boom"]));
    }

    #[test]
    fn garuda2_attack_payload_rejects_invalid_context_or_attack() {
        let error =
            garuda2_attack_json(DEFAULT_SEED, "encounter garuda_1\nm1 attack\n", 2, "attack")
                .unwrap_err();
        assert!(error.to_string().contains("garuda_2 encounter"));

        let error =
            garuda2_attack_json(DEFAULT_SEED, "encounter garuda_2\n", 1, "tentacles").unwrap_err();
        assert!(error
            .to_string()
            .contains("Use attack, sonic_boom, or does_nothing for Garuda 2."));
    }

    #[test]
    fn lancet_tutorial_payload_toggles_ragora_lines_before_lancet() {
        let payload: Value = serde_json::from_str(
            &lancet_tutorial_timing_json(
                "encounter lancet_tutorial\n# note\n#m1 seed_cannon_kimahri\nm1 seed_cannon\n",
                3,
                "before",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 1);
        assert_eq!(payload["end_line"], 4);
        assert_eq!(
            payload["lines"],
            serde_json::json!([
                "encounter lancet_tutorial",
                "# note",
                "m1 seed_cannon_kimahri",
                "#m1 seed_cannon"
            ])
        );
    }

    #[test]
    fn lancet_tutorial_payload_toggles_ragora_lines_after_lancet() {
        let payload: Value = serde_json::from_str(
            &lancet_tutorial_timing_json(
                "encounter lancet_tutorial\nm1 seed_cannon_kimahri\n#m1 seed_cannon\n",
                2,
                "after",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["lines"],
            serde_json::json!([
                "encounter lancet_tutorial",
                "#m1 seed_cannon_kimahri",
                "m1 seed_cannon"
            ])
        );
    }

    #[test]
    fn lancet_tutorial_payload_rejects_missing_or_block_commented_lines() {
        let error = lancet_tutorial_timing_json(
            "encounter lancet_tutorial\n/*\nm1 seed_cannon_kimahri\n*/\nm1 seed_cannon\n",
            5,
            "before",
        )
        .unwrap_err();
        assert!(error
            .to_string()
            .contains("Could not find both Ragora attack lines"));
    }

    #[test]
    fn lancet_tutorial_payload_rejects_invalid_context_or_timing() {
        let error = lancet_tutorial_timing_json(
            "encounter garuda_1\nm1 seed_cannon_kimahri\nm1 seed_cannon\n",
            2,
            "before",
        )
        .unwrap_err();
        assert!(error.to_string().contains("lancet_tutorial encounter"));

        let error =
            lancet_tutorial_timing_json("encounter lancet_tutorial\n", 1, "during").unwrap_err();
        assert!(error
            .to_string()
            .contains("Use before or after for the Lancet Tutorial timing."));
    }

    #[test]
    fn tanker_pattern_payload_ignores_block_commented_tanker_slots() {
        let payload: Value = serde_json::from_str(
            &tanker_pattern_json("encounter tanker\n/*\nm5 attack\n*/\nstatus atb\n", 5, "aa")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 5);
        assert_eq!(payload["end_line"], 4);
    }

    #[test]
    fn tanker_pattern_payload_keeps_block_comment_open_until_trailing_marker_like_python() {
        let payload: Value = serde_json::from_str(
            &tanker_pattern_json(
                "encounter tanker\n/* m5 attack */ trailing text\nm5 attack\nnote */\nstatus atb\n",
                5,
                "aa",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 5);
        assert_eq!(payload["end_line"], 4);
    }

    #[test]
    fn tanker_pattern_payload_accepts_trailing_blank_line_inside_last_encounter() {
        let payload: Value = serde_json::from_str(
            &tanker_pattern_json("encounter tanker\nstatus atb\n", 3, "aa").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["start_line"], 3);
        assert_eq!(payload["end_line"], 2);
    }

    #[test]
    fn chocobo_auto_swap_ignores_dead_reserves_like_python() {
        let mut state = SimulationState::new(DEFAULT_SEED);
        state.execute_raw_line("stat yuna hp 9999");
        state.execute_raw_line("status yuna death");

        let replacement = choose_chocobo_swap_replacement(&state);

        assert_ne!(replacement, Some(crate::model::Character::Yuna));
        assert_eq!(replacement, Some(crate::model::Character::Kimahri));
    }

    #[test]
    fn chocobo_cursor_state_ticks_shadow_ctbs_after_party_swaps() {
        let mut cursor_state = ChocoboCursorState::new(DEFAULT_SEED);
        cursor_state.apply_line("encounter chocobo_eater");
        cursor_state.apply_line("party taw");
        let wakka_swap_ctb = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka should be tracked after swapping in");

        cursor_state.apply_line("party ta");
        cursor_state.apply_line("tidus defend");

        let wakka_after_turn = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after swapping out");
        assert!(
            wakka_after_turn < wakka_swap_ctb,
            "Wakka shadow CTB did not tick down: {wakka_swap_ctb} -> {wakka_after_turn}"
        );
    }

    #[test]
    fn chocobo_cursor_state_skips_hash_comments_without_shadow_ctb_tick() {
        let mut cursor_state = ChocoboCursorState::new(DEFAULT_SEED);
        cursor_state.apply_line("encounter chocobo_eater");
        cursor_state.apply_line("party taw");
        cursor_state.apply_line("party ta");
        cursor_state.apply_line("tidus defend");
        let before_comment = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should be tracked after swapping out");

        cursor_state.apply_line("   # cursor note");

        let after_comment = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after comments");
        assert_eq!(after_comment, before_comment);
    }

    #[test]
    fn chocobo_cursor_state_status_atb_does_not_tick_shadow_ctbs_after_turn() {
        let mut cursor_state = ChocoboCursorState::new(DEFAULT_SEED);
        cursor_state.apply_line("encounter chocobo_eater");
        cursor_state.apply_line("party taw");
        cursor_state.apply_line("party ta");
        cursor_state.apply_line("tidus defend");
        let before_status = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should be tracked after swapping out");

        cursor_state.apply_line("status atb");

        let after_status = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after status atb");
        assert_eq!(after_status, before_status);
    }

    #[test]
    fn chocobo_cursor_replay_skips_indented_block_comments_like_python() {
        let mut baseline = ChocoboCursorState::new(DEFAULT_SEED);
        baseline.apply_line("encounter chocobo_eater");
        baseline.apply_line("party taw");
        baseline.apply_line("party ta");
        baseline.apply_line("tidus defend");
        let expected = baseline
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should be tracked after swapping out");

        let cursor_state = simulate_to_chocobo_cursor_state(
            DEFAULT_SEED,
            "encounter chocobo_eater\nparty taw\nparty ta\ntidus defend\n  /*\ntidus defend\n  */\nstatus atb\n",
            8,
        );
        let actual = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after block comments");

        assert_eq!(actual, expected);
    }

    #[test]
    fn chocobo_cursor_replay_ends_block_comments_on_trailing_marker_like_python() {
        let mut baseline = ChocoboCursorState::new(DEFAULT_SEED);
        baseline.apply_line("encounter chocobo_eater");
        baseline.apply_line("party taw");
        baseline.apply_line("party ta");
        baseline.apply_line("tidus defend");
        baseline.apply_line("tidus defend");
        let expected = baseline
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should be tracked after two active turns");

        let cursor_state = simulate_to_chocobo_cursor_state(
            DEFAULT_SEED,
            "encounter chocobo_eater\nparty taw\nparty ta\ntidus defend\n/*\ntidus defend\nnote */\ntidus defend\nstatus atb\n",
            9,
        );
        let actual = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after trailing comment close");

        assert_eq!(actual, expected);
    }

    #[test]
    fn chocobo_cursor_replay_infers_encounter_party_from_character_rows_like_python() {
        let cursor_state = simulate_to_chocobo_cursor_state(
            DEFAULT_SEED,
            "encounter chocobo_eater\ntidus defend\nlulu defend\nstatus atb\n",
            4,
        );
        assert_eq!(
            cursor_state.state.party(),
            &[
                crate::model::Character::Tidus,
                crate::model::Character::Lulu
            ]
        );
    }

    #[test]
    fn chocobo_cursor_replay_prefers_explicit_party_swaps_over_inferred_rows_like_python() {
        let cursor_state = simulate_to_chocobo_cursor_state(
            DEFAULT_SEED,
            "encounter chocobo_eater\nlulu defend\nparty ta\nstatus atb\n",
            3,
        );
        assert_eq!(
            cursor_state.state.party(),
            &[
                crate::model::Character::Tidus,
                crate::model::Character::Auron
            ]
        );
    }

    #[test]
    fn chocobo_cursor_state_party_change_does_not_tick_shadow_ctbs_after_turn() {
        let mut cursor_state = ChocoboCursorState::new(DEFAULT_SEED);
        cursor_state.apply_line("encounter chocobo_eater");
        cursor_state.apply_line("party taw");
        cursor_state.apply_line("party ta");
        cursor_state.apply_line("tidus defend");
        let before_party = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should be tracked after swapping out");

        cursor_state.apply_line("party tay");

        let after_party = cursor_state
            .shadow_ctbs
            .as_ref()
            .and_then(|shadow| shadow.get(&crate::model::Character::Wakka))
            .copied()
            .expect("Wakka shadow CTB should remain tracked after party changes");
        assert_eq!(after_party, before_party);
    }

    #[test]
    fn chocobo_cursor_state_clears_shadow_ctbs_when_monsters_die() {
        let mut cursor_state = ChocoboCursorState::new(DEFAULT_SEED);
        cursor_state.apply_line("encounter chocobo_eater");
        cursor_state.apply_line("party taw");
        assert!(cursor_state.shadow_ctbs.is_some());

        cursor_state.apply_line("stat m1 hp 0");

        assert!(cursor_state.shadow_ctbs.is_none());
    }

    #[test]
    fn chocobo_swap_payload_generates_party_line_inside_encounter() {
        let payload: Value = serde_json::from_str(
            &chocobo_swap_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\nstatus atb\n",
                2,
                1,
                "wakka",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 2);
        assert_eq!(payload["lines"][0], "party tw");
    }

    #[test]
    fn chocobo_swap_payload_uses_python_frontend_insert_line_not_haste_line() {
        let payload: Value = serde_json::from_str(
            &chocobo_swap_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\ntidus haste chocobo_eater\nstatus atb\n",
                2,
                1,
                "wakka",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 2);
        assert_eq!(payload["lines"][0], "party tw");
    }

    #[test]
    fn chocobo_swap_payload_rejects_current_party_members() {
        let error = chocobo_swap_json(
            DEFAULT_SEED,
            "encounter chocobo_eater\nstatus atb\n",
            2,
            0,
            "tidus",
        )
        .unwrap_err();
        assert!(error.to_string().contains("already in the active party"));
    }

    #[test]
    fn chocobo_swap_payload_inserts_after_encounter_line_like_python_frontend() {
        let payload: Value = serde_json::from_str(
            &chocobo_swap_json(
                DEFAULT_SEED,
                "encounter chocobo_eater\nstatus atb\n",
                1,
                1,
                "wakka",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 2);
        assert_eq!(payload["lines"][0], "party tw");
    }

    #[test]
    fn chocobo_swap_payload_uses_raw_cursor_line_after_default_macros() {
        let payload: Value = serde_json::from_str(
            &chocobo_swap_json(DEFAULT_SEED, DEFAULT_INPUT, 624, 1, "lulu").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["insert_line"], 624);
        assert_eq!(payload["lines"][0], "party tlw");
    }

    #[test]
    fn tracker_default_payloads_match_web_editor_shape() {
        let drops: Value =
            serde_json::from_str(&tracker_default_json("drops", DEFAULT_SEED).unwrap()).unwrap();
        assert_eq!(drops["tracker"], "drops");
        assert_eq!(drops["input_filename"], "drops_notes.txt");
        assert!(drops["input"]
            .as_str()
            .is_some_and(|input| input.contains("# -- Zanarkand --")));

        let encounters: Value =
            serde_json::from_str(&tracker_default_json("encounters", DEFAULT_SEED).unwrap())
                .unwrap();
        assert_eq!(encounters["tracker"], "encounters");
        assert_eq!(encounters["input_filename"], "encounters_notes.csv");
        assert!(encounters["input"]
            .as_str()
            .is_some_and(|input| input.contains("encounter tanker")));
        assert!(encounters["input"]
            .as_str()
            .is_some_and(|input| input.contains("#    Underwater Ruins")));
        assert!(encounters["input"]
            .as_str()
            .is_some_and(|input| input.contains("# encounter besaid_lagoon")));
        assert!(encounters["sliders"]
            .as_array()
            .is_some_and(|sliders| !sliders.is_empty()));
    }

    #[test]
    fn encounters_tracker_render_payload_uses_ctb_output_cleanup() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\n/usage\nencounter tanker\nweapon tidus 1 initiative\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(payload["tracker"], "encounters");
        assert_eq!(payload["output_filename"], "encounters_output.txt");
        let output = payload["output"].as_str().unwrap();
        let output_lower = output.to_ascii_lowercase();
        assert!(output_lower.contains("1 | tanker"));
        assert!(output_lower.contains("2 | sahagins"));
        assert!(!output.contains("Equipment:"));
        assert!(!output.contains("Command:"));
        assert!(!output.contains("Ti["));
    }

    #[test]
    fn encounters_tracker_cleanup_hides_commented_encounter_commands() {
        let output = edit_encounters_tracker_output(
            concat!(
                "# Encounter:   1 | Shown | Ghost Normal\n",
                "# encounter hidden_zone\n",
                "Random Encounter:   2   2   2 | Shown | Ragora Normal\n",
                "/* encounter hidden_zone */\n",
                "/*\n",
                "encounter hidden_zone\n",
                "*/\n",
                "Encounter:   3 | Shown | Condor Normal\n",
            ),
            false,
        );

        assert!(!output.contains("hidden_zone"), "{output}");
        assert!(output.contains("1 | Shown | Ghost"), "{output}");
        assert!(output.contains("2   2   2 | Ragora"), "{output}");
        assert!(output.contains("3 | Shown | Condor"), "{output}");
    }

    #[test]
    fn encounters_tracker_without_hide_marker_preserves_first_encounter_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "encounter tanker\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.contains("1 | Tanker"), "{output}");
        assert!(output.contains("2 | Sahagins"), "{output}");
    }

    #[test]
    fn encounters_tracker_boss_rows_match_python_exactly() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\nencounter tanker\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "  1 | Tanker | Tanker, Sinscale#6, Sinscale#6, Sinscale#6, ",
                "Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6 \n",
                "  2 | Sahagins | Sahagin#4, Sahagin#4, Sahagin#4\n",
            )
        );
    }

    #[test]
    fn encounters_tracker_padded_boss_rows_match_python_exactly() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "encounter tanker\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "  1 | Tanker   | Tanker, Sinscale#6, Sinscale#6, Sinscale#6, ",
                "Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6\n",
                "  2 | Sahagins | Sahagin#4, Sahagin#4, Sahagin#4\n",
            )
        );
    }

    #[test]
    fn encounters_tracker_handles_directives_like_python_tracker() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\n/usage\n/macro nope\nencounter tanker\n///\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(
            output.starts_with("  2 | Sahagins | Sahagin#4, Sahagin#4, Sahagin#4\n"),
            "{output}"
        );
        assert!(!output.contains("/usage"), "{output}");
        assert!(!output.contains("/nopadding"), "{output}");
        assert!(!output.contains("Possible macros"), "{output}");
        assert!(!output.contains("Tanker"), "{output}");
    }

    #[test]
    fn encounters_tracker_usage_accepts_trailing_text_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("encounters", DEFAULT_SEED, "/usage please\n").unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("encounters_count"), "{output}");
        assert!(!output.contains("Command: /usage please"), "{output}");
    }

    #[test]
    fn encounters_tracker_macro_and_unknown_directives_match_python() {
        let macro_payload: Value = serde_json::from_str(
            &tracker_render_json("encounters", DEFAULT_SEED, "/nopadding\n/macro nope\n").unwrap(),
        )
        .unwrap();
        assert_eq!(macro_payload["output"], "Error: Possible macros are\n");

        let unknown_payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\n/notreal\nencounter tanker\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            unknown_payload["output"],
            concat!(
                "  1 | Tanker | Tanker, Sinscale#6, Sinscale#6, Sinscale#6, ",
                "Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6\n",
            )
        );
    }

    #[test]
    fn encounters_tracker_random_rows_hide_zone_name_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\nencounter besaid_lagoon\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("1   1   1 | Piranha"), "{output}");
        assert!(!output.contains("Besaid Lagoon"), "{output}");
        assert!(!output.contains("Ti["), "{output}");
    }

    #[test]
    fn encounters_tracker_count_rows_and_multizone_cleanup_match_python() {
        let count_payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                1,
                concat!(
                    "/nopadding\n",
                    "encounters_count total 5\n",
                    "encounters_count random +2\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            count_payload["output"],
            "Total encounters count set to 5\nRandom encounters count set to 2\n"
        );

        let zone_count_payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                1,
                "/nopadding\nencounter besaid_road\nencounters_count besaid_road +2\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            zone_count_payload["output"],
            "  1   1   1 | Condor, Water Flan \nBesaid Road encounters count set to 3\n"
        );

        let multizone_payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                1,
                "/nopadding\nencounter multizone besaid_road kilika_woods\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            multizone_payload["output"],
            "  1   1   1  | Condor, Water Flan | Ragora, Killer Bee, Killer Bee\n"
        );
    }

    #[test]
    fn encounters_tracker_simulated_and_condition_aliases_match_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                1,
                "/nopadding\nencounter simulated\nencounter normal\nencounter preemptive\nencounter ambush\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "  0 | Empty \n",
                "  1 | Boss | Empty \n",
                "  2 | Boss | Empty Preemptive \n",
                "  3 | Boss | Empty Ambush\n",
            )
        );
    }

    #[test]
    fn encounters_tracker_inserts_section_spacers_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\n#    First\nencounter tanker\n#    Second\nencounter tanker\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.starts_with("#    First\n"), "{output}");
        assert!(output.contains("\n===="), "{output}");
        assert!(output.contains("\n#    Second\n"), "{output}");
    }

    #[test]
    fn encounters_tracker_section_spacers_match_python_exactly() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\n#    First\nencounter tanker\n#    Second\nencounter tanker\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "#    First\n",
                "  1 | Tanker | Tanker, Sinscale#6, Sinscale#6, Sinscale#6, ",
                "Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6 \n",
                "==========================================================================================================\n",
                "#    Second\n",
                "  2 | Tanker | Tanker, Sinscale#6, Sinscale#6, Sinscale#6, ",
                "Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6\n",
            )
        );
    }

    #[test]
    fn encounters_tracker_pads_columns_unless_nopadding_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/usage\nencounter tanker\nweapon tidus 1 initiative\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        let encounter_lines = output
            .lines()
            .filter(|line| line.trim_start().starts_with(char::is_numeric))
            .filter(|line| line.contains('|'))
            .collect::<Vec<_>>();
        assert_eq!(encounter_lines.len(), 2, "{output}");
        let first_second_pipe = encounter_lines[0].match_indices('|').nth(1).unwrap().0;
        let second_second_pipe = encounter_lines[1].match_indices('|').nth(1).unwrap().0;
        assert_eq!(first_second_pipe, second_second_pipe, "{output}");
    }

    #[test]
    fn encounters_tracker_ignores_nopadding_inside_block_comments_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/*\n/nopadding\n*/\nencounter tanker\nweapon tidus 1 initiative\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        let encounter_lines = output
            .lines()
            .filter(|line| line.trim_start().starts_with(char::is_numeric))
            .filter(|line| line.contains('|'))
            .collect::<Vec<_>>();
        assert_eq!(encounter_lines.len(), 2, "{output}");
        let first_second_pipe = encounter_lines[0].match_indices('|').nth(1).unwrap().0;
        let second_second_pipe = encounter_lines[1].match_indices('|').nth(1).unwrap().0;
        assert_eq!(first_second_pipe, second_second_pipe, "{output}");
    }

    #[test]
    fn encounters_tracker_repeat_inside_block_comments_does_not_expand_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "encounters",
                DEFAULT_SEED,
                "/nopadding\nencounter tanker\n/*\n/repeat 2 1\n*/\nencounter sahagins\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert_eq!(output.matches("# /repeat 2 1").count(), 1, "{output}");
        assert_eq!(output.matches("# /*").count(), 1, "{output}");
        assert!(output.contains("1 | Tanker"), "{output}");
        assert!(output.contains("2 | Sahagins"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_handles_first_pass_item_rng() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "steal tanker\ntanker tidus\n").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["tracker"], "drops");
        let output = payload["output"].as_str().unwrap();
        assert_eq!(output, "Steal: Tanker | Failed\nTanker        | - | 0 AP\n");
    }

    #[test]
    fn drops_tracker_ghost_item_is_always_mana_but_equipment_changes() {
        let baseline: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "ghost ixion\n/repeat 4\n").unwrap(),
        )
        .unwrap();
        let advanced: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "advance rng10 1\nghost ixion\n/repeat 4\n",
            )
            .unwrap(),
        )
        .unwrap();
        let baseline_output = baseline["output"].as_str().unwrap();
        let advanced_output = advanced["output"].as_str().unwrap();
        assert!(
            baseline_output.contains("Ghost | Mana Sphere"),
            "{baseline_output}"
        );
        assert!(
            advanced_output.contains("Ghost | Mana Sphere"),
            "{advanced_output}"
        );
        let baseline_ghost_lines = baseline_output
            .lines()
            .filter(|line| line.contains("Ghost"))
            .collect::<Vec<_>>();
        let advanced_ghost_lines = advanced_output
            .lines()
            .filter(|line| line.contains("Ghost"))
            .collect::<Vec<_>>();
        assert_ne!(baseline_ghost_lines, advanced_ghost_lines);
    }

    #[test]
    fn drops_tracker_render_payload_accepts_generated_search_result_preamble() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "Drop Search Result\n",
                    "Policy: {}\n",
                    "No Encounters armor: None (drop #-)\n",
                    "\n",
                    "Resolved drop route\n",
                    "party ta\n",
                    "sinscale_6 auron\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(
            output.starts_with("Party: Tidus, Auron -> Tidus, Auron\n"),
            "{output}"
        );
        assert!(output.contains("Sinscale#6"), "{output}");
        assert!(!output.contains("Impossible to parse"), "{output}");
        assert!(!output.contains("Policy:"), "{output}");
    }

    #[test]
    fn drops_steal_rarity_uses_rng11_like_python() {
        for seed in 0..1_000 {
            let mut rng = FfxRngTracker::new(seed);
            let mut inventory = DropsInventory::new();
            let output = render_steal_line(&mut rng, &mut inventory, "piranha", 0);
            if output.contains("Failed") {
                continue;
            }

            let positions = rng.current_positions();
            assert_eq!(positions[10], 1, "{output}");
            assert_eq!(positions[11], 1, "{output}");
            return;
        }
        panic!("expected to find a successful piranha steal in the first 1000 seeds");
    }

    #[test]
    fn drops_tracker_render_payload_handles_bribe_drop_notes() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "bribe piranha rikku\n").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["output"], "Piranha | Water Gem x1 | 1 AP\n");
    }

    #[test]
    fn drops_tracker_render_payload_handles_route_state_commands() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "party ta\ndeath tidus\nroll rng10 1\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Party: Tidus, Auron -> Tidus, Auron\nCharacter death: Tidus\nAdvanced rng10 1 times\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_party_and_roll_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "party\n",
                    "party z\n",
                    "roll rng10 -1\n",
                    "roll rng10 201\n",
                    "roll rngx 1\n",
                    "roll rng10 nope\n",
                    "roll rng10 x5\n",
                    "roll rng-1 1\n",
                    "roll\n",
                    "advance\n",
                    "waste\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Error: Usage: party [characters initials]\n",
                "Error: no characters initials in \"z\"\n",
                "Error: amount needs to be an greater or equal to 0\n",
                "Error: Can't advance rng more than 200 times\n",
                "Error: rng needs to be an integer\n",
                "Error: rng needs to be an integer\n",
                "Error: rng needs to be an integer\n",
                "Error: Can't advance rng index -1\n",
                "Error: rng needs to be an integer\n",
                "Error: rng needs to be an integer\n",
                "Error: rng needs to be an integer\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_steal_count_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "steal tanker nope\nsteal tanker -1\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Error: successful steals must be an integer\nError: successful steals must be greater or equal to 0\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_drop_command_usage_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "kill\nkill tanker\nsteal\nbribe\nbribe piranha\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Error: Usage: kill [monster name] [killer] (characters initials) (overkill/ok)\n",
                "Error: Usage: kill [monster name] [killer] (characters initials) (overkill/ok)\n",
                "Error: Usage: steal [monster name] (successful steals)\n",
                "Error: Usage: bribe [monster name] [user] (characters initials)\n",
                "Error: Usage: bribe [monster name] [user] (characters initials)\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_drop_command_parse_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "kill tanker badchar\n",
                    "bribe piranha badchar\n",
                    "steal badmonster\n",
                    "kill badmonster tidus\n",
                    "bribe badmonster rikku\n",
                    "tanker badchar\n",
                    "tanker\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Error: killer can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Error: user can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Error: No monster named \"badmonster\"\n",
                "Error: No monster named \"badmonster\"\n",
                "Error: No monster named \"badmonster\"\n",
                "Error: killer can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Error: Usage: kill [monster name] [killer] (characters initials) (overkill/ok)\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_handles_ap_command() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "ap tidus\nap tidus 5\nap yuna 15\n")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Tidus: 0 S. Lv (0 AP Total, 5 for next level)\nTidus: 1 S. Lv (5 AP Total, 10 for next level) (added 5 AP)\nYuna: 1 S. Lv (15 AP Total, 20 for next level) (added 15 AP)\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_ap_character_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "ap z 5\nap tidus nope\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Tidus: 0 S. Lv (0 AP Total, 5 for next level)\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_credits_kill_ap_state() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "bribe piranha rikku t\nap tidus\n")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Piranha | Water Gem x1 | 1 AP to T\nTidus: 0 S. Lv (1 AP Total, 4 for next level)\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_handles_gil_inventory_commands() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "inventory show gil\ninventory get gil 50\ninventory use gil 20\ninventory show gil\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Gil: 300\n\nAdded 50 Gil (350 Gil total)\nUsed 20 Gil (330 Gil total)\nGil: 330\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_handles_item_inventory_commands() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "inventory get potion 2\ninventory use potion 1\ninventory sell phoenix_down 1\ninventory buy antidote 1\ninventory show gil\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Added Potion x2 to inventory\nUsed Potion x1\nSold Phoenix Down x1 for 25 gil\nBought Antidote x1 for 50 gil\nGil: 275\n"
        );
    }

    #[test]
    fn drops_tracker_inventory_show_sizes_internal_empty_slots_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/nopadding\ninventory use potion 10\ninventory show\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "Command: /nopadding\n",
                "Used Potion x10\n",
                "+--------+----------------+\n",
                "| -      | Phoenix Down 3 |\n",
                "+--------+----------------+\n",
            )
        );
    }

    #[test]
    fn drops_tracker_inventory_switch_empty_slots_render_none_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "inventory switch 1 3\n").unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "Switched Potion (slot 1) for None (slot 3)\n"
        );
    }

    #[test]
    fn drops_tracker_inventory_switch_and_autosort_order_like_python() {
        let switched: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "/nopadding\n",
                    "inventory get potion 1\n",
                    "inventory get antidote 1\n",
                    "inventory switch 1 3\n",
                    "inventory show\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            switched["output"],
            concat!(
                "Command: /nopadding\n",
                "Added Potion x1 to inventory\n",
                "Added Antidote x1 to inventory\n",
                "Switched Potion (slot 1) for Antidote (slot 3)\n",
                "+------------+----------------+\n",
                "| Antidote 1 | Phoenix Down 3 |\n",
                "| Potion  11 | -              |\n",
                "+------------+----------------+\n",
            )
        );

        let autosorted: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "/nopadding\n",
                    "inventory get antidote 1\n",
                    "inventory get potion 1\n",
                    "inventory autosort\n",
                    "inventory show\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            autosorted["output"],
            concat!(
                "Command: /nopadding\n",
                "Added Antidote x1 to inventory\n",
                "Added Potion x1 to inventory\n",
                "Autosorted inventory\n",
                "+------------+----------------+\n",
                "| Potion  11 | Phoenix Down 3 |\n",
                "| Antidote 1 | -              |\n",
                "+------------+----------------+\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_inventory_usage_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory\n",
                    "inventory get\n",
                    "inventory get potion\n",
                    "inventory get gil\n",
                    "inventory use gil\n",
                    "inventory switch\n",
                    "inventory switch -1 2\n",
                    "inventory sell equipment\n",
                    "inventory sell equipment -1\n",
                    "inventory sell equipment weapon\n",
                    "inventory get equipment weapon tidus\n",
                    "inventory buy equipment\n",
                    "inventory nope\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]\n",
                "Error: Usage: inventory get [item] [amount]\n",
                "Error: Usage: inventory get [item] [amount]\n",
                "Error: Usage: inventory get gil [amount]\n",
                "Error: Usage: inventory use gil [amount]\n",
                "Error: Usage: inventory switch [slot 1] [slot 2]\n",
                "Error: Inventory slot needs to be between 1 and 112\n",
                "Error: Usage: inventory sell equipment [equipment slot]\n",
                "Error: Usage: inventory sell equipment [equipment slot]\n",
                "Error: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory get equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory buy equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]\n",
            )
        );
    }

    #[test]
    fn drops_tracker_inventory_amount_and_slot_errors_match_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory get potion nope\n",
                    "inventory get potion -1\n",
                    "inventory use potion nope\n",
                    "inventory use potion -1\n",
                    "inventory sell potion nope\n",
                    "inventory sell potion -1\n",
                    "inventory buy potion nope\n",
                    "inventory buy potion -1\n",
                    "inventory get gil nope\n",
                    "inventory get gil -1\n",
                    "inventory use gil nope\n",
                    "inventory use gil -1\n",
                    "inventory switch nope 2\n",
                    "inventory switch 1 nope\n",
                    "inventory switch 113 1\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "Error: Amount needs to be an integer\n",
                "Error: Amount needs to be more than 0\n",
                "Error: Amount needs to be an integer\n",
                "Error: Amount needs to be more than 0\n",
                "Error: Amount needs to be an integer\n",
                "Error: Amount needs to be more than 0\n",
                "Error: Amount needs to be an integer\n",
                "Error: Amount needs to be more than 0\n",
                "Error: Gil amount needs to be an integer\n",
                "Error: Gil amount needs to be more than 0\n",
                "Error: Gil amount needs to be an integer\n",
                "Error: Gil amount needs to be more than 0\n",
                "Error: Inventory slot needs to be an integer\n",
                "Error: Inventory slot needs to be an integer\n",
                "Error: Inventory slot needs to be between 1 and 112\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_item_parse_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "inventory get not_an_item 1\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            format!(
                "Error: item can only be one of these values: {}\n",
                drops_item_values()
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_reports_equipment_parse_errors_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory get equipment relic tidus 1\n",
                    "inventory get equipment weapon nope 1\n",
                    "inventory get equipment weapon tidus nope\n",
                    "inventory get equipment weapon tidus 5\n",
                    "inventory get equipment weapon tidus 1 nope\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let expected = format!(
            "{}{}",
            concat!(
                "Error: equipment type can only be one of these values: weapon, armor\n",
                "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Error: Slots must be between 0 and 4\n",
                "Error: Slots must be between 0 and 4\n",
            ),
            format!(
                "Error: ability can only be one of these values: {}\n",
                drops_autoability_values()
            )
        );
        assert_eq!(payload["output"], expected);
    }

    #[test]
    fn drops_tracker_equipment_abilities_use_python_spellings() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "inventory get equipment armor tidus 1 auto-haste\ninventory get equipment armor tidus 1 auto_haste\n",
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            format!(
                "{}{}",
                "Added Haste Shield (Tidus) [Auto-Haste][12512 gil]\n",
                format!(
                    "Error: ability can only be one of these values: {}\n",
                    drops_autoability_values()
                )
            )
        );
    }

    #[test]
    fn drops_tracker_equipment_slot_and_ability_limits_match_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory get equipment weapon tidus 1 sensor sensor\n",
                    "inventory get equipment weapon tidus 1 sensor first_strike initiative piercing alchemy\n",
                    "inventory get equipment weapon tidus 1 sensor sensor first_strike initiative piercing\n",
                    "inventory get equipment weapon tidus 1 sensor first_strike initiative piercing nope\n",
                    "inventory get equipment weapon tidus 0 sensor\n",
                    "inventory get equipment weapon tidus 0\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "Added Hunter's Sword (Tidus) [Sensor][62 gil]\n",
                "Added Sonic Steel (Tidus) [Sensor, First Strike, Initiative, Piercing][15312 gil]\n",
                "Added Sonic Steel (Tidus) [Sensor, First Strike, Initiative, Piercing][15312 gil]\n",
                "Added Sonic Steel (Tidus) [Sensor, First Strike, Initiative, Piercing][15312 gil]\n",
                "Added Hunter's Sword (Tidus) [Sensor][62 gil]\n",
                "Added Longsword (Tidus) [][12 gil]\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_handles_empty_equipment_inventory_show() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "inventory show equipment\n").unwrap(),
        )
        .unwrap();
        assert_eq!(payload["output"], "Equipment: Empty\n");
    }

    #[test]
    fn drops_tracker_render_payload_generates_first_pass_equipment_drops() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "party tyk\nbiran_ronso kimahri\ninventory show equipment\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Equipment #1"), "{output}");
        assert!(output.contains("Equipment: #1"), "{output}");
        assert!(!output.contains("Equipment pending"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_keeps_ability_rolls_on_drop_rows_only() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/nopadding\nparty tyk\nbiran_ronso kimahri\ninventory show equipment\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            concat!(
                "Command: /nopadding\n",
                "Party: Tidus, Auron -> Tidus, Yuna, Kimahri\n",
                "Biran Ronso | Friend Sphere x1 (rare) | 4500 AP | Equipment #1 Halberd (Kimahri) [Piercing, -][18 gil](guaranteed)(for killer) (1 ability roll)\n",
                "Equipment: #1 Halberd (Kimahri) [Piercing, -][18 gil](guaranteed)(for killer)\n",
            )
        );
    }

    #[test]
    fn drops_tracker_render_payload_handles_manual_equipment_inventory_commands() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "inventory get equipment weapon tidus 1 sensor\ninventory show equipment\ninventory sell equipment 1\ninventory show gil\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(
            output.contains("Added Hunter's Sword (Tidus) [Sensor][62 gil]"),
            "{output}"
        );
        assert!(
            output.contains("Equipment: #1 Hunter's Sword (Tidus) [Sensor][62 gil]"),
            "{output}"
        );
        assert!(
            output.contains("Sold Hunter's Sword (Tidus) [Sensor][62 gil]"),
            "{output}"
        );
        assert!(output.contains("Gil: 362"), "{output}");
    }

    #[test]
    fn drops_tracker_manual_equipment_buy_and_sell_spec_match_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory buy equipment weapon tidus 1 sensor\n",
                    "inventory show gil\n",
                    "inventory show equipment\n",
                    "inventory sell equipment weapon tidus 1 sensor\n",
                    "inventory show gil\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            concat!(
                "Bought Hunter's Sword (Tidus) [Sensor][62 gil] for 250 gil\n",
                "Gil: 50\n\n",
                "Equipment: #1 Hunter's Sword (Tidus) [Sensor][62 gil]\n",
                "Sold Hunter's Sword (Tidus) [Sensor][62 gil]\n",
                "Gil: 112\n",
            )
        );
    }

    #[test]
    fn drops_tracker_equipment_inventory_shows_internal_empty_slots_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory get equipment weapon tidus 1 sensor\n",
                    "inventory get equipment armor tidus 1 auto-haste\n",
                    "inventory sell equipment 1\n",
                    "inventory show equipment\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.contains("Equipment: #1 None"), "{output}");
        assert!(
            output.contains("#2 Haste Shield (Tidus) [Auto-Haste][12512 gil]"),
            "{output}"
        );
    }

    #[test]
    fn drops_tracker_render_payload_uses_python_equipment_name_priorities() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                concat!(
                    "inventory get equipment weapon tidus 4 triple_overdrive triple_ap overdrive_->_ap\n",
                    "inventory get equipment weapon tidus 4 strength_+3% strength_+5% strength_+10% strength_+20%\n",
                    "inventory get equipment armor tidus 4 break_hp_limit break_mp_limit\n",
                    "inventory get equipment armor tidus 4 fireproof iceproof lightningproof waterproof\n",
                    "inventory get equipment weapon seymour 1 sensor\n",
                    "inventory get equipment armor valefor 1 auto-haste\n",
                ),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Added Ragnarok (Tidus)"), "{output}");
        assert!(output.contains("Added Master Sword (Tidus)"), "{output}");
        assert!(output.contains("Added Endless Road (Tidus)"), "{output}");
        assert!(output.contains("Added Aegis Shield (Tidus)"), "{output}");
        assert!(
            output.contains("[Triple Overdrive, Triple AP, Overdrive -> AP, -][156312 gil]"),
            "{output}"
        );
        assert!(
            output.contains("[Fireproof, Iceproof, Lightningproof, Waterproof]"),
            "{output}"
        );
        assert!(output.contains("Added Seymour Staff (Seymour)"), "{output}");
        assert!(
            output.contains("Added Valefor's armor (Valefor)"),
            "{output}"
        );
    }

    #[test]
    fn drops_tracker_render_payload_adds_drops_to_inventory() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "bribe piranha rikku\ninventory show\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Piranha | Water Gem x1 | 1 AP"));
        assert!(output.contains("Water Gem"));
    }

    #[test]
    fn drops_tracker_render_payload_handles_common_directives() {
        let nopadding: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/nopadding\n").unwrap(),
        )
        .unwrap();
        assert_eq!(nopadding["output"], "Command: /nopadding\n");

        let usage: Value =
            serde_json::from_str(&tracker_render_json("drops", DEFAULT_SEED, "/usage\n").unwrap())
                .unwrap();
        assert!(usage["output"]
            .as_str()
            .is_some_and(|output| output.contains("party [characters initials]")));

        let hide_marker: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "party ta\n///\nparty tw\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            hide_marker["output"],
            "Party: Tidus, Auron -> Tidus, Wakka\n"
        );

        let macro_usage: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/macro\n/macro nope\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            macro_usage["output"],
            "Error: Possible macros are\nError: Possible macros are\n"
        );

        let macro_with_nopadding: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/nopadding\n/macro nope\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            macro_with_nopadding["output"],
            "Command: /nopadding\nError: Possible macros are\n"
        );

        let unknown_directive: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/nopadding\n/notreal\nparty ta\n")
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            unknown_directive["output"],
            "Command: /nopadding\nCommand: /notreal\nParty: Tidus, Auron -> Tidus, Auron\n"
        );

        let unknown: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "definitely unknown\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            unknown["output"],
            "Error: Impossible to parse \"definitely unknown\"\n"
        );
    }

    #[test]
    fn tracker_render_payload_omits_rust_status_fields_like_python() {
        let payload: Value =
            serde_json::from_str(&tracker_render_json("drops", DEFAULT_SEED, "").unwrap()).unwrap();
        let fields = payload.as_object().unwrap();
        assert!(fields.contains_key("tracker"));
        assert!(fields.contains_key("output"));
        assert!(fields.contains_key("duration_seconds"));
        assert!(fields.contains_key("output_filename"));
        assert!(!fields.contains_key("implemented"));
        assert!(!fields.contains_key("message"));
    }

    #[test]
    fn drops_tracker_render_payload_preserves_block_comments_without_effects() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/*\nparty tw\n*/\nparty ty\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "# /*\n# party tw\n# */\nParty: Tidus, Auron -> Tidus, Yuna\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_preserves_indented_block_comments_without_effects() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "  /*\nparty tw\n  */\nparty ty\n")
                .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "#   /*\n# party tw\n#   */\nParty: Tidus, Auron -> Tidus, Yuna\n"
        );
    }

    #[test]
    fn drops_tracker_ignores_nopadding_inside_block_comments_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/*\n/nopadding\n*/\nsteal piranha\nkill tanker tidus\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("# /nopadding"), "{output}");
        let steal_line = output
            .lines()
            .find(|line| line.starts_with("Steal:"))
            .unwrap();
        let drops_line = output.lines().find(|line| line.contains("Tanker")).unwrap();
        let steal_pipe = steal_line.find('|').unwrap();
        let drops_pipe = drops_line.find('|').unwrap();
        assert_eq!(steal_pipe, drops_pipe, "{output}");
    }

    #[test]
    fn drops_tracker_indented_comments_and_directives_parse_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, " # comment\n /usage\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Error: Impossible to parse \" # comment\"\nError: Impossible to parse \" /usage\"\n"
        );
    }

    #[test]
    fn drops_tracker_directives_are_case_and_column_sensitive_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                " /nopadding\n/USAGE\n/MACRO nope\n/usage please\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(
            output.contains("Error: Impossible to parse \" /nopadding\""),
            "{output}"
        );
        assert!(output.contains("Command: /USAGE"), "{output}");
        assert!(output.contains("Command: /MACRO nope"), "{output}");
        assert!(output.contains("# Events:"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_expands_repeat_directives() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "party tw\ndeath tidus\n/repeat 2 2\n",
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Party: Tidus, Auron -> Tidus, Wakka\nCharacter death: Tidus\nCommand: /repeat 2 2\nParty: Tidus, Wakka -> Tidus, Wakka\nCharacter death: Tidus\nParty: Tidus, Wakka -> Tidus, Wakka\nCharacter death: Tidus\n"
        );
    }

    #[test]
    fn drops_tracker_repeat_directive_is_column_sensitive_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "party tw\n /repeat 2 1\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Party: Tidus, Auron -> Tidus, Wakka\nError: Impossible to parse \" /repeat 2 1\"\n"
        );
    }

    #[test]
    fn drops_tracker_repeat_inside_block_comments_does_not_expand_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "party tw\n/*\n/repeat 2 1\n*/\n").unwrap(),
        )
        .unwrap();
        assert_eq!(
            payload["output"],
            "Party: Tidus, Auron -> Tidus, Wakka\n# /*\n# /repeat 2 1\n# */\n"
        );
    }

    #[test]
    fn drops_tracker_render_payload_uses_python_duplicate_monster_names() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "sinscale_6 auron\npiranha_2 tidus\n")
                .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Sinscale#6 |"));
        assert!(output.contains("Piranha#2"));
    }

    #[test]
    fn drops_tracker_render_payload_accepts_python_style_case_insensitive_monsters() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/nopadding\nkill Tanker Tidus\nsteal TANKER\nbribe Piranha Rikku\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Tanker | - | 0 AP"), "{output}");
        assert!(output.contains("Steal: Tanker |"), "{output}");
        assert!(output.contains("Piranha | Water Gem x1 | 1 AP"), "{output}");
        assert!(!output.contains("Unknown monster"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_lowercases_event_tokens_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/nopadding\nKILL Tanker Tidus t OK\nINVENTORY SHOW GIL\nAP TIDUS nope\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Tanker | - | 0 AP to T (OK)"), "{output}");
        assert!(output.contains("Gil: 300"), "{output}");
        assert!(output.contains("Tidus: 0 S. Lv"), "{output}");
        assert!(!output.contains("Impossible to parse"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_accepts_bare_monster_kill_lines_like_python() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json("drops", DEFAULT_SEED, "/nopadding\npiranha tidus ta\n").unwrap(),
        )
        .unwrap();

        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Piranha |"), "{output}");
        assert!(output.contains("to TA"), "{output}");
        assert!(!output.contains("Impossible to parse"), "{output}");
    }

    #[test]
    fn drops_tracker_render_payload_accepts_case_insensitive_overkill_tokens() {
        let payload: Value = serde_json::from_str(
            &tracker_render_json(
                "drops",
                DEFAULT_SEED,
                "/nopadding\nkill Tanker Tidus OK\nTanker Tidus t OverKill\n",
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(output.contains("Tanker | - | 0 AP to K"), "{output}");
        assert!(output.contains("Tanker | - | 0 AP to T (OK)"), "{output}");
    }

    #[test]
    fn no_encounters_routes_reports_empty_input_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(DEFAULT_SEED, "", 1, None, None).unwrap(),
        )
        .unwrap();

        assert_eq!(payload["output"], "No Encounters search: input is empty.");
        assert_eq!(payload["edited_input"], "");
    }

    #[test]
    fn no_encounters_routes_reports_whitespace_input_as_missing_ghost_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(DEFAULT_SEED, " \n\t\n", 1, None, None).unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: no Ghost line was found below the cursor."
        );
        assert_eq!(payload["edited_input"], " \n\t\n");
    }

    #[test]
    fn no_encounters_routes_reports_missing_ghost_below_cursor_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "piranha tidus\n/*\nkill ghost tidus\n*/\n",
                1,
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: no Ghost line was found below the cursor."
        );
        assert_eq!(
            payload["edited_input"],
            "piranha tidus\n/*\nkill ghost tidus\n*/\n"
        );
    }

    #[test]
    fn no_encounters_routes_counts_commented_ghost_as_optional_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(DEFAULT_SEED, "# ghost ixion\n", 1, None, None).unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.starts_with("No Encounters search:\n"), "{output}");
        assert!(
            output.contains("Cursor line 1, first Ghost line 1, tested 1 "),
            "{output}"
        );
        assert!(output.contains("Search mode: notes-fallback"), "{output}");
        assert!(output.contains("Future random sections:"), "{output}");
        assert!(
            output
                .contains("No route found that gives a No Encounters armor from the first Ghost."),
            "{output}"
        );
        assert!(!output.contains("Candidates tested:"), "{output}");
        assert!(!output.contains("no Ghost line was found"), "{output}");
        assert_eq!(payload["edited_input"], "# ghost ixion\n");
    }

    #[test]
    fn no_encounters_detector_accepts_padded_ghost_rows() {
        let rendered = "Ghost                     | Mana Sphere x1 | 1450 AP | Equipment #26 Peaceful Ring (Yuna) [No Encounters, -, -][7368 gil]\n";

        assert_eq!(
            find_no_encounters_ghost_drop(rendered).as_deref(),
            Some("Equipment #26 Peaceful Ring (Yuna) [No Encounters, -, -][7368 gil]")
        );
    }

    #[test]
    fn no_encounters_routes_ignores_ghost_inside_indented_block_comment_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(DEFAULT_SEED, "  /*\nghost ixion\n  */\n", 1, None, None)
                .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: no Ghost line was found below the cursor."
        );
        assert_eq!(payload["edited_input"], "  /*\nghost ixion\n  */\n");
    }

    #[test]
    fn exact_future_encounter_output_parses_rows_and_stops_at_ghost_like_python() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "# Kilika:\n",
                "1 | Ragora, Killer Bee Normal\n",
                "2 | Ghost Normal\n",
                "3 | Piranha Normal\n",
            ),
            "",
        )
        .expect("future encounter output should parse");

        assert_eq!(parsed.rows.len(), 2);
        assert!(parsed.rows[0].random);
        assert_eq!(
            parsed.rows[0].encounter_options,
            vec![vec!["ragora".to_string(), "killer_bee".to_string()]]
        );
        assert_eq!(
            parsed.rows[1].encounter_options,
            vec![vec!["ghost".to_string()]]
        );
        assert_eq!(parsed.total_counts.get("ragora"), Some(&1));
        assert_eq!(parsed.total_counts.get("killer_bee"), Some(&1));
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
        assert_eq!(
            parsed
                .section_maxima
                .get("Kilika")
                .and_then(|counts| counts.get("ghost")),
            Some(&1)
        );
        assert!(!parsed.total_counts.contains_key("piranha"));
    }

    #[test]
    fn exact_future_encounter_output_normalizes_purifico_underwater_section_like_python() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "# Via Purifico (Maze):\n",
                "1 | Sahagin Normal\n",
                "# Via Purifico (Underwater):\n",
                "1 | Ghost Normal\n",
            ),
            "Underwater",
        )
        .expect("underwater future rows should parse from the mapped section");

        assert_eq!(parsed.rows.len(), 1);
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
        assert!(!parsed.total_counts.contains_key("sahagin"));
        assert_eq!(
            parsed
                .section_maxima
                .get("Underwater")
                .and_then(|counts| counts.get("ghost")),
            Some(&1)
        );
    }

    #[test]
    fn exact_future_encounter_output_normalizes_bevelle_guard_labels_like_python() {
        let parsed = parse_exact_future_encounter_output(
            "1 | bevelle_guards_1 | Warrior Monk Normal\n2 | Ghost Normal\n",
            "Bevelle",
        )
        .expect("Bevelle guard label should start the Bevelle future section");

        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total_counts.get("warrior_monk"), Some(&1));
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
    }

    #[test]
    fn exact_future_encounter_output_parses_raw_ctb_random_rows() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "Random Encounter:   1   1   1 | Kilika Woods | Ragora, Killer Bee Normal\n",
                "Random Encounter:   2   2   2 | Kilika Woods | Ghost Normal\n",
                "Random Encounter:   3   3   3 | Kilika Woods | Piranha Normal\n",
            ),
            "Kilika",
        )
        .expect("raw CTB random rows should parse");

        assert_eq!(parsed.rows.len(), 2);
        assert!(parsed.rows[0].random);
        assert_eq!(
            parsed.rows[0].encounter_options,
            vec![vec!["ragora".to_string(), "killer_bee".to_string()]]
        );
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
        assert_eq!(
            parsed
                .section_maxima
                .get("Kilika")
                .and_then(|counts| counts.get("ragora")),
            Some(&1)
        );
        assert!(!parsed.total_counts.contains_key("piranha"));
    }

    #[test]
    fn exact_future_encounter_output_parses_raw_ctb_fixed_rows_as_not_random() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "Encounter:   1 | Kilika Woods | Ragora Normal\n",
                "Encounter:   2 | Kilika Woods | Ghost Normal\n",
            ),
            "Kilika",
        )
        .expect("raw CTB fixed rows should parse");

        assert_eq!(parsed.rows.len(), 2);
        assert!(!parsed.rows[0].random);
        assert_eq!(parsed.total_counts.get("ragora"), Some(&1));
        assert!(
            parsed
                .section_maxima
                .get("Kilika")
                .map(|counts| counts.is_empty())
                .unwrap_or(true),
            "fixed raw CTB rows should not feed random-section maxima"
        );
    }

    #[test]
    fn exact_future_encounter_output_parses_multizone_and_simulated_raw_ctb_rows() {
        let multizone = parse_exact_future_encounter_output(
            concat!(
                "Multizone encounter:   1 | Kilika Woods/Besaid Road | Ragora Normal\n",
                "Multizone encounter:   2 | Kilika Woods/Besaid Road | Ghost Normal\n",
            ),
            "Kilika",
        )
        .expect("raw multizone rows should parse");

        assert_eq!(multizone.rows.len(), 2);
        assert!(multizone.rows[0].random);
        assert_eq!(
            multizone
                .section_maxima
                .get("Kilika")
                .and_then(|counts| counts.get("ragora")),
            Some(&1)
        );

        let simulated = parse_exact_future_encounter_output(
            concat!(
                "Simulated Encounter:   1 | Simulation | Ragora Normal\n",
                "Simulated Encounter:   2 | Simulation | Ghost Normal\n",
            ),
            "Simulation",
        )
        .expect("raw simulated rows should parse");

        assert_eq!(simulated.rows.len(), 2);
        assert!(!simulated.rows[0].random);
        assert_eq!(simulated.total_counts.get("ghost"), Some(&1));
        assert!(
            simulated
                .section_maxima
                .get("Simulation")
                .map(|counts| counts.is_empty())
                .unwrap_or(true),
            "simulated rows should not feed random-section maxima"
        );
    }

    #[test]
    fn exact_future_encounter_output_aligns_multizone_raw_rows_to_later_zone() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "Multizone encounter:   1 | Kilika Woods/Besaid Road | Ragora Normal\n",
                "Multizone encounter:   2 | Kilika Woods/Besaid Road | Ghost Normal\n",
            ),
            "Besaid",
        )
        .expect("raw multizone rows should align to either displayed zone");

        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
        assert_eq!(
            parsed
                .section_maxima
                .get("Besaid")
                .and_then(|counts| counts.get("ragora")),
            Some(&1)
        );
        assert!(
            !parsed.section_maxima.contains_key("Kilika"),
            "matched later-zone rows should be attributed to the requested Drops section"
        );
    }

    #[test]
    fn exact_future_encounter_output_ignores_block_commented_raw_rows_like_python() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "/*\n",
                "Random Encounter:   1   1   1 | Kilika Woods | Ghost Normal\n",
                "*/\n",
                "Random Encounter:   2   2   2 | Kilika Woods | Ragora Normal\n",
                "Random Encounter:   3   3   3 | Kilika Woods | Ghost Normal\n",
            ),
            "Kilika",
        )
        .expect("raw rows outside block comments should parse");

        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total_counts.get("ragora"), Some(&1));
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
        assert_eq!(
            parsed
                .section_maxima
                .get("Kilika")
                .and_then(|counts| counts.get("ghost")),
            Some(&1),
            "the commented Ghost row should not be counted"
        );
    }

    #[test]
    fn exact_future_encounter_output_ignores_one_line_block_commented_raw_rows_like_python() {
        let parsed = parse_exact_future_encounter_output(
            concat!(
                "/* Random Encounter:   1   1   1 | Kilika Woods | Ghost Normal */\n",
                "Random Encounter:   2   2   2 | Kilika Woods | Ragora Normal\n",
                "Random Encounter:   3   3   3 | Kilika Woods | Ghost Normal\n",
            ),
            "Kilika",
        )
        .expect("raw rows after one-line block comments should parse");

        assert_eq!(parsed.rows.len(), 2);
        assert_eq!(parsed.total_counts.get("ragora"), Some(&1));
        assert_eq!(parsed.total_counts.get("ghost"), Some(&1));
    }

    #[test]
    fn no_encounters_routes_reports_unparseable_encounters_output_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                None,
                Some("not an encounters tracker table"),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: the Encounters tracker output could not be parsed. Refresh the Encounters tracker and try again."
        );
        assert_eq!(payload["edited_input"], "ghost ixion\n");
    }

    #[test]
    fn no_encounters_routes_reports_unparseable_encounters_input_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                Some("not a tracker row"),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: the Encounters tracker input could not be parsed. Refresh the Encounters tracker and try again."
        );
        assert_eq!(payload["edited_input"], "ghost ixion\n");
    }

    #[test]
    fn no_encounters_routes_reports_current_route_preview() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(DEFAULT_SEED, DROPS_NOTES, 380, None, None).unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.starts_with("No Encounters search:\n"), "{output}");
        assert!(
            output.contains("Cursor line 380, first Ghost line"),
            "{output}"
        );
        assert!(
            output.contains("1. add ")
                || output.contains("No route found that gives a No Encounters armor"),
            "{output}"
        );
        assert!(
            !output.contains("route synthesis is not implemented yet"),
            "{output}"
        );
        assert!(output.contains("tested "), "{output}");
    }

    #[test]
    fn no_encounters_first_ghost_window_keeps_immediate_repeat_only() {
        let input = concat!(
            "party tyk\n",
            "ghost ixion\n",
            "# a note between Ghost and repeat\n",
            "/repeat 4\n",
            "ragora tidus\n",
            "ghost ixion\n",
        );

        assert_eq!(
            first_ghost_search_window_input(input, 1).as_deref(),
            Some("party tyk\nghost ixion\n# a note between Ghost and repeat\n/repeat 4")
        );
    }

    #[test]
    fn no_encounters_first_ghost_window_excludes_later_ghost_routes() {
        let input = concat!(
            "ghost ixion\n",
            "ragora tidus\n",
            "ghost ixion\n",
            "/repeat 4\n",
        );

        assert_eq!(
            first_ghost_search_window_input(input, 1).as_deref(),
            Some("ghost ixion")
        );
    }

    #[test]
    fn no_encounters_first_ghost_window_keeps_commented_optional_ghost_first() {
        let input = concat!(
            "# ghost ixion\n",
            "ragora tidus\n",
            "ghost ixion\n",
            "/repeat 4\n",
        );

        assert_eq!(
            first_ghost_search_window_input(input, 1).as_deref(),
            Some("# ghost ixion")
        );
    }

    #[test]
    fn no_encounters_routes_does_not_accept_later_ghost_drop_for_first_ghost() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                1,
                "ghost ixion\nragora tidus\nghost ixion\n/repeat 4\n",
                1,
                None,
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            !output.contains("Route status: current Ghost route already produces"),
            "{output}"
        );
        assert!(
            output.contains("Suggested Ghost kills: 5")
                || output.contains("No route found that gives a No Encounters armor"),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_reports_future_rows_when_encounters_output_is_available() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                None,
                Some("# Cavern of the Stolen Fayth:\n1 | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Future encounters parsed: 1 row(s) through Ghost."),
            "{output}"
        );
        assert!(
            output.contains("Search mode: exact-encounters-output, prefer-guaranteed."),
            "{output}"
        );
        assert!(
            output.contains("Future random sections: Cavern of the Stolen Fayth: ghostx1."),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_reports_guaranteed_only_for_fixed_exact_ghost_row() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                1,
                "ghost ixion\n/repeat 4\n",
                1,
                None,
                Some("Encounter:   1 | Cave | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-output."),
            "{output}"
        );
        assert!(!output.contains("prefer-guaranteed"), "{output}");
    }

    #[test]
    fn no_encounters_routes_reports_random_rows_for_random_exact_ghost_row() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                1,
                "ghost ixion\n/repeat 4\n",
                1,
                None,
                Some("Random Encounter:   1   1   1 | Cave | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-output, prefer-guaranteed."),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_aligns_future_output_to_drops_section_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Cavern of the Stolen Fayth --\nghost ixion\n",
                1,
                None,
                Some("# Kilika:\n1 | Ragora Normal\n# Cave:\n1 | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Future encounters parsed: 1 row(s) through Ghost."),
            "{output}"
        );
        assert!(
            output.contains("Future random sections: Cavern of the Stolen Fayth: ghostx1."),
            "{output}"
        );
        assert!(!output.contains("ragorax1"), "{output}");
    }

    #[test]
    fn no_encounters_routes_anchors_future_output_to_first_search_route_section_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Kilika --\npiranha tidus\n# -- Cavern of the Stolen Fayth --\nghost ixion\n",
                1,
                None,
                Some("# Kilika:\n1 | Ragora Normal\n# Cave:\n1 | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Future encounters parsed: 2 row(s) through Ghost."),
            "{output}"
        );
        assert!(
            output.contains(
                "Future random sections: Cavern of the Stolen Fayth: ghostx1 | Kilika: ragorax1."
            ),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_uses_encounters_input_when_output_is_missing() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                Some("/nopadding\nencounter cave_white_zone\n"),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.contains("Future encounters parsed:"), "{output}");
        assert!(
            output.contains("Search mode: exact-encounters-input"),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_accepts_single_line_block_comment_in_encounters_input_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                Some("/* exact rows */\n/nopadding\nencounter cave_white_zone\n"),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-input"),
            "{output}"
        );
        assert!(output.contains("Future encounters parsed:"), "{output}");
    }

    #[test]
    fn no_encounters_routes_rejects_encounters_input_for_wrong_section_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Kilika --\nghost ixion\n",
                1,
                Some("/nopadding\nencounter cave_white_zone\n"),
                None,
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(
            payload["output"],
            "No Encounters search: the Encounters tracker input could not be parsed. Refresh the Encounters tracker and try again."
        );
        assert_eq!(payload["edited_input"], "# -- Kilika --\nghost ixion\n");
    }

    #[test]
    fn no_encounters_routes_falls_back_to_input_when_output_is_stale() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "ghost ixion\n",
                1,
                Some("/nopadding\nencounter cave_white_zone\n"),
                Some("not an encounters tracker table"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.contains("Future encounters parsed:"), "{output}");
        assert!(
            output.contains("Search mode: exact-encounters-input"),
            "{output}"
        );
        assert!(
            !output.contains("could not be parsed"),
            "valid input should recover from stale output: {output}"
        );
    }

    #[test]
    fn no_encounters_routes_accepts_raw_ctb_encounters_output() {
        let raw_output = concat!(
            "Random Encounter:   1   1   1 | Kilika Woods | Ragora, Killer Bee Normal\n",
            "Random Encounter:   2   2   2 | Kilika Woods | Ghost Normal\n",
            "Random Encounter:   3   3   3 | Kilika Woods | Piranha Normal\n",
        );
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Kilika --\nghost ixion\n",
                2,
                None,
                Some(raw_output),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-output, prefer-guaranteed."),
            "{output}"
        );
        assert!(
            output.contains("Future encounters parsed: 2 row(s) through Ghost."),
            "{output}"
        );
        assert!(
            output.contains("Future random sections: Kilika: ghostx1, killer_beex1, ragorax1."),
            "{output}"
        );
    }

    #[test]
    fn no_encounters_routes_accepts_raw_ctb_rows_as_encounters_input() {
        let raw_input = concat!(
            "Random Encounter:   1   1   1 | Kilika Woods | Ragora, Killer Bee Normal\n",
            "Random Encounter:   2   2   2 | Kilika Woods | Ghost Normal\n",
            "Random Encounter:   3   3   3 | Kilika Woods | Piranha Normal\n",
        );
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Kilika --\nghost ixion\n",
                2,
                Some(raw_input),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-input"),
            "{output}"
        );
        assert!(
            output.contains("Future encounters parsed: 2 row(s) through Ghost."),
            "{output}"
        );
        assert!(
            !output.contains("could not be parsed"),
            "raw rows should parse directly as fallback input: {output}"
        );
    }

    #[test]
    fn no_encounters_routes_aligns_multizone_raw_rows_to_later_zone() {
        let raw_output = concat!(
            "Multizone encounter:   1 | Kilika Woods/Besaid Road | Ragora Normal\n",
            "Multizone encounter:   2 | Kilika Woods/Besaid Road | Ghost Normal\n",
        );
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Besaid --\nghost ixion\n",
                2,
                None,
                Some(raw_output),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-output, prefer-guaranteed."),
            "{output}"
        );
        assert!(
            output.contains("Future random sections: Besaid: ghostx1, ragorax1."),
            "{output}"
        );
        assert!(!output.contains("Kilika: ragorax1"), "{output}");
    }

    #[test]
    fn no_encounters_routes_aligns_multizone_raw_fallback_input_to_later_zone() {
        let raw_input = concat!(
            "Multizone encounter:   1 | Kilika Woods/Besaid Road | Ragora Normal\n",
            "Multizone encounter:   2 | Kilika Woods/Besaid Road | Ghost Normal\n",
        );
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "# -- Besaid --\nghost ixion\n",
                2,
                Some(raw_input),
                None,
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Search mode: exact-encounters-input"),
            "{output}"
        );
        assert!(
            output.contains("Future random sections: Besaid: ghostx1, ragorax1."),
            "{output}"
        );
        assert!(
            !output.contains("could not be parsed"),
            "raw multizone input should parse directly: {output}"
        );
    }

    #[test]
    fn no_encounters_ghost_repeat_synthesis_reports_none_without_simple_candidate() {
        let input = DROPS_NOTES.replace("/repeat 30", "/repeat 1");
        let search = synthesize_no_encounters_ghost_route(DEFAULT_SEED, &input, 380, 3, 0);

        assert_eq!(search.candidate, None);
        assert_eq!(search.candidates_tested, 6);
    }

    #[test]
    fn no_encounters_routes_can_uncomment_pre_ghost_candidate() {
        let base_input = "ghost ixion\n";
        let candidate_input = "death ???\nghost ixion\n";
        let seed = (0..1_000)
            .find(|seed| {
                find_no_encounters_ghost_drop(&render_drops_tracker(*seed, base_input)).is_none()
                    && find_no_encounters_ghost_drop(&render_drops_tracker(*seed, candidate_input))
                        .is_some()
            })
            .expect("expected a seed where a pre-Ghost death creates No Encounters");
        let input = "# death ???\nghost ixion\n";
        let payload: Value =
            serde_json::from_str(&no_encounters_routes_json(seed, input, 1, None, None).unwrap())
                .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(output.contains("1. add 1 action"), "{output}");
        assert!(output.contains("line 1: death ???"), "{output}");
        assert!(
            payload["edited_input"]
                .as_str()
                .is_some_and(|input| input.contains("death ???")),
            "{payload}"
        );
    }

    #[test]
    fn no_encounters_routes_applies_synthesized_ghost_repeat_candidate() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(1, "ghost ixion\n/repeat 1\n", 1, None, None).unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output
                .contains("No route found that gives a No Encounters armor from the first Ghost."),
            "{output}"
        );
        assert_eq!(payload["edited_input"], "ghost ixion\n/repeat 1\n");
    }

    #[test]
    fn no_encounters_synthesis_ignores_repeat_inside_block_comment() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(1, "ghost ixion\n/*\n/repeat 1\n*/\n", 1, None, None)
                .unwrap(),
        )
        .unwrap();

        assert_eq!(payload["edited_input"], "ghost ixion\n/*\n/repeat 1\n*/\n");
    }

    #[test]
    fn no_encounters_synthesis_adds_rikku_missing_monster_package_like_python() {
        let input = concat!(
            "# -- Calm Lands --\n",
            "party tyakwlr\n",
            "kill defender_x bahamut\n",
            "# -- Cavern of the Stolen Fayth --\n",
            "kill ghost ixion\n",
        );
        let lines = input.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
        let (_, pre_ghost_lines, _) = drops_search_window(&lines, 1);
        let future_section_maxima = HashMap::from([(
            "Calm Lands".to_string(),
            HashMap::from([("nebiros".to_string(), 8), ("mech_scouter".to_string(), 6)]),
        )]);
        let families = no_encounters_synthesis_families(&pre_ghost_lines, &future_section_maxima);
        let descriptions = families
            .iter()
            .flat_map(|family| family.description_lines.iter())
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            descriptions.contains("before line 3: steal nebiros"),
            "{descriptions}"
        );
        assert!(
            descriptions.contains("before line 3: nebiros rikku"),
            "{descriptions}"
        );
    }

    #[test]
    fn no_encounters_repeat_scan_keeps_repeat_after_single_line_block_comment_like_python() {
        let lines = ["kill ghost ixion", "/* old note */", "/repeat 1"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        assert_eq!(find_active_repeat_after_ghost(&lines, 0), Some(2));
    }

    #[test]
    fn no_encounters_ghost_scan_allows_single_line_block_comment_like_python() {
        let lines = ["/* route note */", "# -- Kilika --", "kill ghost ixion"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();

        assert_eq!(find_first_ghost_route_line(&lines, 1), Some(2));
        assert_eq!(
            drops_search_first_ghost_line(
                &["/* route note */", "# -- Kilika --", "kill ghost ixion"],
                1,
            ),
            Some(3)
        );
    }

    #[test]
    fn no_encounters_start_section_allows_single_line_block_comment_like_python() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(
                DEFAULT_SEED,
                "/* route note */\n# -- Kilika --\nkill ghost ixion\n",
                1,
                None,
                Some("# Besaid:\n1 | Piranha Normal\n# Kilika:\n1 | Ghost Normal\n"),
            )
            .unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output.contains("Future random sections: Kilika: ghostx1."),
            "{output}"
        );
        assert!(!output.contains("piranha"), "{output}");
    }

    #[test]
    fn no_encounters_routes_applies_pre_ghost_death_candidate() {
        let payload: Value = serde_json::from_str(
            &no_encounters_routes_json(140, "ghost ixion\n/repeat 1\n", 1, None, None).unwrap(),
        )
        .unwrap();
        let output = payload["output"].as_str().unwrap();

        assert!(
            output
                .contains("No route found that gives a No Encounters armor from the first Ghost."),
            "{output}"
        );
        assert_eq!(payload["edited_input"], "ghost ixion\n/repeat 1\n");
    }

    #[test]
    fn drops_tracker_default_route_has_no_unsupported_rows() {
        let payload: Value =
            serde_json::from_str(&tracker_render_json("drops", DEFAULT_SEED, DROPS_NOTES).unwrap())
                .unwrap();
        let output = payload["output"].as_str().unwrap();
        assert!(!output.contains("Unsupported drops command"), "{output}");
        assert!(!output.contains("Impossible to parse"), "{output}");
        assert!(!output.contains("Unknown monster"), "{output}");
        assert!(!output.contains("Equipment pending"), "{output}");
    }
}

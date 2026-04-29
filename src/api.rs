use serde::Serialize;
use std::collections::HashMap;
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
use crate::script::prepare_action_lines;
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
    state.run_until_raw_line(input, cursor_line);
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

fn render_encounters_tracker(seed: u32, input: &str) -> String {
    let render_input = protect_tracker_block_comment_repeats(input);
    let rendered = ctb::render_ctb(seed, &render_input);
    let padding = !tracker_has_active_nopadding(input);
    edit_encounters_tracker_output(&rendered.output, padding)
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
        if raw_line.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            let line = raw_line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
            lines.push(format!("# {line}"));
            if raw_line.ends_with("*/") {
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

fn expand_tracker_repeats(input: &str) -> Vec<String> {
    let mut lines = input.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut index = 0;
    let mut multiline_comment = false;
    while index < lines.len() {
        let line = lines[index].clone();
        if line.starts_with("/*") {
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
        if multiline_comment && line.ends_with("*/") {
            multiline_comment = false;
        }
        index += 1;
    }
    lines
}

fn tracker_has_active_nopadding(input: &str) -> bool {
    let mut multiline_comment = false;
    for raw_line in input.lines() {
        if raw_line.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            if raw_line.ends_with("*/") {
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
        if raw_line.starts_with("/*") {
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
        if multiline_comment && raw_line.ends_with("*/") {
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
        _ => format!("Unsupported inventory command: {command} {item_name} {amount}"),
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
    for ability in abilities.iter().take(4) {
        let ability = normalize_drops_ability_name(ability);
        if data::autoability_gil_value(&ability).is_none() {
            return Err(DropsEquipmentParseError::Ability);
        }
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
        if raw_line.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            let line = raw_line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
            lines.push(format!("# {line}"));
            if raw_line.ends_with("*/") {
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
        let mut line = raw_line.to_string();
        if line.starts_with("Random Encounter:") || line.starts_with("Simulated Encounter:") {
            let mut parts = line.split('|').map(str::trim).collect::<Vec<_>>();
            if parts.len() > 2 {
                parts.pop();
                parts.remove(1);
                line = parts.join(" | ");
            }
        } else if line.starts_with("Encounter:") || line.starts_with("Multizone encounter:") {
            let mut parts = line.split('|').map(str::trim).collect::<Vec<_>>();
            if parts.len() > 2 {
                parts.pop();
                line = parts.join(" | ");
            }
        }
        line = line.replace("__ctb_tracker_block_comment_repeat__", "/repeat");
        let line = line.trim().to_string();
        if !line.is_empty() {
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
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
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
        if cursor_line >= encounter.start_line {
            return Some(encounter.clone());
        }
    }
    encounters.into_iter().next()
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
    let mut cursor_state = ChocoboCursorState::new(seed);
    let mut multiline_comment = false;
    for raw_line in prepared
        .lines
        .into_iter()
        .take(cursor_line.saturating_sub(1))
    {
        if raw_line.starts_with("/*") {
            multiline_comment = true;
        }
        if multiline_comment {
            if raw_line.ends_with("*/") {
                multiline_comment = false;
            }
            continue;
        }
        if raw_line.trim().is_empty() || raw_line.trim_start().starts_with('#') {
            continue;
        }
        cursor_state.apply_line(&raw_line);
    }
    cursor_state
}

fn get_chocobo_effective_insert_line(
    input: &str,
    encounter: &ctb::EncounterBlock,
    cursor_line: usize,
) -> usize {
    let lines = input.lines().collect::<Vec<_>>();
    let encounter_end = encounter.end_line.min(lines.len());
    let haste_line = (encounter.start_line..=encounter_end).find(|line_number| {
        lines
            .get(line_number.saturating_sub(1))
            .is_some_and(|line| {
                line.trim()
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
    use serde_json::Value;

    use crate::rng::FfxRngTracker;
    use crate::simulator::SimulationState;

    use super::{
        chocobo_action_json, chocobo_swap_json, choose_chocobo_swap_replacement,
        drops_autoability_values, drops_item_values, party_json, render_ctb_diff_json,
        render_ctb_json, render_steal_line, sample_json, tracker_default_json, tracker_render_json,
        ChocoboCursorState, DropsInventory, DEFAULT_SEED, DROPS_NOTES,
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
        assert!(output.starts_with("2 | Sahagins"), "{output}");
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
                "Error: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory get equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory buy equipment [equip type] [character] [slots] (abilities)\n",
                "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]\n",
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
                    "inventory get equipment armor tidus 1 auto_haste\n",
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
                    "inventory get equipment armor valefor 1 auto_haste\n",
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

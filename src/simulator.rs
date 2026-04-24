use crate::battle::{ActorId, BattleActor, BattleState};
use crate::encounter::{character_initial_ctb, encounter_condition_from_roll};
use crate::model::{Character, EncounterCondition};
use crate::parser::{parse_raw_action_line, ParsedCommand};
use crate::rng::FfxRngTracker;

#[derive(Debug, Clone)]
pub struct SimulationOutput {
    pub text: String,
    pub unsupported_count: usize,
}

pub struct SimulationState {
    rng: FfxRngTracker,
    party: Vec<Character>,
    character_actors: Vec<BattleActor>,
    monsters: Vec<BattleActor>,
    encounters_count: usize,
    unsupported_count: usize,
}

impl SimulationState {
    pub fn new(seed: u32) -> Self {
        Self {
            rng: FfxRngTracker::new(seed),
            party: vec![Character::Tidus, Character::Auron],
            character_actors: default_character_actors(),
            monsters: Vec::new(),
            encounters_count: 0,
            unsupported_count: 0,
        }
    }

    pub fn run_lines(&mut self, lines: &[String]) -> SimulationOutput {
        let mut rendered = Vec::with_capacity(lines.len());
        for line in lines {
            rendered.push(self.execute_line(line));
        }
        SimulationOutput {
            text: rendered.join("\n"),
            unsupported_count: self.unsupported_count,
        }
    }

    fn execute_line(&mut self, line: &str) -> String {
        match parse_raw_action_line(line) {
            ParsedCommand::Blank => String::new(),
            ParsedCommand::Comment(comment) => comment,
            ParsedCommand::Directive(directive) => format!("Command: {directive}"),
            ParsedCommand::Party { initials } => self.change_party(&initials),
            ParsedCommand::AdvanceRng { index, amount } => self.advance_rng(index, amount),
            ParsedCommand::Encounter { name, multizone } => self.start_encounter(&name, multizone),
            ParsedCommand::Status { args }
                if args
                    .first()
                    .is_some_and(|arg| arg.eq_ignore_ascii_case("atb")) =>
            {
                format!("ATB: {}", self.current_battle_state().ctb_order_string())
            }
            ParsedCommand::CharacterAction { .. }
            | ParsedCommand::MonsterAction { .. }
            | ParsedCommand::Equip { .. }
            | ParsedCommand::Status { .. }
            | ParsedCommand::Stat { .. }
            | ParsedCommand::Heal
            | ParsedCommand::Unknown { .. } => {
                self.unsupported_count += 1;
                format!("Error: Unsupported by Rust port yet: {line}")
            }
        }
    }

    fn change_party(&mut self, initials: &str) -> String {
        let old_party = self.format_party();
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
        self.party = new_party;
        format!("Party: {old_party} -> {}", self.format_party())
    }

    fn advance_rng(&mut self, index: u32, amount: u32) -> String {
        if index >= 68 {
            return format!("Error: Can't advance rng index {index}");
        }
        if amount > 200 {
            return "Error: Can't advance rng more than 200 times".to_string();
        }
        for _ in 0..amount {
            self.rng.advance_rng(index as usize);
        }
        format!("Advanced rng{index} {amount} times")
    }

    fn start_encounter(&mut self, name: &str, multizone: bool) -> String {
        self.process_start_of_encounter();
        let condition_roll = self.rng.advance_rng(1);
        let condition = encounter_condition_from_roll(condition_roll, false, None);
        self.encounters_count += 1;
        self.set_party_icvs(condition);
        self.normalize_ctbs();

        let prefix = if multizone {
            "Multizone encounter"
        } else {
            "Encounter"
        };
        format!(
            "{prefix}: {:>3} | {name} | {} | {}",
            self.encounters_count,
            format_condition(condition),
            self.current_battle_state().ctb_order_string()
        )
    }

    fn process_start_of_encounter(&mut self) {
        for actor in &mut self.character_actors {
            actor.ctb = 0;
            actor.statuses.clear();
        }
        self.monsters.clear();
    }

    fn set_party_icvs(&mut self, condition: EncounterCondition) {
        match condition {
            EncounterCondition::Preemptive => {}
            EncounterCondition::Ambush => {
                let party = self.party.clone();
                for actor in &mut self.character_actors {
                    if let Some(ctb) = character_initial_ctb(
                        actor,
                        condition,
                        None,
                        party.contains(&character_id(actor)),
                        false,
                    ) {
                        actor.ctb = ctb;
                    }
                }
            }
            EncounterCondition::Normal => {
                let party = self.party.clone();
                for actor in &mut self.character_actors {
                    if matches!(actor.id, ActorId::Character(Character::Unknown)) {
                        continue;
                    }
                    let rng_index = (20 + actor.index).min(27);
                    let variance_roll = self.rng.advance_rng(rng_index);
                    if let Some(ctb) = character_initial_ctb(
                        actor,
                        condition,
                        Some(variance_roll),
                        party.contains(&character_id(actor)),
                        false,
                    ) {
                        actor.ctb = ctb;
                    }
                }
            }
        }
    }

    fn normalize_ctbs(&mut self) {
        let min_ctb = self
            .current_party_actors()
            .iter()
            .chain(self.monsters.iter())
            .map(|actor| actor.ctb)
            .min()
            .unwrap_or(0);
        if min_ctb == 0 {
            return;
        }
        for actor in &mut self.character_actors {
            if self.party.contains(&character_id(actor)) {
                actor.ctb -= min_ctb;
            }
        }
        for monster in &mut self.monsters {
            monster.ctb -= min_ctb;
        }
    }

    fn current_battle_state(&self) -> BattleState {
        BattleState::new(self.current_party_actors(), self.monsters.clone())
    }

    fn current_party_actors(&self) -> Vec<BattleActor> {
        self.party
            .iter()
            .filter_map(|character| self.character_actor(*character).cloned())
            .collect()
    }

    fn character_actor(&self, character: Character) -> Option<&BattleActor> {
        self.character_actors
            .iter()
            .find(|actor| actor.id == ActorId::Character(character))
    }

    fn format_party(&self) -> String {
        self.party
            .iter()
            .map(|character| character.display_name())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

pub fn simulate(seed: u32, lines: &[String]) -> SimulationOutput {
    SimulationState::new(seed).run_lines(lines)
}

fn character_id(actor: &BattleActor) -> Character {
    match actor.id {
        ActorId::Character(character) => character,
        ActorId::Monster(_) => Character::Unknown,
    }
}

fn format_condition(condition: EncounterCondition) -> &'static str {
    match condition {
        EncounterCondition::Preemptive => "Preemptive",
        EncounterCondition::Normal => "Normal",
        EncounterCondition::Ambush => "Ambush",
    }
}

fn default_character_actors() -> Vec<BattleActor> {
    [
        (Character::Tidus, 0, 10, 520),
        (Character::Yuna, 1, 10, 475),
        (Character::Auron, 2, 5, 1030),
        (Character::Kimahri, 3, 6, 644),
        (Character::Wakka, 4, 7, 618),
        (Character::Lulu, 5, 5, 380),
        (Character::Rikku, 6, 16, 360),
        (Character::Seymour, 7, 20, 1200),
        (Character::Valefor, 8, 0, 99999),
        (Character::Ifrit, 9, 0, 99999),
        (Character::Ixion, 10, 0, 99999),
        (Character::Shiva, 11, 0, 99999),
        (Character::Bahamut, 12, 0, 99999),
        (Character::Anima, 13, 0, 99999),
        (Character::Yojimbo, 14, 0, 99999),
        (Character::Cindy, 15, 0, 99999),
        (Character::Sandy, 16, 0, 99999),
        (Character::Mindy, 17, 0, 99999),
        (Character::Unknown, 18, 0, 99999),
    ]
    .into_iter()
    .map(|(character, index, agility, max_hp)| {
        BattleActor::character(character, index, agility, max_hp)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::simulate;

    #[test]
    fn simulates_party_and_rng_commands_like_python() {
        let lines = vec![
            "party tay".to_string(),
            "roll rng20 x3".to_string(),
            "party z".to_string(),
        ];
        let output = simulate(1, &lines);
        assert_eq!(
            output.text,
            "Party: Tidus, Auron -> Tidus, Auron, Yuna\nAdvanced rng20 3 times\nError: no characters initials in \"z\""
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn simulates_encounter_party_icvs_and_atb() {
        let lines = vec![
            "encounter tanker".to_string(),
            "status atb".to_string(),
            "tidus attack m1".to_string(),
        ];
        let output = simulate(1, &lines);
        assert!(output.text.contains("Encounter:   1 | tanker | Normal |"));
        assert!(output.text.contains("ATB:"));
        assert!(output.text.contains("Unsupported by Rust port yet"));
        assert_eq!(output.unsupported_count, 1);
    }
}

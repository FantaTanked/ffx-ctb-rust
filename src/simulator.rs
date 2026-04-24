use crate::battle::{ActorId, BattleActor, BattleState};
use crate::data;
use crate::encounter::{character_initial_ctb, encounter_condition_from_roll, monster_initial_ctb};
use crate::model::{Character, EncounterCondition, MonsterSlot, Status};
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
            ParsedCommand::CharacterAction {
                actor,
                action,
                args,
            } => self.apply_character_action(actor, &action, &args),
            ParsedCommand::MonsterAction { slot, action, args } => {
                self.apply_monster_action(slot, &action, &args)
            }
            ParsedCommand::Equip { kind, args } => self.change_equipment(&kind, &args),
            ParsedCommand::Summon { aeon } => self.summon(&aeon),
            ParsedCommand::Spawn {
                monster,
                slot,
                forced_ctb,
            } => self.spawn_monster(&monster, slot, forced_ctb),
            ParsedCommand::Element { args } => self.change_element(&args),
            ParsedCommand::Stat { args } => self.change_stat(&args),
            ParsedCommand::Heal { args } => self.heal_party(&args),
            ParsedCommand::EndEncounter => self.end_encounter(),
            ParsedCommand::Status { args }
                if args
                    .first()
                    .is_some_and(|arg| arg.eq_ignore_ascii_case("atb")) =>
            {
                format!("ATB: {}", self.current_battle_state().ctb_order_string())
            }
            ParsedCommand::Status { args } => self.change_status(&args),
            ParsedCommand::Unknown { .. } => {
                self.unsupported_count += 1;
                format!("Error: Unsupported by Rust port yet: {line}")
            }
        }
    }

    fn change_party(&mut self, initials: &str) -> String {
        let old_party = self.format_party();
        if self.set_party_from_initials(initials) {
            return format!("Party: {old_party} -> {}", self.format_party());
        }
        format!("Error: no characters initials in \"{initials}\"")
    }

    fn set_party_from_initials(&mut self, initials: &str) -> bool {
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
            return false;
        }
        self.party = new_party;
        true
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
        let formation = data::boss_or_simulated_formation(name).or_else(|| {
            if !data::has_random_zone(name) {
                return None;
            }
            let formation_roll = self.rng.advance_rng(1);
            data::random_formation(name, formation_roll)
        });
        if let Some(forced_party) = formation
            .as_ref()
            .and_then(|formation| formation.forced_party.as_deref())
        {
            self.set_party_from_initials(forced_party);
        }
        let condition_roll = self.rng.advance_rng(1);
        let condition = encounter_condition_from_roll(
            condition_roll,
            false,
            formation
                .as_ref()
                .and_then(|formation| formation.forced_condition),
        );
        self.encounters_count += 1;
        self.create_monster_party(
            name,
            formation
                .as_ref()
                .map(|formation| formation.monsters.as_slice()),
        );
        self.set_party_icvs(condition);
        self.set_monster_icvs(condition);
        self.normalize_ctbs();

        let prefix = if formation
            .as_ref()
            .is_some_and(|formation| formation.is_random)
            || multizone
        {
            "Multizone encounter"
        } else {
            "Encounter"
        };
        let display_name = formation
            .as_ref()
            .map(|formation| formation.display_name.as_str())
            .unwrap_or(name);
        format!(
            "{prefix}: {:>3} | {display_name} | {} | {}",
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

    fn create_monster_party(&mut self, encounter_name: &str, monster_names: Option<&[String]>) {
        let monster_names = match monster_names {
            Some(monster_names) if !monster_names.is_empty() => monster_names.to_vec(),
            _ => vec![encounter_name.to_string()],
        };
        let templates = monster_names
            .iter()
            .map(|name| monster_template(name))
            .collect::<Vec<_>>();
        for (index, template) in templates.iter().enumerate() {
            self.monsters.push(BattleActor::monster_with_key(
                MonsterSlot(index + 1),
                Some(template.key.clone()),
                template.agility,
                template.max_hp,
            ));
        }
        self.advance_duplicate_monster_rngs(&templates);
    }

    fn advance_duplicate_monster_rngs(&mut self, templates: &[MonsterTemplate]) {
        for (index, template) in templates.iter().enumerate() {
            let count = templates
                .iter()
                .filter(|candidate| candidate.key == template.key)
                .count();
            if count <= 1 {
                continue;
            }
            if templates
                .iter()
                .position(|candidate| candidate.key == template.key)
                == Some(index)
            {
                for _ in 0..count {
                    self.rng.advance_rng(28 + index);
                }
            }
            self.rng.advance_rng(28 + index);
        }
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

    fn set_monster_icvs(&mut self, condition: EncounterCondition) {
        match condition {
            EncounterCondition::Preemptive => {
                for actor in &mut self.monsters {
                    if let Some(ctb) = monster_initial_ctb(actor, condition, None) {
                        actor.ctb = ctb;
                    }
                }
            }
            EncounterCondition::Ambush => {}
            EncounterCondition::Normal => {
                for actor in &mut self.monsters {
                    let rng_index = (28 + actor.index).min(35);
                    let variance_roll = self.rng.advance_rng(rng_index);
                    if let Some(ctb) = monster_initial_ctb(actor, condition, Some(variance_roll)) {
                        actor.ctb = ctb;
                    }
                }
                for rng_index in (28 + self.monsters.len())..36 {
                    self.rng.advance_rng(rng_index);
                }
            }
        }
    }

    fn apply_character_action(
        &mut self,
        actor: Character,
        action: &str,
        args: &[String],
    ) -> String {
        let actor_id = ActorId::Character(actor);
        let Some(spent_ctb) = self.apply_actor_turn(actor_id, action_rank(action)) else {
            self.unsupported_count += 1;
            return format!("Error: Unknown actor for action: {actor}");
        };
        self.apply_action_status(action, args);
        format!(
            "{} -> {} [{spent_ctb}] | {}",
            actor.display_name(),
            display_action_name(action),
            self.current_battle_state().ctb_order_string()
        )
    }

    fn apply_monster_action(&mut self, slot: MonsterSlot, action: &str, args: &[String]) -> String {
        self.ensure_monster_slot(slot);
        let actor_id = ActorId::Monster(slot);
        let Some(spent_ctb) = self.apply_actor_turn(actor_id, action_rank(action)) else {
            self.unsupported_count += 1;
            return format!("Error: Unknown monster slot for action: m{}", slot.0);
        };
        self.apply_action_status(action, args);
        format!(
            "M{} -> {} [{spent_ctb}] | {}",
            slot.0,
            display_action_name(action),
            self.current_battle_state().ctb_order_string()
        )
    }

    fn apply_action_status(&mut self, action: &str, args: &[String]) {
        let lowered = action.to_ascii_lowercase();
        if !matches!(lowered.as_str(), "haste" | "slow") {
            return;
        }
        let Some(target) = args.first().and_then(|arg| self.resolve_actor_id(arg)) else {
            return;
        };
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        match lowered.as_str() {
            "haste" => {
                actor.statuses.remove(&Status::Slow);
                actor.statuses.insert(Status::Haste);
            }
            "slow" => {
                actor.statuses.remove(&Status::Haste);
                actor.statuses.insert(Status::Slow);
            }
            _ => {}
        }
    }

    fn change_equipment(&mut self, kind: &str, args: &[String]) -> String {
        format!("Equipment: {kind} {}", args.join(" "))
    }

    fn summon(&mut self, aeon_name: &str) -> String {
        let Some(aeon) = parse_summon_name(aeon_name) else {
            self.unsupported_count += 1;
            return format!("Error: No aeon named \"{aeon_name}\"");
        };
        let old_party = self.format_party();
        self.party = vec![aeon];
        format!("Party: {old_party} -> {}", self.format_party())
    }

    fn spawn_monster(
        &mut self,
        monster_name: &str,
        slot: MonsterSlot,
        forced_ctb: Option<i32>,
    ) -> String {
        let template = monster_template(monster_name);
        let mut actor = BattleActor::monster_with_key(
            slot,
            Some(monster_name.to_string()),
            template.agility,
            template.max_hp,
        );
        if let Some(ctb) = forced_ctb {
            actor.ctb = ctb;
        }
        if let Some(existing) = self
            .monsters
            .iter_mut()
            .find(|actor| actor.id == ActorId::Monster(slot))
        {
            *existing = actor;
        } else {
            self.monsters.push(actor);
            self.monsters.sort_by_key(|actor| actor.index);
        }
        format!("Spawn: {monster_name} -> m{}", slot.0)
    }

    fn change_element(&mut self, args: &[String]) -> String {
        format!("Element: {}", args.join(" "))
    }

    fn change_stat(&mut self, args: &[String]) -> String {
        let Some(actor_id) = args.first().and_then(|arg| self.resolve_actor_id(arg)) else {
            self.unsupported_count += 1;
            return format!("Error: invalid stat actor: {}", args.join(" "));
        };
        let stat_name = args.get(1).map(|value| value.to_ascii_lowercase());
        let amount = args.get(2).map(String::as_str).unwrap_or("0");
        if let ActorId::Monster(slot) = actor_id {
            self.ensure_monster_slot(slot);
        }
        let Some(actor) = self.actor_mut(actor_id) else {
            self.unsupported_count += 1;
            return format!("Error: unknown stat actor: {}", args[0]);
        };
        match stat_name.as_deref() {
            Some("ctb") => {
                actor.ctb = parse_signed_amount(amount, actor.ctb);
                format!("Stat: {} CTB -> {}", actor_label(actor), actor.ctb)
            }
            Some("hp") => {
                actor.current_hp = parse_signed_amount(amount, actor.current_hp);
                if actor.current_hp <= 0 {
                    actor.statuses.insert(Status::Death);
                } else {
                    actor.statuses.remove(&Status::Death);
                }
                format!("Stat: {} HP -> {}", actor_label(actor), actor.current_hp)
            }
            Some("agility") => {
                let agility = parse_signed_amount(amount, actor.agility as i32).clamp(0, 255);
                actor.agility = agility as u8;
                format!("Stat: {} Agility -> {}", actor_label(actor), actor.agility)
            }
            Some(stat) => format!("Stat: {} {stat} {}", actor_label(actor), amount),
            None => format!("Stat: {}", actor_label(actor)),
        }
    }

    fn change_status(&mut self, args: &[String]) -> String {
        let Some(actor_id) = args.first().and_then(|arg| self.resolve_actor_id(arg)) else {
            self.unsupported_count += 1;
            return format!("Error: invalid status actor: {}", args.join(" "));
        };
        let status = args.get(1).and_then(|status| parse_status(status));
        let stacks = args
            .get(2)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(254);
        if let ActorId::Monster(slot) = actor_id {
            self.ensure_monster_slot(slot);
        }
        let Some(actor) = self.actor_mut(actor_id) else {
            self.unsupported_count += 1;
            return format!("Error: unknown status actor: {}", args[0]);
        };
        if let Some(status) = status {
            if stacks <= 0 {
                actor.statuses.remove(&status);
            } else {
                actor.statuses.insert(status);
            }
        }
        format!(
            "Status: {} {}/{} HP",
            actor_label(actor),
            actor.current_hp,
            actor.max_hp
        )
    }

    fn heal_party(&mut self, args: &[String]) -> String {
        if let Some(target) = args.first().and_then(|arg| self.resolve_actor_id(arg)) {
            if let Some(actor) = self.actor_mut(target) {
                actor.current_hp = actor.max_hp;
                actor.statuses.remove(&Status::Death);
                actor.statuses.remove(&Status::Poison);
                return format!("Heal: {} HP restored", actor_label(actor));
            }
        } else {
            for actor in &mut self.character_actors {
                actor.current_hp = actor.max_hp;
                actor.statuses.remove(&Status::Death);
                actor.statuses.remove(&Status::Poison);
            }
        }
        "Heal: party HP restored".to_string()
    }

    fn end_encounter(&mut self) -> String {
        self.process_start_of_encounter();
        "End Encounter".to_string()
    }

    fn ensure_monster_slot(&mut self, slot: MonsterSlot) {
        if self
            .monsters
            .iter()
            .any(|actor| actor.id == ActorId::Monster(slot))
        {
            return;
        }
        while self.monsters.len() < slot.0 {
            let next_slot = MonsterSlot(self.monsters.len() + 1);
            self.monsters
                .push(BattleActor::monster_with_key(next_slot, None, 10, 1_000));
        }
    }

    fn apply_actor_turn(&mut self, actor_id: ActorId, rank: i32) -> Option<i32> {
        let spent_ctb = {
            let actor = self.actor_mut(actor_id)?;
            let spent_ctb = actor.turn_ctb(rank);
            actor.ctb = (actor.ctb + spent_ctb).max(0);
            spent_ctb
        };
        self.normalize_after_turn();
        Some(spent_ctb)
    }

    fn normalize_after_turn(&mut self) {
        let min_ctb = self
            .current_party_actors()
            .iter()
            .chain(self.monsters.iter())
            .filter(|actor| actor.is_alive())
            .map(|actor| actor.ctb)
            .min()
            .unwrap_or(0);
        if min_ctb == 0 {
            return;
        }
        let party = self.party.clone();
        for actor in &mut self.character_actors {
            if party.contains(&character_id(actor)) && !actor.statuses.contains(&Status::Petrify) {
                actor.ctb = (actor.ctb - min_ctb).max(0);
            }
        }
        for actor in &mut self.monsters {
            if !actor.statuses.contains(&Status::Petrify) {
                actor.ctb = (actor.ctb - min_ctb).max(0);
            }
        }
    }

    fn parse_actor_id(&self, value: &str) -> Option<ActorId> {
        if let Ok(character) = value.parse::<Character>() {
            return Some(ActorId::Character(character));
        }
        value.parse::<MonsterSlot>().ok().map(ActorId::Monster)
    }

    fn resolve_actor_id(&self, value: &str) -> Option<ActorId> {
        if let Some(actor_id) = self.parse_actor_id(value) {
            return Some(actor_id);
        }
        let value_family = monster_family(value);
        self.monsters
            .iter()
            .filter(|actor| actor.is_alive())
            .find(|actor| {
                actor
                    .monster_key
                    .as_deref()
                    .is_some_and(|key| key == value || monster_family(key) == value_family)
            })
            .map(|actor| actor.id)
    }

    fn actor_mut(&mut self, actor_id: ActorId) -> Option<&mut BattleActor> {
        self.character_actors
            .iter_mut()
            .chain(self.monsters.iter_mut())
            .find(|actor| actor.id == actor_id)
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonsterTemplate {
    key: String,
    agility: u8,
    max_hp: i32,
}

fn monster_template(name: &str) -> MonsterTemplate {
    data::monster_stats(name)
        .map(|stats| MonsterTemplate {
            key: stats.key,
            agility: stats.agility,
            max_hp: stats.max_hp,
        })
        .unwrap_or_else(|| MonsterTemplate {
            key: name.to_string(),
            agility: 10,
            max_hp: 1_000,
        })
}

fn action_rank(action: &str) -> i32 {
    data::action_rank(action).unwrap_or_else(|| match action.to_ascii_lowercase().as_str() {
        "quick_hit_ps2" => 1,
        "defend" | "quick_hit_hd" | "use" => 2,
        "haste" => 4,
        "delay_attack" => 6,
        "delay_buster" => 8,
        _ => 3,
    })
}

fn parse_summon_name(name: &str) -> Option<Character> {
    match name.to_ascii_lowercase().as_str() {
        "valefor" => Some(Character::Valefor),
        "ifrit" => Some(Character::Ifrit),
        "ixion" => Some(Character::Ixion),
        "shiva" => Some(Character::Shiva),
        "bahamut" => Some(Character::Bahamut),
        "anima" => Some(Character::Anima),
        "yojimbo" => Some(Character::Yojimbo),
        _ => None,
    }
}

fn parse_status(name: &str) -> Option<Status> {
    match name.to_ascii_lowercase().as_str() {
        "death" => Some(Status::Death),
        "eject" => Some(Status::Eject),
        "petrify" => Some(Status::Petrify),
        "sleep" => Some(Status::Sleep),
        "haste" => Some(Status::Haste),
        "slow" => Some(Status::Slow),
        "regen" => Some(Status::Regen),
        "poison" => Some(Status::Poison),
        "doom" => Some(Status::Doom),
        _ => None,
    }
}

fn parse_signed_amount(amount: &str, current: i32) -> i32 {
    let parsed = amount.parse::<i32>().unwrap_or(current);
    if amount.starts_with(['+', '-']) {
        current + parsed
    } else {
        parsed
    }
}

fn monster_family(name: &str) -> &str {
    name.rsplit_once('_')
        .filter(|(_, suffix)| suffix.chars().all(|character| character.is_ascii_digit()))
        .map(|(family, _)| family)
        .unwrap_or(name)
}

fn actor_label(actor: &BattleActor) -> String {
    match actor.id {
        ActorId::Character(character) => character.display_name().to_string(),
        ActorId::Monster(slot) => actor
            .monster_key
            .as_deref()
            .map(|key| format!("{key} (M{})", slot.0))
            .unwrap_or_else(|| format!("M{}", slot.0)),
    }
}

fn display_action_name(action: &str) -> String {
    action
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut word = first.to_ascii_uppercase().to_string();
                    word.push_str(chars.as_str());
                    word
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
        assert!(output.text.contains("Encounter:   1 | Tanker | Normal |"));
        assert!(output.text.contains("ATB:"));
        assert!(output.text.contains("Tidus -> Attack ["));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn simulates_basic_action_ctb_and_statuses() {
        let lines = vec![
            "party tay".to_string(),
            "encounter tanker".to_string(),
            "tidus haste auron".to_string(),
            "auron defend".to_string(),
            "m5 spines".to_string(),
            "heal".to_string(),
        ];
        let output = simulate(1, &lines);
        assert!(output.text.contains("Tidus -> Haste ["));
        assert!(output.text.contains("Auron -> Defend ["));
        assert!(output.text.contains("M5 -> Spines ["));
        assert!(output.text.contains("Heal: party HP restored"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn simulates_shallow_state_commands() {
        let lines = vec![
            "encounter tanker".to_string(),
            "stat m1 ctb -1".to_string(),
            "status m1 sleep".to_string(),
            "status m1 sleep 0".to_string(),
            "spawn sinscale_3 4 -2".to_string(),
            "tidus attack tanker".to_string(),
            "summon valefor".to_string(),
            "element m1 thunder weak".to_string(),
            "weapon tidus 4 strength_+5%".to_string(),
            "roll rng4".to_string(),
        ];
        let output = simulate(1, &lines);
        assert!(output.text.contains("Stat: tanker (M1) CTB ->"));
        assert!(output.text.contains("Status: tanker (M1)"));
        assert!(output.text.contains("Spawn: sinscale_3 -> m4"));
        assert!(output.text.contains("Tidus -> Attack ["));
        assert!(output.text.contains("Party: Tidus, Auron -> Valefor"));
        assert!(output.text.contains("Element: m1 thunder weak"));
        assert!(output
            .text
            .contains("Equipment: weapon tidus 4 strength_+5%"));
        assert!(output.text.contains("Advanced rng4 1 times"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn renders_full_default_script_without_parser_gaps() {
        let input = include_str!(
            "../../ctb-live-editor-pages/search_outputs/3096296922/ctb_actions_input.txt"
        );
        let prepared = crate::script::prepare_action_lines(input);
        let output = simulate(3096296922, &prepared.lines);
        let errors = output
            .text
            .lines()
            .filter(|line| line.starts_with("Error:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(output.unsupported_count, 0, "{errors}");
        assert!(output.text.contains("Encounter:"));
    }
}

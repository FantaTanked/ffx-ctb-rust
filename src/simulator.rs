use crate::battle::{ActorId, BattleActor, BattleState, CombatStats};
use crate::data::{self, ActionData, ActionTarget, DamageFormula, DamageType};
use crate::encounter::{character_initial_ctb, encounter_condition_from_roll, monster_initial_ctb};
use crate::model::{
    AutoAbility, Buff, Character, Element, ElementalAffinity, EncounterCondition, MonsterSlot,
    Status,
};
use crate::parser::{parse_raw_action_line, ParsedCommand};
use crate::rng::FfxRngTracker;
use std::collections::HashSet;

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
        let has_initiative = self.party.iter().any(|character| {
            self.character_actor(*character)
                .is_some_and(|actor| actor.has_auto_ability(AutoAbility::Initiative))
        });
        let condition = encounter_condition_from_roll(
            condition_roll,
            has_initiative,
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
            actor.buffs.clear();
            actor.statuses.clear();
            apply_auto_statuses(actor);
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
            let mut actor = BattleActor::monster_with_key(
                MonsterSlot(index + 1),
                Some(template.key.clone()),
                template.agility,
                template.immune_to_delay,
                template.max_hp,
            );
            actor.set_combat_stats(template.combat_stats);
            actor.set_elemental_affinities(template.elemental_affinities.clone());
            actor.set_status_resistances(template.status_resistances.clone());
            apply_damage_traits(&mut actor, template);
            apply_template_auto_statuses(&mut actor, template);
            self.monsters.push(actor);
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
                        actor.has_auto_ability(AutoAbility::FirstStrike),
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
                        actor.has_auto_ability(AutoAbility::FirstStrike),
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
        let action_data = self.action_data_for_actor(actor_id, action);
        let rank = action_data
            .as_ref()
            .map(|action| action.rank)
            .unwrap_or_else(|| fallback_action_rank(action));
        let Some(spent_ctb) = self.apply_actor_turn(actor_id, rank) else {
            self.unsupported_count += 1;
            return format!("Error: Unknown actor for action: {actor}");
        };
        self.apply_action_effects(actor_id, action_data.as_ref(), args);
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
        let action_data = self.action_data_for_actor(actor_id, action);
        let rank = action_data
            .as_ref()
            .map(|action| action.rank)
            .unwrap_or_else(|| fallback_action_rank(action));
        let Some(spent_ctb) = self.apply_actor_turn(actor_id, rank) else {
            self.unsupported_count += 1;
            return format!("Error: Unknown monster slot for action: m{}", slot.0);
        };
        self.apply_action_effects(actor_id, action_data.as_ref(), args);
        format!(
            "M{} -> {} [{spent_ctb}] | {}",
            slot.0,
            display_action_name(action),
            self.current_battle_state().ctb_order_string()
        )
    }

    fn action_data_for_actor(&self, user: ActorId, action: &str) -> Option<ActionData> {
        if let ActorId::Monster(_) = user {
            let monster_key = self
                .actor(user)
                .and_then(|actor| actor.monster_key.as_deref());
            if let Some(action_data) =
                monster_key.and_then(|key| data::monster_action_data(key, action))
            {
                return Some(action_data);
            }
        }
        data::action_data(action)
    }

    fn apply_action_effects(
        &mut self,
        user: ActorId,
        action_data: Option<&ActionData>,
        args: &[String],
    ) {
        let Some(action_data) = action_data else {
            return;
        };
        let user_actor = self.actor(user).cloned();
        let targets = self.resolve_action_targets(user, &action_data, args);
        for target in targets {
            if action_misses_target(action_data, self.actor(target)) {
                continue;
            }
            if let Some(user_actor) = user_actor.as_ref() {
                self.apply_action_damage(user_actor, target, action_data);
            }
            if action_data.removes_statuses {
                for status in &action_data.statuses {
                    self.remove_status_from_actor(target, *status);
                }
            } else {
                for application in &action_data.status_applications {
                    self.apply_action_status_to_actor(target, application);
                }
            }
            if action_data.heals && !action_data.damages_hp {
                self.heal_actor(target);
            }
            for buff in &action_data.buffs {
                self.apply_buff_to_actor(target, buff.buff, buff.amount);
            }
            if action_data.uses_weapon_properties {
                self.apply_weapon_statuses(user, target);
            }
            if action_data.has_weak_delay {
                self.apply_delay(target, 3, 2);
            }
            if action_data.has_strong_delay {
                self.apply_delay(target, 3, 1);
            }
        }
    }

    fn change_equipment(&mut self, kind: &str, args: &[String]) -> String {
        let Some(character) = args.first().and_then(|arg| arg.parse::<Character>().ok()) else {
            self.unsupported_count += 1;
            return format!("Error: invalid equipment actor: {kind} {}", args.join(" "));
        };
        let abilities = parse_equipment_abilities(args);
        let Some(actor) = self.actor_mut(ActorId::Character(character)) else {
            self.unsupported_count += 1;
            return format!(
                "Error: unknown equipment actor: {}",
                character.display_name()
            );
        };
        match kind.to_ascii_lowercase().as_str() {
            "weapon" => actor.set_weapon_abilities(abilities),
            "armor" => actor.set_armor_abilities(abilities),
            _ => {
                self.unsupported_count += 1;
                return format!("Error: invalid equipment type: {kind}");
            }
        }
        apply_auto_statuses(actor);
        apply_equipment_elements(actor);
        format!(
            "Equipment: {kind} {} [{}]",
            character.display_name(),
            args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ")
        )
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
            template.immune_to_delay,
            template.max_hp,
        );
        actor.set_combat_stats(template.combat_stats);
        actor.set_elemental_affinities(template.elemental_affinities.clone());
        actor.set_status_resistances(template.status_resistances.clone());
        apply_damage_traits(&mut actor, &template);
        apply_template_auto_statuses(&mut actor, &template);
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
        let Some(actor_id) = args.first().and_then(|arg| self.resolve_actor_id(arg)) else {
            self.unsupported_count += 1;
            return format!("Error: invalid element actor: {}", args.join(" "));
        };
        let Some(element) = args.get(1).and_then(|arg| arg.parse::<Element>().ok()) else {
            self.unsupported_count += 1;
            return format!("Error: invalid element: {}", args.join(" "));
        };
        let Some(affinity) = args
            .get(2)
            .and_then(|arg| arg.parse::<ElementalAffinity>().ok())
        else {
            self.unsupported_count += 1;
            return format!("Error: invalid element affinity: {}", args.join(" "));
        };
        if let ActorId::Monster(slot) = actor_id {
            self.ensure_monster_slot(slot);
        }
        if let Some(actor) = self.actor_mut(actor_id) {
            actor.set_elemental_affinity(element, affinity);
        }
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
            Some("strength") => {
                actor.combat_stats.strength =
                    parse_signed_amount(amount, actor.combat_stats.strength).clamp(0, 255);
                format!(
                    "Stat: {} Strength -> {}",
                    actor_label(actor),
                    actor.combat_stats.strength
                )
            }
            Some("defense") => {
                actor.combat_stats.defense =
                    parse_signed_amount(amount, actor.combat_stats.defense).clamp(0, 255);
                format!(
                    "Stat: {} Defense -> {}",
                    actor_label(actor),
                    actor.combat_stats.defense
                )
            }
            Some("magic") => {
                actor.combat_stats.magic =
                    parse_signed_amount(amount, actor.combat_stats.magic).clamp(0, 255);
                format!(
                    "Stat: {} Magic -> {}",
                    actor_label(actor),
                    actor.combat_stats.magic
                )
            }
            Some("magic_defense" | "magic_def") => {
                actor.combat_stats.magic_defense =
                    parse_signed_amount(amount, actor.combat_stats.magic_defense).clamp(0, 255);
                format!(
                    "Stat: {} Magic defense -> {}",
                    actor_label(actor),
                    actor.combat_stats.magic_defense
                )
            }
            Some("luck") => {
                actor.combat_stats.luck =
                    parse_signed_amount(amount, actor.combat_stats.luck).clamp(0, 255);
                format!(
                    "Stat: {} Luck -> {}",
                    actor_label(actor),
                    actor.combat_stats.luck
                )
            }
            Some("evasion") => {
                actor.combat_stats.evasion =
                    parse_signed_amount(amount, actor.combat_stats.evasion).clamp(0, 255);
                format!(
                    "Stat: {} Evasion -> {}",
                    actor_label(actor),
                    actor.combat_stats.evasion
                )
            }
            Some("accuracy") => {
                actor.combat_stats.accuracy =
                    parse_signed_amount(amount, actor.combat_stats.accuracy).clamp(0, 255);
                format!(
                    "Stat: {} Accuracy -> {}",
                    actor_label(actor),
                    actor.combat_stats.accuracy
                )
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
                Self::restore_actor(actor);
                return format!("Heal: {} HP restored", actor_label(actor));
            }
        } else {
            for actor in &mut self.character_actors {
                Self::restore_actor(actor);
            }
        }
        "Heal: party HP restored".to_string()
    }

    fn resolve_action_targets(
        &self,
        user: ActorId,
        action: &ActionData,
        args: &[String],
    ) -> Vec<ActorId> {
        let explicit_targets = args
            .iter()
            .flat_map(|arg| self.resolve_action_target_arg(arg))
            .collect::<Vec<_>>();
        if !explicit_targets.is_empty() {
            return explicit_targets;
        }

        match action.target {
            ActionTarget::SelfTarget | ActionTarget::CounterSelf => vec![user],
            ActionTarget::CharactersParty | ActionTarget::CounterCharactersParty => match user {
                ActorId::Character(_) => {
                    self.party.iter().copied().map(ActorId::Character).collect()
                }
                ActorId::Monster(_) => self.party.iter().copied().map(ActorId::Character).collect(),
            },
            ActionTarget::MonstersParty => match user {
                ActorId::Character(_) => self.monsters.iter().map(|actor| actor.id).collect(),
                ActorId::Monster(_) => self.monsters.iter().map(|actor| actor.id).collect(),
            },
            ActionTarget::SingleCharacter | ActionTarget::RandomCharacter => self
                .party
                .first()
                .copied()
                .map(ActorId::Character)
                .into_iter()
                .collect(),
            ActionTarget::SingleMonster | ActionTarget::RandomMonster => self
                .monsters
                .first()
                .map(|actor| actor.id)
                .into_iter()
                .collect(),
            ActionTarget::Counter => self.current_battle_state().last_actor.into_iter().collect(),
            ActionTarget::Character(character) => vec![ActorId::Character(character)],
            ActionTarget::Monster(slot) => vec![ActorId::Monster(slot)],
            ActionTarget::Single | ActionTarget::EitherParty | ActionTarget::None => Vec::new(),
        }
    }

    fn apply_status_to_actor(&mut self, target: ActorId, status: Status) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        match status {
            Status::Haste => {
                actor.statuses.remove(&Status::Slow);
                actor.statuses.insert(Status::Haste);
            }
            Status::Slow => {
                actor.statuses.remove(&Status::Haste);
                actor.statuses.insert(Status::Slow);
            }
            Status::Petrify => {
                actor.buffs.clear();
                actor.statuses.clear();
                actor.statuses.insert(Status::Petrify);
            }
            Status::Death => {
                actor.current_hp = 0;
                actor.buffs.clear();
                actor.statuses.clear();
                actor.statuses.insert(Status::Death);
            }
            other => {
                actor.statuses.insert(other);
            }
        }
    }

    fn apply_action_status_to_actor(&mut self, target: ActorId, application: &data::ActionStatus) {
        if self
            .actor(target)
            .and_then(|actor| actor.status_resistances.get(&application.status))
            .is_some_and(|resistance| *resistance == 255)
            && !application.ignores_resistance
        {
            return;
        }
        self.apply_status_to_actor(target, application.status);
    }

    fn remove_status_from_actor(&mut self, target: ActorId, status: Status) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        if status == Status::Death {
            if actor.statuses.contains(&Status::Zombie) {
                if !actor.immune_to_life {
                    actor.current_hp = 0;
                    actor.buffs.clear();
                    actor.statuses.clear();
                    actor.statuses.insert(Status::Death);
                }
                return;
            }
            if !actor.statuses.remove(&status) {
                return;
            }
            actor.current_hp = actor.max_hp.max(1);
            actor.ctb = actor.base_ctb() * 3;
            return;
        }
        if !actor.statuses.remove(&status) {
            return;
        }
    }

    fn apply_buff_to_actor(&mut self, target: ActorId, buff: Buff, amount: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.add_buff(buff, amount);
    }

    fn apply_weapon_statuses(&mut self, user: ActorId, target: ActorId) {
        let Some(actor) = self.actor(user) else {
            return;
        };
        let statuses = actor
            .weapon_abilities
            .iter()
            .filter_map(|ability| weapon_status(*ability))
            .collect::<Vec<_>>();
        for status in statuses {
            self.apply_status_to_actor(target, status);
        }
    }

    fn apply_action_damage(&mut self, user: &BattleActor, target: ActorId, action: &ActionData) {
        if action.damage_formula == DamageFormula::NoDamage {
            return;
        }
        if !(action.damages_hp || action.damages_mp || action.damages_ctb) {
            return;
        }
        let Some(target_actor) = self.actor(target).cloned() else {
            return;
        };
        let damage = calculate_action_damage(user, &target_actor, action);
        if action.damages_hp {
            self.apply_hp_damage(target, damage);
            if action.drains {
                self.apply_hp_damage(user.id, -damage);
            }
        }
        if action.damages_ctb {
            if let Some(actor) = self.actor_mut(target) {
                actor.ctb += damage;
            }
        }
    }

    fn apply_hp_damage(&mut self, target: ActorId, damage: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.current_hp = (actor.current_hp - damage).clamp(0, actor.max_hp.max(1));
        if actor.current_hp <= 0 {
            actor.buffs.clear();
            actor.statuses.clear();
            actor.statuses.insert(Status::Death);
        } else {
            actor.statuses.remove(&Status::Death);
        }
    }

    fn heal_actor(&mut self, target: ActorId) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        Self::restore_actor(actor);
    }

    fn restore_actor(actor: &mut BattleActor) {
        actor.current_hp = actor.max_hp;
        actor.statuses.remove(&Status::Death);
        actor.statuses.remove(&Status::Poison);
        actor.statuses.remove(&Status::Zombie);
    }

    fn apply_delay(&mut self, target: ActorId, numerator: i32, divisor: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        if actor.immune_to_delay {
            return;
        }
        actor.ctb += actor.base_ctb() * numerator / divisor;
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
            self.monsters.push(BattleActor::monster_with_key(
                next_slot, None, 10, false, 1_000,
            ));
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

    fn resolve_action_target_arg(&self, value: &str) -> Vec<ActorId> {
        match value.to_ascii_lowercase().as_str() {
            "party" | "characters" | "chars" => {
                self.party.iter().copied().map(ActorId::Character).collect()
            }
            "monsters" | "monster" | "enemies" | "enemy" => {
                self.monsters.iter().map(|actor| actor.id).collect()
            }
            _ => self.resolve_actor_id(value).into_iter().collect(),
        }
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

    fn actor(&self, actor_id: ActorId) -> Option<&BattleActor> {
        self.character_actors
            .iter()
            .chain(self.monsters.iter())
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
    all_characters()
        .into_iter()
        .map(default_character_actor)
        .collect()
}

fn all_characters() -> [Character; 19] {
    [
        Character::Tidus,
        Character::Yuna,
        Character::Auron,
        Character::Kimahri,
        Character::Wakka,
        Character::Lulu,
        Character::Rikku,
        Character::Seymour,
        Character::Valefor,
        Character::Ifrit,
        Character::Ixion,
        Character::Shiva,
        Character::Bahamut,
        Character::Anima,
        Character::Yojimbo,
        Character::Cindy,
        Character::Sandy,
        Character::Mindy,
        Character::Unknown,
    ]
}

fn default_character_actor(character: Character) -> BattleActor {
    let fallback = fallback_character_defaults(character);
    let stats = data::character_stats(character);
    let index = stats
        .as_ref()
        .map(|stats| stats.index)
        .unwrap_or(fallback.0);
    let agility = stats
        .as_ref()
        .map(|stats| stats.agility)
        .filter(|agility| *agility > 0 || character == Character::Unknown)
        .unwrap_or(fallback.1);
    let max_hp = stats
        .as_ref()
        .map(|stats| stats.max_hp)
        .filter(|max_hp| *max_hp > 0)
        .unwrap_or(fallback.2);
    let mut actor = BattleActor::character(character, index, agility, max_hp);
    if let Some(stats) = stats {
        actor.set_combat_stats(CombatStats {
            strength: stats.strength,
            defense: stats.defense,
            magic: stats.magic,
            magic_defense: stats.magic_defense,
            luck: stats.luck,
            evasion: stats.evasion,
            accuracy: stats.accuracy,
            base_weapon_damage: stats.base_weapon_damage,
        });
    }
    actor
}

fn fallback_character_defaults(character: Character) -> (usize, u8, i32) {
    match character {
        Character::Tidus => (0, 10, 520),
        Character::Yuna => (1, 10, 475),
        Character::Auron => (2, 5, 1030),
        Character::Kimahri => (3, 6, 644),
        Character::Wakka => (4, 7, 618),
        Character::Lulu => (5, 5, 380),
        Character::Rikku => (6, 16, 360),
        Character::Seymour => (7, 20, 1200),
        Character::Valefor => (8, 0, 99_999),
        Character::Ifrit => (9, 0, 99_999),
        Character::Ixion => (10, 0, 99_999),
        Character::Shiva => (11, 0, 99_999),
        Character::Bahamut => (12, 0, 99_999),
        Character::Anima => (13, 0, 99_999),
        Character::Yojimbo => (14, 0, 99_999),
        Character::Cindy => (15, 0, 99_999),
        Character::Sandy => (16, 0, 99_999),
        Character::Mindy => (17, 0, 99_999),
        Character::Unknown => (18, 0, 99_999),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonsterTemplate {
    key: String,
    agility: u8,
    immune_to_delay: bool,
    max_hp: i32,
    combat_stats: CombatStats,
    elemental_affinities: std::collections::HashMap<Element, ElementalAffinity>,
    status_resistances: std::collections::HashMap<Status, u8>,
    armored: bool,
    immune_to_damage: bool,
    immune_to_percentage_damage: bool,
    immune_to_physical_damage: bool,
    immune_to_magical_damage: bool,
    immune_to_life: bool,
    auto_statuses: Vec<Status>,
}

fn monster_template(name: &str) -> MonsterTemplate {
    data::monster_stats(name)
        .map(|stats| MonsterTemplate {
            key: stats.key,
            agility: stats.agility,
            immune_to_delay: stats.immune_to_delay,
            max_hp: stats.max_hp,
            combat_stats: CombatStats {
                strength: stats.strength,
                defense: stats.defense,
                magic: stats.magic,
                magic_defense: stats.magic_defense,
                luck: stats.luck,
                evasion: stats.evasion,
                accuracy: stats.accuracy,
                base_weapon_damage: stats.base_weapon_damage,
            },
            elemental_affinities: stats.elemental_affinities,
            status_resistances: stats.status_resistances,
            armored: stats.armored,
            immune_to_damage: stats.immune_to_damage,
            immune_to_percentage_damage: stats.immune_to_percentage_damage,
            immune_to_physical_damage: stats.immune_to_physical_damage,
            immune_to_magical_damage: stats.immune_to_magical_damage,
            immune_to_life: stats.immune_to_life,
            auto_statuses: stats.auto_statuses,
        })
        .unwrap_or_else(|| MonsterTemplate {
            key: name.to_string(),
            agility: 10,
            immune_to_delay: false,
            max_hp: 1_000,
            combat_stats: CombatStats::default(),
            elemental_affinities: crate::battle::neutral_elemental_affinities(),
            status_resistances: std::collections::HashMap::new(),
            armored: false,
            immune_to_damage: false,
            immune_to_percentage_damage: false,
            immune_to_physical_damage: false,
            immune_to_magical_damage: false,
            immune_to_life: false,
            auto_statuses: Vec::new(),
        })
}

fn apply_damage_traits(actor: &mut BattleActor, template: &MonsterTemplate) {
    actor.armored = template.armored;
    actor.immune_to_damage = template.immune_to_damage;
    actor.immune_to_percentage_damage = template.immune_to_percentage_damage;
    actor.immune_to_physical_damage = template.immune_to_physical_damage;
    actor.immune_to_magical_damage = template.immune_to_magical_damage;
    actor.immune_to_life = template.immune_to_life;
}

fn apply_template_auto_statuses(actor: &mut BattleActor, template: &MonsterTemplate) {
    for status in &template.auto_statuses {
        actor.statuses.insert(*status);
    }
}

fn fallback_action_rank(action: &str) -> i32 {
    data::action_rank(action).unwrap_or_else(|| match action.to_ascii_lowercase().as_str() {
        "quick_hit_ps2" => 1,
        "defend" | "quick_hit_hd" | "use" => 2,
        "haste" => 4,
        "delay_attack" => 6,
        "delay_buster" => 8,
        _ => 3,
    })
}

fn calculate_action_damage(user: &BattleActor, target: &BattleActor, action: &ActionData) -> i32 {
    if target.immune_to_damage {
        return 0;
    }
    if target.immune_to_physical_damage && action.damage_type == DamageType::Physical {
        return 0;
    }
    if target.immune_to_magical_damage && action.damage_type == DamageType::Magical {
        return 0;
    }
    let base_damage = if action.uses_weapon_properties {
        user.combat_stats.base_weapon_damage
    } else {
        action.base_damage
    };
    let mut damage = match action.damage_formula {
        DamageFormula::NoDamage => 0,
        DamageFormula::Fixed => base_damage * 50 * 0xf0 / 256,
        DamageFormula::FixedNoVariance => base_damage * 50,
        DamageFormula::PercentageTotal | DamageFormula::PercentageTotalMp => {
            if target.immune_to_percentage_damage {
                0
            } else {
                target.max_hp * base_damage / 16
            }
        }
        DamageFormula::PercentageCurrent | DamageFormula::PercentageCurrentMp => {
            if target.immune_to_percentage_damage {
                0
            } else {
                target.current_hp * base_damage / 16
            }
        }
        DamageFormula::Hp => user.max_hp * base_damage / 10,
        DamageFormula::Ctb | DamageFormula::BaseCtb => target.ctb * base_damage / 16,
        DamageFormula::Deal9999 => 9999 * action.base_damage,
        DamageFormula::Strength
        | DamageFormula::PiercingStrength
        | DamageFormula::Magic
        | DamageFormula::PiercingMagic
        | DamageFormula::SpecialMagic
        | DamageFormula::SpecialMagicNoVariance
        | DamageFormula::PiercingStrengthNoVariance
        | DamageFormula::Healing => stat_based_action_damage(user, target, action, base_damage),
        DamageFormula::CelestialHighHp
        | DamageFormula::CelestialHighMp
        | DamageFormula::CelestialLowHp
        | DamageFormula::Gil
        | DamageFormula::Kills => 0,
    };
    damage *= action.n_of_hits.max(1);
    damage = apply_damage_status_modifiers(damage, user, target, action);
    if action.drains {
        if user.statuses.contains(&Status::Zombie) {
            damage = -damage;
        }
        if target.statuses.contains(&Status::Zombie) {
            damage = -damage;
        }
    }
    if action.heals && !target.statuses.contains(&Status::Zombie) {
        -damage
    } else {
        damage
    }
}

fn action_misses_target(action: &ActionData, target: Option<&BattleActor>) -> bool {
    action.misses_if_target_alive
        && target.is_some_and(|target| {
            !target.statuses.contains(&Status::Death) && !target.statuses.contains(&Status::Zombie)
        })
}

fn stat_based_action_damage(
    user: &BattleActor,
    target: &BattleActor,
    action: &ActionData,
    base_damage: i32,
) -> i32 {
    let (offensive_stat, defensive_stat, defensive_buffs) = match action.damage_formula {
        DamageFormula::Strength => (
            user.combat_stats.strength + buff_stacks(user, Buff::Cheer),
            if target.statuses.contains(&Status::ArmorBreak) {
                0
            } else {
                target.combat_stats.defense.max(1)
            },
            buff_stacks(target, Buff::Cheer),
        ),
        DamageFormula::PiercingStrength | DamageFormula::PiercingStrengthNoVariance => (
            user.combat_stats.strength + buff_stacks(user, Buff::Cheer),
            0,
            buff_stacks(target, Buff::Cheer),
        ),
        DamageFormula::Magic => (
            user.combat_stats.magic + buff_stacks(user, Buff::Focus),
            if target.statuses.contains(&Status::MentalBreak) {
                0
            } else {
                target.combat_stats.magic_defense.max(1)
            },
            buff_stacks(target, Buff::Focus),
        ),
        DamageFormula::PiercingMagic
        | DamageFormula::SpecialMagic
        | DamageFormula::SpecialMagicNoVariance => (
            user.combat_stats.magic + buff_stacks(user, Buff::Focus),
            0,
            buff_stacks(target, Buff::Focus),
        ),
        DamageFormula::Healing => (
            user.combat_stats.magic + buff_stacks(user, Buff::Focus),
            0,
            buff_stacks(target, Buff::Focus),
        ),
        _ => (0, 0, 0),
    };

    let power = damage_power(action.damage_formula, base_damage, offensive_stat);
    let mitigation = damage_mitigation(defensive_stat);
    let damage_1 = i64::from(power) * i64::from(mitigation);
    let damage_2 = python_div_i64(damage_1 * -1_282_606_671, 0xffff_ffff);
    let damage_3 = (damage_1 + damage_2) / 0x200 * i64::from(15 - defensive_buffs);
    let damage_4 = python_div_i64(damage_3 * -2_004_318_071, 0xffff_ffff);
    let mut damage = ((damage_3 + damage_4) / 0x8) as i32;
    if matches!(
        action.damage_formula,
        DamageFormula::Strength | DamageFormula::PiercingStrength | DamageFormula::SpecialMagic
    ) {
        damage = damage * base_damage / 0x10;
    }
    damage * 0xf0 / 256
}

fn damage_power(formula: DamageFormula, base_damage: i32, offensive_stat: i32) -> i32 {
    match formula {
        DamageFormula::Strength | DamageFormula::PiercingStrength | DamageFormula::SpecialMagic => {
            offensive_stat * offensive_stat * offensive_stat / 0x20 + 0x1e
        }
        DamageFormula::Healing => (offensive_stat + base_damage) / 2 * base_damage,
        DamageFormula::Magic | DamageFormula::PiercingMagic => {
            let power = offensive_stat * offensive_stat;
            let power = (power as i64 * 0x2AAAAAAB / 0xffff_ffff) as i32;
            (power + base_damage) * base_damage / 4
        }
        DamageFormula::PiercingStrengthNoVariance => {
            offensive_stat * offensive_stat * offensive_stat / 0x20 + 0x1e
        }
        DamageFormula::SpecialMagicNoVariance => {
            offensive_stat * offensive_stat * offensive_stat / 0x20 + 0x1e
        }
        _ => 0,
    }
}

fn damage_mitigation(defensive_stat: i32) -> i32 {
    let mitigation_1 = defensive_stat * defensive_stat;
    let mitigation_1 = (mitigation_1 as i64 * 0x2E8BA2E9 / 0xffff_ffff) as i32 / 2;
    let mitigation = defensive_stat * 0x33 - mitigation_1;
    let mitigation = (mitigation as i64 * 0x66666667 / 0xffff_ffff) as i32;
    0x2da - mitigation / 4
}

fn apply_damage_status_modifiers(
    mut damage: i32,
    user: &BattleActor,
    target: &BattleActor,
    action: &ActionData,
) -> i32 {
    damage = apply_elemental_modifiers(damage, user, target, action);
    if target.statuses.contains(&Status::Boost) {
        damage = damage * 3 / 2;
    }
    if target.statuses.contains(&Status::Shield) {
        damage /= 4;
    }
    if action.damage_type == DamageType::Physical {
        if target.statuses.contains(&Status::Protect) {
            damage /= 2;
        }
        if user.statuses.contains(&Status::Berserk) {
            damage = damage * 3 / 2;
        }
        if user.statuses.contains(&Status::PowerBreak) {
            damage /= 2;
        }
        if target.statuses.contains(&Status::Defend) {
            damage /= 2;
        }
    }
    if action.damage_type == DamageType::Magical {
        if target.statuses.contains(&Status::Shell) {
            damage /= 2;
        }
        if user.statuses.contains(&Status::MagicBreak) {
            damage /= 2;
        }
    }
    if target.armored
        && action.damage_type == DamageType::Physical
        && !action.ignores_armored
        && !user.has_auto_ability(AutoAbility::Piercing)
        && !target.statuses.contains(&Status::ArmorBreak)
    {
        damage /= 3;
    }
    damage.min(9999)
}

fn apply_elemental_modifiers(
    mut damage: i32,
    user: &BattleActor,
    target: &BattleActor,
    action: &ActionData,
) -> i32 {
    let mut elements = action.elements.iter().copied().collect::<Vec<_>>();
    if action.uses_weapon_properties {
        for element in &user.weapon_elements {
            if !elements.contains(element) {
                elements.push(*element);
            }
        }
    }
    if elements.is_empty() {
        return damage;
    }
    let mut strongest = ElementalAffinity::Absorbs;
    let mut extra_weaknesses = 0;
    for element in elements {
        let affinity = target
            .elemental_affinities
            .get(&element)
            .copied()
            .unwrap_or(ElementalAffinity::Neutral);
        if strongest == ElementalAffinity::Weak && affinity == ElementalAffinity::Weak {
            extra_weaknesses += 1;
        } else if affinity.modifier_value() > strongest.modifier_value() {
            strongest = affinity;
        }
    }
    damage = apply_elemental_affinity_modifier(damage, strongest);
    for _ in 0..extra_weaknesses {
        damage = apply_elemental_affinity_modifier(damage, ElementalAffinity::Weak);
    }
    damage
}

fn apply_elemental_affinity_modifier(damage: i32, affinity: ElementalAffinity) -> i32 {
    match affinity {
        ElementalAffinity::Absorbs => -damage,
        ElementalAffinity::Immune => 0,
        ElementalAffinity::Resists => damage / 2,
        ElementalAffinity::Weak => damage * 3 / 2,
        ElementalAffinity::Neutral => damage,
    }
}

fn buff_stacks(actor: &BattleActor, buff: Buff) -> i32 {
    actor.buffs.get(&buff).copied().unwrap_or_default()
}

fn python_div_i64(numerator: i64, denominator: i64) -> i64 {
    if numerator >= 0 {
        numerator / denominator
    } else {
        -((-numerator + denominator - 1) / denominator)
    }
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

fn parse_equipment_abilities(args: &[String]) -> HashSet<AutoAbility> {
    args.iter()
        .skip(2)
        .filter_map(|ability| ability.parse::<AutoAbility>().ok())
        .take(4)
        .collect()
}

fn apply_auto_statuses(actor: &mut BattleActor) {
    for (ability, status) in [
        (AutoAbility::AutoShell, Status::Shell),
        (AutoAbility::AutoProtect, Status::Protect),
        (AutoAbility::AutoHaste, Status::Haste),
        (AutoAbility::AutoRegen, Status::Regen),
        (AutoAbility::AutoReflect, Status::Reflect),
    ] {
        if actor.has_auto_ability(ability) {
            actor.statuses.insert(status);
        }
    }
}

fn apply_equipment_elements(actor: &mut BattleActor) {
    let weapon_elements = actor
        .weapon_abilities
        .iter()
        .filter_map(|ability| weapon_element(*ability))
        .collect::<HashSet<_>>();
    actor.set_weapon_elements(weapon_elements);

    for element in [
        Element::Fire,
        Element::Ice,
        Element::Thunder,
        Element::Water,
    ] {
        actor.set_elemental_affinity(element, ElementalAffinity::Neutral);
    }
    for (ability, element, affinity) in [
        (
            AutoAbility::FireWard,
            Element::Fire,
            ElementalAffinity::Resists,
        ),
        (
            AutoAbility::IceWard,
            Element::Ice,
            ElementalAffinity::Resists,
        ),
        (
            AutoAbility::LightningWard,
            Element::Thunder,
            ElementalAffinity::Resists,
        ),
        (
            AutoAbility::WaterWard,
            Element::Water,
            ElementalAffinity::Resists,
        ),
        (
            AutoAbility::Fireproof,
            Element::Fire,
            ElementalAffinity::Immune,
        ),
        (
            AutoAbility::Iceproof,
            Element::Ice,
            ElementalAffinity::Immune,
        ),
        (
            AutoAbility::Lightningproof,
            Element::Thunder,
            ElementalAffinity::Immune,
        ),
        (
            AutoAbility::Waterproof,
            Element::Water,
            ElementalAffinity::Immune,
        ),
        (
            AutoAbility::FireEater,
            Element::Fire,
            ElementalAffinity::Absorbs,
        ),
        (
            AutoAbility::IceEater,
            Element::Ice,
            ElementalAffinity::Absorbs,
        ),
        (
            AutoAbility::LightningEater,
            Element::Thunder,
            ElementalAffinity::Absorbs,
        ),
        (
            AutoAbility::WaterEater,
            Element::Water,
            ElementalAffinity::Absorbs,
        ),
    ] {
        if actor.armor_abilities.contains(&ability) {
            actor.set_elemental_affinity(element, affinity);
        }
    }
}

fn weapon_element(ability: AutoAbility) -> Option<Element> {
    match ability {
        AutoAbility::Firestrike => Some(Element::Fire),
        AutoAbility::Icestrike => Some(Element::Ice),
        AutoAbility::Lightningstrike => Some(Element::Thunder),
        AutoAbility::Waterstrike => Some(Element::Water),
        _ => None,
    }
}

fn weapon_status(ability: AutoAbility) -> Option<Status> {
    match ability {
        AutoAbility::Deathtouch | AutoAbility::Deathstrike => Some(Status::Death),
        AutoAbility::Zombietouch | AutoAbility::Zombiestrike => Some(Status::Zombie),
        AutoAbility::Stonetouch | AutoAbility::Stonestrike => Some(Status::Petrify),
        AutoAbility::Poisontouch | AutoAbility::Poisonstrike => Some(Status::Poison),
        AutoAbility::Sleeptouch | AutoAbility::Sleepstrike => Some(Status::Sleep),
        AutoAbility::Silencetouch | AutoAbility::Silencestrike => Some(Status::Silence),
        AutoAbility::Darktouch | AutoAbility::Darkstrike => Some(Status::Dark),
        AutoAbility::Slowtouch | AutoAbility::Slowstrike => Some(Status::Slow),
        _ => None,
    }
}

fn parse_status(name: &str) -> Option<Status> {
    name.parse().ok()
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
    use super::{monster_template, simulate, SimulationState};
    use crate::battle::{ActorId, BattleActor};
    use crate::model::{Buff, Character, Element, ElementalAffinity, MonsterSlot, Status};

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
            .contains("Equipment: weapon Tidus [4 strength_+5%]"));
        assert!(output.text.contains("Advanced rng4 1 times"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn stat_command_updates_combat_stats_used_by_damage() {
        let mut state = SimulationState::new(1);
        let tidus_strength = state
            .character_actor(Character::Tidus)
            .unwrap()
            .combat_stats
            .strength;

        state.change_stat(&[
            "tidus".to_string(),
            "strength".to_string(),
            "+10".to_string(),
        ]);
        state.change_stat(&["tidus".to_string(), "luck".to_string(), "1".to_string()]);

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.combat_stats.strength, tidus_strength + 10);
        assert_eq!(tidus.combat_stats.luck, 1);
    }

    #[test]
    fn renders_full_default_script_without_parser_gaps() {
        let input = include_str!("../fixtures/ctb_actions_input.txt");
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

    #[test]
    fn applies_status_effects_from_action_data() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("tanker".to_string()),
            10,
            false,
            1_000,
        ));

        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Wakka), "dark_attack");
        state.apply_action_effects(
            ActorId::Character(Character::Wakka),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(state.monsters[0].statuses.contains(&Status::Dark));
    }

    #[test]
    fn status_immunity_blocks_action_statuses() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("tanker".to_string()),
            10,
            false,
            1_000,
        );
        monster.status_resistances.insert(Status::Dark, 255);
        state.monsters.push(monster);

        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Wakka), "dark_attack");
        state.apply_action_effects(
            ActorId::Character(Character::Wakka),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(!state.monsters[0].statuses.contains(&Status::Dark));
    }

    #[test]
    fn applies_and_clamps_buffs_from_action_data() {
        let mut state = SimulationState::new(1);

        for _ in 0..8 {
            let action_data =
                state.action_data_for_actor(ActorId::Character(Character::Tidus), "cheer");
            state.apply_action_effects(
                ActorId::Character(Character::Tidus),
                action_data.as_ref(),
                &[String::from("tidus")],
            );
        }

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.buffs.get(&Buff::Cheer), Some(&5));
    }

    #[test]
    fn respects_delay_immunity_from_monster_data() {
        let mut state = SimulationState::new(1);
        let template = monster_template("geosgaeno");
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some(template.key),
            template.agility,
            template.immune_to_delay,
            template.max_hp,
        );
        monster.ctb = 12;
        state.monsters.push(monster);

        state.apply_delay(ActorId::Monster(MonsterSlot(1)), 3, 2);

        assert_eq!(state.monsters[0].ctb, 12);
    }

    #[test]
    fn uses_monster_action_target_overrides() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Wakka];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("sinspawn_echuilles".to_string()),
            20,
            false,
            2_000,
        ));

        let action_data = state
            .action_data_for_actor(ActorId::Monster(MonsterSlot(1)), "blender")
            .unwrap();
        let targets =
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &action_data, &[]);

        assert_eq!(
            targets,
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Wakka),
            ]
        );
    }

    #[test]
    fn resolves_party_and_monsters_action_target_aliases() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Wakka];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Rikku), "grenade");

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Rikku),
                action_data.as_ref().unwrap(),
                &[String::from("monsters")]
            ),
            vec![
                ActorId::Monster(MonsterSlot(1)),
                ActorId::Monster(MonsterSlot(2))
            ]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Yuna),
                action_data.as_ref().unwrap(),
                &[String::from("party")]
            ),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Wakka)
            ]
        );
    }

    #[test]
    fn equipment_first_strike_skips_initial_character_ctb() {
        let lines = vec![
            "weapon tidus 1 first_strike".to_string(),
            "encounter tanker".to_string(),
        ];
        let output = simulate(1, &lines);

        assert!(output
            .text
            .contains("Equipment: weapon Tidus [1 first_strike]"));
        assert!(output.text.contains("Ti[0]"));
    }

    #[test]
    fn weapon_status_abilities_apply_to_weapon_actions() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "slowtouch".to_string(),
            ],
        );
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(state.monsters[0].statuses.contains(&Status::Slow));
    }

    #[test]
    fn action_damage_reduces_hp_from_action_data() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(state.monsters[0].current_hp < 1_000);
        assert!(!state.monsters[0].statuses.contains(&Status::Death));
    }

    #[test]
    fn healing_actions_restore_partial_hp_without_full_reset() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 100;
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Wakka), "potion");

        state.apply_action_effects(
            ActorId::Character(Character::Wakka),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert!(tidus.current_hp > 100);
        assert!(tidus.current_hp < tidus.max_hp);
    }

    #[test]
    fn drain_actions_restore_user_hp_by_damage_dealt() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.current_hp = 100;
        state.monsters.push(monster);
        let tidus_hp = state.character_actor(Character::Tidus).unwrap().current_hp;
        let action_data =
            state.action_data_for_actor(ActorId::Monster(MonsterSlot(1)), "drain_touch");

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        assert!(state.monsters[0].current_hp > 100);
        assert!(state.character_actor(Character::Tidus).unwrap().current_hp < tidus_hp);
    }

    #[test]
    fn element_command_updates_actor_affinity() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));

        state.change_element(&["m1".to_string(), "thunder".to_string(), "weak".to_string()]);

        assert_eq!(
            state.monsters[0]
                .elemental_affinities
                .get(&Element::Thunder),
            Some(&ElementalAffinity::Weak)
        );
    }

    #[test]
    fn weapon_elements_apply_damage_affinities() {
        let mut neutral_state = SimulationState::new(1);
        neutral_state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "lightningstrike".to_string(),
            ],
        );
        neutral_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            neutral_state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");
        neutral_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );
        let neutral_hp = neutral_state.monsters[0].current_hp;

        let mut weak_state = SimulationState::new(1);
        weak_state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "lightningstrike".to_string(),
            ],
        );
        weak_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        weak_state.change_element(&["m1".to_string(), "thunder".to_string(), "weak".to_string()]);
        let action_data =
            weak_state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");
        weak_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(weak_state.monsters[0].current_hp < neutral_hp);
    }

    #[test]
    fn armored_targets_reduce_non_piercing_physical_damage() {
        let mut normal_state = SimulationState::new(1);
        normal_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            normal_state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");
        normal_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );
        let normal_hp = normal_state.monsters[0].current_hp;

        let mut armored_state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.armored = true;
        armored_state.monsters.push(monster);
        let action_data =
            armored_state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");
        armored_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert!(armored_state.monsters[0].current_hp > normal_hp);
    }

    #[test]
    fn damage_immunity_prevents_action_damage() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.immune_to_damage = true;
        state.monsters.push(monster);
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Tidus), "attack");

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("m1")],
        );

        assert_eq!(state.monsters[0].current_hp, 1_000);
    }

    #[test]
    fn life_effects_damage_zombie_targets_unless_immune() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.statuses.insert(Status::Death);
        monster.statuses.insert(Status::Zombie);
        state.monsters.push(monster);

        state.remove_status_from_actor(ActorId::Monster(MonsterSlot(1)), Status::Death);

        assert_eq!(state.monsters[0].current_hp, 0);
        assert!(state.monsters[0].statuses.contains(&Status::Death));

        let mut immune_state = SimulationState::new(1);
        let mut immune_monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        immune_monster.immune_to_life = true;
        immune_monster.statuses.insert(Status::Death);
        immune_monster.statuses.insert(Status::Zombie);
        immune_state.monsters.push(immune_monster);

        immune_state.remove_status_from_actor(ActorId::Monster(MonsterSlot(1)), Status::Death);

        assert_eq!(immune_state.monsters[0].current_hp, 1_000);
        assert!(immune_state.monsters[0].statuses.contains(&Status::Death));
        assert!(immune_state.monsters[0].statuses.contains(&Status::Zombie));
    }

    #[test]
    fn life_actions_miss_living_non_zombie_targets() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 100;
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Lulu), "phoenix_down");

        state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_hp,
            100
        );
    }

    #[test]
    fn life_actions_can_kill_living_zombie_targets() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.statuses.insert(Status::Zombie);
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Lulu), "phoenix_down");

        state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, 0);
        assert!(tidus.statuses.contains(&Status::Death));
    }
}

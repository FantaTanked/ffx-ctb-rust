use std::collections::{HashMap, HashSet};

use crate::model::{AutoAbility, Buff, Character, Element, ElementalAffinity, MonsterSlot, Status};

const ICV_BASE: [u16; 256] = [
    28, 28, 26, 24, 20, 16, 16, 15, 15, 15, 14, 14, 13, 13, 13, 12, 12, 11, 11, 10, 10, 10, 10, 9,
    9, 9, 9, 9, 9, 8, 8, 8, 8, 8, 8, 7, 7, 7, 7, 7, 7, 7, 7, 7, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6,
    6, 6, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5,
    5, 5, 5, 5, 5, 5, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3,
    3, 3, 3, 3, 3, 3, 3, 3,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ActorId {
    Character(Character),
    Monster(MonsterSlot),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CombatStats {
    pub strength: i32,
    pub defense: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub luck: i32,
    pub evasion: i32,
    pub accuracy: i32,
    pub base_weapon_damage: i32,
}

impl Default for CombatStats {
    fn default() -> Self {
        Self {
            strength: 10,
            defense: 10,
            magic: 10,
            magic_defense: 10,
            luck: 1,
            evasion: 0,
            accuracy: 10,
            base_weapon_damage: 16,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BattleActor {
    pub id: ActorId,
    pub monster_key: Option<String>,
    pub display_slot: Option<MonsterSlot>,
    pub temporary: bool,
    pub index: usize,
    pub agility: u8,
    pub immune_to_delay: bool,
    pub armored: bool,
    pub immune_to_damage: bool,
    pub immune_to_percentage_damage: bool,
    pub immune_to_physical_damage: bool,
    pub immune_to_magical_damage: bool,
    pub immune_to_life: bool,
    pub immune_to_bribe: bool,
    pub combat_stats: CombatStats,
    pub current_hp: i32,
    pub max_hp: i32,
    pub hp_multiplier: i32,
    pub break_hp_limit: bool,
    pub current_mp: i32,
    pub max_mp: i32,
    pub mp_multiplier: i32,
    pub break_mp_limit: bool,
    pub break_damage_limit: bool,
    pub equipment_crit: i32,
    pub weapon_bonus_crit: i32,
    pub armor_bonus_crit: i32,
    pub bribe_gil_spent: i32,
    pub ctb: i32,
    pub buffs: HashMap<Buff, i32>,
    pub weapon_slots: u8,
    pub armor_slots: u8,
    pub weapon_abilities: HashSet<AutoAbility>,
    pub armor_abilities: HashSet<AutoAbility>,
    pub weapon_elements: HashSet<Element>,
    pub elemental_affinities: HashMap<Element, ElementalAffinity>,
    pub status_resistances: HashMap<Status, u8>,
    pub statuses: HashSet<Status>,
    pub status_order: Vec<Status>,
    pub status_stacks: HashMap<Status, i32>,
}

impl BattleActor {
    pub fn character(
        character: Character,
        index: usize,
        agility: u8,
        max_hp: i32,
        max_mp: i32,
    ) -> Self {
        let mut actor = Self::new(
            ActorId::Character(character),
            None,
            index,
            agility,
            false,
            max_hp,
            max_mp,
        );
        actor.status_resistances.insert(Status::Threaten, 255);
        actor
    }

    pub fn monster(slot: MonsterSlot, agility: u8, max_hp: i32) -> Self {
        Self::monster_with_key(slot, None, agility, false, max_hp)
    }

    pub fn monster_with_key(
        slot: MonsterSlot,
        monster_key: Option<String>,
        agility: u8,
        immune_to_delay: bool,
        max_hp: i32,
    ) -> Self {
        Self::new(
            ActorId::Monster(slot),
            monster_key,
            slot.0 - 1,
            agility,
            immune_to_delay,
            max_hp,
            0,
        )
    }

    fn new(
        id: ActorId,
        monster_key: Option<String>,
        index: usize,
        agility: u8,
        immune_to_delay: bool,
        max_hp: i32,
        max_mp: i32,
    ) -> Self {
        Self {
            id,
            monster_key,
            display_slot: None,
            temporary: false,
            index,
            agility,
            immune_to_delay,
            armored: false,
            immune_to_damage: false,
            immune_to_percentage_damage: false,
            immune_to_physical_damage: false,
            immune_to_magical_damage: false,
            immune_to_life: false,
            immune_to_bribe: false,
            combat_stats: CombatStats::default(),
            current_hp: max_hp,
            max_hp,
            hp_multiplier: 100,
            break_hp_limit: false,
            current_mp: max_mp,
            max_mp,
            mp_multiplier: 100,
            break_mp_limit: false,
            break_damage_limit: false,
            equipment_crit: 0,
            weapon_bonus_crit: 0,
            armor_bonus_crit: 0,
            bribe_gil_spent: 0,
            ctb: 0,
            buffs: HashMap::new(),
            weapon_slots: 0,
            armor_slots: 0,
            weapon_abilities: HashSet::new(),
            armor_abilities: HashSet::new(),
            weapon_elements: HashSet::new(),
            elemental_affinities: neutral_elemental_affinities(),
            status_resistances: HashMap::new(),
            statuses: HashSet::new(),
            status_order: Vec::new(),
            status_stacks: HashMap::new(),
        }
    }

    pub fn base_ctb(&self) -> i32 {
        ICV_BASE[self.agility as usize] as i32
    }

    pub fn effective_max_hp(&self) -> i32 {
        let multiplier = if self.statuses.contains(&Status::MaxHpX2) {
            2
        } else {
            1
        };
        let max_value = match self.id {
            ActorId::Monster(_) => i32::MAX,
            ActorId::Character(_) if self.break_hp_limit => 99_999,
            ActorId::Character(_) => 9_999,
        };
        (i64::from(self.max_hp) * i64::from(self.hp_multiplier) / 100 * i64::from(multiplier))
            .clamp(0, i64::from(max_value)) as i32
    }

    pub fn effective_max_mp(&self) -> i32 {
        let multiplier = if self.statuses.contains(&Status::MaxMpX2) {
            2
        } else {
            1
        };
        let max_value = match self.id {
            ActorId::Monster(_) => i32::MAX,
            ActorId::Character(_) if self.break_mp_limit => 9_999,
            ActorId::Character(_) => 999,
        };
        (i64::from(self.max_mp) * i64::from(self.mp_multiplier) / 100 * i64::from(multiplier))
            .clamp(0, i64::from(max_value)) as i32
    }

    pub fn is_alive(&self) -> bool {
        self.current_hp > 0
            && !self.statuses.contains(&Status::Death)
            && !self.statuses.contains(&Status::Eject)
            && !self.statuses.contains(&Status::Petrify)
    }

    pub fn can_take_turn(&self) -> bool {
        self.is_alive() && !self.statuses.contains(&Status::Sleep)
    }

    pub fn turn_ctb(&self, rank: i32) -> i32 {
        let mut ctb = self.base_ctb() * rank;
        if self.statuses.contains(&Status::Haste) {
            ctb /= 2;
        } else if self.statuses.contains(&Status::Slow) {
            ctb *= 2;
        }
        ctb
    }

    pub fn add_buff(&mut self, buff: Buff, amount: i32) {
        let value = self.buffs.get(&buff).copied().unwrap_or_default() + amount;
        self.buffs.insert(buff, value.clamp(0, 5));
    }

    pub fn set_combat_stats(&mut self, stats: CombatStats) {
        self.combat_stats = stats;
    }

    pub fn set_weapon_abilities(&mut self, abilities: HashSet<AutoAbility>) {
        self.weapon_abilities = abilities;
    }

    pub fn set_armor_abilities(&mut self, abilities: HashSet<AutoAbility>) {
        self.armor_abilities = abilities;
    }

    pub fn set_weapon_slots(&mut self, slots: u8) {
        self.weapon_slots = slots;
    }

    pub fn set_armor_slots(&mut self, slots: u8) {
        self.armor_slots = slots;
    }

    pub fn set_weapon_elements(&mut self, elements: HashSet<Element>) {
        self.weapon_elements = elements;
    }

    pub fn set_elemental_affinities(&mut self, affinities: HashMap<Element, ElementalAffinity>) {
        self.elemental_affinities = affinities;
    }

    pub fn set_elemental_affinity(&mut self, element: Element, affinity: ElementalAffinity) {
        self.elemental_affinities.insert(element, affinity);
    }

    pub fn set_status_resistances(&mut self, resistances: HashMap<Status, u8>) {
        self.status_resistances = resistances;
    }

    pub fn set_status(&mut self, status: Status, stacks: i32) {
        if stacks <= 0 {
            self.remove_status(status);
            return;
        }
        if self.statuses.insert(status) {
            self.status_order.push(status);
        }
        self.status_stacks.insert(status, stacks);
    }

    pub fn remove_status(&mut self, status: Status) -> bool {
        self.status_stacks.remove(&status);
        self.status_order.retain(|ordered| *ordered != status);
        self.statuses.remove(&status)
    }

    pub fn clear_statuses(&mut self) {
        self.statuses.clear();
        self.status_order.clear();
        self.status_stacks.clear();
    }

    pub fn status_stack(&self, status: Status) -> i32 {
        self.status_stacks.get(&status).copied().unwrap_or(254)
    }

    pub fn has_auto_ability(&self, ability: AutoAbility) -> bool {
        self.weapon_abilities.contains(&ability) || self.armor_abilities.contains(&ability)
    }
}

pub fn neutral_elemental_affinities() -> HashMap<Element, ElementalAffinity> {
    Element::VALUES
        .into_iter()
        .map(|element| (element, ElementalAffinity::Neutral))
        .collect()
}

#[derive(Debug, Clone)]
pub struct BattleState {
    pub party: Vec<BattleActor>,
    pub monsters: Vec<BattleActor>,
    pub ctb_since_last_action: i32,
    pub last_actor: Option<ActorId>,
}

impl BattleState {
    pub fn new(party: Vec<BattleActor>, monsters: Vec<BattleActor>) -> Self {
        Self {
            party,
            monsters,
            ctb_since_last_action: 0,
            last_actor: None,
        }
    }

    pub fn next_actor(&self) -> Option<ActorId> {
        self.living_actor_refs()
            .into_iter()
            .min_by_key(|actor| actor_sort_key(actor))
            .map(|actor| actor.id)
    }

    pub fn apply_virtual_turn(&mut self, actor_id: ActorId, rank: i32) -> Option<i32> {
        let actor = self.actor_mut(actor_id)?;
        let ctb = actor.turn_ctb(rank);
        actor.ctb = (actor.ctb + ctb).max(0);
        self.last_actor = Some(actor_id);
        self.normalize_after_turn();
        Some(ctb)
    }

    pub fn actor(&self, actor_id: ActorId) -> Option<&BattleActor> {
        self.party
            .iter()
            .chain(self.monsters.iter())
            .find(|actor| actor.id == actor_id)
    }

    pub fn actor_mut(&mut self, actor_id: ActorId) -> Option<&mut BattleActor> {
        self.party
            .iter_mut()
            .chain(self.monsters.iter_mut())
            .find(|actor| actor.id == actor_id)
    }

    pub fn ctb_order_string(&self) -> String {
        let mut actors = self
            .party
            .iter()
            .chain(self.monsters.iter())
            .collect::<Vec<_>>();
        actors.sort_by_key(|actor| actor_sort_key(actor));
        actors
            .into_iter()
            .map(|actor| match actor.id {
                ActorId::Character(character) => {
                    let name = character.display_name();
                    format!("{:2}[{}]", &name[..2.min(name.len())], actor.ctb)
                }
                ActorId::Monster(slot) => format!("M{}[{}]", slot.0, actor.ctb),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn living_actor_refs(&self) -> Vec<&BattleActor> {
        self.party
            .iter()
            .chain(self.monsters.iter())
            .filter(|actor| actor.can_take_turn())
            .collect()
    }

    fn normalize_after_turn(&mut self) {
        let min_ctb = self
            .party
            .iter()
            .chain(self.monsters.iter())
            .filter(|actor| actor.is_alive())
            .map(|actor| actor.ctb)
            .min()
            .unwrap_or(0);
        self.ctb_since_last_action = min_ctb;
        if min_ctb == 0 {
            return;
        }
        for actor in self.party.iter_mut().chain(self.monsters.iter_mut()) {
            if actor.statuses.contains(&Status::Petrify) {
                continue;
            }
            actor.ctb = (actor.ctb - min_ctb).max(0);
        }
    }
}

fn actor_sort_key(actor: &BattleActor) -> (i32, u8, usize, i32) {
    match actor.id {
        ActorId::Character(_) => (
            actor.ctb,
            0,
            256usize.saturating_sub(actor.agility as usize),
            actor.index as i32,
        ),
        ActorId::Monster(_) => (actor.ctb, 1, actor.index, 256 - actor.agility as i32),
    }
}

#[cfg(test)]
mod tests {
    use super::{ActorId, BattleActor, BattleState};
    use crate::model::{Character, MonsterSlot, Status};

    #[test]
    fn sorts_next_actor_like_python_ctb_sort_key() {
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520, 12);
        let mut auron = BattleActor::character(Character::Auron, 2, 5, 1030, 33);
        let mut m1 = BattleActor::monster(MonsterSlot(1), 30, 1000);
        tidus.ctb = 5;
        auron.ctb = 5;
        m1.ctb = 5;

        let state = BattleState::new(vec![tidus, auron], vec![m1]);
        assert_eq!(
            state.next_actor(),
            Some(ActorId::Character(Character::Tidus))
        );
        assert_eq!(state.ctb_order_string(), "Ti[5] Au[5] M1[5]");
    }

    #[test]
    fn skips_unavailable_actors_for_next_actor() {
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520, 12);
        let mut m1 = BattleActor::monster(MonsterSlot(1), 30, 1000);
        tidus.statuses.insert(Status::Sleep);
        m1.ctb = 2;
        let state = BattleState::new(vec![tidus], vec![m1]);
        assert_eq!(state.next_actor(), Some(ActorId::Monster(MonsterSlot(1))));
    }

    #[test]
    fn applies_virtual_turn_with_haste_and_normalizes_ctb() {
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520, 12);
        let mut auron = BattleActor::character(Character::Auron, 2, 5, 1030, 33);
        tidus.statuses.insert(Status::Haste);
        tidus.ctb = 0;
        auron.ctb = 7;
        let mut state = BattleState::new(vec![tidus, auron], Vec::new());

        let spent = state.apply_virtual_turn(ActorId::Character(Character::Tidus), 3);
        assert_eq!(spent, Some(15));
        assert_eq!(state.ctb_since_last_action, 7);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            8
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Auron))
                .unwrap()
                .ctb,
            0
        );
    }

    #[test]
    fn slow_doubles_turn_ctb() {
        let mut m1 = BattleActor::monster(MonsterSlot(1), 20, 1000);
        m1.statuses.insert(Status::Slow);
        assert_eq!(m1.turn_ctb(3), 60);
    }
}

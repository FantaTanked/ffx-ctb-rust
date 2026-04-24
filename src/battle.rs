use std::collections::HashSet;

use crate::model::{Character, MonsterSlot, Status};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorId {
    Character(Character),
    Monster(MonsterSlot),
}

#[derive(Debug, Clone)]
pub struct BattleActor {
    pub id: ActorId,
    pub index: usize,
    pub agility: u8,
    pub current_hp: i32,
    pub max_hp: i32,
    pub ctb: i32,
    pub statuses: HashSet<Status>,
}

impl BattleActor {
    pub fn character(character: Character, index: usize, agility: u8, max_hp: i32) -> Self {
        Self::new(ActorId::Character(character), index, agility, max_hp)
    }

    pub fn monster(slot: MonsterSlot, agility: u8, max_hp: i32) -> Self {
        Self::new(ActorId::Monster(slot), slot.0 - 1, agility, max_hp)
    }

    fn new(id: ActorId, index: usize, agility: u8, max_hp: i32) -> Self {
        Self {
            id,
            index,
            agility,
            current_hp: max_hp,
            max_hp,
            ctb: 0,
            statuses: HashSet::new(),
        }
    }

    pub fn base_ctb(&self) -> i32 {
        ICV_BASE[self.agility as usize] as i32
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
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520);
        let mut auron = BattleActor::character(Character::Auron, 2, 5, 1030);
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
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520);
        let mut m1 = BattleActor::monster(MonsterSlot(1), 30, 1000);
        tidus.statuses.insert(Status::Sleep);
        m1.ctb = 2;
        let state = BattleState::new(vec![tidus], vec![m1]);
        assert_eq!(state.next_actor(), Some(ActorId::Monster(MonsterSlot(1))));
    }

    #[test]
    fn applies_virtual_turn_with_haste_and_normalizes_ctb() {
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520);
        let mut auron = BattleActor::character(Character::Auron, 2, 5, 1030);
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

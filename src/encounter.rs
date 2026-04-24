use crate::battle::{ActorId, BattleActor};
use crate::model::{Character, EncounterCondition, Status};

pub const ICV_VARIANCE: [u16; 256] = [
    0, 1, 1, 1, 1, 1, 2, 1, 2, 3, 1, 2, 1, 2, 3, 1, 2, 1, 2, 1, 2, 3, 4, 1, 2, 3, 4, 5, 6, 1, 2, 3,
    4, 5, 6, 1, 2, 3, 4, 5, 6, 7, 8, 9, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 1, 1,
    1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 6, 6, 6, 6, 7, 7, 7, 7, 8, 8, 8, 8, 9, 9,
    9, 9, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4,
    4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8,
    8, 8, 9, 9, 9, 9, 9, 9, 9, 9, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2,
    2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 3, 4, 4, 4, 4, 4, 4,
    4, 4, 4, 4, 4, 4, 4, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6,
];

pub fn encounter_condition_from_roll(
    condition_roll: u32,
    has_initiative: bool,
    forced_condition: Option<EncounterCondition>,
) -> EncounterCondition {
    if let Some(condition) = forced_condition {
        return condition;
    }
    let mut roll = (condition_roll & 255) as i32;
    if has_initiative {
        roll -= 33;
    }
    if roll < 32 {
        EncounterCondition::Preemptive
    } else if roll < 255 - 32 {
        EncounterCondition::Normal
    } else {
        EncounterCondition::Ambush
    }
}

pub fn character_initial_ctb(
    actor: &BattleActor,
    condition: EncounterCondition,
    variance_roll: Option<u32>,
    is_active_party_member: bool,
    has_first_strike: bool,
) -> Option<i32> {
    match condition {
        EncounterCondition::Preemptive => None,
        EncounterCondition::Ambush => {
            if is_magis_sister(actor) {
                Some(actor.base_ctb() * 3)
            } else if !is_active_party_member || has_first_strike {
                None
            } else {
                Some(apply_haste_to_raw_ctb(actor, actor.base_ctb() * 3))
            }
        }
        EncounterCondition::Normal => {
            if !is_magis_sister(actor) && (!is_active_party_member || has_first_strike) {
                return None;
            }
            let variance = ICV_VARIANCE[actor.agility as usize] as u32 + 1;
            let roll =
                variance_roll.expect("normal encounter character ICV requires variance roll");
            let raw_ctb = actor.base_ctb() * 3 - (roll % variance) as i32;
            Some(apply_haste_to_raw_ctb(actor, raw_ctb))
        }
    }
}

pub fn monster_initial_ctb(
    actor: &BattleActor,
    condition: EncounterCondition,
    variance_roll: Option<u32>,
) -> Option<i32> {
    match condition {
        EncounterCondition::Preemptive => Some(actor.base_ctb() * 3),
        EncounterCondition::Ambush => None,
        EncounterCondition::Normal => {
            let roll = variance_roll.expect("normal encounter monster ICV requires variance roll");
            let variance = 100 - (roll % 11) as i32;
            Some((actor.base_ctb() * 3 * 100) / variance)
        }
    }
}

fn apply_haste_to_raw_ctb(actor: &BattleActor, raw_ctb: i32) -> i32 {
    if actor.statuses.contains(&Status::Haste) {
        raw_ctb / 2
    } else {
        raw_ctb
    }
}

fn is_magis_sister(actor: &BattleActor) -> bool {
    matches!(
        actor.id,
        ActorId::Character(Character::Sandy | Character::Mindy)
    )
}

#[cfg(test)]
mod tests {
    use super::{character_initial_ctb, encounter_condition_from_roll, monster_initial_ctb};
    use crate::battle::BattleActor;
    use crate::model::{Character, EncounterCondition, MonsterSlot, Status};

    #[test]
    fn derives_condition_like_python() {
        assert_eq!(
            encounter_condition_from_roll(31, false, None),
            EncounterCondition::Preemptive
        );
        assert_eq!(
            encounter_condition_from_roll(32, false, None),
            EncounterCondition::Normal
        );
        assert_eq!(
            encounter_condition_from_roll(223, false, None),
            EncounterCondition::Ambush
        );
        assert_eq!(
            encounter_condition_from_roll(64, true, None),
            EncounterCondition::Preemptive
        );
        assert_eq!(
            encounter_condition_from_roll(255, false, Some(EncounterCondition::Normal)),
            EncounterCondition::Normal
        );
    }

    #[test]
    fn initializes_character_ctb_for_normal_encounter() {
        let tidus = BattleActor::character(Character::Tidus, 0, 20, 520);
        assert_eq!(
            character_initial_ctb(&tidus, EncounterCondition::Normal, Some(123), true, false),
            Some(30)
        );
        assert_eq!(
            character_initial_ctb(&tidus, EncounterCondition::Normal, Some(123), true, true),
            None
        );
    }

    #[test]
    fn initializes_character_ctb_for_ambush_with_haste() {
        let mut tidus = BattleActor::character(Character::Tidus, 0, 20, 520);
        tidus.statuses.insert(Status::Haste);
        assert_eq!(
            character_initial_ctb(&tidus, EncounterCondition::Ambush, None, true, false),
            Some(15)
        );
        assert_eq!(
            character_initial_ctb(&tidus, EncounterCondition::Ambush, None, false, false),
            None
        );
    }

    #[test]
    fn initializes_magis_sisters_like_python() {
        let sandy = BattleActor::character(Character::Sandy, 16, 20, 3200);
        assert_eq!(
            character_initial_ctb(&sandy, EncounterCondition::Ambush, None, false, true),
            Some(30)
        );
        assert_eq!(
            character_initial_ctb(&sandy, EncounterCondition::Normal, Some(123), false, true),
            Some(30)
        );
    }

    #[test]
    fn initializes_monster_ctb_for_conditions() {
        let monster = BattleActor::monster(MonsterSlot(1), 20, 1000);
        assert_eq!(
            monster_initial_ctb(&monster, EncounterCondition::Preemptive, None),
            Some(30)
        );
        assert_eq!(
            monster_initial_ctb(&monster, EncounterCondition::Ambush, None),
            None
        );
        assert_eq!(
            monster_initial_ctb(&monster, EncounterCondition::Normal, Some(123)),
            Some(30)
        );
    }
}

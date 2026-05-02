use crate::battle::{ActorId, BattleActor, BattleState, CombatStats};
use crate::data::{self, ActionData, ActionTarget, DamageFormula, DamageType, HitChanceFormula};
use crate::encounter::{encounter_condition_from_roll, monster_initial_ctb, ICV_VARIANCE};
use crate::model::{
    AutoAbility, Buff, Character, Element, ElementalAffinity, EncounterCondition, MonsterSlot,
    Status,
};
use crate::parser::{
    parse_edited_action_line, parse_raw_action_line, MonsterActionActor, ParsedCommand,
};
use crate::rng::FfxRngTracker;
use crate::script::{edit_action_line, prepare_action_lines_before_raw_line};
use std::collections::{HashMap, HashSet};

const CHARACTER_VALUES: &str = "tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown";
const HIT_CHANCE_TABLE: [i32; 9] = [25, 30, 30, 40, 40, 50, 60, 80, 100];
const YOJIMBO_ZANMATO_RESISTANCES: [f64; 6] = [0.8, 0.8, 0.8, 0.4, 0.4, 0.4];
const YOJIMBO_COMPATIBILITY_MODIFIER: i32 = 10;
const YOJIMBO_OVERDRIVE_MOTIVATION: i32 = 20;
const YOJIMBO_GIL_MOTIVATION_MODIFIER: i64 = 4;
const MAX_BRIBE_GIL_SPENT: i32 = 999_999_999;
const AEON_BONUS_CHARACTERS: [Character; 8] = [
    Character::Seymour,
    Character::Valefor,
    Character::Ifrit,
    Character::Ixion,
    Character::Shiva,
    Character::Bahamut,
    Character::Anima,
    Character::Yojimbo,
];

#[derive(Debug, Clone, Copy, Default)]
struct AeonStatBlock {
    hp: i32,
    mp: i32,
    strength: i32,
    defense: i32,
    magic: i32,
    magic_defense: i32,
    agility: i32,
    luck: i32,
    evasion: i32,
    accuracy: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActorStat {
    Hp,
    Mp,
    Agility,
    Strength,
    Defense,
    Magic,
    MagicDefense,
    Luck,
    Evasion,
    Accuracy,
}

impl ActorStat {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "hp" => Some(Self::Hp),
            "mp" => Some(Self::Mp),
            "agility" => Some(Self::Agility),
            "strength" => Some(Self::Strength),
            "defense" => Some(Self::Defense),
            "magic" => Some(Self::Magic),
            "magic_defense" | "magic_def" => Some(Self::MagicDefense),
            "luck" => Some(Self::Luck),
            "evasion" => Some(Self::Evasion),
            "accuracy" => Some(Self::Accuracy),
            _ => None,
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            Self::Hp => "HP",
            Self::Mp => "MP",
            Self::Agility => "Agility",
            Self::Strength => "Strength",
            Self::Defense => "Defense",
            Self::Magic => "Magic",
            Self::MagicDefense => "Magic defense",
            Self::Luck => "Luck",
            Self::Evasion => "Evasion",
            Self::Accuracy => "Accuracy",
        }
    }

    fn value(self, actor: &BattleActor) -> i32 {
        match self {
            Self::Hp => actor.max_hp,
            Self::Mp => actor.max_mp,
            Self::Agility => actor.agility as i32,
            Self::Strength => actor.combat_stats.strength,
            Self::Defense => actor.combat_stats.defense,
            Self::Magic => actor.combat_stats.magic,
            Self::MagicDefense => actor.combat_stats.magic_defense,
            Self::Luck => actor.combat_stats.luck,
            Self::Evasion => actor.combat_stats.evasion,
            Self::Accuracy => actor.combat_stats.accuracy,
        }
    }

    fn bonus_value(self, stats: AeonStatBlock) -> i32 {
        match self {
            Self::Hp => stats.hp,
            Self::Mp => stats.mp,
            Self::Agility => stats.agility,
            Self::Strength => stats.strength,
            Self::Defense => stats.defense,
            Self::Magic => stats.magic,
            Self::MagicDefense => stats.magic_defense,
            Self::Luck => stats.luck,
            Self::Evasion => stats.evasion,
            Self::Accuracy => stats.accuracy,
        }
    }

    fn set_bonus(self, stats: &mut AeonStatBlock, value: i32) {
        match self {
            Self::Hp => stats.hp = value,
            Self::Mp => stats.mp = value,
            Self::Agility => stats.agility = value,
            Self::Strength => stats.strength = value,
            Self::Defense => stats.defense = value,
            Self::Magic => stats.magic = value,
            Self::MagicDefense => stats.magic_defense = value,
            Self::Luck => stats.luck = value,
            Self::Evasion => stats.evasion = value,
            Self::Accuracy => stats.accuracy = value,
        }
    }

    fn set_value(self, actor: &mut BattleActor, value: i32) {
        match self {
            Self::Hp => {
                actor.max_hp = if matches!(actor.id, ActorId::Monster(_)) {
                    value.max(0)
                } else {
                    value.clamp(0, 99_999)
                };
                actor.current_hp = actor.current_hp.min(actor.effective_max_hp());
                if actor.current_hp <= 0 {
                    actor.buffs.clear();
                    actor.clear_statuses();
                    actor.set_status(Status::Death, 254);
                }
            }
            Self::Mp => {
                actor.max_mp = if matches!(actor.id, ActorId::Monster(_)) {
                    value.max(0)
                } else {
                    value.clamp(0, 9_999)
                };
                actor.current_mp = actor.current_mp.min(actor.effective_max_mp());
            }
            Self::Agility => actor.agility = value.clamp(0, 255) as u8,
            Self::Strength => actor.combat_stats.strength = value.clamp(0, 255),
            Self::Defense => actor.combat_stats.defense = value.clamp(0, 255),
            Self::Magic => actor.combat_stats.magic = value.clamp(0, 255),
            Self::MagicDefense => actor.combat_stats.magic_defense = value.clamp(0, 255),
            Self::Luck => actor.combat_stats.luck = value.clamp(0, 255),
            Self::Evasion => actor.combat_stats.evasion = value.clamp(0, 255),
            Self::Accuracy => actor.combat_stats.accuracy = value.clamp(0, 255),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AeonStatFormula {
    character: Character,
    hp: (i32, i32, i32),
    mp: (i32, i32, i32),
    strength: (i32, i32, i32),
    defense: (i32, i32, i32),
    magic: (i32, i32, i32),
    magic_defense: (i32, i32, i32),
    agility: (i32, i32, i32),
    evasion: (i32, i32, i32),
    accuracy: (i32, i32, i32),
}

const AEON_STAT_FORMULAS: [AeonStatFormula; 7] = [
    AeonStatFormula {
        character: Character::Valefor,
        hp: (20, 6, 1),
        mp: (4, 1, 5),
        strength: (60, 1, 7),
        defense: (50, 1, 5),
        magic: (100, 1, 70),
        magic_defense: (100, 1, 30),
        agility: (50, 1, 20),
        evasion: (50, 1, 24),
        accuracy: (200, 1, 20),
    },
    AeonStatFormula {
        character: Character::Ifrit,
        hp: (70, 5, 1),
        mp: (3, 1, 5),
        strength: (80, 1, 7),
        defense: (170, 1, 5),
        magic: (90, 1, 33),
        magic_defense: (90, 1, 34),
        agility: (40, 1, 20),
        evasion: (30, 1, 70),
        accuracy: (200, 1, 20),
    },
    AeonStatFormula {
        character: Character::Ixion,
        hp: (55, 6, 1),
        mp: (5, 1, 5),
        strength: (100, 1, 7),
        defense: (100, 1, 5),
        magic: (90, 1, 38),
        magic_defense: (130, 1, 30),
        agility: (30, 1, 20),
        evasion: (30, 1, 47),
        accuracy: (250, 1, 20),
    },
    AeonStatFormula {
        character: Character::Shiva,
        hp: (40, 6, 1),
        mp: (7, 1, 5),
        strength: (120, 1, 8),
        defense: (40, 1, 7),
        magic: (100, 1, 28),
        magic_defense: (100, 1, 25),
        agility: (100, 1, 23),
        evasion: (100, 1, 44),
        accuracy: (200, 1, 20),
    },
    AeonStatFormula {
        character: Character::Bahamut,
        hp: (100, 7, 1),
        mp: (5, 3, 10),
        strength: (160, 1, 7),
        defense: (200, 1, 6),
        magic: (90, 1, 250),
        magic_defense: (100, 1, 12),
        agility: (50, 1, 20),
        evasion: (50, 1, 20),
        accuracy: (200, 1, 20),
    },
    AeonStatFormula {
        character: Character::Anima,
        hp: (120, 8, 1),
        mp: (4, 2, 5),
        strength: (330, 1, 6),
        defense: (100, 1, 5),
        magic: (70, 1, 12),
        magic_defense: (100, 1, 30),
        agility: (40, 1, 20),
        evasion: (50, 1, 20),
        accuracy: (200, 1, 20),
    },
    AeonStatFormula {
        character: Character::Yojimbo,
        hp: (18, 9, 1),
        mp: (0, 0, 10),
        strength: (240, 1, 6),
        defense: (250, 1, 8),
        magic: (60, 1, 23),
        magic_defense: (100, 1, 30),
        agility: (40, 1, 20),
        evasion: (180, 1, 20),
        accuracy: (300, 1, 10),
    },
];

const ENCOUNTER_YUNA_STATS: [AeonStatBlock; 20] = [
    AeonStatBlock {
        hp: 475,
        mp: 84,
        strength: 5,
        defense: 5,
        magic: 20,
        magic_defense: 20,
        agility: 10,
        luck: 17,
        evasion: 30,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 475,
        mp: 104,
        strength: 5,
        defense: 5,
        magic: 23,
        magic_defense: 23,
        agility: 10,
        luck: 17,
        evasion: 32,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 675,
        mp: 104,
        strength: 5,
        defense: 7,
        magic: 26,
        magic_defense: 23,
        agility: 13,
        luck: 17,
        evasion: 32,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 875,
        mp: 104,
        strength: 5,
        defense: 7,
        magic: 26,
        magic_defense: 26,
        agility: 13,
        luck: 17,
        evasion: 32,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 875,
        mp: 124,
        strength: 5,
        defense: 7,
        magic: 29,
        magic_defense: 29,
        agility: 13,
        luck: 17,
        evasion: 32,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1075,
        mp: 144,
        strength: 5,
        defense: 7,
        magic: 29,
        magic_defense: 32,
        agility: 16,
        luck: 17,
        evasion: 36,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1075,
        mp: 144,
        strength: 5,
        defense: 7,
        magic: 32,
        magic_defense: 36,
        agility: 16,
        luck: 17,
        evasion: 36,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1075,
        mp: 164,
        strength: 5,
        defense: 7,
        magic: 36,
        magic_defense: 36,
        agility: 20,
        luck: 17,
        evasion: 36,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1275,
        mp: 184,
        strength: 5,
        defense: 7,
        magic: 40,
        magic_defense: 36,
        agility: 20,
        luck: 17,
        evasion: 40,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1475,
        mp: 184,
        strength: 5,
        defense: 11,
        magic: 40,
        magic_defense: 40,
        agility: 24,
        luck: 17,
        evasion: 40,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1475,
        mp: 204,
        strength: 5,
        defense: 11,
        magic: 44,
        magic_defense: 40,
        agility: 24,
        luck: 17,
        evasion: 44,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1475,
        mp: 204,
        strength: 5,
        defense: 11,
        magic: 44,
        magic_defense: 44,
        agility: 28,
        luck: 17,
        evasion: 44,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1675,
        mp: 224,
        strength: 5,
        defense: 11,
        magic: 48,
        magic_defense: 48,
        agility: 28,
        luck: 17,
        evasion: 44,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1875,
        mp: 224,
        strength: 5,
        defense: 11,
        magic: 48,
        magic_defense: 48,
        agility: 32,
        luck: 17,
        evasion: 48,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1875,
        mp: 244,
        strength: 5,
        defense: 11,
        magic: 52,
        magic_defense: 52,
        agility: 32,
        luck: 17,
        evasion: 48,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 1875,
        mp: 244,
        strength: 5,
        defense: 15,
        magic: 52,
        magic_defense: 52,
        agility: 36,
        luck: 17,
        evasion: 52,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 2075,
        mp: 264,
        strength: 5,
        defense: 15,
        magic: 56,
        magic_defense: 52,
        agility: 36,
        luck: 17,
        evasion: 52,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 2075,
        mp: 264,
        strength: 5,
        defense: 15,
        magic: 56,
        magic_defense: 56,
        agility: 36,
        luck: 17,
        evasion: 56,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 2275,
        mp: 304,
        strength: 5,
        defense: 15,
        magic: 60,
        magic_defense: 56,
        agility: 40,
        luck: 17,
        evasion: 56,
        accuracy: 3,
    },
    AeonStatBlock {
        hp: 2275,
        mp: 304,
        strength: 5,
        defense: 15,
        magic: 60,
        magic_defense: 60,
        agility: 40,
        luck: 17,
        evasion: 60,
        accuracy: 3,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct YojimboActionData {
    key: &'static str,
    name: &'static str,
    compatibility_modifier: i32,
    needed_motivation: Option<i32>,
}

const YOJIMBO_ACTIONS: [YojimboActionData; 8] = [
    YojimboActionData {
        key: "daigoro",
        name: "Daigoro",
        compatibility_modifier: -1,
        needed_motivation: Some(0),
    },
    YojimboActionData {
        key: "kozuka",
        name: "Kozuka",
        compatibility_modifier: 0,
        needed_motivation: Some(32),
    },
    YojimboActionData {
        key: "wakizashi_st",
        name: "Wakizashi ST",
        compatibility_modifier: 1,
        needed_motivation: Some(48),
    },
    YojimboActionData {
        key: "wakizashi_mt",
        name: "Wakizashi MT",
        compatibility_modifier: 3,
        needed_motivation: Some(63),
    },
    YojimboActionData {
        key: "zanmato",
        name: "Zanmato",
        compatibility_modifier: 4,
        needed_motivation: Some(80),
    },
    YojimboActionData {
        key: "dismiss",
        name: "Dismiss",
        compatibility_modifier: 0,
        needed_motivation: None,
    },
    YojimboActionData {
        key: "first_turn_dismiss",
        name: "First turn Dismiss",
        compatibility_modifier: -3,
        needed_motivation: None,
    },
    YojimboActionData {
        key: "autodismiss",
        name: "Autodismiss",
        compatibility_modifier: -20,
        needed_motivation: None,
    },
];

#[derive(Debug, Clone)]
pub struct SimulationOutput {
    pub text: String,
    pub unsupported_count: usize,
}

#[derive(Clone)]
pub struct SimulationRenderCheckpoint {
    pub line_index: usize,
    pub output_index: usize,
    pub state: SimulationState,
    pub encounter_counter: usize,
}

#[derive(Clone)]
pub struct IncrementalSimulationOutput {
    pub output_lines: Vec<String>,
    pub checkpoints: Vec<SimulationRenderCheckpoint>,
    pub unsupported_count: usize,
}

impl IncrementalSimulationOutput {
    pub fn text(&self) -> String {
        self.output_lines.join("\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct EncounterCheck {
    encounter: bool,
    distance: i32,
}

#[derive(Debug, Clone)]
struct ActionDamageResult {
    damage_rng: u32,
    damage: i32,
    pool: &'static str,
    crit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DamagePool {
    Hp,
    Mp,
}

#[derive(Debug, Clone, Default)]
struct ActionDamageResults {
    hp: Option<ActionDamageResult>,
    mp: Option<ActionDamageResult>,
    ctb: Option<ActionDamageResult>,
}

#[derive(Debug, Clone)]
struct ActionEffectResult {
    target: ActorId,
    reflected_from: Option<ActorId>,
    hit: bool,
    damage: ActionDamageResults,
    statuses: Vec<(Status, bool)>,
    removed_statuses: Vec<Status>,
    buffs: Vec<(Buff, i32)>,
    auto_life_triggered: bool,
}

impl ActionEffectResult {
    fn new(target: ActorId) -> Self {
        Self {
            target,
            reflected_from: None,
            hit: true,
            damage: ActionDamageResults::default(),
            statuses: Vec::new(),
            removed_statuses: Vec::new(),
            buffs: Vec::new(),
            auto_life_triggered: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VirtualMonsterAction {
    action: String,
    preview_display: Option<String>,
}

impl VirtualMonsterAction {
    fn display_line(&self, slot: MonsterSlot) -> String {
        self.preview_display
            .clone()
            .unwrap_or_else(|| format!("m{} {}", slot.0, self.action))
    }
}

#[derive(Debug, Clone)]
struct MonsterActionOutcome {
    output: String,
    damage_comment: Option<String>,
    resource_state: Vec<(ActorId, i32, i32)>,
}

impl MonsterActionOutcome {
    fn from_output(output: String) -> Self {
        Self {
            output,
            damage_comment: None,
            resource_state: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct CharacterActionOutcome {
    pre_lines: Vec<String>,
    output: String,
    damage_comment: Option<String>,
    command_echoable: bool,
}

impl CharacterActionOutcome {
    fn from_output(output: String) -> Self {
        Self {
            pre_lines: Vec::new(),
            output,
            damage_comment: None,
            command_echoable: false,
        }
    }
}

#[derive(Debug, Clone)]
struct InventoryEquipment {
    display: String,
    gil_value: i32,
    sell_value: i32,
}

#[derive(Clone)]
pub struct SimulationState {
    rng: FfxRngTracker,
    party: Vec<Character>,
    character_actors: Vec<BattleActor>,
    monsters: Vec<BattleActor>,
    temporary_monsters: Vec<BattleActor>,
    retired_temporary_monsters: Vec<BattleActor>,
    ctb_since_last_action: i32,
    last_actor: Option<ActorId>,
    last_targets: Vec<ActorId>,
    actor_last_targets: HashMap<ActorId, Vec<ActorId>>,
    actor_last_attackers: HashMap<ActorId, ActorId>,
    actor_provokers: HashMap<ActorId, ActorId>,
    encounters_count: i32,
    random_encounters_count: i32,
    zone_encounters_counts: HashMap<String, i32>,
    bonus_aeon_stats: HashMap<Character, AeonStatBlock>,
    live_distance: i32,
    gil: i32,
    item_inventory: Vec<(Option<String>, i32)>,
    equipment_inventory: Vec<Option<InventoryEquipment>>,
    compatibility: i32,
    character_ap: HashMap<Character, i32>,
    magus_last_commands: HashMap<Character, i32>,
    magus_last_actions: HashMap<Character, &'static str>,
    magus_last_action_lists: HashMap<Character, Vec<&'static str>>,
    magus_motivation: HashMap<Character, i32>,
    current_encounter_name: Option<String>,
    current_encounter_condition: Option<EncounterCondition>,
    current_formation_monsters: Vec<String>,
    scripted_turn_index: usize,
    respawn_wave_index: usize,
    sahagin_fourth_unlocked: bool,
    echo_state_edits: bool,
    unsupported_count: usize,
}

impl SimulationState {
    pub fn new(seed: u32) -> Self {
        let mut state = Self {
            rng: FfxRngTracker::new(seed),
            party: vec![Character::Tidus, Character::Auron],
            character_actors: default_character_actors(),
            monsters: Vec::new(),
            temporary_monsters: Vec::new(),
            retired_temporary_monsters: Vec::new(),
            ctb_since_last_action: 0,
            last_actor: Some(ActorId::Character(Character::Tidus)),
            last_targets: Vec::new(),
            actor_last_targets: HashMap::new(),
            actor_last_attackers: HashMap::new(),
            actor_provokers: HashMap::new(),
            encounters_count: 0,
            random_encounters_count: 0,
            zone_encounters_counts: HashMap::new(),
            bonus_aeon_stats: AEON_BONUS_CHARACTERS
                .into_iter()
                .map(|character| (character, AeonStatBlock::default()))
                .collect(),
            live_distance: 0,
            gil: 300,
            item_inventory: {
                let mut inventory = vec![(None, 0); data::item_names_in_order().len()];
                inventory[0] = (Some("Potion".to_string()), 10);
                inventory[1] = (Some("Phoenix Down".to_string()), 3);
                inventory
            },
            equipment_inventory: Vec::new(),
            compatibility: 128,
            character_ap: HashMap::new(),
            magus_last_commands: HashMap::new(),
            magus_last_actions: HashMap::new(),
            magus_last_action_lists: HashMap::new(),
            magus_motivation: HashMap::new(),
            current_encounter_name: None,
            current_encounter_condition: None,
            current_formation_monsters: Vec::new(),
            scripted_turn_index: 0,
            respawn_wave_index: 0,
            sahagin_fourth_unlocked: false,
            echo_state_edits: false,
            unsupported_count: 0,
        };
        state.calculate_aeon_stats();
        state.reset_aeon_current_resources();
        state
    }

    pub fn with_editor_echoes(mut self) -> Self {
        self.echo_state_edits = true;
        self
    }

    pub fn run_lines(&mut self, lines: &[String]) -> SimulationOutput {
        let output = self.run_lines_with_checkpoints(
            lines,
            0,
            Vec::with_capacity(lines.len()),
            Vec::new(),
            0,
            0,
        );
        SimulationOutput {
            text: output.text(),
            unsupported_count: output.unsupported_count,
        }
    }

    pub fn run_lines_with_checkpoints(
        &mut self,
        lines: &[String],
        start_index: usize,
        mut rendered: Vec<String>,
        mut checkpoints: Vec<SimulationRenderCheckpoint>,
        mut encounter_counter: usize,
        checkpoint_stride: usize,
    ) -> IncrementalSimulationOutput {
        let encounter_parties = infer_encounter_parties(lines);
        let encounter_party_swaps = infer_encounter_party_swaps(lines);
        let mut multiline_comment = multiline_comment_state(&lines[..start_index.min(lines.len())]);
        for (line_index, line) in lines.iter().enumerate().skip(start_index) {
            let stripped = line.trim();
            if stripped.starts_with("/*") {
                multiline_comment = true;
            }
            if multiline_comment {
                rendered.push(line.to_string());
                if stripped.ends_with("*/") {
                    multiline_comment = false;
                }
                continue;
            }
            if self.should_skip_tanker_placeholder_comment(line) {
                continue;
            }
            if checkpoint_stride > 0
                && (line_index == 0
                    || (line.to_ascii_lowercase().starts_with("encounter ")
                        && encounter_counter % checkpoint_stride == 0))
            {
                checkpoints.push(SimulationRenderCheckpoint {
                    line_index,
                    output_index: rendered.len(),
                    state: self.clone(),
                    encounter_counter,
                });
            }
            let line_to_execute = self
                .choose_future_targeted_line(lines, line_index)
                .unwrap_or_else(|| line.to_string());
            let mut restore_planned_party_after_encounter = None;
            if matches!(
                parse_raw_action_line(&line_to_execute),
                ParsedCommand::Encounter { .. }
            ) {
                if encounter_party_swaps.contains(&line_index) {
                    // Python preserves the incoming party for encounters with explicit in-block swaps.
                } else if let Some(planned_party) = encounter_parties.get(&line_index) {
                    restore_planned_party_after_encounter = Some(planned_party.clone());
                    self.sync_party_if_needed(planned_party);
                }
            }
            let rendered_line = self.execute_raw_line(&line_to_execute);
            if let Some(planned_party) = restore_planned_party_after_encounter {
                if self.party_needs_sync(&planned_party) {
                    self.party = planned_party;
                }
            }
            if rendered_line.is_empty() {
                ensure_blank_line(&mut rendered);
            } else {
                append_rendered_lines(&mut rendered, &rendered_line);
            }
            if matches!(
                parse_raw_action_line(&line_to_execute),
                ParsedCommand::Encounter { .. }
            ) {
                encounter_counter += 1;
            }
        }
        IncrementalSimulationOutput {
            output_lines: rendered,
            checkpoints,
            unsupported_count: self.unsupported_count,
        }
    }

    pub fn run_until_prepared_line(&mut self, input: &str, cursor_line: usize) {
        let prepared = prepare_action_lines_before_raw_line(input, cursor_line);
        let prepared_cursor_line = prepared.lines.len() + 1;
        self.run_until_lines(prepared.lines.into_iter(), prepared_cursor_line);
    }

    fn run_until_lines<I>(&mut self, lines: I, cursor_line: usize)
    where
        I: IntoIterator<Item = String>,
    {
        let mut multiline_comment = false;
        for raw_line in lines.into_iter().take(cursor_line.saturating_sub(1)) {
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
            if raw_line.trim().is_empty() {
                continue;
            }
            self.execute_raw_line(&raw_line);
        }
    }

    pub fn party(&self) -> &[Character] {
        &self.party
    }

    pub fn sync_party_if_needed(&mut self, planned_party: &[Character]) {
        if planned_party.is_empty() {
            return;
        }
        if !self.party_needs_sync(planned_party) {
            return;
        }
        self.party = planned_party.to_vec();
    }

    fn party_needs_sync(&self, planned_party: &[Character]) -> bool {
        !planned_party.is_empty()
            && (self.party.is_empty()
                || planned_party
                    .iter()
                    .any(|character| !self.party.contains(character)))
    }

    pub fn next_actor(&self) -> Option<ActorId> {
        self.current_battle_state().next_actor()
    }

    pub fn has_monster_party(&self) -> bool {
        !self.monsters.is_empty()
    }

    pub fn has_living_monsters(&self) -> bool {
        self.monsters.iter().any(BattleActor::is_alive)
    }

    pub fn character_hp(&self, character: Character) -> Option<i32> {
        self.character_actor(character)
            .map(|actor| actor.current_hp)
    }

    pub fn character_ctb(&self, character: Character) -> Option<i32> {
        self.character_actor(character).map(|actor| actor.ctb)
    }

    pub fn character_shadow_ctb_fallback(&self, character: Character) -> Option<i32> {
        self.character_actor(character)
            .map(|actor| apply_haste_to_ctb(actor, actor.base_ctb() * 3))
    }

    pub fn set_character_ctb(&mut self, character: Character, ctb: i32) {
        if let Some(actor) = self.actor_mut(ActorId::Character(character)) {
            actor.ctb = ctb.max(0);
        }
    }

    pub fn ctb_since_last_action(&self) -> i32 {
        self.ctb_since_last_action
    }

    pub fn character_max_hp(&self, character: Character) -> Option<i32> {
        self.character_actor(character).map(|actor| actor.max_hp)
    }

    pub fn character_is_alive(&self, character: Character) -> bool {
        self.character_actor(character)
            .is_some_and(BattleActor::is_alive)
    }

    pub fn execute_raw_line(&mut self, line: &str) -> String {
        self.execute_line(line)
    }

    fn choose_future_targeted_line(&self, lines: &[String], line_index: usize) -> Option<String> {
        let chain_indices = self.consecutive_generic_action_indices(lines, line_index);
        if chain_indices.len() <= 1 {
            return None;
        }
        let (_, _, target_map) = self.choose_best_generic_action_chain(lines, &chain_indices, 0)?;
        target_map.get(&line_index).cloned()
    }

    fn consecutive_generic_action_indices(
        &self,
        lines: &[String],
        line_index: usize,
    ) -> Vec<usize> {
        let mut indices = Vec::new();
        for index in line_index..lines.len() {
            if indices.len() >= 2 {
                break;
            }
            let line = &lines[index];
            if !indices.is_empty() && (line.trim().is_empty() || line.trim_start().starts_with('#'))
            {
                break;
            }
            if self.generic_monster_target_name(line).is_none() {
                if index == line_index {
                    return Vec::new();
                }
                break;
            }
            indices.push(index);
        }
        indices
    }

    fn choose_best_generic_action_chain(
        &self,
        lines: &[String],
        chain_indices: &[usize],
        position: usize,
    ) -> Option<(usize, Vec<usize>, HashMap<usize, String>)> {
        if position >= chain_indices.len() {
            return Some((
                self.future_monster_turns_until_character(),
                Vec::new(),
                HashMap::new(),
            ));
        }

        let line_index = chain_indices[position];
        let target_name = self.generic_monster_target_name(&lines[line_index])?;
        let candidates = self.monster_target_candidates(&target_name);
        if candidates.is_empty() {
            return None;
        }

        let mut best: Option<(usize, Vec<usize>, HashMap<usize, String>)> = None;
        for slot in candidates {
            let edited_line = replace_character_action_target(&lines[line_index], slot);
            let mut preview = self.clone();
            let rendered = preview.execute_line(&edited_line);
            let current_turns = rendered_virtual_monster_turn_count(&rendered);
            if let Some((future_turns, mut future_slots, future_map)) =
                preview.choose_best_generic_action_chain(lines, chain_indices, position + 1)
            {
                let mut slot_tuple = vec![slot.0];
                slot_tuple.append(&mut future_slots);
                let mut target_map = future_map;
                target_map.insert(line_index, edited_line);
                let result = (current_turns + future_turns, slot_tuple, target_map);
                if best
                    .as_ref()
                    .is_none_or(|current| (result.0, &result.1) < (current.0, &current.1))
                {
                    best = Some(result);
                }
            }
        }
        best
    }

    fn generic_monster_target_name(&self, line: &str) -> Option<String> {
        let edited_line = edit_action_line(line);
        let ParsedCommand::CharacterAction { action, args, .. } =
            parse_edited_action_line(&edited_line)
        else {
            return None;
        };
        let target_name = args.first()?;
        if target_name.parse::<MonsterSlot>().is_ok()
            || matches!(target_name.as_str(), "party" | "monsters")
        {
            return None;
        }
        let action_data = data::action_data(&action)?;
        if !matches!(
            action_data.target,
            ActionTarget::Single | ActionTarget::SingleMonster | ActionTarget::EitherParty
        ) {
            return None;
        }
        (!self.monster_target_candidates(target_name).is_empty()).then(|| target_name.clone())
    }

    fn monster_target_candidates(&self, target_name: &str) -> Vec<MonsterSlot> {
        let target_family = monster_family(target_name);
        self.monsters
            .iter()
            .filter(|actor| actor.is_alive())
            .filter(|actor| {
                actor
                    .monster_key
                    .as_deref()
                    .is_some_and(|key| key == target_name || monster_family(key) == target_family)
            })
            .filter_map(|actor| match actor.id {
                ActorId::Monster(slot) => Some(slot),
                ActorId::Character(_) => None,
            })
            .collect()
    }

    fn future_monster_turns_until_character(&self) -> usize {
        let mut preview = self.clone();
        let mut turns = 0;
        for _ in 0..100 {
            let Some(actor_id) = preview.current_battle_state().next_actor() else {
                break;
            };
            match actor_id {
                ActorId::Character(_) => break,
                ActorId::Monster(slot) => {
                    if preview.is_manual_only_virtual_monster_turn(slot) {
                        break;
                    }
                    turns += 1;
                    if let Some(virtual_action) = preview.virtual_monster_action(slot) {
                        preview.preview_virtual_monster_action(slot, &virtual_action.action);
                    } else {
                        preview.process_start_of_turn(actor_id);
                        if preview.apply_actor_turn(actor_id, 3).is_some() {
                            preview.process_end_of_turn(actor_id);
                        } else {
                            break;
                        }
                    }
                }
            }
        }
        turns
    }

    fn should_skip_tanker_placeholder_comment(&self, line: &str) -> bool {
        self.current_encounter_name.as_deref() == Some("tanker")
            && is_tanker_placeholder_comment(line)
    }

    fn execute_line(&mut self, line: &str) -> String {
        let command = parse_raw_action_line(line);
        let apply_respawns = !matches!(
            command,
            ParsedCommand::Blank | ParsedCommand::Comment(_) | ParsedCommand::Encounter { .. }
        );
        let mut output_lines = if apply_respawns {
            self.maybe_apply_scripted_respawns()
        } else {
            Vec::new()
        };
        let rendered = match command {
            ParsedCommand::Blank => String::new(),
            ParsedCommand::Comment(comment) => {
                let mut comment_lines = vec![comment];
                self.sync_monster_party_from_generated_encounter_comment(line);
                comment_lines.extend(self.maybe_apply_sahagin_chief_spawn_comment(line));
                comment_lines.join("\n")
            }
            ParsedCommand::Directive(directive) => self.apply_directive(&directive),
            ParsedCommand::Party { initials } => self.render_party_command(&initials),
            ParsedCommand::AdvanceRng { index, amount } => {
                let output = self.advance_rng(index, amount);
                if self.echo_state_edits && !output.starts_with("Error:") {
                    edit_action_line(line)
                } else {
                    output
                }
            }
            ParsedCommand::Encounter {
                name,
                multizone,
                zones,
            } => self.render_encounter_command(&name, multizone, &zones),
            ParsedCommand::CharacterAction {
                actor,
                action,
                args,
            } => {
                if self.echo_state_edits && matches!(action.as_str(), "escape" | "flee") {
                    let outcome = self.apply_character_action_outcome(actor, &action, &args);
                    if outcome.output.starts_with("Error:")
                        || outcome.output.starts_with("# skipped:")
                    {
                        render_character_action_outcome(outcome)
                    } else {
                        let mut output_lines = outcome.pre_lines;
                        if !output_lines.is_empty() {
                            ensure_blank_line(&mut output_lines);
                        }
                        output_lines.push(line.to_string());
                        output_lines.join("\n")
                    }
                } else {
                    self.render_character_action_command(actor, &action, &args, line)
                }
            }
            ParsedCommand::MonsterAction {
                actor,
                action,
                args,
            } => self.render_monster_action_command(actor, &action, &args, line),
            ParsedCommand::Equip { kind, args } => {
                self.render_equipment_command(&kind, &args, line)
            }
            ParsedCommand::Summon { aeon } => self.render_summon_command(&aeon),
            ParsedCommand::Spawn {
                monster,
                slot,
                forced_ctb,
            } => self.render_spawn_command(&monster, slot, forced_ctb),
            ParsedCommand::Element { args } => self.render_element_command(&args, line),
            ParsedCommand::Stat { args } => self.render_stat_command(&args),
            ParsedCommand::Heal { args } => self.render_heal_command(&args),
            ParsedCommand::Ap { args } => self.change_ap(&args),
            ParsedCommand::Death { character } => self.character_death(character),
            ParsedCommand::Compatibility { amount } => self.change_compatibility(&amount),
            ParsedCommand::YojimboTurn {
                action,
                monster,
                overdrive,
            } => self.yojimbo_turn(&action, &monster, overdrive),
            ParsedCommand::MagusTurn { sister, command } => self.magus_turn(&sister, &command),
            ParsedCommand::EncountersCount { name, amount } => {
                self.change_encounters_count(&name, &amount)
            }
            ParsedCommand::Inventory { args } => self.change_inventory(&args),
            ParsedCommand::Walk {
                zone,
                steps,
                continue_previous_zone,
            } => self.encounter_checks(&zone, &steps, continue_previous_zone),
            ParsedCommand::EndEncounter => {
                let output = self.end_encounter();
                if self.echo_state_edits && !output.starts_with("Error:") {
                    line.to_string()
                } else {
                    output
                }
            }
            ParsedCommand::Status { args }
                if args.first().is_some_and(|arg| {
                    arg.eq_ignore_ascii_case("ctb") || arg.eq_ignore_ascii_case("atb")
                }) =>
            {
                format!("status ctb\n# CTB: {}", self.available_ctb_string())
            }
            ParsedCommand::Status { args } => self.render_status_command(&args),
            ParsedCommand::ParserError { message } => message,
            ParsedCommand::Unknown { .. } => format!("Error: Impossible to parse \"{line}\""),
        };
        if !rendered.is_empty() {
            output_lines.push(rendered);
        }
        output_lines.join("\n")
    }

    fn apply_directive(&self, directive: &str) -> String {
        directive.to_string()
    }

    fn render_party_command(&mut self, initials: &str) -> String {
        let output = self.change_party(initials);
        if self.echo_state_edits && !output.starts_with("Error:") {
            format!("party {initials}")
        } else {
            output
        }
    }

    fn change_party(&mut self, initials: &str) -> String {
        let old_party = self.format_party();
        if initials.is_empty() {
            return "Error: Usage: party [characters initials]".to_string();
        }
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

    fn character_death(&mut self, character: Character) -> String {
        for _ in 0..3 {
            self.rng.advance_rng(10);
        }
        if character == Character::Yojimbo {
            self.compatibility = (self.compatibility - 10).clamp(0, 255);
        }
        format!("Character death: {}", character.display_name())
    }

    fn change_compatibility(&mut self, amount: &str) -> String {
        let Ok(parsed) = amount.parse::<i32>() else {
            return "Error: compatibility must be an integer".to_string();
        };
        let old_compatibility = self.compatibility;
        self.compatibility = if amount.starts_with(['+', '-']) {
            self.compatibility + parsed
        } else {
            parsed
        }
        .clamp(0, 255);
        format!(
            "Compatibility: {old_compatibility} -> {}",
            self.compatibility
        )
    }

    fn change_inventory(&mut self, args: &[String]) -> String {
        match args {
            [command, item, amount, ..]
                if matches!(command.as_str(), "get" | "use") && item == "gil" =>
            {
                let Ok(gil) = amount.parse::<i32>() else {
                    return "Error: Gil amount needs to be an integer".to_string();
                };
                if gil < 1 {
                    return "Error: Gil amount needs to be more than 0".to_string();
                }
                if command == "use" {
                    if gil > self.gil {
                        return format!("Error: Not enough gil (need {} more)", gil - self.gil);
                    }
                    self.gil -= gil;
                    format!("Used {gil} Gil ({} Gil total)", self.gil)
                } else {
                    self.gil += gil;
                    format!("Added {gil} Gil ({} Gil total)", self.gil)
                }
            }
            [command, item, ..] if matches!(command.as_str(), "get" | "use") && item == "gil" => {
                format!("Error: Usage: inventory {command} gil [amount]")
            }
            [command, item, ..] if command == "show" && item == "gil" => {
                format!("Gil: {}\n", self.gil)
            }
            [command, item, ..] if command == "show" && item == "equipment" => {
                self.format_equipment_inventory()
            }
            [command, ..] if command == "show" => self.format_item_inventory(),
            [command, equipment, kind, character, slots, abilities @ ..]
                if matches!(command.as_str(), "get" | "buy" | "sell")
                    && equipment == "equipment" =>
            {
                match self.parse_inventory_equipment(kind, character, slots, abilities) {
                    Ok(equipment) => {
                        if command == "buy" {
                            if equipment.gil_value > self.gil {
                                return format!(
                                    "Error: Not enough gil (need {} more)",
                                    equipment.gil_value - self.gil
                                );
                            }
                            self.gil -= equipment.gil_value;
                            let display = equipment.display.clone();
                            let gil_value = equipment.gil_value;
                            self.add_inventory_equipment(equipment);
                            format!("Bought {display} for {gil_value} gil")
                        } else if command == "sell" {
                            self.gil += equipment.sell_value;
                            format!("Sold {}", equipment.display)
                        } else {
                            let display = equipment.display.clone();
                            self.add_inventory_equipment(equipment);
                            format!("Added {display}")
                        }
                    }
                    Err(message) => message,
                }
            }
            [command, equipment, slot, ..]
                if command == "sell"
                    && equipment == "equipment"
                    && slot.chars().all(|character| character.is_ascii_digit()) =>
            {
                if self.equipment_inventory.is_empty() {
                    return "Error: Equipment inventory is empty".to_string();
                }
                let slot = slot.parse::<usize>().unwrap_or(0);
                if slot == 0 || slot > self.equipment_inventory.len() {
                    return format!(
                        "Error: Equipment slot needs to be between 1 and {}",
                        self.equipment_inventory.len()
                    );
                }
                let Some(equipment) = self.equipment_inventory[slot - 1].take() else {
                    return format!("Error: Slot {slot} is empty");
                };
                self.gil += equipment.sell_value;
                self.clean_equipment_inventory();
                format!("Sold {}", equipment.display)
            }
            [command, equipment, ..]
                if matches!(command.as_str(), "get" | "buy") && equipment == "equipment" =>
            {
                format!(
                    "Error: Usage: inventory {command} equipment [equip type] [character] [slots] (abilities)"
                )
            }
            [command, equipment, kind, ..]
                if command == "sell"
                    && equipment == "equipment"
                    && matches!(kind.as_str(), "weapon" | "armor") =>
            {
                "Error: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)"
                    .to_string()
            }
            [command, equipment, ..] if command == "sell" && equipment == "equipment" => {
                "Error: Usage: inventory sell equipment [equipment slot]".to_string()
            }
            [command, item_name, amount, ..]
                if matches!(command.as_str(), "get" | "buy" | "use" | "sell") =>
            {
                let Some(item) = data::item_name_by_key(item_name) else {
                    return format!(
                        "Error: item can only be one of these values: {}",
                        inventory_item_values()
                    );
                };
                let Ok(amount) = amount.parse::<i32>() else {
                    return "Error: Amount needs to be an integer".to_string();
                };
                if amount < 1 {
                    return "Error: Amount needs to be more than 0".to_string();
                }
                match command.as_str() {
                    "get" => {
                        self.add_inventory_item(item, amount);
                        format!("Added {item} x{amount} to inventory")
                    }
                    "buy" => {
                        let price = data::item_price(item).unwrap_or_default() * amount;
                        if price > self.gil {
                            return format!(
                                "Error: Not enough gil (need {} more)",
                                price - self.gil
                            );
                        }
                        self.gil -= price;
                        self.add_inventory_item(item, amount);
                        format!("Bought {item} x{amount} for {price} gil")
                    }
                    "use" => match self.remove_inventory_item(item, amount) {
                        Ok(()) => format!("Used {item} x{amount}"),
                        Err(message) => format!("Error: {message}"),
                    },
                    "sell" => match self.remove_inventory_item(item, amount) {
                        Ok(()) => {
                            let price =
                                (data::item_price(item).unwrap_or_default() / 4).max(1) * amount;
                            self.gil += price;
                            format!("Sold {item} x{amount} for {price} gil")
                        }
                        Err(message) => format!("Error: {message}"),
                    },
                    _ => unreachable!(),
                }
            }
            [command, ..] if matches!(command.as_str(), "get" | "buy" | "use" | "sell") => {
                format!("Error: Usage: inventory {command} [item] [amount]")
            }
            [command, first, second, ..] if command == "switch" => {
                let Ok(first_slot) = first.parse::<usize>() else {
                    return "Error: Inventory slot needs to be an integer".to_string();
                };
                let Ok(second_slot) = second.parse::<usize>() else {
                    return "Error: Inventory slot needs to be an integer".to_string();
                };
                if first_slot == 0
                    || second_slot == 0
                    || first_slot > self.item_inventory.len()
                    || second_slot > self.item_inventory.len()
                {
                    return format!(
                        "Error: Inventory slot needs to be between 1 and {}",
                        self.item_inventory.len()
                    );
                }
                self.item_inventory.swap(first_slot - 1, second_slot - 1);
                let first_item = self.item_inventory[first_slot - 1]
                    .0
                    .as_deref()
                    .unwrap_or("None");
                let second_item = self.item_inventory[second_slot - 1]
                    .0
                    .as_deref()
                    .unwrap_or("None");
                format!(
                    "Switched {second_item} (slot {first_slot}) for {first_item} (slot {second_slot})"
                )
            }
            [command, ..] if command == "switch" => {
                "Error: Usage: inventory switch [slot 1] [slot 2]".to_string()
            }
            [command, ..] if command == "autosort" => {
                let item_order = data::item_names_in_order();
                let mut sorted = self
                    .item_inventory
                    .iter()
                    .filter_map(|(item, quantity)| {
                        item.as_ref().map(|item| (item.clone(), *quantity))
                    })
                    .collect::<Vec<_>>();
                sorted.sort_by_key(|(item, _)| {
                    item_order
                        .iter()
                        .position(|candidate| candidate == item)
                        .unwrap_or(usize::MAX)
                });
                self.item_inventory = vec![(None, 0); item_order.len()];
                for (index, (item, quantity)) in sorted.into_iter().enumerate() {
                    self.item_inventory[index] = (Some(item), quantity);
                }
                "Autosorted inventory".to_string()
            }
            _ => {
                "Error: Usage: inventory [show/get/buy/use/sell/switch/autosort] [...]".to_string()
            }
        }
    }

    fn format_item_inventory(&self) -> String {
        let entries = self
            .item_inventory
            .iter()
            .map(|(item, quantity)| match item {
                Some(item) => format!("{item} {quantity}"),
                None => "-".to_string(),
            })
            .collect::<Vec<_>>();
        if entries.is_empty() {
            return "+---+\n| - |\n+---+".to_string();
        }
        let left_width = entries
            .iter()
            .step_by(2)
            .map(|entry| entry.len())
            .max()
            .unwrap_or(1);
        let right_width = entries
            .iter()
            .skip(1)
            .step_by(2)
            .map(|entry| entry.len())
            .max()
            .unwrap_or(1);
        let border = format!(
            "+-{}-+-{}-+",
            "-".repeat(left_width),
            "-".repeat(right_width)
        );
        let mut rows = Vec::new();
        let empty_row = format!("| {:left_width$} | {:right_width$} |", "-", "-");
        for pair in entries.chunks(2) {
            let left = pair.first().map(String::as_str).unwrap_or("-");
            let right = pair.get(1).map(String::as_str).unwrap_or("-");
            rows.push(format!("| {left:left_width$} | {right:right_width$} |"));
        }
        while rows.last().is_some_and(|row| row == &empty_row) {
            rows.pop();
        }
        if rows.is_empty() {
            rows.push(empty_row);
        }
        format!("{border}\n{}\n{border}", rows.join("\n"))
    }

    fn format_equipment_inventory(&self) -> String {
        if self.equipment_inventory.is_empty() {
            return "Equipment: Empty".to_string();
        }
        let lines = self
            .equipment_inventory
            .iter()
            .enumerate()
            .map(|(index, equipment)| match equipment {
                Some(equipment) => format!("#{} {}", index + 1, equipment.display),
                None => format!("#{} None", index + 1),
            })
            .collect::<Vec<_>>();
        format!("Equipment: {}", lines.join("\n           "))
    }

    fn parse_inventory_equipment(
        &self,
        kind: &str,
        character: &str,
        slots: &str,
        ability_args: &[String],
    ) -> Result<InventoryEquipment, String> {
        let kind = match kind.to_ascii_lowercase().as_str() {
            "weapon" => data::EquipmentKind::Weapon,
            "armor" => data::EquipmentKind::Armor,
            _ => {
                return Err(
                    "Error: equipment type can only be one of these values: weapon, armor"
                        .to_string(),
                )
            }
        };
        let Ok(character) = character.parse::<Character>() else {
            return Err(format!(
                "Error: character can only be one of these values: {CHARACTER_VALUES}"
            ));
        };
        let Some(raw_slots) = slots.parse::<u8>().ok().filter(|slots| *slots <= 4) else {
            return Err("Error: Slots must be between 0 and 4".to_string());
        };
        let abilities = parse_inventory_equipment_abilities(ability_args)?;
        let slots = raw_slots.max(abilities.display_names.len() as u8);
        let gil_value = equipment_gil_value(slots, &abilities.display_names);
        let display = format_inventory_equipment(
            kind,
            character,
            &abilities.display_names,
            slots,
            gil_value / 4,
        );
        Ok(InventoryEquipment {
            display,
            gil_value,
            sell_value: gil_value / 4,
        })
    }

    fn add_inventory_item(&mut self, item: &str, amount: i32) {
        if let Some((_, quantity)) = self
            .item_inventory
            .iter_mut()
            .find(|(stored, _)| stored.as_deref() == Some(item))
        {
            *quantity += amount.max(0);
        } else {
            let Some((slot, quantity)) = self
                .item_inventory
                .iter_mut()
                .find(|(stored, _)| stored.is_none())
            else {
                return;
            };
            *slot = Some(item.to_string());
            *quantity = amount.max(0);
        }
    }

    fn remove_inventory_item(&mut self, item: &str, amount: i32) -> Result<(), String> {
        let Some(index) = self
            .item_inventory
            .iter()
            .position(|(stored, _)| stored.as_deref() == Some(item))
        else {
            return Err(format!("{item} is not in the inventory"));
        };
        let quantity = self.item_inventory[index].1;
        if amount > quantity {
            return Err(format!("Not enough {item} in inventory"));
        }
        if amount == quantity {
            self.item_inventory[index] = (None, 0);
        } else {
            self.item_inventory[index].1 = quantity - amount;
        }
        Ok(())
    }

    fn clean_equipment_inventory(&mut self) {
        while self.equipment_inventory.last().is_some_and(Option::is_none) {
            self.equipment_inventory.pop();
        }
    }

    fn add_inventory_equipment(&mut self, equipment: InventoryEquipment) {
        self.clean_equipment_inventory();
        if let Some(slot) = self
            .equipment_inventory
            .iter_mut()
            .find(|slot| slot.is_none())
        {
            *slot = Some(equipment);
        } else {
            self.equipment_inventory.push(Some(equipment));
        }
    }

    fn consume_inventory_item(&mut self, item: &str, amount: i32) -> bool {
        self.remove_inventory_item(item, amount).is_ok()
    }

    fn consume_auto_potion_item(&mut self) -> Option<&'static str> {
        for (item, action) in [
            ("Potion", "auto_potion"),
            ("Hi-Potion", "auto_hi-potion"),
            ("X-Potion", "auto_x-potion"),
        ] {
            if self.consume_inventory_item(item, 1) {
                return Some(action);
            }
        }
        None
    }

    fn yojimbo_turn(&mut self, action_name: &str, monster_name: &str, overdrive: bool) -> String {
        let Some(mut action) = yojimbo_action(action_name) else {
            return format!("Error: No yojimbo action named \"{action_name}\"");
        };
        let Some(monster) = data::monster_stats(monster_name) else {
            return format!("Error: No monster named \"{monster_name}\"");
        };

        let is_attack_free = self.yojimbo_free_attack_check();
        let (gil, motivation) = if is_attack_free {
            let (free_action, free_motivation) = self.yojimbo_free_attack(&monster);
            action = free_action;
            (0, free_motivation)
        } else if let Some(needed_motivation) = action.needed_motivation {
            self.yojimbo_gil_and_motivation(action, needed_motivation, &monster, overdrive)
        } else {
            (0, 0)
        };

        self.compatibility = (self.compatibility + action.compatibility_modifier).clamp(0, 255);
        let cost = if is_attack_free {
            "free".to_string()
        } else {
            format!("{gil} gil")
        };
        let overdrive_label = if overdrive { " (OD used)" } else { "" };
        let needed = action
            .needed_motivation
            .map(|value| value.to_string())
            .unwrap_or_else(|| "None".to_string());

        format!(
            "{} -> {}: {cost}{overdrive_label} [{motivation}/{needed} motivation][{}/255 compatibility]",
            action.name, monster.display_name, self.compatibility
        )
    }

    fn yojimbo_free_attack_check(&mut self) -> bool {
        let rng = (self.rng.advance_rng(17) & 255) as i32;
        self.compatibility / 4 > rng
    }

    fn yojimbo_free_attack(&mut self, monster: &data::MonsterStats) -> (YojimboActionData, i32) {
        let base_motivation = self.compatibility / 4;
        let motivation = base_motivation + (self.rng.advance_rng(17) & 0x3f) as i32;
        let mut attack = yojimbo_action("daigoro").expect("daigoro action exists");
        for candidate in YOJIMBO_ACTIONS
            .iter()
            .copied()
            .filter(|action| action.needed_motivation.is_some())
        {
            if motivation >= candidate.needed_motivation.unwrap_or_default() {
                attack = candidate;
            }
        }
        if attack.key == "zanmato" && monster.zanmato_level > 0 {
            attack = yojimbo_action("wakizashi_mt").expect("wakizashi_mt action exists");
        }
        (attack, motivation)
    }

    fn yojimbo_gil_and_motivation(
        &mut self,
        action: YojimboActionData,
        needed_motivation: i32,
        monster: &data::MonsterStats,
        overdrive: bool,
    ) -> (i64, i32) {
        let zanmato_level = monster.zanmato_level as usize;
        let mut zanmato_resistance =
            YOJIMBO_ZANMATO_RESISTANCES[zanmato_level.min(YOJIMBO_ZANMATO_RESISTANCES.len() - 1)];
        let mut rng_motivation = (self.rng.advance_rng(17) & 0x3f) as i32;
        if action.key != "zanmato" && monster.zanmato_level > 0 {
            zanmato_resistance = YOJIMBO_ZANMATO_RESISTANCES[0];
            rng_motivation = (self.rng.advance_rng(17) & 0x3f) as i32;
        }

        let base_motivation = self.compatibility / YOJIMBO_COMPATIBILITY_MODIFIER;
        let mut fixed_motivation = (base_motivation as f64 * zanmato_resistance) as i32;
        fixed_motivation += rng_motivation;
        if overdrive {
            fixed_motivation += YOJIMBO_OVERDRIVE_MOTIVATION;
        }

        let mut gil = 1_i64;
        let mut motivation = fixed_motivation;
        while motivation < needed_motivation {
            gil *= 2;
            let gil_motivation =
                (yojimbo_gil_to_motivation(gil) as f64 * zanmato_resistance) as i32;
            motivation = fixed_motivation + gil_motivation;
        }
        (gil, motivation)
    }

    fn magus_turn(&mut self, sister_name: &str, command_name: &str) -> String {
        let Some(sister) = magus_sister_from_prefix(sister_name) else {
            return "Error: Usage: magusturn [name] (command)".to_string();
        };
        let mut commands =
            magus_command_list(sister, self.magus_last_commands.get(&sister).copied());
        self.filter_magus_command_menu(sister, &mut commands);
        let starting_motivation = self.magus_motivation(sister);
        let chosen_commands = commands
            .iter()
            .cloned()
            .filter(|command| magus_command_key(command.name).starts_with(command_name))
            .collect::<Vec<_>>();
        if chosen_commands.len() != 1 {
            return format!(
                "Error: Available commands for {}: {}",
                sister.display_name(),
                commands
                    .iter()
                    .map(|command| command.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        let command = &chosen_commands[0];
        if command.name != "Auto-Life" {
            for _ in 0..self.magus_command_menu_rng_rolls(sister) {
                self.rng.advance_rng(18);
            }
        }
        if command.id != 1 {
            self.magus_last_action_lists.remove(&sister);
        }
        let mut success_motivation = 0;
        let mut break_motivation_override = None;
        let mut multi_action_outputs = None;
        let output = if command.name == "Dismiss" {
            self.apply_magus_dismiss(sister)
        } else if magus_command_lacks_monster_target(command, self.monsters.is_empty()) {
            self.apply_magus_taking_break(sister)
        } else {
            let actor = command.actor.unwrap_or(sister);
            let action = self.resolve_magus_action_choice(sister, command, starting_motivation);
            let output = match action {
                MagusResolvedAction::Action {
                    action_key,
                    args,
                    motivation,
                } => {
                    success_motivation = motivation;
                    self.magus_last_actions.insert(sister, action_key);
                    self.apply_character_action(actor, action_key, &args)
                }
                MagusResolvedAction::MindyReflectedActions {
                    first_action_key,
                    first_motivation,
                    remaining_actions,
                } => {
                    success_motivation = first_motivation;
                    let mut action_keys = vec![first_action_key];
                    self.magus_last_actions.insert(sister, first_action_key);
                    let mut outputs = vec![self.apply_character_action(
                        actor,
                        first_action_key,
                        &[Character::Cindy.input_name().to_string()],
                    )];
                    for _ in 0..remaining_actions {
                        let Some((action_key, _action_motivation)) =
                            self.resolve_mindy_reflected_spell_choice(starting_motivation)
                        else {
                            continue;
                        };
                        action_keys.push(action_key);
                        self.magus_last_actions.insert(sister, action_key);
                        outputs.push(self.apply_character_action(
                            actor,
                            action_key,
                            &[Character::Cindy.input_name().to_string()],
                        ));
                    }
                    let output = outputs.join("\n");
                    if outputs.len() > 1 {
                        self.magus_last_action_lists.insert(sister, action_keys);
                        multi_action_outputs = Some(outputs);
                    }
                    output
                }
                MagusResolvedAction::MindyReflectedRepeatList {
                    action_keys,
                    motivation,
                } => {
                    success_motivation = motivation;
                    let mut outputs = Vec::new();
                    for action_key in action_keys {
                        if !self.character_actor(Character::Cindy).is_some_and(|cindy| {
                            cindy.current_hp > 0
                                && !cindy.statuses.contains(&Status::Death)
                                && !cindy.statuses.contains(&Status::Eject)
                        }) || !self.magus_can_attempt_action(Character::Mindy, action_key)
                        {
                            continue;
                        }
                        self.magus_last_actions.insert(sister, action_key);
                        outputs.push(self.apply_character_action(
                            actor,
                            action_key,
                            &[Character::Cindy.input_name().to_string()],
                        ));
                    }
                    if outputs.is_empty() {
                        break_motivation_override = Some(-15);
                        self.apply_magus_taking_break(sister)
                    } else {
                        let output = outputs.join("\n");
                        if outputs.len() > 1 {
                            multi_action_outputs = Some(outputs);
                        }
                        output
                    }
                }
                MagusResolvedAction::TakingBreak(motivation_override) => {
                    break_motivation_override = motivation_override;
                    self.magus_last_actions.remove(&sister);
                    self.magus_last_action_lists.remove(&sister);
                    self.apply_magus_taking_break(sister)
                }
            };
            if multi_action_outputs.is_none() && output.starts_with("Error: No monster in slot ") {
                self.magus_last_actions.remove(&sister);
                self.apply_magus_taking_break(sister)
            } else {
                output
            }
        };
        if magus_command_can_be_repeated(command.id) {
            self.magus_last_commands.insert(sister, command.id);
        }
        let motivation = if output.contains("Taking a break...") {
            self.adjust_magus_motivation(
                sister,
                break_motivation_override.unwrap_or(command.break_motivation),
            )
        } else if success_motivation != 0 {
            self.adjust_magus_motivation(sister, success_motivation)
        } else {
            starting_motivation
        };
        if let Some(outputs) = multi_action_outputs {
            format_magus_multi_action_output(sister, command.name, motivation, &outputs)
        } else {
            format_magus_command_output(sister, command.name, motivation, &output)
        }
    }

    fn resolve_magus_action_choice(
        &mut self,
        sister: Character,
        command: &MagusCommandData,
        motivation: i32,
    ) -> MagusResolvedAction {
        let choices = match (sister, command.id) {
            (Character::Cindy, 2) => &[
                ("camisade", motivation + 20, 4),
                ("attack", motivation + 150, 3),
            ][..],
            (Character::Cindy, 1) => {
                if let Some(last_command_id) = self.magus_last_commands.get(&sister).copied() {
                    if magus_rng_check(&mut self.rng, motivation + 150) {
                        if let Some(action_key) = self.magus_last_actions.get(&sister).copied() {
                            if let Some(repeated) = self.repeat_cindy_magus_action(
                                last_command_id,
                                action_key,
                                &command.args,
                                4,
                            ) {
                                return repeated;
                            }
                        } else if last_command_id == 3 {
                            if let Some(repeated) = self.repeat_monster_magus_action(
                                Character::Cindy,
                                "attack",
                                &command.args,
                                4,
                            ) {
                                return repeated;
                            }
                        }
                        return MagusResolvedAction::TakingBreak(None);
                    }
                    if self.has_live_monster_target() {
                        for (action_key, chance, action_motivation) in
                            [("camisade", motivation, 2), ("attack", 255, 2)]
                        {
                            if data::action_data(action_key).is_some()
                                && magus_rng_check(&mut self.rng, chance)
                            {
                                return MagusResolvedAction::Action {
                                    action_key,
                                    args: command.args.clone(),
                                    motivation: action_motivation,
                                };
                            }
                        }
                    }
                    return MagusResolvedAction::TakingBreak(None);
                }
                return MagusResolvedAction::Action {
                    action_key: command.action_key,
                    args: command.args.clone(),
                    motivation: 0,
                };
            }
            (Character::Cindy, 3) => {
                if let Some((current_hp, max_hp, current_mp, max_mp)) =
                    self.character_actor(Character::Cindy).map(|cindy| {
                        (
                            cindy.current_hp,
                            cindy.effective_max_hp().max(1),
                            cindy.current_mp,
                            cindy.effective_max_mp().max(1),
                        )
                    })
                {
                    if current_hp < max_hp {
                        let chance = (max_hp - current_hp) * 256 / max_hp;
                        if magus_rng_check(&mut self.rng, chance) {
                            return MagusResolvedAction::Action {
                                action_key: "pray",
                                args: vec!["party".to_string()],
                                motivation: 1,
                            };
                        }
                    }
                    if current_mp < max_mp {
                        let chance = (max_mp - current_mp) * 256 / max_mp;
                        if magus_rng_check(&mut self.rng, chance) {
                            return MagusResolvedAction::Action {
                                action_key: "osmose",
                                args: command.args.clone(),
                                motivation: 1,
                            };
                        }
                    }
                }
                if !self.monsters.is_empty() {
                    if magus_rng_check(&mut self.rng, motivation * 2) {
                        return MagusResolvedAction::Action {
                            action_key: "attack",
                            args: command.args.clone(),
                            motivation: 2,
                        };
                    }
                    if magus_rng_check(&mut self.rng, motivation / 2) {
                        for (action_key, chance, action_args, action_motivation) in [
                            (
                                "ultima",
                                Some((motivation - 80).max(0)),
                                vec!["monsters".to_string()],
                                10,
                            ),
                            ("flare", Some(motivation / 8), command.args.clone(), 6),
                            ("drain", Some(motivation / 2), command.args.clone(), 6),
                            ("blizzaga", Some(motivation), command.args.clone(), 2),
                            ("waterga", Some(motivation), command.args.clone(), 2),
                            ("thundaga", Some(motivation), command.args.clone(), 2),
                            ("firaga", Some(motivation), command.args.clone(), 2),
                            ("attack", None, command.args.clone(), 2),
                        ] {
                            let Some(action_data) = data::action_data(action_key) else {
                                continue;
                            };
                            if !self.magus_actor_can_use_action(sister, &action_data) {
                                continue;
                            }
                            if chance.is_none_or(|chance| magus_rng_check(&mut self.rng, chance)) {
                                return MagusResolvedAction::Action {
                                    action_key,
                                    args: action_args,
                                    motivation: action_motivation,
                                };
                            }
                        }
                    }
                    if magus_rng_check(&mut self.rng, motivation) {
                        return MagusResolvedAction::Action {
                            action_key: "camisade",
                            args: command.args.clone(),
                            motivation: 5,
                        };
                    }
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            (Character::Cindy, 4) => {
                if let Some(dead_sister) = self.first_dead_magus_sister() {
                    if magus_rng_check(&mut self.rng, motivation)
                        && self.magus_can_attempt_action(sister, "full-life")
                    {
                        return MagusResolvedAction::Action {
                            action_key: "full-life",
                            args: vec![dead_sister.input_name().to_string()],
                            motivation: 2,
                        };
                    }
                    if magus_rng_check(&mut self.rng, motivation + 150)
                        && self.magus_can_attempt_action(sister, "life")
                    {
                        return MagusResolvedAction::Action {
                            action_key: "life",
                            args: vec![dead_sister.input_name().to_string()],
                            motivation: 1,
                        };
                    }
                }
                if magus_rng_check(&mut self.rng, motivation + 50)
                    && self.magus_can_attempt_action(sister, "auto-life")
                {
                    return MagusResolvedAction::Action {
                        action_key: "auto-life",
                        args: Vec::new(),
                        motivation: 10,
                    };
                }
                let damaged_sister = self.first_damaged_magus_sister_except(sister);
                for (action_key, chance) in [
                    ("cure", motivation / 2),
                    ("cura", motivation / 2),
                    ("curaga", motivation / 2),
                ] {
                    if magus_rng_check(&mut self.rng, chance)
                        && self.magus_can_attempt_action(sister, action_key)
                    {
                        if let Some(damaged_sister) = damaged_sister {
                            return MagusResolvedAction::Action {
                                action_key,
                                args: vec![damaged_sister.input_name().to_string()],
                                motivation: 2,
                            };
                        }
                    }
                }
                if !self.monsters.is_empty() && magus_rng_check(&mut self.rng, motivation) {
                    return MagusResolvedAction::Action {
                        action_key: "camisade",
                        args: command.args.clone(),
                        motivation: 5,
                    };
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            (Character::Sandy, 2) => &[
                ("razzia", motivation + 50, 4),
                ("attack", motivation + 180, 3),
            ][..],
            (Character::Sandy, 1) => {
                if let Some(last_command_id) = self.magus_last_commands.get(&sister).copied() {
                    if magus_rng_check(&mut self.rng, motivation + 120) {
                        if let Some(action_key) = self.magus_last_actions.get(&sister).copied() {
                            if let Some(repeated) = self.repeat_sandy_magus_action(
                                last_command_id,
                                action_key,
                                &command.args,
                                4,
                            ) {
                                return repeated;
                            }
                        }
                        return MagusResolvedAction::TakingBreak(None);
                    }
                    if self.has_live_monster_target() {
                        if magus_rng_check(&mut self.rng, 128) {
                            return MagusResolvedAction::Action {
                                action_key: "razzia",
                                args: command.args.clone(),
                                motivation: 2,
                            };
                        }
                        return MagusResolvedAction::Action {
                            action_key: "attack",
                            args: command.args.clone(),
                            motivation: 2,
                        };
                    }
                    return MagusResolvedAction::TakingBreak(Some(-5));
                }
                return MagusResolvedAction::Action {
                    action_key: command.action_key,
                    args: command.args.clone(),
                    motivation: 0,
                };
            }
            (Character::Mindy, 2) => &[
                ("passado", motivation + 30, 4),
                ("attack", motivation + 180, 3),
            ][..],
            (Character::Mindy, 1) => {
                if let Some(last_command_id) = self.magus_last_commands.get(&sister).copied() {
                    if magus_rng_check(&mut self.rng, motivation + 120) {
                        if let Some(action_key) = self.magus_last_actions.get(&sister).copied() {
                            if let Some(repeated) = self.repeat_mindy_magus_action(
                                last_command_id,
                                action_key,
                                &command.args,
                                4,
                            ) {
                                return repeated;
                            }
                        }
                        return MagusResolvedAction::TakingBreak(None);
                    }
                    if self.has_live_monster_target() {
                        if magus_rng_check(&mut self.rng, 128) {
                            return MagusResolvedAction::Action {
                                action_key: "passado",
                                args: command.args.clone(),
                                motivation: 2,
                            };
                        }
                        if magus_rng_check(&mut self.rng, motivation + 100) {
                            return MagusResolvedAction::Action {
                                action_key: "attack",
                                args: command.args.clone(),
                                motivation: 3,
                            };
                        }
                    }
                    return MagusResolvedAction::TakingBreak(Some(-5));
                }
                return MagusResolvedAction::Action {
                    action_key: command.action_key,
                    args: command.args.clone(),
                    motivation: 0,
                };
            }
            (Character::Cindy, 0) => {
                if magus_rng_check(&mut self.rng, motivation + 50)
                    && self.magus_can_attempt_action(sister, "auto-life")
                {
                    return MagusResolvedAction::Action {
                        action_key: "auto-life",
                        args: Vec::new(),
                        motivation: 10,
                    };
                }
                if self.magus_has_dead_sister() {
                    if magus_rng_check(&mut self.rng, (motivation + 50) / 2)
                        && self.magus_can_attempt_action(sister, "full-life")
                    {
                        return MagusResolvedAction::Action {
                            action_key: "full-life",
                            args: Vec::new(),
                            motivation: 6,
                        };
                    }
                    if magus_rng_check(&mut self.rng, (motivation + 200) / 2)
                        && self.magus_can_attempt_action(sister, "life")
                    {
                        return MagusResolvedAction::Action {
                            action_key: "life",
                            args: Vec::new(),
                            motivation: 5,
                        };
                    }
                }
                if !self.monsters.is_empty() {
                    if magus_rng_check(&mut self.rng, motivation) {
                        return MagusResolvedAction::Action {
                            action_key: "attack",
                            args: command.args.clone(),
                            motivation: 2,
                        };
                    }
                    if data::action_data("flare")
                        .is_some_and(|action| self.magus_actor_can_use_action(sister, &action))
                        && magus_rng_check(&mut self.rng, motivation / 8)
                    {
                        return MagusResolvedAction::Action {
                            action_key: "flare",
                            args: command.args.clone(),
                            motivation: 6,
                        };
                    }
                    if magus_rng_check(&mut self.rng, 128) {
                        for action_key in ["firaga", "thundaga", "blizzaga", "waterga"] {
                            let Some(action_data) = data::action_data(action_key) else {
                                continue;
                            };
                            if self.magus_actor_can_use_action(sister, &action_data)
                                && magus_rng_check(&mut self.rng, motivation / 2)
                            {
                                return MagusResolvedAction::Action {
                                    action_key,
                                    args: command.args.clone(),
                                    motivation: 2,
                                };
                            }
                        }
                    }
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            (Character::Sandy, 0) => {
                if magus_rng_check(&mut self.rng, motivation + 200) {
                    return MagusResolvedAction::Action {
                        action_key: "reflect",
                        args: vec![Character::Cindy.input_name().to_string()],
                        motivation: 2,
                    };
                }
                if self.monsters.is_empty() {
                    return MagusResolvedAction::TakingBreak(None);
                }
                if magus_rng_check(&mut self.rng, 128) {
                    return MagusResolvedAction::Action {
                        action_key: "razzia",
                        args: command.args.clone(),
                        motivation: 2,
                    };
                }
                return MagusResolvedAction::Action {
                    action_key: "attack",
                    args: command.args.clone(),
                    motivation: 2,
                };
            }
            (Character::Mindy, 0) => {
                if self.character_actor(Character::Cindy).is_some_and(|cindy| {
                    cindy.current_hp > 0
                        && cindy.statuses.contains(&Status::Reflect)
                        && !cindy.statuses.contains(&Status::Death)
                        && !cindy.statuses.contains(&Status::Eject)
                }) {
                    let actions_to_use = if magus_rng_check(&mut self.rng, motivation) {
                        2
                    } else {
                        1
                    };
                    if let Some((action_key, action_motivation)) =
                        self.resolve_mindy_reflected_spell_choice(motivation)
                    {
                        if actions_to_use == 1 {
                            return MagusResolvedAction::Action {
                                action_key,
                                args: vec![Character::Cindy.input_name().to_string()],
                                motivation: action_motivation,
                            };
                        }
                        return MagusResolvedAction::MindyReflectedActions {
                            first_action_key: action_key,
                            first_motivation: action_motivation,
                            remaining_actions: actions_to_use - 1,
                        };
                    }
                }
                if self.monsters.is_empty() {
                    return MagusResolvedAction::TakingBreak(None);
                }
                if magus_rng_check(&mut self.rng, motivation + 100) {
                    return MagusResolvedAction::Action {
                        action_key: "passado",
                        args: command.args.clone(),
                        motivation: 3,
                    };
                }
                if magus_rng_check(&mut self.rng, motivation + 100) {
                    return MagusResolvedAction::Action {
                        action_key: "attack",
                        args: command.args.clone(),
                        motivation: 3,
                    };
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            (Character::Mindy, 6) => {
                if self.monsters.is_empty() {
                    return MagusResolvedAction::TakingBreak(None);
                }
                if magus_rng_check(&mut self.rng, motivation / 2) {
                    return MagusResolvedAction::Action {
                        action_key: "lancet",
                        args: command.args.clone(),
                        motivation: 3,
                    };
                }
                if let Some((current_hp, max_hp, current_mp, max_mp)) =
                    self.character_actor(Character::Mindy).map(|mindy| {
                        (
                            mindy.current_hp,
                            mindy.effective_max_hp().max(1),
                            mindy.current_mp,
                            mindy.effective_max_mp().max(1),
                        )
                    })
                {
                    let hp_chance = ((max_hp - current_hp).max(0) * 256) / (max_hp * 2);
                    if data::action_data("drain")
                        .is_some_and(|action| self.magus_actor_can_use_action(sister, &action))
                        && magus_rng_check(&mut self.rng, hp_chance)
                    {
                        return MagusResolvedAction::Action {
                            action_key: "drain",
                            args: command.args.clone(),
                            motivation: 3,
                        };
                    }
                    let mp_chance = ((max_mp - current_mp).max(0) * 256) / (max_mp * 2);
                    if data::action_data("osmose")
                        .is_some_and(|action| self.magus_actor_can_use_action(sister, &action))
                        && magus_rng_check(&mut self.rng, mp_chance)
                    {
                        return MagusResolvedAction::Action {
                            action_key: "osmose",
                            args: command.args.clone(),
                            motivation: 3,
                        };
                    }
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            (Character::Sandy, 5) => {
                for (action_key, chance) in [
                    ("shell", motivation + 50),
                    ("protect", motivation + 50),
                    ("haste", motivation),
                    ("nulall", motivation + 50),
                ] {
                    if magus_rng_check(&mut self.rng, chance) {
                        let args = if matches!(action_key, "shell" | "protect" | "haste") {
                            let Some(target) = self.magus_support_target(action_key) else {
                                continue;
                            };
                            vec![target.input_name().to_string()]
                        } else if action_key == "nulall" {
                            vec!["party".to_string()]
                        } else {
                            Vec::new()
                        };
                        return MagusResolvedAction::Action {
                            action_key,
                            args,
                            motivation: 2,
                        };
                    }
                }
                let has_damaged_sister = self.any_magus_sister_damaged();
                for (action_key, chance) in [
                    ("cure", motivation / 4),
                    ("cura", motivation / 6),
                    ("curaga", motivation / 8),
                ] {
                    if magus_rng_check(&mut self.rng, chance) {
                        if has_damaged_sister {
                            return MagusResolvedAction::Action {
                                action_key,
                                args: vec![Character::Sandy.input_name().to_string()],
                                motivation: 2,
                            };
                        }
                    }
                }
                if !self.monsters.is_empty() {
                    if magus_rng_check(&mut self.rng, motivation) {
                        return MagusResolvedAction::Action {
                            action_key: "razzia",
                            args: command.args.clone(),
                            motivation: 2,
                        };
                    }
                    if magus_rng_check(&mut self.rng, motivation) {
                        return MagusResolvedAction::Action {
                            action_key: "attack",
                            args: command.args.clone(),
                            motivation: 2,
                        };
                    }
                }
                return MagusResolvedAction::TakingBreak(None);
            }
            _ => {
                return MagusResolvedAction::Action {
                    action_key: command.action_key,
                    args: command.args.clone(),
                    motivation: 0,
                }
            }
        };

        for (action_key, chance, action_motivation) in choices {
            if data::action_data(action_key).is_some() && magus_rng_check(&mut self.rng, *chance) {
                return MagusResolvedAction::Action {
                    action_key,
                    args: command.args.clone(),
                    motivation: *action_motivation,
                };
            }
        }
        MagusResolvedAction::TakingBreak(None)
    }

    fn magus_motivation(&self, sister: Character) -> i32 {
        self.magus_motivation.get(&sister).copied().unwrap_or(50)
    }

    fn adjust_magus_motivation(&mut self, sister: Character, amount: i32) -> i32 {
        let motivation = (self.magus_motivation(sister) + amount).clamp(0, 100);
        self.magus_motivation.insert(sister, motivation);
        motivation
    }

    fn magus_actor_can_use_action(&self, sister: Character, action: &ActionData) -> bool {
        let Some(actor) = self.character_actor(sister) else {
            return false;
        };
        if action.affected_by_silence && actor.statuses.contains(&Status::Silence) {
            return false;
        }
        actor.current_mp >= action.mp_cost
    }

    fn magus_can_attempt_action(&self, sister: Character, action_key: &str) -> bool {
        data::action_data(action_key)
            .is_some_and(|action| self.magus_actor_can_use_action(sister, &action))
    }

    fn resolve_mindy_reflected_spell_choice(
        &mut self,
        motivation: i32,
    ) -> Option<(&'static str, i32)> {
        for (action_key, chance, action_motivation) in [
            ("flare", motivation / 2, 3),
            ("bio", motivation / 2, 2),
            ("death", motivation / 2, 2),
            ("firaga", motivation, 2),
            ("thundaga", motivation, 2),
            ("waterga", motivation, 2),
            ("blizzaga", motivation, 2),
            ("fira", motivation, 2),
            ("thundara", motivation, 2),
            ("watera", motivation, 2),
            ("blizzara", motivation, 2),
            ("drain", motivation, 1),
        ] {
            let Some(action_data) = data::action_data(action_key) else {
                continue;
            };
            if self.magus_actor_can_use_action(Character::Mindy, &action_data)
                && magus_rng_check(&mut self.rng, chance)
            {
                return Some((action_key, action_motivation));
            }
        }
        None
    }

    fn has_live_monster_target(&self) -> bool {
        self.monsters.iter().any(|monster| {
            monster.current_hp > 0
                && !monster.statuses.contains(&Status::Death)
                && !monster.statuses.contains(&Status::Eject)
        })
    }

    fn repeat_monster_magus_action(
        &self,
        sister: Character,
        action_key: &'static str,
        args: &[String],
        motivation: i32,
    ) -> Option<MagusResolvedAction> {
        if self.has_live_monster_target() && self.magus_can_attempt_action(sister, action_key) {
            Some(MagusResolvedAction::Action {
                action_key,
                args: args.to_vec(),
                motivation,
            })
        } else {
            None
        }
    }

    fn repeat_cindy_magus_action(
        &self,
        last_command_id: i32,
        action_key: &'static str,
        command_args: &[String],
        motivation: i32,
    ) -> Option<MagusResolvedAction> {
        match last_command_id {
            0 => match action_key {
                "auto-life" if self.magus_can_attempt_action(Character::Cindy, action_key) => {
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: Vec::new(),
                        motivation,
                    })
                }
                "full-life" | "life" => self.first_dead_magus_sister().and_then(|target| {
                    self.magus_can_attempt_action(Character::Cindy, action_key)
                        .then_some(MagusResolvedAction::Action {
                            action_key,
                            args: vec![target.input_name().to_string()],
                            motivation,
                        })
                }),
                "attack" | "flare" | "firaga" | "thundaga" | "blizzaga" | "waterga" => self
                    .repeat_monster_magus_action(
                        Character::Cindy,
                        action_key,
                        command_args,
                        motivation,
                    ),
                _ => None,
            },
            2 if matches!(action_key, "camisade" | "attack") => self.repeat_monster_magus_action(
                Character::Cindy,
                action_key,
                command_args,
                motivation,
            ),
            3 => match action_key {
                "pray" if self.magus_can_attempt_action(Character::Cindy, action_key) => {
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: vec!["party".to_string()],
                        motivation,
                    })
                }
                "ultima"
                    if self.has_live_monster_target()
                        && self.magus_can_attempt_action(Character::Cindy, action_key) =>
                {
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: vec!["monsters".to_string()],
                        motivation,
                    })
                }
                "osmose" | "attack" | "flare" | "drain" | "blizzaga" | "waterga" | "thundaga"
                | "firaga" | "camisade" => self.repeat_monster_magus_action(
                    Character::Cindy,
                    action_key,
                    command_args,
                    motivation,
                ),
                _ => None,
            },
            4 => match action_key {
                "full-life" | "life" => self.first_dead_magus_sister().and_then(|target| {
                    self.magus_can_attempt_action(Character::Cindy, action_key)
                        .then_some(MagusResolvedAction::Action {
                            action_key,
                            args: vec![target.input_name().to_string()],
                            motivation,
                        })
                }),
                "auto-life" if self.magus_can_attempt_action(Character::Cindy, action_key) => {
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: Vec::new(),
                        motivation,
                    })
                }
                "cure" | "cura" | "curaga" => self
                    .first_damaged_magus_sister_except(Character::Cindy)
                    .and_then(|target| {
                        self.magus_can_attempt_action(Character::Cindy, action_key)
                            .then_some(MagusResolvedAction::Action {
                                action_key,
                                args: vec![target.input_name().to_string()],
                                motivation,
                            })
                    }),
                "camisade" => self.repeat_monster_magus_action(
                    Character::Cindy,
                    action_key,
                    command_args,
                    motivation,
                ),
                _ => None,
            },
            _ => None,
        }
    }

    fn repeat_sandy_magus_action(
        &mut self,
        last_command_id: i32,
        action_key: &'static str,
        command_args: &[String],
        motivation: i32,
    ) -> Option<MagusResolvedAction> {
        match last_command_id {
            0 => match action_key {
                "reflect" => self.repeat_monster_magus_action(
                    Character::Sandy,
                    "attack",
                    command_args,
                    motivation,
                ),
                "razzia" | "attack" => self.repeat_monster_magus_action(
                    Character::Sandy,
                    action_key,
                    command_args,
                    motivation,
                ),
                _ => None,
            },
            2 if matches!(action_key, "razzia" | "attack") => self.repeat_monster_magus_action(
                Character::Sandy,
                action_key,
                command_args,
                motivation,
            ),
            5 => match action_key {
                "shell" | "protect" | "haste" => {
                    let target = self.magus_support_target(action_key)?;
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: vec![target.input_name().to_string()],
                        motivation,
                    })
                }
                "nulall" if self.magus_can_attempt_action(Character::Sandy, action_key) => {
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: vec!["party".to_string()],
                        motivation,
                    })
                }
                "cure" | "cura" | "curaga" if self.any_magus_sister_damaged() => self
                    .magus_can_attempt_action(Character::Sandy, action_key)
                    .then_some(MagusResolvedAction::Action {
                        action_key,
                        args: vec![Character::Sandy.input_name().to_string()],
                        motivation,
                    }),
                "razzia" | "attack" => self.repeat_monster_magus_action(
                    Character::Sandy,
                    action_key,
                    command_args,
                    motivation,
                ),
                _ => None,
            },
            _ => None,
        }
    }

    fn repeat_mindy_magus_action(
        &self,
        last_command_id: i32,
        action_key: &'static str,
        command_args: &[String],
        motivation: i32,
    ) -> Option<MagusResolvedAction> {
        match last_command_id {
            0 => match action_key {
                "flare" | "bio" | "death" | "firaga" | "thundaga" | "waterga" | "blizzaga"
                | "fira" | "thundara" | "watera" | "blizzara" | "drain"
                    if self.character_actor(Character::Cindy).is_some_and(|cindy| {
                        cindy.current_hp > 0
                            && !cindy.statuses.contains(&Status::Death)
                            && !cindy.statuses.contains(&Status::Eject)
                    }) && self.magus_can_attempt_action(Character::Mindy, action_key) =>
                {
                    if let Some(action_keys) = self
                        .magus_last_action_lists
                        .get(&Character::Mindy)
                        .filter(|action_keys| action_keys.len() > 1)
                    {
                        return Some(MagusResolvedAction::MindyReflectedRepeatList {
                            action_keys: action_keys.clone(),
                            motivation,
                        });
                    }
                    Some(MagusResolvedAction::Action {
                        action_key,
                        args: vec![Character::Cindy.input_name().to_string()],
                        motivation,
                    })
                }
                "passado" | "attack" => self.repeat_monster_magus_action(
                    Character::Mindy,
                    action_key,
                    command_args,
                    motivation,
                ),
                _ => None,
            },
            2 if matches!(action_key, "passado" | "attack") => self.repeat_monster_magus_action(
                Character::Mindy,
                action_key,
                command_args,
                motivation,
            ),
            6 if matches!(action_key, "lancet" | "drain" | "osmose") => self
                .repeat_monster_magus_action(
                    Character::Mindy,
                    action_key,
                    command_args,
                    motivation,
                ),
            _ => None,
        }
    }

    fn magus_support_target(&mut self, action_key: &str) -> Option<Character> {
        let blocked_status = match action_key {
            "shell" => Status::Shell,
            "protect" => Status::Protect,
            "haste" => Status::Haste,
            _ => return None,
        };
        let candidates: Vec<Character> = [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .filter(|character| {
                self.character_actor(*character).is_some_and(|actor| {
                    !actor.statuses.contains(&Status::Death)
                        && !actor.statuses.contains(&Status::Eject)
                        && !actor.statuses.contains(&Status::Reflect)
                        && !actor.statuses.contains(&blocked_status)
                })
            })
            .collect();
        match candidates.len() {
            0 => None,
            1 => candidates.first().copied(),
            len => {
                let index = self.rng.advance_rng(4) as usize % len;
                candidates.get(index).copied()
            }
        }
    }

    fn mindy_needs_help(&self) -> bool {
        let Some(mindy) = self.character_actor(Character::Mindy) else {
            return false;
        };
        mindy.current_hp * 2 < mindy.effective_max_hp().max(1)
            || mindy.current_mp * 2 < mindy.effective_max_mp().max(1)
    }

    fn magus_command_menu_rng_rolls(&self, sister: Character) -> usize {
        match sister {
            Character::Cindy => 3,
            Character::Sandy => 2,
            Character::Mindy if self.mindy_needs_help() => 2,
            Character::Mindy => 1,
            _ => 0,
        }
    }

    fn filter_magus_command_menu(&self, sister: Character, commands: &mut Vec<MagusCommandData>) {
        let mut rng = self.rng.clone();
        let motivation = self.magus_motivation(sister);
        match sister {
            Character::Cindy => {
                let fight = magus_rng_check(&mut rng, motivation + 50);
                let go_go = magus_rng_check(&mut rng, 128);
                let help = magus_rng_check(&mut rng, 200);
                commands.retain(|command| match command.id {
                    2 => fight,
                    3 => go_go,
                    4 => help,
                    _ => true,
                });
            }
            Character::Sandy => {
                let fight = magus_rng_check(&mut rng, motivation + 150);
                let defense =
                    magus_rng_check(&mut rng, 128) || self.any_magus_sister_below_half_hp();
                commands.retain(|command| match command.id {
                    2 => fight,
                    5 => defense,
                    _ => true,
                });
            }
            Character::Mindy => {
                let fight = magus_rng_check(&mut rng, motivation + 150);
                let are_you_all_right = self.mindy_are_you_all_right_available(&mut rng);
                commands.retain(|command| match command.id {
                    2 => fight,
                    6 => are_you_all_right,
                    _ => true,
                });
            }
            _ => {}
        }
    }

    fn any_magus_sister_below_half_hp(&self) -> bool {
        [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .filter_map(|character| self.character_actor(character))
            .any(|actor| actor.current_hp < actor.effective_max_hp() / 2)
    }

    fn magus_has_dead_sister(&self) -> bool {
        [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .filter_map(|character| self.character_actor(character))
            .any(|actor| actor.statuses.contains(&Status::Death))
    }

    fn first_dead_magus_sister(&self) -> Option<Character> {
        [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .find(|character| {
                self.character_actor(*character)
                    .is_some_and(|actor| actor.statuses.contains(&Status::Death))
            })
    }

    fn any_magus_sister_damaged(&self) -> bool {
        [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .filter_map(|character| self.character_actor(character))
            .any(|actor| actor.current_hp < actor.effective_max_hp())
    }

    fn first_damaged_magus_sister_except(&self, user: Character) -> Option<Character> {
        [Character::Cindy, Character::Sandy, Character::Mindy]
            .into_iter()
            .filter(|character| *character != user)
            .find(|character| {
                self.character_actor(*character).is_some_and(|actor| {
                    actor.current_hp < actor.effective_max_hp()
                        && !actor.statuses.contains(&Status::Death)
                        && !actor.statuses.contains(&Status::Eject)
                })
            })
    }

    fn mindy_are_you_all_right_available(&self, rng: &mut FfxRngTracker) -> bool {
        let Some(mindy) = self.character_actor(Character::Mindy) else {
            return false;
        };
        let max_hp = mindy.effective_max_hp().max(1);
        let max_mp = mindy.effective_max_mp().max(1);
        if mindy.current_hp * 4 / max_hp == 0 || mindy.current_mp * 4 / max_mp == 0 {
            magus_rng_check(rng, 200)
        } else if mindy.current_hp * 2 / max_hp == 0 || mindy.current_mp * 2 / max_mp == 0 {
            magus_rng_check(rng, 128)
        } else {
            false
        }
    }

    fn apply_magus_dismiss(&mut self, sister: Character) -> String {
        let actor_id = ActorId::Character(sister);
        self.process_start_of_turn(actor_id);
        let spent_ctb = self.apply_actor_turn(actor_id, 3).unwrap_or(0);
        self.apply_status_to_actor(actor_id, Status::Eject);
        self.process_end_of_turn(actor_id);
        self.apply_magus_on_dismiss_to_all_sisters();
        format!(
            "{} -> Dismiss [{spent_ctb}] | {}",
            sister.display_name(),
            self.current_battle_state().ctb_order_string()
        )
    }

    fn apply_magus_on_dismiss_to_all_sisters(&mut self) {
        for sister in [Character::Cindy, Character::Sandy, Character::Mindy] {
            let Some(actor) = self.actor_mut(ActorId::Character(sister)) else {
                continue;
            };
            if actor.statuses.contains(&Status::Death) {
                actor.remove_status(Status::Death);
                actor.current_hp = 1;
            }
            actor.ctb = actor.base_ctb() * 3;
        }
    }

    fn apply_magus_taking_break(&mut self, sister: Character) -> String {
        let actor_id = ActorId::Character(sister);
        let mut output_lines = self.advance_virtual_turns_before(actor_id);
        self.process_start_of_turn(actor_id);
        let Some(spent_ctb) = self.apply_actor_turn(actor_id, 3) else {
            output_lines.push(format!("Error: Unknown actor for action: {sister}"));
            return output_lines.join("\n");
        };
        let results = vec![ActionEffectResult::new(actor_id)];
        self.process_end_of_turn(actor_id);
        ensure_blank_line(&mut output_lines);
        output_lines.push(format_action_output(
            sister.display_name(),
            "Taking a break...",
            spent_ctb,
            &results,
            self,
            None,
        ));
        output_lines.join("\n")
    }

    fn change_encounters_count(&mut self, name: &str, amount: &str) -> String {
        match name {
            "total" => {
                let Ok(count) = parse_amount(amount, self.encounters_count, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                self.encounters_count = count;
                self.calculate_aeon_stats();
                format!("Total encounters count set to {count}")
            }
            "random" => {
                let Ok(count) = parse_amount(amount, self.random_encounters_count, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                self.random_encounters_count = count;
                format!("Random encounters count set to {count}")
            }
            zone => {
                let Some(display_name) = data::random_zone_display_name(zone) else {
                    return "Error: Usage: encounters_count [total/random/zone name] [(+/-)amount]"
                        .to_string();
                };
                let current = *self.zone_encounters_counts.get(zone).unwrap_or(&0);
                let Ok(count) = parse_amount(amount, current, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                self.zone_encounters_counts.insert(zone.to_string(), count);
                format!("{display_name} encounters count set to {count}")
            }
        }
    }

    fn calculate_aeon_stats(&mut self) {
        let Some(yuna) = self.character_actor(Character::Yuna) else {
            return;
        };
        let yuna_stats = AeonStatBlock {
            hp: yuna.max_hp.min(9_999),
            mp: yuna.max_mp.min(999),
            strength: yuna.combat_stats.strength,
            defense: yuna.combat_stats.defense,
            magic: yuna.combat_stats.magic,
            magic_defense: yuna.combat_stats.magic_defense,
            agility: yuna.agility as i32,
            luck: yuna.combat_stats.luck,
            evasion: yuna.combat_stats.evasion,
            accuracy: yuna.combat_stats.accuracy,
        };
        let enc_tier = ((self.encounters_count - 30) / 30).clamp(0, 19) as usize;
        let encounter_stats = ENCOUNTER_YUNA_STATS[enc_tier];
        let yuna_power = aeon_power_base(yuna_stats);
        let encounter_power = aeon_power_base(encounter_stats);
        for formula in AEON_STAT_FORMULAS {
            let bonus = self
                .bonus_aeon_stats
                .get(&formula.character)
                .copied()
                .unwrap_or_default();
            let Some(actor) = self.actor_mut(ActorId::Character(formula.character)) else {
                continue;
            };
            actor.max_hp = aeon_formula_value(yuna_stats.hp, yuna_power, formula.hp)
                .max(aeon_formula_value(
                    encounter_stats.hp,
                    encounter_power,
                    formula.hp,
                ))
                .saturating_add(bonus.hp)
                .clamp(0, 99_999);
            actor.max_mp = aeon_formula_value(yuna_stats.mp, yuna_power, formula.mp)
                .max(aeon_formula_value(
                    encounter_stats.mp,
                    encounter_power,
                    formula.mp,
                ))
                .saturating_add(bonus.mp)
                .clamp(0, 9_999);
            actor.combat_stats.strength = aeon_calculated_stat(
                yuna_stats.strength,
                encounter_stats.strength,
                yuna_power,
                encounter_power,
                formula.strength,
                bonus.strength,
            );
            actor.combat_stats.defense = aeon_calculated_stat(
                yuna_stats.defense,
                encounter_stats.defense,
                yuna_power,
                encounter_power,
                formula.defense,
                bonus.defense,
            );
            actor.combat_stats.magic = aeon_calculated_stat(
                yuna_stats.magic,
                encounter_stats.magic,
                yuna_power,
                encounter_power,
                formula.magic,
                bonus.magic,
            );
            actor.combat_stats.magic_defense = aeon_calculated_stat(
                yuna_stats.magic_defense,
                encounter_stats.magic_defense,
                yuna_power,
                encounter_power,
                formula.magic_defense,
                bonus.magic_defense,
            );
            actor.agility = aeon_calculated_stat(
                yuna_stats.agility,
                encounter_stats.agility,
                yuna_power,
                encounter_power,
                formula.agility,
                bonus.agility,
            ) as u8;
            actor.combat_stats.luck = (yuna_stats.luck + bonus.luck).clamp(0, 255);
            actor.combat_stats.evasion = aeon_calculated_stat(
                yuna_stats.evasion,
                encounter_stats.evasion,
                yuna_power,
                encounter_power,
                formula.evasion,
                bonus.evasion,
            );
            actor.combat_stats.accuracy = aeon_calculated_stat(
                yuna_stats.accuracy,
                encounter_stats.accuracy,
                yuna_power,
                encounter_power,
                formula.accuracy,
                bonus.accuracy,
            );
            actor.current_hp = actor.current_hp.min(actor.effective_max_hp());
            actor.current_mp = actor.current_mp.min(actor.effective_max_mp());
        }
    }

    fn reset_aeon_current_resources(&mut self) {
        for formula in AEON_STAT_FORMULAS {
            let character = formula.character;
            if let Some(actor) = self.actor_mut(ActorId::Character(character)) {
                actor.current_hp = actor.effective_max_hp();
                actor.current_mp = actor.effective_max_mp();
            }
        }
    }

    fn encounter_checks(
        &mut self,
        zone_name: &str,
        steps: &str,
        continue_previous_zone: bool,
    ) -> String {
        let Some(zone) = data::random_zone_stats(zone_name) else {
            return format!("Error: No zone named \"{zone_name}\"");
        };
        let Ok(steps) = steps.parse::<i32>() else {
            return "Error: Step must be an integer".to_string();
        };
        if !continue_previous_zone {
            self.live_distance = 0;
        }
        let mut remaining_distance = steps * 10;
        let mut checks = Vec::new();
        while remaining_distance > 0 {
            let check = self.encounter_check(&zone, remaining_distance);
            remaining_distance -= check.distance;
            checks.push(check);
        }

        let encounters_count = checks.iter().filter(|check| check.encounter).count();
        let mut total_distance = 0;
        let mut encounters = Vec::new();
        for check in &checks {
            if check.encounter {
                total_distance += check.distance;
                encounters.push((total_distance / 10).to_string());
            }
        }
        let distance_before_end = if checks.len() == encounters_count {
            0
        } else {
            checks
                .last()
                .map(|check| check.distance)
                .unwrap_or_default()
        };
        format!(
            "Encounter checks: {} ({}) | {encounters_count} Encounters | {} | {} steps before end of the zone",
            zone.display_name,
            zone.grace_period,
            encounters.join(", "),
            distance_before_end / 10
        )
    }

    fn encounter_check(
        &mut self,
        zone: &data::RandomZoneStats,
        max_distance: i32,
    ) -> EncounterCheck {
        let max_steps = (self.live_distance + max_distance) / 10 - zone.grace_period;
        let starting_steps = (self.live_distance / 10 - zone.grace_period).max(0);
        let check = if max_steps <= 0 {
            EncounterCheck {
                encounter: false,
                distance: max_distance,
            }
        } else {
            let mut result = EncounterCheck {
                encounter: false,
                distance: max_distance,
            };
            for steps in (starting_steps + 1)..=max_steps {
                let rng_roll = self.rng.advance_rng(0) & 255;
                let counter = steps * 256 / zone.threat_modifier;
                if rng_roll < counter as u32 {
                    result = EncounterCheck {
                        encounter: true,
                        distance: ((zone.grace_period + steps) * 10) - self.live_distance,
                    };
                    break;
                }
            }
            result
        };

        self.live_distance = if check.encounter { 0 } else { check.distance };
        check
    }

    fn render_encounter_command(
        &mut self,
        name: &str,
        multizone: bool,
        zones: &[String],
    ) -> String {
        let display_line = if multizone {
            let zone_suffix = if zones.is_empty() {
                String::new()
            } else {
                format!(" {}", zones.join(" "))
            };
            format!("encounter multizone{zone_suffix}")
        } else {
            format!("encounter {name}")
        };
        let summary = self.start_encounter(name, multizone, zones);
        if summary.starts_with("Error:") {
            return summary;
        }
        let header_name = self
            .current_encounter_name
            .as_deref()
            .unwrap_or(name)
            .replace('_', " ");
        format!(
            "{display_line}\n# ===== {} =====\n# {summary}",
            titlecase_encounter_header(&header_name)
        )
    }

    fn start_encounter(&mut self, name: &str, multizone: bool, zones: &[String]) -> String {
        if multizone {
            return self.start_multizone_encounter(zones);
        }
        let encounter_name = normalize_encounter_name(name);
        let boss_or_simulated_formation = data::boss_or_simulated_formation(&encounter_name);
        let is_random_zone = data::has_random_zone(&encounter_name);
        if boss_or_simulated_formation.is_none() && !is_random_zone {
            return format!("Error: No encounter named \"{encounter_name}\"");
        }

        let had_prior_encounter = self.encounters_count > 0 || self.random_encounters_count > 0;
        self.process_start_of_encounter();
        let formation = boss_or_simulated_formation.or_else(|| {
            let formation_roll = self.rng.advance_rng(1);
            data::random_formation(&encounter_name, formation_roll)
        });
        if let Some(forced_party) = formation
            .as_ref()
            .and_then(|formation| formation.forced_party.as_deref())
        {
            self.set_party_from_initials(forced_party);
        }
        self.apply_scripted_starting_party(&encounter_name);
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
        let is_simulated = formation
            .as_ref()
            .is_some_and(|formation| formation.is_simulated);
        if !is_simulated {
            self.encounters_count += 1;
        }
        let random_indices = if formation
            .as_ref()
            .is_some_and(|formation| formation.is_random)
            && !multizone
        {
            self.random_encounters_count += 1;
            let zone_count = self
                .zone_encounters_counts
                .entry(encounter_name.clone())
                .and_modify(|count| *count += 1)
                .or_insert(1);
            Some((self.random_encounters_count, *zone_count))
        } else {
            None
        };
        self.create_monster_party(
            &encounter_name,
            formation
                .as_ref()
                .map(|formation| formation.monsters.as_slice()),
        );
        self.current_formation_monsters = formation
            .as_ref()
            .map(|formation| formation.monsters.clone())
            .unwrap_or_default();
        self.current_encounter_name = Some(encounter_name.clone());
        self.current_encounter_condition = Some(condition);
        self.scripted_turn_index = 0;
        self.respawn_wave_index = 1;
        self.sahagin_fourth_unlocked = false;
        self.set_party_icvs(condition);
        self.set_monster_icvs(condition);
        let normalize_before_summary = !had_prior_encounter || encounter_name == "sahagin_chiefs";
        if normalize_before_summary {
            self.normalize_ctbs();
        }
        let ctb_summary = self.current_battle_state().ctb_order_string();
        if had_prior_encounter && !normalize_before_summary {
            self.normalize_ctbs();
        }

        let (prefix, indices) = if is_simulated {
            (
                "Simulated Encounter",
                format!("{:>3}", self.encounters_count),
            )
        } else if let Some((random_index, zone_index)) = random_indices {
            (
                "Random Encounter",
                format!(
                    "{:>3} {:>3} {:>3}",
                    self.encounters_count, random_index, zone_index
                ),
            )
        } else if formation
            .as_ref()
            .is_some_and(|formation| formation.is_random)
            || multizone
        {
            (
                "Multizone encounter",
                format!("{:>3}", self.encounters_count),
            )
        } else {
            ("Encounter", format!("{:>3}", self.encounters_count))
        };
        let display_name = formation
            .as_ref()
            .map(|formation| formation.display_name.as_str())
            .unwrap_or(encounter_name.as_str());
        if prefix == "Random Encounter" {
            let zone_name = formation
                .as_ref()
                .and_then(|formation| formation.zone_display_name.as_deref())
                .unwrap_or(display_name);
            format!(
                "{prefix}: {indices} | {zone_name} | {display_name} {} | {ctb_summary}",
                format_condition(condition)
            )
        } else {
            let active_monster_keys = self
                .monsters
                .iter()
                .filter_map(|monster| monster.monster_key.clone())
                .collect::<Vec<_>>();
            let condition_name = formation.as_ref().map_or_else(
                || format_condition(condition).to_string(),
                |formation| {
                    let monster_keys =
                        if active_monster_keys.is_empty() || formation.monsters.is_empty() {
                            &formation.monsters
                        } else {
                            &active_monster_keys
                        };
                    format!(
                        "{} {}",
                        data::format_monster_list(monster_keys),
                        format_condition(condition)
                    )
                },
            );
            format!("{prefix}: {indices} | {display_name} | {condition_name} | {ctb_summary}")
        }
    }

    fn start_multizone_encounter(&mut self, zones: &[String]) -> String {
        if zones.is_empty() {
            return "Error: Usage: encounter multizone [zones...]".to_string();
        }
        for zone in zones {
            if !data::has_random_zone(zone) {
                return format!("Error: No zone named \"{zone}\"");
            }
        }

        let saved_rng = self.rng.clone();
        let mut first_prefix = String::new();
        let mut zone_names = Vec::new();
        let mut formations = Vec::new();
        let mut ctb_order = String::new();
        let zones_count = zones.len();

        for (index, zone) in zones.iter().enumerate() {
            if index > 0 {
                self.rng = saved_rng.clone();
            }
            let output = self.start_encounter(zone, false, &[]);
            let parts = output.split('|').map(str::trim).collect::<Vec<_>>();
            if index == 0 {
                first_prefix = output.split('|').next().unwrap_or_default().to_string();
            }
            zone_names.push(
                data::random_zone_display_name(zone)
                    .unwrap_or_else(|| normalize_encounter_name(zone)),
            );
            if let Some(formation) = parts.get(2) {
                formations.push((*formation).to_string());
            }
            if let Some(ctb) = parts.get(3) {
                ctb_order = (*ctb).to_string();
            }

            if index + 1 < zones_count {
                self.encounters_count -= 1;
                self.random_encounters_count -= 1;
                self.rng = saved_rng.clone();
            }
        }

        format!(
            "{}| {} | {} | {}",
            first_prefix,
            zone_names.join("/"),
            formations.join(" | "),
            ctb_order
        )
    }

    fn process_start_of_encounter(&mut self) {
        self.ctb_since_last_action = 0;
        self.magus_last_commands.clear();
        self.magus_last_actions.clear();
        self.magus_last_action_lists.clear();
        self.temporary_monsters.clear();
        self.retired_temporary_monsters.clear();
        self.current_encounter_name = None;
        self.current_encounter_condition = None;
        self.current_formation_monsters.clear();
        self.scripted_turn_index = 0;
        self.respawn_wave_index = 0;
        self.sahagin_fourth_unlocked = false;
        self.clear_previous_encounter_monster_memory();
        for actor in &mut self.character_actors {
            if actor.statuses.contains(&Status::Death) {
                actor.current_hp = 1;
            }
            actor.ctb = 0;
            actor.buffs.clear();
            actor.clear_statuses();
            apply_auto_statuses(actor);
        }
        self.calculate_aeon_stats();
        self.monsters.clear();
    }

    fn clear_previous_encounter_monster_memory(&mut self) {
        if matches!(self.last_actor, Some(ActorId::Monster(_))) {
            self.last_actor = None;
        }
        self.last_targets
            .retain(|target| !matches!(target, ActorId::Monster(_)));
        for targets in self.actor_last_targets.values_mut() {
            targets.retain(|target| !matches!(target, ActorId::Monster(_)));
        }
        self.actor_last_targets
            .retain(|actor, _| !matches!(actor, ActorId::Monster(_)));
        self.actor_last_attackers.retain(|actor, attacker| {
            !matches!(actor, ActorId::Monster(_)) && !matches!(attacker, ActorId::Monster(_))
        });
        self.actor_provokers.retain(|actor, provoker| {
            !matches!(actor, ActorId::Monster(_)) && !matches!(provoker, ActorId::Monster(_))
        });
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
            actor.max_mp = template.max_mp;
            actor.current_mp = template.max_mp;
            actor.set_combat_stats(template.combat_stats);
            actor.set_elemental_affinities(template.elemental_affinities.clone());
            actor.set_status_resistances(template.status_resistances.clone());
            apply_damage_traits(&mut actor, template);
            apply_template_auto_statuses(&mut actor, template);
            self.monsters.push(actor);
        }
        self.advance_duplicate_monster_rngs(&templates);
        self.apply_scripted_starting_monster_party_shape(encounter_name);
    }

    fn sync_monster_party_from_generated_encounter_comment(&mut self, line: &str) -> bool {
        if self.current_encounter_name.is_none() || !self.monsters.iter().all(BattleActor::is_alive)
        {
            return false;
        }
        let Some((monster_keys, ctb_summary)) = parse_generated_encounter_comment_party(line)
        else {
            return false;
        };
        let mut monsters = Vec::new();
        for (index, monster_key) in monster_keys.iter().enumerate() {
            let slot = MonsterSlot(index + 1);
            let Some(mut actor) = create_monster_actor(monster_key, slot) else {
                return false;
            };
            if let Some(ctb) = ctb_summary.monsters.get(&slot).copied() {
                actor.ctb = ctb;
            }
            monsters.push(actor);
        }
        for (character, ctb) in ctb_summary.characters {
            if let Some(actor) = self.actor_mut(ActorId::Character(character)) {
                actor.ctb = ctb;
            }
        }
        self.current_formation_monsters = monster_keys.clone();
        self.monsters = monsters;
        true
    }

    fn materialize_formation_monster_slot(&mut self, target_name: &str) -> Option<MonsterSlot> {
        let slot = target_name.parse::<MonsterSlot>().ok()?;
        if self.actor(ActorId::Monster(slot)).is_some() {
            return Some(slot);
        }
        let monster_key = self
            .current_formation_monsters
            .get(slot.0.checked_sub(1)?)
            .cloned()?;
        let mut actor = create_monster_actor(&monster_key, slot)?;
        actor.ctb = i32::MAX / 4;
        self.monsters.push(actor);
        Some(slot)
    }

    fn apply_scripted_starting_monster_party_shape(&mut self, encounter_name: &str) {
        match encounter_name {
            "sahagin_chiefs" | "sinscales" if self.monsters.len() > 3 => {
                self.monsters.truncate(3);
            }
            _ => {}
        }
    }

    fn apply_scripted_starting_party(&mut self, encounter_name: &str) {
        if encounter_name == "geneaux" {
            self.party = vec![Character::Tidus, Character::Yuna, Character::Lulu];
        }
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
                    if matches!(actor.id, ActorId::Character(Character::Unknown)) {
                        continue;
                    }
                    let character = character_id(actor);
                    let active = party.contains(&character);
                    let has_first_strike = actor.has_auto_ability(AutoAbility::FirstStrike);
                    if !active && !matches!(character, Character::Sandy | Character::Mindy) {
                        continue;
                    }
                    if active && has_first_strike {
                        continue;
                    }
                    actor.ctb = apply_haste_to_ctb(actor, actor.base_ctb() * 3);
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
                    let character = character_id(actor);
                    let active = party.contains(&character);
                    let has_first_strike = actor.has_auto_ability(AutoAbility::FirstStrike);
                    if !matches!(character, Character::Sandy | Character::Mindy)
                        && (!active || has_first_strike)
                    {
                        continue;
                    }
                    let variance = ICV_VARIANCE[actor.agility as usize] as u32 + 1;
                    let raw_ctb = actor.base_ctb() * 3 - (variance_roll % variance) as i32;
                    actor.ctb = apply_haste_to_ctb(actor, raw_ctb);
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

    fn render_character_action_command(
        &mut self,
        actor: Character,
        action: &str,
        args: &[String],
        raw_line: &str,
    ) -> String {
        let display_args = self.display_character_action_args(actor, action, args);
        let outcome = self.apply_character_action_outcome(actor, action, args);
        if !outcome.command_echoable
            || outcome.output.starts_with("Error:")
            || outcome.output.starts_with("# skipped:")
        {
            return render_character_action_outcome(outcome);
        }
        let mut output_lines = outcome.pre_lines;
        if !output_lines.is_empty() {
            ensure_blank_line(&mut output_lines);
        }
        let preserve_raw_line = (display_args == args
            && raw_line
                .chars()
                .next()
                .is_some_and(|character| character.is_whitespace()))
            || (args.is_empty()
                && raw_line
                    .chars()
                    .last()
                    .is_some_and(|character| character.is_whitespace()));
        let display_line = if preserve_raw_line {
            raw_line.to_string()
        } else {
            let display_args = if args
                .first()
                .is_some_and(|arg| monster_family(arg) == "worker")
            {
                outcome
                    .damage_comment
                    .as_deref()
                    .and_then(first_monster_slot_in_comment)
                    .map(|slot| {
                        let mut args = args.to_vec();
                        args[0] = format!("m{}", slot.0);
                        args
                    })
                    .unwrap_or(display_args)
            } else {
                display_args
            };
            format_character_action_line(actor, action, &display_args)
        };
        output_lines.push(display_line);
        if let Some(comment) = outcome.damage_comment {
            output_lines.push(comment);
        }
        output_lines.join("\n")
    }

    fn display_character_action_args(
        &self,
        actor: Character,
        action: &str,
        args: &[String],
    ) -> Vec<String> {
        let mut display_args = args.to_vec();
        let Some(first_arg) = display_args.first_mut() else {
            return display_args;
        };
        if first_arg.parse::<MonsterSlot>().is_ok() {
            return display_args;
        }
        let Some(action_data) = self.action_data_for_actor(ActorId::Character(actor), action)
        else {
            return display_args;
        };
        if !action_target_accepts_explicit_monster(&action_data.target) {
            return display_args;
        }
        if let Some(slot) = self.resolve_existing_monster_target_arg(first_arg) {
            *first_arg = format!("m{}", slot.0);
        }
        display_args
    }

    fn apply_character_action(
        &mut self,
        actor: Character,
        action: &str,
        args: &[String],
    ) -> String {
        let outcome = self.apply_character_action_outcome(actor, action, args);
        render_character_action_outcome(outcome)
    }

    fn apply_character_action_outcome(
        &mut self,
        actor: Character,
        action: &str,
        args: &[String],
    ) -> CharacterActionOutcome {
        let normalized_args;
        let args = if action == "use_crane"
            && args.is_empty()
            && self.current_encounter_name.as_deref() == Some("oblitzerator")
        {
            normalized_args = vec!["m2".to_string()];
            normalized_args.as_slice()
        } else {
            args
        };
        let actor_id = ActorId::Character(actor);
        let Some(action_data) = self.action_data_for_actor(actor_id, action) else {
            if !action.ends_with("_counter") {
                if let Some(actor_state) = self.actor(actor_id) {
                    if !actor_state.can_take_turn() {
                        return CharacterActionOutcome::from_output(skipped_action_comment(
                            &format_character_action_line(actor, action, args),
                            actor_state,
                        ));
                    }
                }
            }
            return CharacterActionOutcome::from_output(format!(
                "Error: No action named \"{action}\""
            ));
        };
        let availability_ignored_action = matches!(action, "defend" | "escape" | "flee");
        if !action.ends_with("_counter") && !availability_ignored_action {
            if let Some(actor_state) = self.actor(actor_id) {
                if !actor_state.can_take_turn() {
                    return CharacterActionOutcome::from_output(skipped_action_comment(
                        &format_character_action_line(actor, action, args),
                        actor_state,
                    ));
                }
            }
        }
        if !action_data.can_use_in_combat {
            return CharacterActionOutcome::from_output(format!(
                "Error: Action {} can't be used in battle",
                display_action_name(action)
            ));
        }
        if let Some(target_name) = args.first() {
            self.materialize_formation_monster_slot(target_name);
        }
        let prepared_temporary_target = self.prepare_temporary_explicit_target(&action_data, args);
        if let Some(message) = self.required_character_target_error(action, &action_data, args) {
            if let Some(actor_id) = prepared_temporary_target {
                self.remove_temporary_actor(actor_id);
            }
            return CharacterActionOutcome::from_output(message);
        }
        let bribe_gil = if action.eq_ignore_ascii_case("bribe") {
            match parse_bribe_action_gil(args) {
                Ok(gil) => Some(gil),
                Err(message) => {
                    if let Some(actor_id) = prepared_temporary_target {
                        self.remove_temporary_actor(actor_id);
                    }
                    return CharacterActionOutcome::from_output(message);
                }
            }
        } else {
            None
        };
        let is_counter = action_is_counter(&action_data);
        let mut output_lines = Vec::new();
        let scripted_applied = if is_counter {
            false
        } else {
            self.maybe_apply_scripted_pre_action(&mut output_lines)
        };
        if !is_counter && !scripted_applied {
            output_lines.extend(self.advance_virtual_turns_before(actor_id));
        }
        if !is_counter && is_aeon_character(actor) && !self.party.contains(&actor) {
            self.party = vec![actor];
        }
        let rank = action_data.rank;
        if let Some(gil) = bribe_gil {
            self.process_start_of_turn(actor_id);
            let Some(spent_ctb) = self.apply_actor_turn(actor_id, rank) else {
                output_lines.push(format!("Error: Unknown actor for action: {actor}"));
                return CharacterActionOutcome {
                    pre_lines: Vec::new(),
                    output: output_lines.join("\n"),
                    damage_comment: None,
                    command_echoable: false,
                };
            };
            ensure_blank_line(&mut output_lines);
            output_lines.push(self.apply_bribe_action(actor, args, gil, spent_ctb));
            self.process_end_of_turn(actor_id);
            if let Some(actor_id) = prepared_temporary_target {
                self.retire_temporary_actor(actor_id);
            }
            return CharacterActionOutcome {
                pre_lines: Vec::new(),
                output: output_lines.join("\n"),
                damage_comment: None,
                command_echoable: false,
            };
        }
        if !is_counter {
            self.process_start_of_turn(actor_id);
        }
        let is_escape = action.eq_ignore_ascii_case("escape");
        let escape_succeeded = is_escape
            .then(|| self.escape_succeeds(actor_id))
            .unwrap_or(false);
        let spent_ctb = if is_counter {
            0
        } else {
            let Some(spent_ctb) = self.apply_actor_turn(actor_id, rank) else {
                output_lines.push(format!("Error: Unknown actor for action: {actor}"));
                return CharacterActionOutcome {
                    pre_lines: Vec::new(),
                    output: output_lines.join("\n"),
                    damage_comment: None,
                    command_echoable: false,
                };
            };
            spent_ctb
        };
        if escape_succeeded {
            self.apply_status_to_actor(actor_id, Status::Eject);
        }
        if is_escape {
            if !is_counter {
                self.process_end_of_turn(actor_id);
            }
            if let Some(actor_id) = prepared_temporary_target {
                self.retire_temporary_actor(actor_id);
            }
            let result = if escape_succeeded {
                "Succeeded"
            } else {
                "Failed"
            };
            return CharacterActionOutcome {
                pre_lines: output_lines,
                output: format!("{} -> Escape [{spent_ctb}]: {result}", actor.display_name()),
                damage_comment: None,
                command_echoable: false,
            };
        }
        let results = self.apply_action_effects(actor_id, Some(&action_data), args);
        if !is_counter {
            self.process_end_of_turn(actor_id);
        }
        let damage_comment =
            format_damage_comment("# party rolls: ", &results, self, Some(&action_data));
        let output = format_action_output(
            actor.display_name(),
            &display_action_name(action),
            spent_ctb,
            &results,
            self,
            Some(&action_data),
        );
        if let Some(actor_id) = prepared_temporary_target {
            self.retire_temporary_actor(actor_id);
        }
        CharacterActionOutcome {
            pre_lines: output_lines,
            output,
            damage_comment,
            command_echoable: true,
        }
    }

    fn prepare_temporary_explicit_target(
        &mut self,
        action: &ActionData,
        args: &[String],
    ) -> Option<ActorId> {
        if !matches!(
            action.target,
            ActionTarget::Single
                | ActionTarget::SingleMonster
                | ActionTarget::EitherParty
                | ActionTarget::CharactersParty
                | ActionTarget::MonstersParty
                | ActionTarget::RandomCharacter
                | ActionTarget::RandomMonster
        ) {
            return None;
        }
        let Some(target_name) = args.first() else {
            return None;
        };
        if matches!(target_name.as_str(), "party" | "monsters")
            || self.resolve_existing_actor_id(target_name).is_some()
        {
            return None;
        }
        if data::monster_stats(target_name).is_none()
            && self.resolve_actor_id(target_name).is_some()
        {
            return None;
        }
        let slot = self.next_temporary_monster_slot();
        let Some(mut actor) = create_monster_actor(target_name, slot) else {
            return None;
        };
        actor.temporary = true;
        actor.display_slot = Some(MonsterSlot(1));
        actor.index = 0;
        let actor_id = actor.id;
        self.temporary_monsters.push(actor);
        Some(actor_id)
    }

    fn remove_temporary_actor(&mut self, actor_id: ActorId) {
        self.temporary_monsters.retain(|actor| actor.id != actor_id);
    }

    fn retire_temporary_actor(&mut self, actor_id: ActorId) {
        let Some(index) = self
            .temporary_monsters
            .iter()
            .position(|actor| actor.id == actor_id)
        else {
            return;
        };
        let actor = self.temporary_monsters.remove(index);
        self.retired_temporary_monsters.push(actor);
    }

    fn apply_bribe_action(
        &mut self,
        user: Character,
        args: &[String],
        gil: i32,
        spent_ctb: i32,
    ) -> String {
        let target_id = args
            .first()
            .and_then(|target| self.resolve_monster_target_arg(target))
            .map(ActorId::Monster);
        let Some(target_id) = target_id else {
            return format!(
                "{} -> Bribe [{spent_ctb}]: Unknown -> Fail | Total Gil spent: 0",
                user.display_name()
            );
        };
        let Some(target) = self.actor(target_id).cloned() else {
            return format!(
                "{} -> Bribe [{spent_ctb}]: {} -> Fail | Total Gil spent: 0",
                user.display_name(),
                actor_label_for_id(target_id)
            );
        };
        self.record_action_target_memory(ActorId::Character(user), vec![target_id], false);
        let monster_label = actor_label(&target);
        let bribe_item = target
            .monster_key
            .as_deref()
            .and_then(data::monster_stats)
            .and_then(|monster| monster.bribe);
        let item = self.roll_bribe_item(user, &target, gil, bribe_item.as_ref());
        let total_gil_spent = if let Some(actor) = self.actor_mut(target_id) {
            if gil >= 1 {
                actor.bribe_gil_spent = ((actor.bribe_gil_spent as i64 + gil as i64)
                    .clamp(0, MAX_BRIBE_GIL_SPENT as i64))
                    as i32;
            }
            if item.is_some() {
                actor.set_status(Status::Eject, 254);
            }
            actor.bribe_gil_spent
        } else {
            0
        };
        let result = item
            .as_ref()
            .map(format_item_drop)
            .unwrap_or_else(|| "Fail".to_string());
        format!(
            "{} -> Bribe [{spent_ctb}]: {monster_label} -> {result} | Total Gil spent: {total_gil_spent}",
            user.display_name()
        )
    }

    fn roll_bribe_item(
        &mut self,
        user: Character,
        target: &BattleActor,
        gil: i32,
        bribe_item: Option<&data::ItemDrop>,
    ) -> Option<data::ItemDrop> {
        let bribe_item = bribe_item?;
        let chance_rng = self.rng.advance_rng(self.status_rng_index(target.id));
        if gil < 1 || target.immune_to_bribe || target.statuses.contains(&Status::Sleep) {
            return None;
        }
        let total_gil = gil as i64 + target.bribe_gil_spent as i64;
        let chance = ((total_gil * 256) as f64 / target.max_hp.max(1) as f64 / 20.0) as i32 - 64;
        if (chance_rng & 255) as i32 >= chance {
            return None;
        }
        let rng_index = self
            .actor(ActorId::Character(user))
            .map(|actor| (20 + actor.index).min(27))
            .unwrap_or(20);
        let variance_rng = self.rng.advance_rng(rng_index);
        let rounding_rng = self.rng.advance_rng(rng_index);
        let quantity = ((chance.max(0) as f64).sqrt()
            * f64::from(bribe_item.quantity)
            * 0.0625
            * f64::from((variance_rng % 11) + 20)
            / 25.0
            + f64::from(rounding_rng & 1)) as i32;
        Some(data::ItemDrop {
            item: bribe_item.item.clone(),
            quantity: quantity.clamp(1, 99) as u8,
            rare: false,
        })
    }

    #[cfg(test)]
    fn apply_monster_action(&mut self, slot: MonsterSlot, action: &str, args: &[String]) -> String {
        let outcome = self.apply_monster_action_outcome(slot, action, args, true);
        append_optional_output_line(outcome.output, outcome.damage_comment)
    }

    fn render_monster_action_command(
        &mut self,
        actor: MonsterActionActor,
        action: &str,
        args: &[String],
        raw_line: &str,
    ) -> String {
        let actor_is_slot = matches!(actor, MonsterActionActor::Slot(_));
        let (display_line, outcome) = match actor {
            MonsterActionActor::Slot(slot) => (
                format_monster_action_line(slot, action, args),
                self.apply_monster_action_outcome(slot, action, args, true),
            ),
            MonsterActionActor::Name(monster) => (
                format_named_monster_action_line(&monster, action, args),
                self.apply_named_monster_action_outcome(&monster, action, args),
            ),
        };
        if outcome.output.starts_with("Error: Available actions") && actor_is_slot {
            return raw_line.to_string();
        }
        if outcome.output.starts_with("Error:") {
            return outcome.output;
        }
        if outcome.output.starts_with("# skipped:") {
            return outcome.output;
        }
        let display_line = if raw_line
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace())
        {
            raw_line.to_string()
        } else {
            display_line
        };
        append_optional_output_line(display_line, outcome.damage_comment)
    }

    #[cfg(test)]
    fn apply_named_monster_action(
        &mut self,
        monster_name: &str,
        action: &str,
        args: &[String],
    ) -> String {
        let outcome = self.apply_named_monster_action_outcome(monster_name, action, args);
        append_optional_output_line(outcome.output, outcome.damage_comment)
    }

    fn apply_named_monster_action_outcome(
        &mut self,
        monster_name: &str,
        action: &str,
        args: &[String],
    ) -> MonsterActionOutcome {
        let slot = self.next_temporary_monster_slot();
        let Some(mut actor) = create_monster_actor(monster_name, slot) else {
            return MonsterActionOutcome::from_output(format!(
                "Error: No monster name or slot named \"{monster_name}\""
            ));
        };
        actor.temporary = true;
        actor.display_slot = Some(MonsterSlot(1));
        actor.index = 0;
        let actor_id = actor.id;
        self.temporary_monsters.push(actor);
        let outcome = self.apply_monster_action_outcome(slot, action, args, true);
        let errored = outcome.output.starts_with("Error:");
        if errored {
            self.remove_temporary_actor(actor_id);
        } else {
            self.retire_temporary_actor(actor_id);
        }
        outcome
    }

    fn next_temporary_monster_slot(&self) -> MonsterSlot {
        let next_slot = self
            .temporary_monsters
            .iter()
            .chain(self.retired_temporary_monsters.iter())
            .filter_map(|actor| match actor.id {
                ActorId::Monster(slot) => Some(slot.0 + 1),
                ActorId::Character(_) => None,
            })
            .max()
            .unwrap_or(9);
        MonsterSlot(next_slot.max(9))
    }

    fn apply_monster_action_outcome(
        &mut self,
        slot: MonsterSlot,
        action: &str,
        _args: &[String],
        validate_action_table: bool,
    ) -> MonsterActionOutcome {
        self.apply_monster_action_outcome_inner(slot, action, _args, validate_action_table, true)
    }

    fn apply_monster_action_outcome_inner(
        &mut self,
        slot: MonsterSlot,
        action: &str,
        _args: &[String],
        validate_action_table: bool,
        automatic_reactions: bool,
    ) -> MonsterActionOutcome {
        let actor_id = ActorId::Monster(slot);
        if validate_action_table {
            if self.actor(actor_id).is_none() {
                return MonsterActionOutcome::from_output(format!(
                    "Error: No monster in slot {}",
                    slot.0
                ));
            };
        } else {
            self.ensure_monster_slot(slot);
        }
        let action_name = if validate_action_table {
            match self.resolve_monster_action_name(actor_id, slot, action) {
                Ok(action_name) => action_name,
                Err(message) => {
                    if let Some(actor) = self.actor(actor_id) {
                        if !actor.can_take_turn() {
                            return MonsterActionOutcome::from_output(skipped_action_comment(
                                &format_monster_action_line(slot, action, _args),
                                actor,
                            ));
                        }
                    }
                    return MonsterActionOutcome::from_output(message);
                }
            }
        } else {
            action.to_string()
        };
        let action_data = self.action_data_for_actor(actor_id, &action_name);
        if let Some(action_data) = action_data.as_ref() {
            if !action_data.can_use_in_combat {
                return MonsterActionOutcome::from_output(format!(
                    "Error: Action {} can't be used in battle",
                    display_action_name(&action_name)
                ));
            }
        }
        let actor_name = self
            .actor(actor_id)
            .map(actor_label)
            .unwrap_or_else(|| format!("M{}", slot.0));
        let is_counter = action_data.as_ref().is_some_and(action_is_counter);
        if !is_counter {
            self.process_start_of_turn(actor_id);
        }
        let rank = action_data
            .as_ref()
            .map(|action| action.rank)
            .unwrap_or_else(|| fallback_action_rank(&action_name));
        let spent_ctb = if is_counter {
            0
        } else {
            let Some(spent_ctb) = self.apply_actor_turn(actor_id, rank) else {
                return MonsterActionOutcome::from_output(format!(
                    "Error: Unknown monster slot for action: m{}",
                    slot.0
                ));
            };
            spent_ctb
        };
        let results = self.apply_action_effects(actor_id, action_data.as_ref(), &[]);
        if !is_counter {
            self.process_end_of_turn(actor_id);
        }
        let resource_state = self.action_resource_state(actor_id, &results);
        let damage_comment =
            format_damage_comment("# enemy rolls: ", &results, self, action_data.as_ref());
        let mut output_lines = vec![format_action_output(
            &actor_name,
            &display_action_name(
                action_data
                    .as_ref()
                    .filter(|_| action_name == "forced_action")
                    .map(|action| action.key.as_str())
                    .unwrap_or(&action_name),
            ),
            spent_ctb,
            &results,
            self,
            action_data.as_ref(),
        )];
        if automatic_reactions && !is_counter {
            if let Some(action_data) = action_data.as_ref() {
                output_lines.extend(self.automatic_reaction_outputs(
                    actor_id,
                    action_data,
                    &results,
                ));
            }
        }
        MonsterActionOutcome {
            output: output_lines.join("\n"),
            damage_comment,
            resource_state,
        }
    }

    fn action_resource_state(
        &self,
        user: ActorId,
        results: &[ActionEffectResult],
    ) -> Vec<(ActorId, i32, i32)> {
        let mut actor_ids = vec![user];
        for result in results {
            if !actor_ids.contains(&result.target) {
                actor_ids.push(result.target);
            }
        }
        actor_ids
            .into_iter()
            .filter_map(|actor_id| {
                self.actor(actor_id)
                    .map(|actor| (actor_id, actor.current_hp, actor.current_mp))
            })
            .collect()
    }

    fn automatic_reaction_outputs(
        &mut self,
        user: ActorId,
        action: &ActionData,
        results: &[ActionEffectResult],
    ) -> Vec<String> {
        if !matches!(user, ActorId::Monster(_)) || action_is_counter(action) {
            return Vec::new();
        }
        let mut reactions: Vec<(Character, &'static str, Vec<String>)> = Vec::new();
        let mut dead_characters = Vec::new();
        for result in results {
            let ActorId::Character(character) = result.target else {
                continue;
            };
            let Some(actor) = self.actor(result.target) else {
                continue;
            };
            if result.auto_life_triggered {
                reactions.push((character, "auto-life_counter", Vec::new()));
                continue;
            }
            if actor.statuses.contains(&Status::Death) {
                dead_characters.push(character);
                continue;
            }
            if !actor.is_alive() {
                continue;
            }
            let has_hp_damage = result
                .damage
                .hp
                .as_ref()
                .is_some_and(|damage| damage.damage > 0);
            let counters = match action.damage_type {
                DamageType::Physical => {
                    (result.hit && actor.has_auto_ability(AutoAbility::Counterattack))
                        || (!result.hit && actor.has_auto_ability(AutoAbility::EvadeCounter))
                }
                DamageType::Magical => {
                    result.hit && actor.has_auto_ability(AutoAbility::MagicCounter)
                }
                DamageType::Other => false,
            };
            let uses_auto_potion =
                result.hit && has_hp_damage && actor.has_auto_ability(AutoAbility::AutoPotion);
            let uses_auto_med = result.hit
                && actor.has_auto_ability(AutoAbility::AutoMed)
                && actor
                    .statuses
                    .iter()
                    .any(|status| auto_med_removes(*status) && actor.status_stack(*status) < 255);
            if counters {
                reactions.push((character, "counter", Vec::new()));
            }
            if uses_auto_potion {
                if let Some(action) = self.consume_auto_potion_item() {
                    reactions.push((character, action, Vec::new()));
                }
            }
            if uses_auto_med && self.consume_inventory_item("Remedy", 1) {
                reactions.push((character, "auto_med", Vec::new()));
            }
        }

        for dead_character in dead_characters {
            let Some(rescuer) = self.party.iter().copied().find(|candidate| {
                *candidate != dead_character
                    && self
                        .actor(ActorId::Character(*candidate))
                        .is_some_and(|actor| {
                            actor.is_alive() && actor.has_auto_ability(AutoAbility::AutoPhoenix)
                        })
            }) else {
                continue;
            };
            if !self.consume_inventory_item("Phoenix Down", 1) {
                continue;
            }
            reactions.push((
                rescuer,
                "auto_phoenix",
                vec![dead_character.input_name().to_string()],
            ));
        }

        reactions
            .into_iter()
            .map(|(character, action, args)| {
                let output = self.apply_character_action(character, action, &args);
                if action == "auto-life_counter" {
                    self.apply_auto_life_revive(character);
                }
                output
            })
            .collect()
    }

    fn apply_auto_life_revive(&mut self, character: Character) {
        let ctb_since_last_action = self.ctb_since_last_action;
        let Some(actor) = self.actor_mut(ActorId::Character(character)) else {
            return;
        };
        actor.remove_status(Status::Death);
        actor.remove_status(Status::AutoLife);
        actor.current_hp = (actor.effective_max_hp() / 4).max(1);
        actor.ctb = actor.base_ctb() * 3 - ctb_since_last_action;
    }

    fn resolve_monster_action_name(
        &self,
        actor_id: ActorId,
        slot: MonsterSlot,
        action: &str,
    ) -> Result<String, String> {
        if action.is_empty() {
            if let Some(monster_key) = self
                .actor(actor_id)
                .and_then(|actor| actor.monster_key.as_deref())
            {
                if let Some(action_names) = data::monster_action_names(monster_key) {
                    if action_names.len() == 1 {
                        return Ok(action_names[0].clone());
                    }
                }
            }
        }
        if let Some(message) = self.invalid_monster_action_error(actor_id, slot, action) {
            return Err(message);
        }
        Ok(action.to_string())
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

    fn invalid_monster_action_error(
        &self,
        actor_id: ActorId,
        slot: MonsterSlot,
        action: &str,
    ) -> Option<String> {
        if action == "does_nothing" || action == "forced_action" {
            return None;
        }
        if self.is_scripted_monster_action(actor_id, action) {
            return None;
        }
        let monster_key = self
            .actor(actor_id)
            .and_then(|actor| actor.monster_key.as_deref())?;
        let action_names = data::monster_action_names(monster_key)?;
        if action_names.iter().any(|name| name == action) {
            return None;
        }
        let mut available_actions = action_names;
        available_actions.push("does_nothing".to_string());
        available_actions.push("forced_action".to_string());
        let actor_name = self
            .actor(actor_id)
            .map(actor_label)
            .unwrap_or_else(|| format!("M{}", slot.0));
        Some(format!(
            "Error: Available actions for {actor_name}: {}",
            available_actions.join(", ")
        ))
    }

    fn is_scripted_monster_action(&self, actor_id: ActorId, action: &str) -> bool {
        let Some(encounter_name) = self.current_encounter_name.as_deref() else {
            return false;
        };
        let Some(monster_key) = self
            .actor(actor_id)
            .and_then(|actor| actor.monster_key.as_deref())
        else {
            return false;
        };
        matches!(
            (encounter_name, monster_key, action),
            ("ammes", "sinspawn_ammes", "demi") | ("wakka_tutorial", "condor_2", "attack_wakka")
        )
    }

    fn required_character_target_error(
        &self,
        action_name: &str,
        action: &ActionData,
        args: &[String],
    ) -> Option<String> {
        if let Some(target_name) = args.first() {
            return match action.target {
                ActionTarget::Single if self.empty_monster_slot_error(target_name).is_some() => {
                    self.empty_monster_slot_error(target_name)
                }
                ActionTarget::Single if self.resolve_existing_actor_id(target_name).is_none() => {
                    Some(format!("Error: \"{target_name}\" is not a valid target"))
                }
                ActionTarget::SingleCharacter | ActionTarget::CounterSingleCharacter
                    if self.resolve_character_target_arg(target_name).is_none() =>
                {
                    Some(format!(
                        "Error: target can only be one of these values: {CHARACTER_VALUES}"
                    ))
                }
                ActionTarget::SingleMonster
                    if self.empty_monster_slot_error(target_name).is_some() =>
                {
                    self.empty_monster_slot_error(target_name)
                }
                ActionTarget::SingleMonster
                    if self
                        .resolve_existing_monster_target_arg(target_name)
                        .is_none() =>
                {
                    Some(format!(
                        "Error: \"{target_name}\" is not a valid monster name or slot"
                    ))
                }
                ActionTarget::EitherParty
                    if self.empty_monster_slot_error(target_name).is_some() =>
                {
                    self.empty_monster_slot_error(target_name)
                }
                ActionTarget::EitherParty
                    if matches!(target_name.as_str(), "party" | "monsters") =>
                {
                    None
                }
                ActionTarget::EitherParty
                    if self.explicit_action_targets(action, args).is_empty() =>
                {
                    Some(format!("Error: \"{target_name}\" is not a valid target"))
                }
                _ => None,
            };
        }
        let display_name = display_action_name(action_name);
        let message = match action.target {
            ActionTarget::Single => {
                format!("Error: Action \"{display_name}\" requires a target (Character/Monster/Monster Slot)")
            }
            ActionTarget::SingleCharacter => {
                format!("Error: Action \"{display_name}\" requires a target (Character)")
            }
            ActionTarget::SingleMonster => {
                format!("Error: Action \"{display_name}\" requires a target (Monster/Monster Slot)")
            }
            ActionTarget::EitherParty => {
                format!("Error: Action \"{display_name}\" requires a target (Character/Monster/Monster Slot/\"monsters\"/\"party\")")
            }
            _ => return None,
        };
        Some(message)
    }

    fn advance_virtual_turns_before(&mut self, expected_actor: ActorId) -> Vec<String> {
        let mut output_lines = Vec::new();
        if let ActorId::Character(character) = expected_actor {
            if !self.party.contains(&character) {
                return output_lines;
            }
        }
        for virtual_turns in 0..100 {
            let Some(actor_id) = self.current_battle_state().next_actor() else {
                break;
            };
            if actor_id == expected_actor {
                break;
            }
            match actor_id {
                ActorId::Monster(slot) => {
                    if self.is_manual_only_virtual_monster_turn(slot) {
                        break;
                    } else if let Some(virtual_action) = self.virtual_monster_action(slot) {
                        let outcome =
                            self.preview_virtual_monster_action(slot, &virtual_action.action);
                        output_lines.push(virtual_action.display_line(slot));
                        if let Some(comment) = outcome.damage_comment {
                            output_lines.push(comment);
                        }
                    } else {
                        self.process_start_of_turn(actor_id);
                        if self.apply_actor_turn(actor_id, 3).is_some() {
                            self.process_end_of_turn(actor_id);
                        } else {
                            break;
                        }
                    }
                }
                ActorId::Character(_) => {
                    self.process_start_of_turn(actor_id);
                    if self.apply_actor_turn(actor_id, 3).is_some() {
                        self.process_end_of_turn(actor_id);
                    } else {
                        break;
                    }
                }
            }
            if self
                .actor(expected_actor)
                .is_some_and(|actor| !actor.can_take_turn())
            {
                break;
            }
            if virtual_turns == 99 {
                output_lines.push(format!(
                    "# warning: stopped after {} virtual turns before '{}'",
                    virtual_turns + 1,
                    actor_label_for_id(expected_actor)
                ));
            }
        }
        output_lines
    }

    fn preview_virtual_monster_action(
        &mut self,
        slot: MonsterSlot,
        action: &str,
    ) -> MonsterActionOutcome {
        let snapshot = self.clone();
        let outcome = self.apply_monster_action_outcome_inner(slot, action, &[], true, false);
        if outcome.output.starts_with("Error:") || outcome.output.starts_with("# skipped:") {
            return outcome;
        }
        let post_rng = self.rng.clone();
        let resource_state = outcome.resource_state.clone();
        *self = snapshot;
        self.rng = post_rng;
        for (actor_id, current_hp, current_mp) in resource_state {
            if let Some(actor) = self.actor_mut(actor_id) {
                actor.current_hp = current_hp;
                actor.current_mp = current_mp;
                if actor.current_hp <= 0 {
                    actor.buffs.clear();
                    actor.clear_statuses();
                    actor.set_status(Status::Death, 254);
                }
            }
        }
        let actor_id = ActorId::Monster(slot);
        self.process_start_of_turn(actor_id);
        if self.apply_actor_turn(actor_id, 3).is_some() {
            self.process_end_of_turn(actor_id);
        }
        outcome
    }

    fn maybe_apply_scripted_pre_action(&mut self, output_lines: &mut Vec<String>) -> bool {
        let Some(encounter_name) = self.current_encounter_name.as_deref() else {
            return false;
        };
        let Some(condition) = self.current_encounter_condition else {
            return false;
        };
        let scripted_action = match (encounter_name, condition, self.scripted_turn_index) {
            ("geosgaeno", EncounterCondition::Ambush, 0 | 1) => Some((MonsterSlot(1), "half_hp")),
            _ => None,
        };
        let Some((slot, action)) = scripted_action else {
            return false;
        };
        let outcome = self.apply_monster_action_outcome(slot, action, &[], true);
        output_lines.push(format!("m{} {action}", slot.0));
        if let Some(comment) = outcome.damage_comment {
            output_lines.push(comment);
        }
        self.scripted_turn_index += 1;
        true
    }

    fn maybe_apply_scripted_respawns(&mut self) -> Vec<String> {
        let Some(encounter_name) = self.current_encounter_name.as_deref() else {
            return Vec::new();
        };
        if self.monsters.iter().any(BattleActor::is_alive) {
            return Vec::new();
        }
        if encounter_name == "sinscales" {
            if self.respawn_wave_index >= 2 {
                return Vec::new();
            }
            self.respawn_wave_index = 2;
            return (1..=5)
                .map(|slot| self.render_hidden_spawn("sinscale_6", slot))
                .collect();
        }
        let Some(monster_name) = scripted_respawn_monster(encounter_name, self.respawn_wave_index)
        else {
            return Vec::new();
        };
        self.respawn_wave_index += 1;
        vec![
            self.render_hidden_spawn(monster_name, 1),
            self.render_hidden_spawn(monster_name, 2),
        ]
    }

    fn render_hidden_spawn(&mut self, monster_name: &str, slot: usize) -> String {
        let output = self.spawn_monster(monster_name, MonsterSlot(slot), None);
        if output.starts_with("Error:") {
            output
        } else {
            format!("spawn {monster_name} {slot}")
        }
    }

    fn maybe_apply_sahagin_chief_spawn_comment(&mut self, line: &str) -> Vec<String> {
        if self.current_encounter_name.as_deref() != Some("sahagin_chiefs") {
            return Vec::new();
        }
        let Some((spawn_count, fourth_appears)) = parse_sahagin_chief_spawn_comment(line) else {
            return Vec::new();
        };
        let slot_limit = if self.sahagin_fourth_unlocked { 4 } else { 3 };
        let dead_slots = (1..=slot_limit)
            .filter(|slot| {
                self.actor(ActorId::Monster(MonsterSlot(*slot)))
                    .is_none_or(|actor| !actor.is_alive())
            })
            .take(spawn_count)
            .collect::<Vec<_>>();
        let mut output_lines = dead_slots
            .into_iter()
            .map(|slot| self.render_hidden_spawn("sahagin_chief", slot))
            .collect::<Vec<_>>();
        if fourth_appears {
            if self
                .actor(ActorId::Monster(MonsterSlot(4)))
                .is_none_or(|actor| !actor.is_alive())
            {
                output_lines.push(self.render_hidden_spawn("sahagin_chief", 4));
            }
            self.sahagin_fourth_unlocked = true;
        }
        output_lines
    }

    fn is_manual_only_virtual_monster_turn(&self, slot: MonsterSlot) -> bool {
        let Some(actor) = self.actor(ActorId::Monster(slot)) else {
            return false;
        };
        let Some(monster_key) = actor.monster_key.as_deref() else {
            return false;
        };
        if self.current_encounter_name.as_deref() == Some("echuilles") {
            return true;
        }
        monster_family(monster_key) == "sinscale"
    }

    fn virtual_monster_action(&self, slot: MonsterSlot) -> Option<VirtualMonsterAction> {
        let actor = self.actor(ActorId::Monster(slot))?;
        let monster_key = actor.monster_key.as_deref()?;
        let slot_display = format!("m{}", slot.0);
        if let Some(action) = self.scripted_virtual_monster_action_name(monster_key) {
            if self
                .action_data_for_actor(ActorId::Monster(slot), action)
                .is_some()
            {
                return Some(VirtualMonsterAction {
                    action: action.to_string(),
                    preview_display: None,
                });
            }
        }
        let action_names = data::monster_action_names(monster_key)?;
        if let Some(action) = data::default_monster_action(monster_key) {
            if action_names.iter().any(|name| name == action) {
                return Some(VirtualMonsterAction {
                    action: action.to_string(),
                    preview_display: None,
                });
            }
        }
        if action_names.iter().any(|action| action == "attack") {
            return Some(VirtualMonsterAction {
                action: "attack".to_string(),
                preview_display: Some(slot_display),
            });
        }
        if action_names.len() == 1 {
            return Some(VirtualMonsterAction {
                action: action_names[0].clone(),
                preview_display: Some(slot_display),
            });
        }
        data::monster_action_data(monster_key, "forced_action").map(|_| VirtualMonsterAction {
            action: "forced_action".to_string(),
            preview_display: Some(slot_display),
        })
    }

    fn scripted_virtual_monster_action_name(&self, monster_key: &str) -> Option<&'static str> {
        match (self.current_encounter_name.as_deref()?, monster_key) {
            ("ammes", "sinspawn_ammes") => Some("demi"),
            ("wakka_tutorial", "condor_2") => Some("attack_wakka"),
            _ => None,
        }
    }

    fn escape_succeeds(&mut self, actor_id: ActorId) -> bool {
        let Some(actor) = self.actor(actor_id) else {
            return false;
        };
        let rng_index = 20 + actor.index;
        let escape_roll = self.rng.advance_rng(rng_index) & 255;
        escape_roll < 191
    }

    fn apply_action_effects(
        &mut self,
        user: ActorId,
        action_data: Option<&ActionData>,
        args: &[String],
    ) -> Vec<ActionEffectResult> {
        let Some(action_data) = action_data else {
            return Vec::new();
        };
        let user_actor = self.actor(user).cloned();
        let targets = self.resolve_action_targets(user, &action_data, args);
        let od_time_remaining = overdrive_time_remaining_ms(action_data, args);
        let damage_parameter = if action_data.damage_formula == DamageFormula::Gil {
            gil_damage_parameter_ms(args)
        } else {
            od_time_remaining
        };
        let mut resolved_targets = Vec::new();
        let mut results = Vec::new();
        let is_counter = action_is_counter(action_data);
        self.apply_action_mp_cost(user, action_data);
        for target in targets {
            let mut target = target;
            let original_target = target;
            let mut target_actor = self.actor(target).cloned();
            let check_target_actor = target_actor.clone();
            let mut result = ActionEffectResult::new(target);
            if action_data.affected_by_reflect
                && target_actor
                    .as_ref()
                    .is_some_and(|actor| actor.statuses.contains(&Status::Reflect))
            {
                result.reflected_from = Some(target);
                target = self.reflected_action_target(target, action_data);
                result.target = target;
                target_actor = self.actor(target).cloned();
            }
            resolved_targets.push(target);
            if action_misses_target(action_data, check_target_actor.as_ref()) {
                result.hit = false;
                results.push(result);
                continue;
            }
            if self.consume_nul_statuses_if_blocked(
                user_actor.as_ref(),
                original_target,
                action_data,
            ) {
                result.hit = false;
                results.push(result);
                continue;
            }
            if !self.action_hits_target(
                user_actor.as_ref(),
                check_target_actor.as_ref(),
                action_data,
            ) {
                result.hit = false;
                results.push(result);
                continue;
            }
            let had_auto_life = target_actor
                .as_ref()
                .is_some_and(|actor| actor.statuses.contains(&Status::AutoLife));
            if let Some(user_actor) = user_actor.as_ref() {
                result.damage =
                    self.apply_action_damage(user_actor, target, action_data, damage_parameter);
            }
            if action_data.removes_statuses {
                for status in &action_data.statuses {
                    if self.remove_status_from_actor_for_action(target, *status, is_counter) {
                        result.removed_statuses.push(*status);
                    }
                }
            } else {
                let status_applications = self.merged_status_applications(user, action_data);
                for application in &status_applications {
                    let applied = self.apply_action_status_to_actor(user, target, application);
                    result.statuses.push((application.status, applied));
                }
                for status in &action_data.status_flags {
                    if self.apply_action_status_flag_to_actor(target, *status) {
                        result.statuses.push((*status, true));
                    }
                }
            }
            if action_data.heals && !action_data.damages_hp && !action_data.damages_ctb {
                self.heal_actor(target);
            }
            for buff in &action_data.buffs {
                if let Some(stacks) = self.apply_buff_to_actor(target, buff.buff, buff.amount) {
                    result.buffs.push((buff.buff, stacks));
                }
            }
            self.apply_petrify_final_status_cleanup(target);
            if action_data.has_weak_delay {
                self.apply_delay(target, 3, 2);
            }
            if action_data.has_strong_delay {
                self.apply_delay(target, 3, 1);
            }
            result
                .statuses
                .extend(self.apply_shatter_check(user, target, action_data));
            result.auto_life_triggered = had_auto_life
                && self
                    .actor(target)
                    .is_some_and(|actor| actor.statuses.contains(&Status::Death));
            results.push(result);
        }
        self.record_action_target_memory(user, resolved_targets, action_data.key == "provoke");
        if action_data.destroys_user {
            self.apply_status_to_actor(user, Status::Eject);
        }
        results
    }

    fn record_action_target_memory(
        &mut self,
        user: ActorId,
        resolved_targets: Vec<ActorId>,
        records_provoke: bool,
    ) {
        self.last_targets = resolved_targets;
        self.actor_last_targets
            .insert(user, self.last_targets.clone());
        if self.last_targets.is_empty() {
            return;
        }
        for target in &self.last_targets {
            self.actor_last_attackers.insert(*target, user);
        }
        if records_provoke {
            for target in &self.last_targets {
                self.actor_provokers.insert(*target, user);
            }
        }
    }

    fn merged_status_applications(
        &self,
        user: ActorId,
        action: &ActionData,
    ) -> Vec<data::ActionStatus> {
        let mut applications = action.status_applications.clone();
        if !action.uses_weapon_properties {
            return applications;
        }
        let Some(actor) = self.actor(user) else {
            return applications;
        };
        for weapon_application in actor
            .weapon_abilities
            .iter()
            .filter_map(|ability| weapon_status_application(*ability))
        {
            if let Some(existing) = applications
                .iter_mut()
                .find(|application| application.status == weapon_application.status)
            {
                if weapon_application.chance > existing.chance {
                    *existing = weapon_application;
                }
            } else {
                applications.push(weapon_application);
            }
        }
        applications
    }

    fn reflected_action_target(&mut self, target: ActorId, action: &ActionData) -> ActorId {
        let possible_targets = match target {
            ActorId::Monster(_) => self.possible_character_targets(action),
            ActorId::Character(_) => self.possible_monster_targets(action),
        };
        match possible_targets.as_slice() {
            [] => ActorId::Character(Character::Unknown),
            [target] => *target,
            targets => {
                let target_rng = self.rng.advance_rng(6);
                targets[target_rng as usize % targets.len()]
            }
        }
    }

    fn apply_action_mp_cost(&mut self, user: ActorId, action: &ActionData) {
        if action.mp_cost <= 0 {
            return;
        }
        let Some(actor) = self.actor_mut(user) else {
            return;
        };
        if actor.statuses.contains(&Status::Mp0) {
            return;
        }
        let mp_cost = if actor.has_auto_ability(AutoAbility::OneMpCost) {
            1
        } else {
            let mut cost = action.mp_cost;
            if action.uses_magic_booster && actor.has_auto_ability(AutoAbility::MagicBooster) {
                cost *= 2;
            }
            if actor.has_auto_ability(AutoAbility::HalfMpCost) {
                cost /= 2;
            }
            cost
        };
        actor.current_mp = (actor.current_mp - mp_cost).max(0);
    }

    fn change_equipment(&mut self, kind: &str, args: &[String]) -> String {
        if kind.is_empty() || args.len() < 2 {
            return "Error: Usage: equip [equip type] [character] [# of slots] (abilities)"
                .to_string();
        }
        let kind = match kind.to_ascii_lowercase().as_str() {
            "weapon" => data::EquipmentKind::Weapon,
            "armor" => data::EquipmentKind::Armor,
            _ => {
                return "Error: equipment type can only be one of these values: weapon, armor"
                    .to_string()
            }
        };
        let Ok(character) = args[0].parse::<Character>() else {
            return format!("Error: character can only be one of these values: {CHARACTER_VALUES}");
        };
        let Some(raw_slots) = args[1].parse::<u8>().ok().filter(|slots| *slots <= 4) else {
            return "Error: Slots must be between 0 and 4".to_string();
        };
        let abilities = match parse_equipment_abilities(args) {
            Ok(abilities) => abilities,
            Err(message) => return message,
        };
        let slots = raw_slots.max(abilities.display_names.len() as u8);
        let Some(actor) = self.actor_mut(ActorId::Character(character)) else {
            return format!(
                "Error: unknown equipment actor: {}",
                character.display_name()
            );
        };
        let old_equipment = format_actor_equipment(kind, character, actor);
        match kind {
            data::EquipmentKind::Weapon => {
                actor.set_weapon_slots(slots);
                actor.set_weapon_abilities(abilities.modeled);
                actor.weapon_bonus_crit = 3;
                actor.combat_stats.base_weapon_damage =
                    default_equipment_base_weapon_damage(character);
            }
            data::EquipmentKind::Armor => {
                actor.set_armor_slots(slots);
                actor.set_armor_abilities(abilities.modeled);
                actor.armor_bonus_crit = 3;
            }
        }
        actor.equipment_crit = actor.weapon_bonus_crit + actor.armor_bonus_crit;
        apply_auto_statuses(actor);
        apply_equipment_elements(actor);
        apply_equipment_status_resistances(actor);
        apply_equipment_resource_bonuses(actor);
        let new_equipment = format_equipment(kind, character, &abilities.display_names, slots);
        format!(
            "Equipment: {} | {} | {old_equipment} -> {new_equipment}",
            character.display_name(),
            equipment_kind_display_name(kind)
        )
    }

    fn render_equipment_command(&mut self, kind: &str, args: &[String], raw_line: &str) -> String {
        let output = self.change_equipment(kind, args);
        if self.echo_state_edits && !output.starts_with("Error:") {
            raw_line.to_string()
        } else {
            output
        }
    }

    fn summon(&mut self, aeon_name: &str) -> String {
        if aeon_name.is_empty() {
            return "Error: Usage: summon [aeon name]".to_string();
        }
        let Some(party) = parse_summon_party(aeon_name) else {
            return format!("Error: No aeon named \"{aeon_name}\"");
        };
        let old_party = self.format_party();
        self.party = party;
        format!("Party: {old_party} -> {}", self.format_party())
    }

    fn render_summon_command(&mut self, aeon_name: &str) -> String {
        let output = self.summon(aeon_name);
        if self.echo_state_edits && !output.starts_with("Error:") {
            format!("summon {aeon_name}")
        } else {
            output
        }
    }

    fn spawn_monster(
        &mut self,
        monster_name: &str,
        slot: MonsterSlot,
        forced_ctb: Option<i32>,
    ) -> String {
        let Some(mut actor) = create_monster_actor(monster_name, slot) else {
            return format!("Error: No monster named \"{monster_name}\"");
        };
        let template = monster_template(monster_name);
        let ctb = forced_ctb.unwrap_or_else(|| actor.base_ctb() * 3);
        actor.ctb = ctb - self.ctb_since_last_action;
        if actor.ctb < 0 {
            let negative_ctb = actor.ctb;
            self.normalize_ctbs_by(negative_ctb);
            self.ctb_since_last_action += negative_ctb;
            actor.ctb = 0;
        }
        let spawned_ctb = actor.ctb;
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
        format!(
            "Spawn: {} (M{}) with {spawned_ctb} CTB",
            template.display_name, slot.0
        )
    }

    fn spawn_monster_from_command(
        &mut self,
        monster_name: &str,
        slot: usize,
        forced_ctb: Option<i32>,
    ) -> String {
        if data::monster_stats(monster_name).is_none() {
            return format!("Error: No monster named \"{monster_name}\"");
        }
        let slot_limit = (self.monsters.len() + 1).min(8);
        if !(1..=slot_limit).contains(&slot) {
            return format!("Error: Slot must be between 1 and {slot_limit}");
        }
        self.spawn_monster(monster_name, MonsterSlot(slot), forced_ctb)
    }

    fn render_spawn_command(
        &mut self,
        monster_name: &str,
        slot: usize,
        forced_ctb: Option<i32>,
    ) -> String {
        let output = self.spawn_monster_from_command(monster_name, slot, forced_ctb);
        if output.starts_with("Error:") {
            return output;
        }
        match forced_ctb {
            Some(ctb) => format!("spawn {monster_name} {slot} {ctb}"),
            None => format!("spawn {monster_name} {slot}"),
        }
    }

    fn change_element(&mut self, args: &[String]) -> String {
        if args.len() < 3 {
            return "Error: Usage: element [monster slot] [element] [affinity]".to_string();
        }
        let actor_id = match self.python_element_monster_slot(&args[0]) {
            Ok(actor_id) => actor_id,
            Err(message) => return message,
        };
        let Some(element) = args.get(1).and_then(|arg| arg.parse::<Element>().ok()) else {
            return format!(
                "Error: element can only be one of these values: {}",
                element_values()
            );
        };
        let Some(affinity) = args
            .get(2)
            .and_then(|arg| arg.parse::<ElementalAffinity>().ok())
        else {
            return format!(
                "Error: affinity can only be one of these values: {}",
                elemental_affinity_values()
            );
        };
        if let Some(actor) = self.actor_mut(actor_id) {
            actor.set_elemental_affinity(element, affinity);
        }
        let actor_label = self
            .actor(actor_id)
            .map(actor_label)
            .unwrap_or_else(|| "M?".to_string());
        format!(
            "Elemental affinity to {} of {actor_label} changed to {}",
            element_display_name(element),
            elemental_affinity_display_name(affinity)
        )
    }

    fn render_element_command(&mut self, args: &[String], raw_line: &str) -> String {
        let output = self.change_element(args);
        if self.echo_state_edits && !output.starts_with("Error:") {
            raw_line.to_string()
        } else {
            output
        }
    }

    fn python_element_monster_slot(&self, slot: &str) -> Result<ActorId, String> {
        let Some(slot_digit) = slot.chars().nth(1) else {
            return Err("Error: Monster slot must be in the form m#".to_string());
        };
        let Some(slot_number) = slot_digit.to_digit(10).map(|digit| digit as isize) else {
            return Err("Error: Monster slot must be in the form m#".to_string());
        };
        let index = slot_number - 1;
        let actor = if index < 0 {
            self.monsters.last()
        } else {
            self.monsters.get(index as usize)
        };
        actor
            .map(|actor| actor.id)
            .ok_or_else(|| format!("Error: No monster in slot {slot_number}"))
    }

    fn change_stat(&mut self, args: &[String]) -> String {
        let Some(actor_id) = args
            .first()
            .and_then(|arg| self.resolve_state_actor_id(arg))
        else {
            return if let Some(actor) = args.first() {
                format!("Error: \"{actor}\" is not a valid actor")
            } else {
                "Error: Usage: stat [character/monster slot] (stat) [(+/-)amount]".to_string()
            };
        };
        let stat_name = args.get(1).map(|value| value.to_ascii_lowercase());
        let amount = args.get(2).map(String::as_str).unwrap_or_default();
        if let ActorId::Monster(slot) = actor_id {
            if self.actor(actor_id).is_none() {
                return format!("Error: No monster in slot {}", slot.0);
            }
        }
        if let ActorId::Character(character) = actor_id {
            if let Some(stat_name) = stat_name.as_deref() {
                if stat_name != "ctb" {
                    return self.change_character_stat(character, stat_name, amount);
                }
            }
        }
        let Some(actor) = self.actor_mut(actor_id) else {
            return format!("Error: unknown stat actor: {}", args[0]);
        };
        match stat_name.as_deref() {
            Some("ctb") => {
                let Ok(ctb) = parse_amount(amount, actor.ctb, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.ctb = ctb.max(0);
                format!("{}'s CTB changed to {}", actor_label(actor), actor.ctb)
            }
            Some("hp") => {
                let old_hp = actor.max_hp;
                let Ok(hp) = parse_amount(amount, actor.max_hp, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.max_hp = if matches!(actor.id, ActorId::Monster(_)) {
                    hp.max(0)
                } else {
                    hp.clamp(0, 99_999)
                };
                actor.current_hp = actor.current_hp.min(actor.effective_max_hp());
                if actor.current_hp <= 0 {
                    actor.buffs.clear();
                    actor.clear_statuses();
                    actor.set_status(Status::Death, 254);
                }
                format_stat_change(actor, "HP", old_hp, actor.max_hp)
            }
            Some("mp") => {
                let old_mp = actor.max_mp;
                let Ok(mp) = parse_amount(amount, actor.max_mp, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.max_mp = if matches!(actor.id, ActorId::Monster(_)) {
                    mp.max(0)
                } else {
                    mp.clamp(0, 9_999)
                };
                actor.current_mp = actor.current_mp.min(actor.effective_max_mp());
                format_stat_change(actor, "MP", old_mp, actor.max_mp)
            }
            Some("agility") => {
                let old_agility = actor.agility as i32;
                let Ok(agility) = parse_amount(amount, actor.agility as i32, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                let agility = agility.clamp(0, 255);
                actor.agility = agility as u8;
                format_stat_change(actor, "Agility", old_agility, actor.agility as i32)
            }
            Some("strength") => {
                let old_strength = actor.combat_stats.strength;
                let Ok(strength) = parse_amount(amount, actor.combat_stats.strength, "amount")
                else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.strength = strength.clamp(0, 255);
                format_stat_change(actor, "Strength", old_strength, actor.combat_stats.strength)
            }
            Some("defense") => {
                let old_defense = actor.combat_stats.defense;
                let Ok(defense) = parse_amount(amount, actor.combat_stats.defense, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.defense = defense.clamp(0, 255);
                format_stat_change(actor, "Defense", old_defense, actor.combat_stats.defense)
            }
            Some("magic") => {
                let old_magic = actor.combat_stats.magic;
                let Ok(magic) = parse_amount(amount, actor.combat_stats.magic, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.magic = magic.clamp(0, 255);
                format_stat_change(actor, "Magic", old_magic, actor.combat_stats.magic)
            }
            Some("magic_defense" | "magic_def") => {
                let old_magic_defense = actor.combat_stats.magic_defense;
                let Ok(magic_defense) =
                    parse_amount(amount, actor.combat_stats.magic_defense, "amount")
                else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.magic_defense = magic_defense.clamp(0, 255);
                format_stat_change(
                    actor,
                    "Magic defense",
                    old_magic_defense,
                    actor.combat_stats.magic_defense,
                )
            }
            Some("luck") => {
                let old_luck = actor.combat_stats.luck;
                let Ok(luck) = parse_amount(amount, actor.combat_stats.luck, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.luck = luck.clamp(0, 255);
                format_stat_change(actor, "Luck", old_luck, actor.combat_stats.luck)
            }
            Some("evasion") => {
                let old_evasion = actor.combat_stats.evasion;
                let Ok(evasion) = parse_amount(amount, actor.combat_stats.evasion, "amount") else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.evasion = evasion.clamp(0, 255);
                format_stat_change(actor, "Evasion", old_evasion, actor.combat_stats.evasion)
            }
            Some("accuracy") => {
                let old_accuracy = actor.combat_stats.accuracy;
                let Ok(accuracy) = parse_amount(amount, actor.combat_stats.accuracy, "amount")
                else {
                    return "Error: amount must be an integer".to_string();
                };
                actor.combat_stats.accuracy = accuracy.clamp(0, 255);
                format_stat_change(actor, "Accuracy", old_accuracy, actor.combat_stats.accuracy)
            }
            Some(_) => format!(
                "Error: stat can only be one of these values: {}",
                stat_values()
            ),
            None => format_actor_stats(actor),
        }
    }

    fn change_character_stat(
        &mut self,
        character: Character,
        stat_name: &str,
        amount: &str,
    ) -> String {
        let Some(stat) = ActorStat::from_name(stat_name) else {
            return format!(
                "Error: stat can only be one of these values: {}",
                stat_values()
            );
        };
        let actor_id = ActorId::Character(character);
        let Some(current) = self.actor(actor_id).map(|actor| stat.value(actor)) else {
            return format!("Error: unknown stat actor: {}", character.input_name());
        };
        let Ok(target) = parse_amount(amount, current, "amount") else {
            return "Error: amount must be an integer".to_string();
        };

        if self.bonus_aeon_stats.contains_key(&character) {
            let old_bonus = self
                .bonus_aeon_stats
                .get(&character)
                .map(|stats| stat.bonus_value(*stats))
                .unwrap_or_default();
            let base = current - old_bonus;
            stat.set_bonus(
                self.bonus_aeon_stats.entry(character).or_default(),
                (target - base).max(0),
            );
            let new_bonus = self
                .bonus_aeon_stats
                .get(&character)
                .map(|stats| stat.bonus_value(*stats))
                .unwrap_or_default();
            let Some(actor) = self.actor_mut(actor_id) else {
                return format!("Error: unknown stat actor: {}", character.input_name());
            };
            stat.set_value(actor, base + new_bonus);
            return format_stat_change(actor, stat.display_name(), current, stat.value(actor));
        }

        let Some(actor) = self.actor_mut(actor_id) else {
            return format!("Error: unknown stat actor: {}", character.input_name());
        };
        stat.set_value(actor, target);
        let output = format_stat_change(actor, stat.display_name(), current, stat.value(actor));
        if character == Character::Yuna {
            self.calculate_aeon_stats();
        }
        output
    }

    fn render_stat_command(&mut self, args: &[String]) -> String {
        let output = self.change_stat(args);
        if !self.echo_state_edits || output.starts_with("Error:") || args.len() < 2 {
            output
        } else {
            format!("stat {}", args.join(" "))
        }
    }

    fn render_status_command(&mut self, args: &[String]) -> String {
        let output = self.change_status(args);
        if self.echo_state_edits && !output.starts_with("Error:") {
            format!("status {}\n# {output}", args.join(" "))
        } else {
            output
        }
    }

    fn change_status(&mut self, args: &[String]) -> String {
        let Some(actor_id) = args
            .first()
            .and_then(|arg| self.resolve_state_actor_id(arg))
        else {
            return if let Some(actor) = args.first() {
                format!("Error: \"{actor}\" is not a valid actor")
            } else {
                "Error: Usage: status [character/monster slot]".to_string()
            };
        };
        let status = match args.get(1) {
            Some(status_name) => match parse_status(status_name) {
                Some(status) => Some(status),
                None if is_upstream_status(status_name) => None,
                None => {
                    return format!(
                        "Error: status can only be one of these values: {}",
                        status_values()
                    )
                }
            },
            None => None,
        };
        let stacks = match args.get(2) {
            Some(value) => match value.parse::<i32>() {
                Ok(stacks) => stacks,
                Err(_) => return "Error: status stacks must be an integer".to_string(),
            },
            None => 254,
        };
        if let ActorId::Monster(slot) = actor_id {
            if self.actor(actor_id).is_none() {
                return format!("Error: No monster in slot {}", slot.0);
            }
        }
        let Some(actor) = self.actor_mut(actor_id) else {
            return format!("Error: unknown status actor: {}", args[0]);
        };
        if let Some(status) = status {
            if stacks <= 0 {
                actor.remove_status(status);
            } else {
                actor.set_status(status, stacks);
            }
        }
        format_status(actor)
    }

    fn heal_party(&mut self, args: &[String]) -> String {
        let amount = args
            .get(1)
            .and_then(|amount| amount.parse::<i32>().ok())
            .unwrap_or(99_999);
        if let Some(character_name) = args.first() {
            let Ok(character) = character_name.parse::<Character>() else {
                return format!(
                    "Error: character can only be one of these values: {CHARACTER_VALUES}"
                );
            };
            if let Some(actor) = self.actor_mut(ActorId::Character(character)) {
                Self::heal_actor_by(actor, amount);
            }
            return format!(
                "Heal: {} healed by {amount} HP and MP",
                character.display_name()
            );
        }
        for actor in &mut self.character_actors {
            Self::heal_actor_by(actor, amount);
        }
        format!("Heal: every Character healed by {amount} HP and MP")
    }

    fn render_heal_command(&mut self, args: &[String]) -> String {
        let output = self.heal_party(args);
        if output.starts_with("Error:") {
            output
        } else if args.is_empty() {
            "heal".to_string()
        } else {
            format!("heal {}", args.join(" "))
        }
    }

    fn change_ap(&mut self, args: &[String]) -> String {
        let characters = if let Some(character_name) = args.first() {
            let Ok(character) = character_name.parse::<Character>() else {
                return format!(
                    "Error: character can only be one of these values: {CHARACTER_VALUES}"
                );
            };
            vec![character]
        } else {
            vec![
                Character::Tidus,
                Character::Yuna,
                Character::Auron,
                Character::Kimahri,
                Character::Wakka,
                Character::Lulu,
                Character::Rikku,
            ]
        };
        let amount = args
            .get(1)
            .and_then(|amount| amount.parse::<i32>().ok())
            .unwrap_or(0);

        characters
            .into_iter()
            .map(|character| self.change_character_ap(character, amount))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn change_character_ap(&mut self, character: Character, amount: i32) -> String {
        let current_ap = self
            .character_ap
            .get(&character)
            .copied()
            .unwrap_or_default();
        let total_ap = if amount != 0 {
            (current_ap as i64 + amount as i64).clamp(0, i32::MAX as i64) as i32
        } else {
            current_ap
        };
        self.character_ap.insert(character, total_ap);
        let starting_s_lv = data::character_stats(character)
            .map(|stats| stats.starting_s_lv)
            .unwrap_or_default();
        let s_lv = total_ap_to_s_lv(total_ap, starting_s_lv);
        let next_s_lv_ap = s_lv_to_total_ap(s_lv + 1, starting_s_lv) - total_ap;
        let added = if amount != 0 {
            format!(" (added {amount} AP)")
        } else {
            String::new()
        };
        format!(
            "{}: {s_lv} S. Lv ({total_ap} AP Total, {next_s_lv_ap} for next level){added}",
            character.display_name()
        )
    }

    fn resolve_action_targets(
        &mut self,
        user: ActorId,
        action: &ActionData,
        args: &[String],
    ) -> Vec<ActorId> {
        let explicit_targets = self.explicit_action_targets(action, args);
        let n_of_hits = effective_target_hits(action, args);
        if !explicit_targets.is_empty() {
            self.advance_overdrive_target_rng(user, action, explicit_targets.len(), n_of_hits);
            return expand_targets_for_hits(explicit_targets, n_of_hits);
        }

        let targets = match action.target {
            ActionTarget::SelfTarget | ActionTarget::CounterSelf => vec![user],
            ActionTarget::CharactersParty | ActionTarget::CounterCharactersParty => match user {
                ActorId::Character(_) => {
                    with_unknown_if_empty(self.possible_character_targets(action))
                }
                ActorId::Monster(_) => {
                    with_unknown_if_empty(self.possible_character_targets(action))
                }
            },
            ActionTarget::MonstersParty => match user {
                ActorId::Character(_) => {
                    with_unknown_if_empty(self.possible_monster_targets(action))
                }
                ActorId::Monster(_) => with_unknown_if_empty(self.possible_monster_targets(action)),
            },
            ActionTarget::SingleCharacter | ActionTarget::CounterSingleCharacter => {
                with_unknown_if_empty(self.possible_character_targets(action))
                    .into_iter()
                    .take(1)
                    .collect()
            }
            ActionTarget::RandomCharacter | ActionTarget::CounterRandomCharacter => {
                let rng_index = if action.overdrive_user == Some(Character::Rikku) {
                    4
                } else if matches!(user, ActorId::Monster(_)) {
                    Self::monster_random_target_rng_index(n_of_hits)
                } else {
                    5
                };
                let possible_targets =
                    with_unknown_if_empty(self.possible_character_targets(action));
                self.advance_overdrive_target_rng(user, action, possible_targets.len(), n_of_hits);
                return self.random_targets(possible_targets, n_of_hits, rng_index);
            }
            ActionTarget::SingleMonster => {
                with_unknown_if_empty(self.possible_monster_targets(action))
                    .into_iter()
                    .take(1)
                    .collect()
            }
            ActionTarget::RandomMonster => {
                let possible_targets = with_unknown_if_empty(self.possible_monster_targets(action));
                self.advance_overdrive_target_rng(user, action, possible_targets.len(), n_of_hits);
                let rng_index = if action.overdrive_user == Some(Character::Tidus) && n_of_hits == 3
                {
                    16
                } else if action.overdrive_user == Some(Character::Wakka) {
                    19
                } else {
                    5
                };
                let possible_target_count = possible_targets.len();
                let targets = self.random_targets(possible_targets, n_of_hits, rng_index);
                if action.overdrive_user == Some(Character::Lulu) && possible_target_count > 1 {
                    let extra_rolls = (16 - n_of_hits.max(1)).clamp(0, 16);
                    for _ in 0..extra_rolls {
                        self.rng.advance_rng(rng_index);
                    }
                }
                return targets;
            }
            ActionTarget::HighestHpCharacter => {
                self.party_character_by_metric(action, true, |actor| actor.current_hp)
            }
            ActionTarget::HighestMpCharacter => {
                self.party_character_by_metric(action, true, |actor| actor.current_mp)
            }
            ActionTarget::LowestHpCharacter => {
                self.party_character_by_metric(action, false, |actor| actor.current_hp)
            }
            ActionTarget::HighestStrengthCharacter => {
                self.party_character_by_metric(action, true, |actor| actor.combat_stats.strength)
            }
            ActionTarget::LowestMagicDefenseCharacter => {
                self.party_character_by_metric(action, false, |actor| {
                    actor.combat_stats.magic_defense
                })
            }
            ActionTarget::RandomCharacterWith(status) => {
                let possible_targets: Vec<ActorId> = self
                    .possible_character_targets(action)
                    .into_iter()
                    .filter(|actor_id| {
                        self.actor(*actor_id)
                            .is_some_and(|actor| actor.statuses.contains(&status))
                    })
                    .collect();
                self.advance_overdrive_target_rng(
                    user,
                    action,
                    possible_targets.len().max(1),
                    n_of_hits,
                );
                return self.random_targets(
                    possible_targets,
                    n_of_hits,
                    Self::filtered_random_target_rng_index(user, n_of_hits),
                );
            }
            ActionTarget::RandomCharacterWithout(status) => {
                let possible_targets: Vec<ActorId> = self
                    .possible_character_targets(action)
                    .into_iter()
                    .filter(|actor_id| {
                        self.actor(*actor_id)
                            .is_some_and(|actor| !actor.statuses.contains(&status))
                    })
                    .collect();
                self.advance_overdrive_target_rng(
                    user,
                    action,
                    possible_targets.len().max(1),
                    n_of_hits,
                );
                return self.random_targets(
                    possible_targets,
                    n_of_hits,
                    Self::filtered_random_target_rng_index(user, n_of_hits),
                );
            }
            ActionTarget::RandomCharacterWithoutEither(left, right) => {
                let possible_targets: Vec<ActorId> = self
                    .possible_character_targets(action)
                    .into_iter()
                    .filter(|actor_id| {
                        self.actor(*actor_id).is_some_and(|actor| {
                            !actor.statuses.contains(&left) && !actor.statuses.contains(&right)
                        })
                    })
                    .collect();
                self.advance_overdrive_target_rng(
                    user,
                    action,
                    possible_targets.len().max(1),
                    n_of_hits,
                );
                return self.random_targets(
                    possible_targets,
                    n_of_hits,
                    Self::filtered_random_target_rng_index(user, n_of_hits),
                );
            }
            ActionTarget::RandomMonsterWithout(status) => {
                let possible_targets: Vec<ActorId> = self
                    .possible_monster_targets(action)
                    .into_iter()
                    .filter(|actor_id| {
                        self.actor(*actor_id)
                            .is_some_and(|actor| !actor.statuses.contains(&status))
                    })
                    .collect();
                self.advance_overdrive_target_rng(
                    user,
                    action,
                    possible_targets.len().max(1),
                    n_of_hits,
                );
                return self.random_targets(
                    possible_targets,
                    n_of_hits,
                    Self::filtered_random_target_rng_index(user, n_of_hits),
                );
            }
            ActionTarget::Provoker => self
                .actor_provokers
                .get(&user)
                .copied()
                .map(|target| vec![target])
                .unwrap_or_else(|| vec![ActorId::Character(Character::Unknown)]),
            ActionTarget::LastTarget | ActionTarget::CounterLastTarget => {
                let targets = self
                    .actor_last_targets
                    .get(&user)
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|actor_id| self.last_target_memory_is_valid(*actor_id))
                    .collect::<Vec<_>>();
                if targets.is_empty() && matches!(user, ActorId::Monster(_)) {
                    return self.random_targets(
                        self.possible_character_targets(action),
                        n_of_hits,
                        4,
                    );
                } else {
                    targets
                }
            }
            ActionTarget::LastAttacker => self
                .actor_last_attackers
                .get(&user)
                .copied()
                .map(|target| vec![target])
                .unwrap_or_else(|| vec![ActorId::Character(Character::Unknown)]),
            ActionTarget::Counter => with_unknown_if_empty(self.last_actor.into_iter().collect()),
            ActionTarget::Character(character) => {
                let target = ActorId::Character(character);
                let target_is_valid = if matches!(user, ActorId::Monster(_)) {
                    self.possible_character_targets(action).contains(&target)
                } else {
                    self.actor(target).is_some()
                };
                if target_is_valid {
                    vec![target]
                } else {
                    vec![ActorId::Character(Character::Unknown)]
                }
            }
            ActionTarget::Monster(slot) => {
                let target = ActorId::Monster(slot);
                if self.actor(target).is_some() {
                    vec![target]
                } else {
                    vec![ActorId::Character(Character::Unknown)]
                }
            }
            ActionTarget::EitherParty | ActionTarget::CounterAll => with_unknown_if_empty(
                self.possible_character_targets(action)
                    .into_iter()
                    .chain(self.possible_monster_targets(action))
                    .collect::<Vec<_>>(),
            ),
            ActionTarget::Single | ActionTarget::None => Vec::new(),
        };
        self.advance_overdrive_target_rng(user, action, targets.len(), n_of_hits);
        expand_targets_for_hits(targets, n_of_hits)
    }

    fn advance_overdrive_target_rng(
        &mut self,
        user: ActorId,
        action: &ActionData,
        n_of_targets: usize,
        n_of_hits: i32,
    ) {
        if action.overdrive_index <= 1 {
            return;
        }
        let Some(overdrive_user) = action.overdrive_user else {
            return;
        };
        let Some(actor) = self.actor(user).cloned() else {
            return;
        };
        let damage_rng_index = damage_rng_index(&actor);
        let status_rng_index = self.status_rng_index(user);
        let n_of_targets = n_of_targets as i32;
        let mut advances = Vec::new();
        match overdrive_user {
            Character::Tidus => match action.overdrive_index {
                2 => advances.push((damage_rng_index, 2)),
                3 => {
                    advances.push((damage_rng_index, 12));
                    if n_of_hits == 3 {
                        advances.push((5, 6));
                    }
                }
                4 => advances.push((damage_rng_index, 2 * n_of_targets)),
                5 if n_of_hits > 1 => advances.push((damage_rng_index, 18)),
                _ => {}
            },
            Character::Auron => match action.overdrive_index {
                2 => advances.push((damage_rng_index, 2)),
                3 => advances.push((damage_rng_index, 2 * n_of_targets)),
                4 => {
                    advances.push((damage_rng_index, 2));
                    advances.push((status_rng_index, 4));
                }
                5 => advances.push((damage_rng_index, 4 * n_of_targets)),
                _ => {}
            },
            Character::Wakka => {
                advances.push((damage_rng_index, n_of_targets));
                if n_of_targets == 1 {
                    advances.push((19, 1));
                }
            }
            Character::Lulu => {
                if action.target == ActionTarget::RandomMonster {
                    if action.base_damage != 0 {
                        advances.push((damage_rng_index, 16));
                    } else {
                        advances.push((status_rng_index, 16));
                    }
                    advances.push((16, (16 - n_of_hits).clamp(0, 16)));
                } else {
                    advances.push((damage_rng_index, 16 * n_of_targets));
                }
            }
            Character::Rikku if action.target != ActionTarget::RandomCharacter => {
                advances.push((5, 1));
            }
            _ => {}
        }
        for (rng_index, times) in advances {
            for _ in 0..times {
                self.rng.advance_rng(rng_index);
            }
        }
    }

    fn possible_character_targets(&self, action: &ActionData) -> Vec<ActorId> {
        let mut targets = self
            .party
            .iter()
            .copied()
            .map(ActorId::Character)
            .filter(|actor_id| {
                self.actor(*actor_id).is_some_and(|actor| {
                    (action.can_target_dead
                        || (actor.current_hp > 0 && !actor.statuses.contains(&Status::Death)))
                        && !actor.statuses.contains(&Status::Eject)
                })
            })
            .collect::<Vec<_>>();
        targets.sort_by_key(|target| actor_sort_index(*target));
        targets
    }

    fn last_target_memory_is_valid(&self, actor_id: ActorId) -> bool {
        let Some(actor) = self.actor(actor_id) else {
            return false;
        };
        if actor.statuses.contains(&Status::Death) {
            return false;
        }
        match actor_id {
            ActorId::Character(character) => self.party.contains(&character),
            ActorId::Monster(_) => self.monsters.iter().any(|monster| monster.id == actor_id),
        }
    }

    fn possible_monster_targets(&self, action: &ActionData) -> Vec<ActorId> {
        self.monsters
            .iter()
            .filter(|actor| {
                action.can_target_dead
                    || (actor.current_hp > 0 && !actor.statuses.contains(&Status::Death))
            })
            .map(|actor| actor.id)
            .collect()
    }

    fn random_targets(
        &mut self,
        possible_targets: Vec<ActorId>,
        n_of_hits: i32,
        single_target_rng_index: usize,
    ) -> Vec<ActorId> {
        let n_of_hits = n_of_hits.max(1) as usize;
        match possible_targets.as_slice() {
            [] => vec![ActorId::Character(Character::Unknown); n_of_hits],
            [target] => vec![*target; n_of_hits],
            targets if n_of_hits <= 1 => {
                let target_rng = self.rng.advance_rng(single_target_rng_index);
                vec![targets[target_rng as usize % targets.len()]]
            }
            targets => {
                let mut selected = Vec::with_capacity(n_of_hits);
                for _ in 0..n_of_hits {
                    let target_rng = self.rng.advance_rng(single_target_rng_index);
                    selected.push(targets[target_rng as usize % targets.len()]);
                }
                selected.sort_by_key(|target| actor_sort_index(*target));
                selected
            }
        }
    }

    fn monster_random_target_rng_index(n_of_hits: i32) -> usize {
        if n_of_hits > 1 {
            5
        } else {
            4
        }
    }

    fn filtered_random_target_rng_index(user: ActorId, n_of_hits: i32) -> usize {
        if matches!(user, ActorId::Monster(_)) {
            Self::monster_random_target_rng_index(n_of_hits)
        } else {
            4
        }
    }

    fn party_character_by_metric(
        &mut self,
        action: &ActionData,
        prefer_high: bool,
        metric: impl Fn(&BattleActor) -> i32,
    ) -> Vec<ActorId> {
        let possible_targets = self.possible_character_targets(action);
        if possible_targets.len() > 1 {
            self.rng.advance_rng(4);
        }
        let mut selected_value = None;
        let mut selected_targets = Vec::new();
        for actor_id in possible_targets {
            let Some(actor) = self.actor(actor_id) else {
                continue;
            };
            let value = metric(actor);
            match selected_value {
                None => {
                    selected_value = Some(value);
                    selected_targets.push(actor_id);
                }
                Some(current)
                    if (prefer_high && value > current) || (!prefer_high && value < current) =>
                {
                    selected_value = Some(value);
                    selected_targets.clear();
                    selected_targets.push(actor_id);
                }
                Some(current) if value == current => {
                    selected_targets.push(actor_id);
                }
                Some(_) => {}
            }
        }
        if selected_targets.len() > 1 {
            let target_rng = self.rng.advance_rng(4);
            return vec![selected_targets[target_rng as usize % selected_targets.len()]];
        }
        with_unknown_if_empty(selected_targets)
    }

    fn apply_status_to_actor(&mut self, target: ActorId, status: Status) {
        self.apply_status_to_actor_with_stacks(target, status, 254);
    }

    fn apply_status_to_actor_with_stacks(&mut self, target: ActorId, status: Status, stacks: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        match status {
            Status::Haste => {
                if actor
                    .status_stacks
                    .get(&Status::Slow)
                    .is_some_and(|stacks| *stacks >= 255)
                {
                    return;
                }
                actor.remove_status(Status::Slow);
                actor.set_status(Status::Haste, stacks);
            }
            Status::Slow => {
                if actor
                    .status_stacks
                    .get(&Status::Haste)
                    .is_some_and(|stacks| *stacks >= 255)
                {
                    return;
                }
                actor.remove_status(Status::Haste);
                actor.set_status(Status::Slow, stacks);
            }
            Status::Petrify => {
                actor.clear_statuses();
                actor.set_status(Status::Petrify, stacks);
            }
            Status::Death => {
                actor.current_hp = 0;
                actor.set_status(Status::Death, stacks);
            }
            other => {
                actor.set_status(other, stacks);
            }
        }
    }

    fn apply_action_status_to_actor(
        &mut self,
        user: ActorId,
        target: ActorId,
        application: &data::ActionStatus,
    ) -> bool {
        let resistance = self
            .actor(target)
            .and_then(|actor| actor.status_resistances.get(&application.status))
            .copied()
            .unwrap_or(0);
        let status_rng = if status_uses_rng(application.status) {
            let rng_index = self.status_rng_index(user);
            (self.rng.advance_rng(rng_index) % 101) as i32
        } else {
            0
        };
        let applied = if application.ignores_resistance || application.chance == 255 {
            true
        } else if resistance == 255 {
            false
        } else if application.chance == 254 {
            true
        } else {
            (application.chance as i32 - resistance as i32) > status_rng
        };
        if applied {
            self.apply_status_to_actor_with_stacks(target, application.status, application.stacks);
        }
        applied
    }

    fn apply_action_status_flag_to_actor(&mut self, target: ActorId, status: Status) -> bool {
        if self
            .actor(target)
            .and_then(|actor| actor.status_resistances.get(&status))
            .copied()
            == Some(255)
        {
            return false;
        }
        self.apply_status_to_actor_with_stacks(target, status, 254);
        true
    }

    fn apply_petrify_final_status_cleanup(&mut self, target: ActorId) {
        let Some(stacks) = self
            .actor(target)
            .and_then(|actor| actor.status_stacks.get(&Status::Petrify).copied())
        else {
            return;
        };
        if let Some(actor) = self.actor_mut(target) {
            actor.clear_statuses();
            actor.set_status(Status::Petrify, stacks);
        }
    }

    fn status_rng_index(&self, user: ActorId) -> usize {
        self.actor(user)
            .map(|actor| match user {
                ActorId::Character(_) => (52 + actor.index).min(59),
                ActorId::Monster(_) => (60 + actor.index).min(67),
            })
            .unwrap_or(52)
    }

    fn action_hits_target(
        &mut self,
        user: Option<&BattleActor>,
        target: Option<&BattleActor>,
        action: &ActionData,
    ) -> bool {
        let Some(user) = user else {
            return true;
        };
        let Some(target) = target else {
            return true;
        };
        if action.hit_chance_formula == HitChanceFormula::Always
            || target.statuses.contains(&Status::Sleep)
            || target.statuses.contains(&Status::Petrify)
        {
            return true;
        }
        let hit_rng = (self.rng.advance_rng(hit_rng_index(user)) % 101) as i32;
        let Some(base_hit_chance) = base_hit_chance(user, target, action) else {
            return true;
        };
        let base_hit_chance = if action.affected_by_dark && user.statuses.contains(&Status::Dark) {
            python_div_i64(
                python_div_i64(base_hit_chance as i64 * 0x6666_6667, 0xffff_ffff),
                4,
            ) as i32
        } else {
            base_hit_chance
        };
        let hit_chance = base_hit_chance + user.combat_stats.luck.max(1)
            - target.combat_stats.luck.max(1)
            + ((buff_stacks(user, Buff::Aim) - buff_stacks(target, Buff::Reflex)) * 10);
        hit_chance > hit_rng
    }

    fn consume_nul_statuses_if_blocked(
        &mut self,
        user: Option<&BattleActor>,
        target: ActorId,
        action: &ActionData,
    ) -> bool {
        let mut nul_statuses = action
            .elements
            .iter()
            .filter_map(|element| nul_status_for_element(*element))
            .collect::<Vec<_>>();
        if action.uses_weapon_properties {
            if let Some(user) = user {
                for element in &user.weapon_elements {
                    let Some(status) = nul_status_for_element(*element) else {
                        continue;
                    };
                    if !nul_statuses.contains(&status) {
                        nul_statuses.push(status);
                    }
                }
            }
        }
        if nul_statuses.is_empty()
            || !self.actor(target).is_some_and(|actor| {
                nul_statuses
                    .iter()
                    .all(|status| actor.statuses.contains(status))
            })
        {
            return false;
        }
        if let Some(actor) = self.actor_mut(target) {
            for status in nul_statuses {
                let stacks = actor.status_stack(status);
                if stacks >= 254 {
                    continue;
                }
                if stacks <= 1 {
                    actor.remove_status(status);
                } else {
                    actor.set_status(status, stacks - 1);
                }
            }
        }
        true
    }

    #[cfg(test)]
    fn remove_status_from_actor(&mut self, target: ActorId, status: Status) {
        self.remove_status_from_actor_for_action(target, status, false);
    }

    fn remove_status_from_actor_for_action(
        &mut self,
        target: ActorId,
        status: Status,
        is_counter: bool,
    ) -> bool {
        let ctb_since_last_action = self.ctb_since_last_action;
        let Some(actor) = self.actor_mut(target) else {
            return false;
        };
        if status == Status::Death {
            if actor.statuses.contains(&Status::Zombie) {
                if !actor.immune_to_life {
                    actor.current_hp = 0;
                    actor.buffs.clear();
                    actor.clear_statuses();
                    actor.set_status(Status::Death, 254);
                    return true;
                }
                return false;
            }
            if actor.status_stack(status) >= 255 {
                return false;
            }
            if !actor.remove_status(status) {
                return false;
            }
            actor.ctb = actor.base_ctb() * 3;
            if is_counter {
                actor.ctb -= ctb_since_last_action;
                if actor.has_auto_ability(AutoAbility::AutoRegen) {
                    let regen_healing =
                        ctb_since_last_action * actor.effective_max_hp().max(0) / 256 + 100;
                    actor.current_hp =
                        (actor.current_hp + regen_healing).clamp(0, actor.effective_max_hp());
                }
            }
            return true;
        }
        if actor.status_stack(status) >= 255 {
            return false;
        }
        if !actor.remove_status(status) {
            return false;
        }
        true
    }

    fn apply_buff_to_actor(&mut self, target: ActorId, buff: Buff, amount: i32) -> Option<i32> {
        let Some(actor) = self.actor_mut(target) else {
            return None;
        };
        if actor.statuses.contains(&Status::Petrify) {
            return None;
        }
        actor.add_buff(buff, amount);
        Some(buff_stacks(actor, buff))
    }

    fn apply_shatter_check(
        &mut self,
        user: ActorId,
        target: ActorId,
        action: &ActionData,
    ) -> Vec<(Status, bool)> {
        if !self
            .actor(target)
            .is_some_and(|actor| actor.statuses.contains(&Status::Petrify))
        {
            return Vec::new();
        }
        let shatter_rng = (self.rng.advance_rng(self.status_rng_index(user)) % 101) as i32;
        if !matches!(target, ActorId::Monster(_)) && action.shatter_chance <= shatter_rng {
            return vec![(Status::Eject, false)];
        }
        let Some(actor) = self.actor_mut(target) else {
            return Vec::new();
        };
        actor.current_hp = 0;
        actor.buffs.clear();
        actor.clear_statuses();
        actor.set_status(Status::Death, 254);
        actor.set_status(Status::Eject, 254);
        vec![(Status::Death, true), (Status::Eject, true)]
    }

    fn apply_action_damage(
        &mut self,
        user: &BattleActor,
        target: ActorId,
        action: &ActionData,
        od_time_remaining: i32,
    ) -> ActionDamageResults {
        let mut results = ActionDamageResults::default();
        if action.damage_formula == DamageFormula::NoDamage {
            return results;
        }
        if !(action.damages_hp || action.damages_mp || action.damages_ctb) {
            return results;
        }
        if action.damages_hp {
            let Some(target_actor) = self.actor(target).cloned() else {
                return results;
            };
            let damage_rng = self.rng.advance_rng(damage_rng_index(user)) & 31;
            let crit = self.action_crits(user, &target_actor, action);
            let mut damage = 0;
            if !target_actor.statuses.contains(&Status::Petrify) {
                damage = calculate_action_damage(
                    user,
                    &target_actor,
                    action,
                    damage_rng,
                    crit,
                    od_time_remaining,
                    DamagePool::Hp,
                );
                self.apply_hp_damage(target, damage);
                if action.drains {
                    self.apply_drain_hp_recovery(user.id, damage);
                }
            }
            results.hp = Some(ActionDamageResult {
                damage_rng,
                damage,
                pool: "HP",
                crit,
            });
        }
        if action.damages_mp {
            let Some(target_actor) = self.actor(target).cloned() else {
                return results;
            };
            let damage_rng = self.rng.advance_rng(damage_rng_index(user)) & 31;
            let crit = self.action_crits(user, &target_actor, action);
            let mut damage = 0;
            if !target_actor.statuses.contains(&Status::Petrify) {
                damage = calculate_action_damage(
                    user,
                    &target_actor,
                    action,
                    damage_rng,
                    crit,
                    od_time_remaining,
                    DamagePool::Mp,
                );
                damage = if damage > 0 {
                    damage.min(target_actor.current_mp)
                } else {
                    damage
                };
                self.apply_mp_damage(target, damage);
                if action.drains {
                    self.apply_drain_mp_recovery(user.id, damage);
                }
            }
            results.mp = Some(ActionDamageResult {
                damage_rng,
                damage,
                pool: "MP",
                crit,
            });
        }
        if action.damages_ctb {
            let Some(target_actor) = self.actor(target).cloned() else {
                return results;
            };
            let damage_rng = self.rng.advance_rng(damage_rng_index(user)) & 31;
            let crit = self.action_crits(user, &target_actor, action);
            let (damage, displayed_damage) = if target_actor.statuses.contains(&Status::Petrify) {
                (0, 0)
            } else {
                let mut damage = calculate_action_damage(
                    user,
                    &target_actor,
                    action,
                    damage_rng,
                    crit,
                    od_time_remaining,
                    DamagePool::Hp,
                );
                if damage < 0 {
                    damage = damage.max(-target_actor.ctb);
                }
                let displayed_damage = if target == user.id && damage < 0 {
                    0
                } else {
                    damage
                };
                (damage, displayed_damage)
            };
            if !target_actor.statuses.contains(&Status::Petrify) {
                if let Some(actor) = self.actor_mut(target) {
                    actor.ctb += damage;
                }
            }
            results.ctb = Some(ActionDamageResult {
                damage_rng,
                damage: displayed_damage,
                pool: "CTB",
                crit,
            });
        }
        results
    }

    fn action_crits(
        &mut self,
        user: &BattleActor,
        target: &BattleActor,
        action: &ActionData,
    ) -> bool {
        if !action.can_crit {
            return false;
        }
        let crit_roll = (self.rng.advance_rng(damage_rng_index(user)) % 101) as i32;
        if user.statuses.contains(&Status::Critical) {
            return true;
        }
        let mut crit_chance = user.combat_stats.luck + (buff_stacks(user, Buff::Luck) * 10);
        if action.adds_equipment_crit {
            crit_chance += user.equipment_crit;
        } else {
            crit_chance += action.bonus_crit;
        }
        let target_luck = target.combat_stats.luck.max(1) - (buff_stacks(target, Buff::Jinx) * 10);
        crit_roll < (crit_chance - target_luck)
    }

    fn apply_hp_damage(&mut self, target: ActorId, damage: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.current_hp = (actor.current_hp - damage).clamp(0, actor.effective_max_hp().max(1));
        if actor.current_hp <= 0 {
            actor.buffs.clear();
            actor.clear_statuses();
            actor.set_status(Status::Death, 254);
        }
    }

    fn apply_mp_damage(&mut self, target: ActorId, damage: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.current_mp = (actor.current_mp - damage).clamp(0, actor.effective_max_mp().max(0));
    }

    fn apply_drain_hp_recovery(&mut self, target: ActorId, damage: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.current_hp = (actor.current_hp + damage).clamp(0, actor.effective_max_hp().max(0));
    }

    fn apply_drain_mp_recovery(&mut self, target: ActorId, damage: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.current_mp = (actor.current_mp + damage).clamp(0, actor.effective_max_mp().max(0));
    }

    fn heal_actor(&mut self, target: ActorId) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        Self::restore_actor(actor);
    }

    fn restore_actor(actor: &mut BattleActor) {
        actor.current_hp = actor.effective_max_hp();
        actor.current_mp = actor.effective_max_mp();
        actor.remove_status(Status::Death);
        actor.remove_status(Status::Poison);
        actor.remove_status(Status::Zombie);
    }

    fn heal_actor_by(actor: &mut BattleActor, amount: i32) {
        actor.current_hp = (actor.current_hp + amount).clamp(0, actor.effective_max_hp().max(0));
        if actor.current_hp == 0 {
            actor.buffs.clear();
            actor.clear_statuses();
            actor.set_status(Status::Death, 254);
        }
        actor.current_mp = (actor.current_mp + amount).clamp(0, actor.effective_max_mp().max(0));
    }

    fn apply_delay(&mut self, target: ActorId, numerator: i32, divisor: i32) {
        let Some(actor) = self.actor_mut(target) else {
            return;
        };
        actor.ctb += actor.base_ctb() * numerator / divisor;
    }

    fn end_encounter(&mut self) -> String {
        let ctb_string = self.current_battle_state().ctb_order_string();
        let character_hps = self.end_encounter_character_hps();
        let monster_hps = self.end_encounter_monster_hps();
        self.process_start_of_encounter();
        format!(
            "End: CTBs: {ctb_string}\n     Characters HPs: {character_hps}\n     Monsters HPs: {monster_hps}"
        )
    }

    fn end_encounter_character_hps(&self) -> String {
        let hps = self
            .character_actors
            .iter()
            .filter_map(|actor| {
                let ActorId::Character(character) = actor.id else {
                    return None;
                };
                (actor.current_hp < actor.effective_max_hp()).then(|| {
                    let display = character.display_name();
                    let short = display.get(..2).unwrap_or(display);
                    format!("{short}[{}]", actor.current_hp)
                })
            })
            .collect::<Vec<_>>();
        if hps.is_empty() {
            "Characters at full HP".to_string()
        } else {
            hps.join(" ")
        }
    }

    fn end_encounter_monster_hps(&self) -> String {
        let hps = self
            .monsters
            .iter()
            .map(|actor| format!("{}[{}]", actor_label(actor), actor.current_hp))
            .collect::<Vec<_>>();
        if hps.is_empty() {
            "Monsters at full HP".to_string()
        } else {
            hps.join(" ")
        }
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

    fn process_start_of_turn(&mut self, actor_id: ActorId) {
        let Some(actor) = self.actor_mut(actor_id) else {
            return;
        };
        for status in temporary_statuses() {
            actor.remove_status(status);
        }
        apply_auto_statuses(actor);
    }

    fn process_end_of_turn(&mut self, actor_id: ActorId) {
        self.last_actor = Some(actor_id);
        let poison_damage = self.actor(actor_id).and_then(|actor| {
            actor
                .statuses
                .contains(&Status::Poison)
                .then_some(actor.effective_max_hp() / 4)
        });
        if let Some(damage) = poison_damage {
            self.apply_hp_damage(actor_id, damage);
        }
        self.tick_duration_statuses(actor_id);
        let elapsed_ctb = self.normalize_after_turn();
        self.ctb_since_last_action = elapsed_ctb;
        self.apply_regen(elapsed_ctb);
    }

    fn tick_duration_statuses(&mut self, actor_id: ActorId) {
        let Some(actor) = self.actor_mut(actor_id) else {
            return;
        };
        let statuses = actor
            .status_stacks
            .iter()
            .map(|(status, stacks)| (*status, *stacks))
            .collect::<Vec<_>>();
        for (status, stacks) in statuses {
            if stacks >= 254 || !duration_statuses().contains(&status) {
                continue;
            }
            let stacks = stacks - 1;
            if stacks <= 0 {
                actor.remove_status(status);
            } else {
                actor.set_status(status, stacks);
            }
        }
    }

    fn apply_actor_turn(&mut self, actor_id: ActorId, rank: i32) -> Option<i32> {
        let spent_ctb = {
            let actor = self.actor_mut(actor_id)?;
            let spent_ctb = actor.turn_ctb(rank);
            actor.ctb = (actor.ctb + spent_ctb).max(0);
            spent_ctb
        };
        Some(spent_ctb)
    }

    fn normalize_after_turn(&mut self) -> i32 {
        let min_ctb = self
            .current_party_actors()
            .iter()
            .chain(self.monsters.iter())
            .filter(|actor| actor.is_alive())
            .map(|actor| actor.ctb)
            .min()
            .unwrap_or(0);
        if min_ctb == 0 {
            return 0;
        }
        let party = self.party.clone();
        for actor in &mut self.character_actors {
            if party.contains(&character_id(actor))
                && !matches!(actor.id, ActorId::Character(Character::Unknown))
                && !actor.statuses.contains(&Status::Petrify)
            {
                actor.ctb = (actor.ctb - min_ctb).max(0);
            }
        }
        for actor in &mut self.monsters {
            if !actor.statuses.contains(&Status::Petrify) {
                actor.ctb = (actor.ctb - min_ctb).max(0);
            }
        }
        min_ctb
    }

    fn normalize_ctbs_by(&mut self, ctb: i32) {
        if ctb == 0 {
            return;
        }
        let party = self.party.clone();
        for actor in &mut self.character_actors {
            if party.contains(&character_id(actor))
                && !matches!(actor.id, ActorId::Character(Character::Unknown))
                && !actor.statuses.contains(&Status::Petrify)
            {
                actor.ctb -= ctb;
            }
        }
        for actor in &mut self.monsters {
            if !actor.statuses.contains(&Status::Petrify) {
                actor.ctb -= ctb;
            }
        }
    }

    fn apply_regen(&mut self, elapsed_ctb: i32) {
        for actor in self
            .character_actors
            .iter_mut()
            .chain(self.monsters.iter_mut())
        {
            if !actor.statuses.contains(&Status::Regen) || actor.current_hp <= 0 {
                continue;
            }
            let healing = elapsed_ctb * actor.effective_max_hp() / 256 + 100;
            actor.current_hp =
                (actor.current_hp + healing).clamp(0, actor.effective_max_hp().max(1));
            actor.remove_status(Status::Death);
        }
    }

    fn parse_actor_id(&self, value: &str) -> Option<ActorId> {
        if let Ok(character) = value.parse::<Character>() {
            return Some(ActorId::Character(character));
        }
        value.parse::<MonsterSlot>().ok().map(ActorId::Monster)
    }

    fn explicit_action_targets(&self, action: &ActionData, args: &[String]) -> Vec<ActorId> {
        let Some(target_name) = args.first() else {
            return Vec::new();
        };
        match action.target {
            ActionTarget::Single => self
                .resolve_existing_actor_id(target_name)
                .into_iter()
                .collect(),
            ActionTarget::SingleCharacter | ActionTarget::CounterSingleCharacter => self
                .resolve_character_target_arg(target_name)
                .map(ActorId::Character)
                .into_iter()
                .collect(),
            ActionTarget::SingleMonster => self
                .resolve_existing_monster_target_arg(target_name)
                .map(ActorId::Monster)
                .into_iter()
                .collect(),
            ActionTarget::EitherParty => match target_name.to_ascii_lowercase().as_str() {
                "party" => with_unknown_if_empty(self.possible_character_targets(action)),
                "monsters" => with_unknown_if_empty(self.possible_monster_targets(action)),
                _ => self.resolve_action_target_arg(target_name),
            },
            ActionTarget::CharactersParty
            | ActionTarget::MonstersParty
            | ActionTarget::RandomCharacter
            | ActionTarget::RandomMonster => self
                .resolve_existing_actor_id(target_name)
                .into_iter()
                .collect(),
            _ => Vec::new(),
        }
    }

    fn resolve_action_target_arg(&self, value: &str) -> Vec<ActorId> {
        match value.to_ascii_lowercase().as_str() {
            "party" => {
                with_unknown_if_empty(self.party.iter().copied().map(ActorId::Character).collect())
            }
            "monsters" => {
                with_unknown_if_empty(self.monsters.iter().map(|actor| actor.id).collect())
            }
            _ => self.resolve_existing_actor_id(value).into_iter().collect(),
        }
    }

    fn resolve_actor_id(&self, value: &str) -> Option<ActorId> {
        if let Some(character) = self.resolve_character_target_arg(value) {
            return Some(ActorId::Character(character));
        }
        if let Some(actor_id) = self.parse_actor_id(value) {
            return Some(actor_id);
        }
        self.resolve_monster_target_arg(value).map(ActorId::Monster)
    }

    fn resolve_state_actor_id(&self, value: &str) -> Option<ActorId> {
        match value.to_ascii_lowercase().as_str() {
            "m1" | "m2" | "m3" | "m4" | "m5" | "m6" | "m7" | "m8" => {
                value.parse::<MonsterSlot>().ok().map(ActorId::Monster)
            }
            _ => self
                .resolve_character_target_arg(value)
                .map(ActorId::Character),
        }
    }

    fn resolve_existing_actor_id(&self, value: &str) -> Option<ActorId> {
        if !value.ends_with("_c") {
            if let Some(slot) = self.resolve_existing_monster_target_arg(value) {
                return Some(ActorId::Monster(slot));
            }
        }
        if let Some(character) = self.resolve_character_target_arg(value) {
            return Some(ActorId::Character(character));
        }
        self.resolve_existing_monster_target_arg(value)
            .map(ActorId::Monster)
    }

    fn resolve_character_target_arg(&self, value: &str) -> Option<Character> {
        value
            .strip_suffix("_c")
            .unwrap_or(value)
            .parse::<Character>()
            .ok()
    }

    fn resolve_existing_monster_target_arg(&self, value: &str) -> Option<MonsterSlot> {
        if let Ok(slot) = value.parse::<MonsterSlot>() {
            return self.actor(ActorId::Monster(slot)).map(|_| slot);
        }
        self.resolve_monster_target_arg(value)
            .or_else(|| self.resolve_existing_monster_slot_by_name(value))
    }

    fn empty_monster_slot_error(&self, value: &str) -> Option<String> {
        let slot = value.parse::<MonsterSlot>().ok()?;
        self.actor(ActorId::Monster(slot))
            .is_none()
            .then(|| format!("Error: No monster in slot {}", slot.0))
    }

    fn resolve_monster_target_arg(&self, value: &str) -> Option<MonsterSlot> {
        if let Ok(slot) = value.parse::<MonsterSlot>() {
            return Some(slot);
        }
        let value_family = monster_family(value);
        self.monsters
            .iter()
            .chain(self.temporary_monsters.iter().rev())
            .filter(|actor| actor.is_alive())
            .filter(|actor| {
                actor
                    .monster_key
                    .as_deref()
                    .is_some_and(|key| key == value || monster_family(key) == value_family)
            })
            .next()
            .map(|actor| actor.id)
            .and_then(|actor_id| match actor_id {
                ActorId::Monster(slot) => Some(slot),
                ActorId::Character(_) => None,
            })
    }

    fn resolve_existing_monster_slot_by_name(&self, value: &str) -> Option<MonsterSlot> {
        let value_family = monster_family(value);
        self.monsters
            .iter()
            .filter(|actor| {
                actor
                    .monster_key
                    .as_deref()
                    .is_some_and(|key| key == value || monster_family(key) == value_family)
            })
            .filter_map(|actor| match actor.id {
                ActorId::Monster(slot) => Some(slot),
                ActorId::Character(_) => None,
            })
            .next()
    }

    fn actor_mut(&mut self, actor_id: ActorId) -> Option<&mut BattleActor> {
        self.character_actors
            .iter_mut()
            .chain(self.monsters.iter_mut())
            .chain(self.temporary_monsters.iter_mut())
            .chain(self.retired_temporary_monsters.iter_mut())
            .find(|actor| actor.id == actor_id)
    }

    fn actor(&self, actor_id: ActorId) -> Option<&BattleActor> {
        self.character_actors
            .iter()
            .chain(self.monsters.iter())
            .chain(self.temporary_monsters.iter())
            .chain(self.retired_temporary_monsters.iter())
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
            if self.party.contains(&character_id(actor))
                && !matches!(actor.id, ActorId::Character(Character::Unknown))
            {
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

    fn available_ctb_string(&self) -> String {
        let party = self
            .current_party_actors()
            .into_iter()
            .filter(|actor| actor.can_take_turn())
            .collect::<Vec<_>>();
        let monsters = self
            .monsters
            .iter()
            .filter(|actor| actor.can_take_turn())
            .cloned()
            .collect::<Vec<_>>();
        BattleState::new(party, monsters).ctb_order_string()
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

fn ensure_blank_line(lines: &mut Vec<String>) {
    if lines.is_empty() || lines.last().is_some_and(|line| line.is_empty()) {
        return;
    }
    lines.push(String::new());
}

fn append_rendered_lines(rendered: &mut Vec<String>, rendered_line: &str) {
    for line in rendered_line.split('\n') {
        if line.trim_start().starts_with("encounter ") {
            ensure_blank_line(rendered);
        }
        if let Some(current_side) = output_block_side(line) {
            if let Some(previous_side) = last_output_block_side(rendered) {
                if current_side != previous_side {
                    ensure_blank_line(rendered);
                }
            }
        }
        rendered.push(line.to_string());
    }
}

fn multiline_comment_state(lines: &[String]) -> bool {
    let mut state = false;
    for line in lines {
        let stripped = line.trim();
        if stripped.starts_with("/*") {
            state = true;
        }
        if state && stripped.ends_with("*/") {
            state = false;
        }
    }
    state
}

fn last_output_block_side(lines: &[String]) -> Option<OutputBlockSide> {
    lines.iter().rev().find_map(|line| output_block_side(line))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputBlockSide {
    Party,
    Enemy,
}

fn output_block_side(line: &str) -> Option<OutputBlockSide> {
    let stripped = line.trim();
    if stripped.is_empty() || stripped.starts_with('#') {
        return None;
    }
    let token = stripped.split_whitespace().next()?;
    if token == "spawn" {
        return Some(OutputBlockSide::Enemy);
    }
    if token.parse::<Character>().is_ok() {
        return Some(OutputBlockSide::Party);
    }
    if token.parse::<MonsterSlot>().is_ok() || data::monster_stats(token).is_some() {
        return Some(OutputBlockSide::Enemy);
    }
    None
}

#[derive(Debug, Clone)]
struct MagusCommandData {
    id: i32,
    name: &'static str,
    action_key: &'static str,
    actor: Option<Character>,
    args: Vec<String>,
    break_motivation: i32,
}

enum MagusResolvedAction {
    Action {
        action_key: &'static str,
        args: Vec<String>,
        motivation: i32,
    },
    MindyReflectedActions {
        first_action_key: &'static str,
        first_motivation: i32,
        remaining_actions: usize,
    },
    MindyReflectedRepeatList {
        action_keys: Vec<&'static str>,
        motivation: i32,
    },
    TakingBreak(Option<i32>),
}

fn magus_sister_from_prefix(name: &str) -> Option<Character> {
    if name.is_empty() {
        None
    } else if "cindy".starts_with(name) {
        Some(Character::Cindy)
    } else if "sandy".starts_with(name) {
        Some(Character::Sandy)
    } else if "mindy".starts_with(name) {
        Some(Character::Mindy)
    } else {
        None
    }
}

fn magus_command_list(sister: Character, last_command: Option<i32>) -> Vec<MagusCommandData> {
    let mut commands = Vec::new();
    commands.push(MagusCommandData {
        id: 0,
        name: "Do as you will.",
        action_key: "attack",
        actor: None,
        args: vec!["m1".to_string()],
        break_motivation: -5,
    });
    if let Some(last_command) = last_command {
        let mut repeated = magus_command_for_id(sister, last_command);
        repeated.id = 1;
        repeated.name = "One more time.";
        repeated.break_motivation = if sister == Character::Mindy { -15 } else { -6 };
        commands.push(repeated);
    }
    match sister {
        Character::Cindy => {
            commands.push(MagusCommandData {
                id: 2,
                name: "Fight!",
                action_key: first_existing_action_key(&["camisade", "attack"]),
                actor: None,
                args: vec!["m1".to_string()],
                break_motivation: -5,
            });
            commands.push(MagusCommandData {
                id: 3,
                name: "Go, go!",
                action_key: "attack",
                actor: None,
                args: vec!["m1".to_string()],
                break_motivation: -5,
            });
            commands.push(MagusCommandData {
                id: 4,
                name: "Help each other!",
                action_key: first_existing_action_key(&["cure", "attack"]),
                actor: None,
                args: vec![Character::Cindy.input_name().to_string()],
                break_motivation: -5,
            });
        }
        Character::Sandy => {
            commands.push(MagusCommandData {
                id: 2,
                name: "Fight!",
                action_key: first_existing_action_key(&["razzia", "attack"]),
                actor: None,
                args: vec!["m1".to_string()],
                break_motivation: -5,
            });
            commands.push(MagusCommandData {
                id: 5,
                name: "Defense!",
                action_key: first_existing_action_key(&["shell", "attack"]),
                actor: None,
                args: vec![Character::Cindy.input_name().to_string()],
                break_motivation: -5,
            });
        }
        Character::Mindy => {
            commands.push(MagusCommandData {
                id: 2,
                name: "Fight!",
                action_key: first_existing_action_key(&["passado", "attack"]),
                actor: None,
                args: vec!["m1".to_string()],
                break_motivation: -5,
            });
            commands.push(MagusCommandData {
                id: 6,
                name: "Are you all right?",
                action_key: first_existing_action_key(&["lancet", "attack"]),
                actor: None,
                args: vec!["m1".to_string()],
                break_motivation: -1,
            });
        }
        _ => {}
    }
    commands.push(MagusCommandData {
        id: 7,
        name: "Combine powers!",
        action_key: first_existing_action_key(&["delta_attack", "attack"]),
        actor: Some(Character::Cindy),
        args: vec!["monsters".to_string()],
        break_motivation: -10,
    });
    commands.push(MagusCommandData {
        id: 8,
        name: "Dismiss",
        action_key: "dismiss",
        actor: None,
        args: Vec::new(),
        break_motivation: -5,
    });
    commands.push(MagusCommandData {
        id: 9,
        name: "Auto-Life",
        action_key: "auto-life_counter",
        actor: None,
        args: Vec::new(),
        break_motivation: -5,
    });
    commands
}

fn magus_command_for_id(sister: Character, command_id: i32) -> MagusCommandData {
    magus_command_list(sister, None)
        .into_iter()
        .find(|command| command.id == command_id)
        .unwrap_or(MagusCommandData {
            id: 0,
            name: "Do as you will.",
            action_key: "attack",
            actor: None,
            args: vec!["m1".to_string()],
            break_motivation: -5,
        })
}

fn first_existing_action_key(keys: &[&'static str]) -> &'static str {
    keys.iter()
        .copied()
        .find(|key| data::action_data(key).is_some())
        .unwrap_or_else(|| keys[0])
}

fn magus_command_key(command: &str) -> String {
    command
        .to_ascii_lowercase()
        .replace(' ', "_")
        .replace(['(', ')', '\''], "")
}

fn magus_command_can_be_repeated(command_id: i32) -> bool {
    matches!(command_id, 0 | 2 | 3 | 4 | 5 | 6)
}

fn magus_rng_check(rng: &mut FfxRngTracker, chance: i32) -> bool {
    let chance = chance.clamp(0, 255) as u32;
    (rng.advance_rng(18) & 255) < chance
}

fn magus_command_lacks_monster_target(command: &MagusCommandData, monsters_empty: bool) -> bool {
    monsters_empty
        && command.id != 0
        && command
            .args
            .iter()
            .any(|arg| matches!(arg.as_str(), "m1" | "monsters"))
}

fn format_magus_command_output(
    sister: Character,
    command_name: &str,
    motivation: i32,
    output: &str,
) -> String {
    if output.starts_with("Error:") {
        return output.to_string();
    }
    let mut lines = output.lines().map(str::to_string).collect::<Vec<_>>();
    let sister_prefix = sister.display_name();
    let Some(line_index) = lines
        .iter()
        .position(|line| line.starts_with(sister_prefix) && line.contains("->"))
        .or_else(|| lines.iter().position(|line| line.contains("->")))
    else {
        return output.to_string();
    };
    let Some(index) = lines[line_index].find("->") else {
        return output.to_string();
    };
    let insert = format!("-> {command_name} [{motivation}/100] ");
    lines[line_index].insert_str(index, &insert);
    if !lines[line_index].starts_with(sister_prefix) {
        return lines.join("\n");
    }
    let padding = " ".repeat(insert.len());
    for line in lines.iter_mut().skip(line_index + 1) {
        line.insert_str(0, &padding);
    }
    lines.join("\n")
}

fn format_magus_multi_action_output(
    sister: Character,
    command_name: &str,
    motivation: i32,
    action_outputs: &[String],
) -> String {
    let sister_action_prefix = format!("{} -> ", sister.display_name());
    let mut lines = vec![format!(
        "{} -> {command_name} [{motivation}/100]:",
        sister.display_name()
    )];
    for output in action_outputs {
        for line in output.lines() {
            let action_line = line
                .strip_prefix(&sister_action_prefix)
                .unwrap_or(line)
                .to_string();
            lines.push(format!("    {action_line}"));
        }
    }
    lines.join("\n")
}

fn yojimbo_action(name: &str) -> Option<YojimboActionData> {
    YOJIMBO_ACTIONS
        .iter()
        .copied()
        .find(|action| action.key == name)
}

fn yojimbo_gil_to_motivation(gil: i64) -> i32 {
    let motivation = ((gil as f64 / YOJIMBO_GIL_MOTIVATION_MODIFIER as f64).log2() as i32)
        * YOJIMBO_GIL_MOTIVATION_MODIFIER as i32;
    motivation.max(0)
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

pub fn simulate(seed: u32, lines: &[String]) -> SimulationOutput {
    SimulationState::new(seed).run_lines(lines)
}

fn character_id(actor: &BattleActor) -> Character {
    match actor.id {
        ActorId::Character(character) => character,
        ActorId::Monster(_) => Character::Unknown,
    }
}

fn expand_targets_for_hits(targets: Vec<ActorId>, n_of_hits: i32) -> Vec<ActorId> {
    let n_of_hits = n_of_hits.max(1) as usize;
    if n_of_hits <= 1 {
        return targets;
    }
    let mut expanded = Vec::with_capacity(targets.len() * n_of_hits);
    for _ in 0..n_of_hits {
        expanded.extend(targets.iter().copied());
    }
    expanded
}

fn effective_target_hits(action: &ActionData, args: &[String]) -> i32 {
    let requested_hits = args.iter().find_map(|arg| arg.parse::<i32>().ok());
    if action.overdrive_user == Some(Character::Lulu) {
        requested_hits.unwrap_or(1).clamp(0, 16)
    } else if action.overdrive_user == Some(Character::Wakka) && action.base_damage == 10 {
        timed_overdrive_hit_count(args).clamp(0, 12)
    } else {
        action.n_of_hits
    }
}

fn timed_overdrive_hit_count(args: &[String]) -> i32 {
    if args.first().is_some_and(|arg| arg.parse::<f64>().is_ok()) {
        decimal_i32_arg(args, 1)
            .or_else(|| decimal_i32_arg(args, 2))
            .unwrap_or(1)
    } else {
        decimal_i32_arg(args, 2).unwrap_or(1)
    }
}

fn decimal_i32_arg(args: &[String], index: usize) -> Option<i32> {
    let arg = args.get(index)?;
    (!arg.is_empty() && arg.chars().all(|character| character.is_ascii_digit()))
        .then(|| arg.parse::<i32>().ok())
        .flatten()
}

fn overdrive_time_remaining_ms(action: &ActionData, args: &[String]) -> i32 {
    if !matches!(
        action.overdrive_user,
        Some(Character::Tidus | Character::Auron | Character::Wakka)
    ) {
        return 0;
    }
    let seconds = args
        .first()
        .and_then(|arg| arg.parse::<f64>().ok())
        .or_else(|| args.get(1).and_then(|arg| arg.parse::<f64>().ok()))
        .unwrap_or(0.0);
    (seconds * 1000.0) as i32
}

fn gil_damage_parameter_ms(args: &[String]) -> i32 {
    args.iter()
        .find_map(|arg| arg.parse::<i32>().ok().filter(|amount| *amount >= 0))
        .unwrap_or(0)
        .saturating_mul(1000)
}

fn overdrive_timer_ms(character: Option<Character>) -> Option<i32> {
    match character {
        Some(Character::Tidus) => Some(3_000),
        Some(Character::Auron) => Some(4_000),
        Some(Character::Wakka) => Some(20_000),
        _ => None,
    }
}

fn with_unknown_if_empty(targets: Vec<ActorId>) -> Vec<ActorId> {
    if targets.is_empty() {
        vec![ActorId::Character(Character::Unknown)]
    } else {
        targets
    }
}

fn actor_sort_index(actor_id: ActorId) -> usize {
    match actor_id {
        ActorId::Character(character) => character as usize,
        ActorId::Monster(slot) => slot.0,
    }
}

fn format_condition(condition: EncounterCondition) -> &'static str {
    match condition {
        EncounterCondition::Preemptive => "Preemptive",
        EncounterCondition::Normal => "Normal",
        EncounterCondition::Ambush => "Ambush",
    }
}

fn action_target_accepts_explicit_monster(target: &ActionTarget) -> bool {
    !matches!(
        target,
        ActionTarget::SelfTarget
            | ActionTarget::SingleCharacter
            | ActionTarget::CounterSingleCharacter
            | ActionTarget::Character(_)
            | ActionTarget::CounterSelf
            | ActionTarget::CounterCharactersParty
            | ActionTarget::CounterRandomCharacter
            | ActionTarget::CounterAll
            | ActionTarget::CounterLastTarget
            | ActionTarget::Counter
            | ActionTarget::None
    )
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
    let max_mp = stats
        .as_ref()
        .map(|stats| stats.max_mp)
        .filter(|max_mp| *max_mp > 0)
        .unwrap_or(fallback.3);
    let mut actor = BattleActor::character(character, index, agility, max_hp, max_mp);
    if let Some(stats) = stats {
        let combat_stats =
            fallback_character_combat_stats(character).unwrap_or_else(|| CombatStats {
                strength: stats.strength,
                defense: stats.defense,
                magic: stats.magic,
                magic_defense: stats.magic_defense,
                luck: stats.luck,
                evasion: stats.evasion,
                accuracy: stats.accuracy,
                base_weapon_damage: stats.base_weapon_damage,
            });
        actor.set_combat_stats(CombatStats {
            base_weapon_damage: stats.base_weapon_damage,
            ..combat_stats
        });
        actor.equipment_crit = stats.equipment_crit;
        actor.weapon_bonus_crit = stats.weapon_bonus_crit;
        actor.armor_bonus_crit = stats.armor_bonus_crit;
        actor.set_weapon_slots(stats.weapon_slots);
        actor.set_armor_slots(stats.armor_slots);
        actor.set_weapon_abilities(stats.weapon_abilities.iter().copied().collect());
        actor.set_armor_abilities(stats.armor_abilities.iter().copied().collect());
        apply_auto_statuses(&mut actor);
        apply_equipment_elements(&mut actor);
        apply_equipment_status_resistances(&mut actor);
        apply_equipment_resource_bonuses(&mut actor);
    }
    actor
}

fn normalize_encounter_name(name: &str) -> String {
    if name.is_empty() {
        "dummy".to_string()
    } else if "simulated".starts_with(name) {
        "simulation".to_string()
    } else if "normal".starts_with(name) {
        "dummy_normal".to_string()
    } else if "preemptive".starts_with(name) {
        "dummy_preemptive".to_string()
    } else if "ambush".starts_with(name) {
        "dummy_ambush".to_string()
    } else {
        name.to_string()
    }
}

fn fallback_character_defaults(character: Character) -> (usize, u8, i32, i32) {
    match character {
        Character::Tidus => (0, 10, 520, 12),
        Character::Yuna => (1, 10, 475, 84),
        Character::Auron => (2, 5, 1030, 33),
        Character::Kimahri => (3, 6, 644, 78),
        Character::Wakka => (4, 7, 618, 10),
        Character::Lulu => (5, 5, 380, 92),
        Character::Rikku => (6, 16, 360, 85),
        Character::Seymour => (7, 20, 1200, 999),
        Character::Valefor => (8, 0, 99_999, 9_999),
        Character::Ifrit => (9, 0, 99_999, 9_999),
        Character::Ixion => (10, 0, 99_999, 9_999),
        Character::Shiva => (11, 0, 99_999, 9_999),
        Character::Bahamut => (12, 0, 99_999, 9_999),
        Character::Anima => (13, 0, 99_999, 9_999),
        Character::Yojimbo => (14, 0, 99_999, 9_999),
        Character::Cindy => (15, 10, 2_190, 46),
        Character::Sandy => (16, 10, 1_790, 35),
        Character::Mindy => (17, 12, 1_237, 58),
        Character::Unknown => (18, 0, 99_999, 9_999),
    }
}

fn fallback_character_combat_stats(character: Character) -> Option<CombatStats> {
    let (strength, defense, magic, magic_defense, luck, evasion, accuracy) = match character {
        Character::Cindy => (28, 32, 21, 28, 17, 20, 11),
        Character::Sandy => (42, 26, 24, 28, 17, 17, 13),
        Character::Mindy => (23, 24, 28, 28, 17, 23, 12),
        _ => return None,
    };
    Some(CombatStats {
        strength,
        defense,
        magic,
        magic_defense,
        luck,
        evasion,
        accuracy,
        base_weapon_damage: 16,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MonsterTemplate {
    key: String,
    display_name: String,
    agility: u8,
    immune_to_delay: bool,
    max_hp: i32,
    max_mp: i32,
    combat_stats: CombatStats,
    elemental_affinities: std::collections::HashMap<Element, ElementalAffinity>,
    status_resistances: std::collections::HashMap<Status, u8>,
    armored: bool,
    immune_to_damage: bool,
    immune_to_percentage_damage: bool,
    immune_to_physical_damage: bool,
    immune_to_magical_damage: bool,
    immune_to_life: bool,
    immune_to_bribe: bool,
    auto_statuses: Vec<Status>,
}

fn monster_template(name: &str) -> MonsterTemplate {
    let template = data::monster_stats(name)
        .map(|stats| MonsterTemplate {
            key: stats.key,
            display_name: stats.display_name,
            agility: stats.agility,
            immune_to_delay: stats.immune_to_delay,
            max_hp: stats.max_hp,
            max_mp: stats.max_mp,
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
            immune_to_bribe: stats.immune_to_bribe,
            auto_statuses: stats.auto_statuses,
        })
        .unwrap_or_else(|| MonsterTemplate {
            key: name.to_string(),
            display_name: name.to_string(),
            agility: 10,
            immune_to_delay: false,
            max_hp: 1_000,
            max_mp: 0,
            combat_stats: CombatStats::default(),
            elemental_affinities: crate::battle::neutral_elemental_affinities(),
            status_resistances: std::collections::HashMap::new(),
            armored: false,
            immune_to_damage: false,
            immune_to_percentage_damage: false,
            immune_to_physical_damage: false,
            immune_to_magical_damage: false,
            immune_to_life: false,
            immune_to_bribe: false,
            auto_statuses: Vec::new(),
        });
    template
}

fn create_monster_actor(monster_name: &str, slot: MonsterSlot) -> Option<BattleActor> {
    data::monster_stats(monster_name)?;
    let template = monster_template(monster_name);
    let mut actor = BattleActor::monster_with_key(
        slot,
        Some(monster_name.to_string()),
        template.agility,
        template.immune_to_delay,
        template.max_hp,
    );
    actor.max_mp = template.max_mp;
    actor.current_mp = template.max_mp;
    actor.set_combat_stats(template.combat_stats);
    actor.set_elemental_affinities(template.elemental_affinities.clone());
    actor.set_status_resistances(template.status_resistances.clone());
    apply_damage_traits(&mut actor, &template);
    apply_template_auto_statuses(&mut actor, &template);
    Some(actor)
}

fn apply_damage_traits(actor: &mut BattleActor, template: &MonsterTemplate) {
    actor.armored = template.armored;
    actor.immune_to_damage = template.immune_to_damage;
    actor.immune_to_percentage_damage = template.immune_to_percentage_damage;
    actor.immune_to_physical_damage = template.immune_to_physical_damage;
    actor.immune_to_magical_damage = template.immune_to_magical_damage;
    actor.immune_to_life = template.immune_to_life;
    actor.immune_to_bribe = template.immune_to_bribe;
}

fn apply_template_auto_statuses(actor: &mut BattleActor, template: &MonsterTemplate) {
    for status in &template.auto_statuses {
        actor.set_status(*status, 254);
    }
}

fn fallback_action_rank(action: &str) -> i32 {
    data::action_rank(action).unwrap_or_else(|| match action.to_ascii_lowercase().as_str() {
        "escape" | "quick_hit_ps2" => 1,
        "defend" | "quick_hit_hd" | "use" => 2,
        "haste" => 4,
        "delay_attack" => 6,
        "delay_buster" => 8,
        _ => 3,
    })
}

fn infer_encounter_parties(lines: &[String]) -> HashMap<usize, Vec<Character>> {
    let mut encounter_parties = HashMap::new();
    let mut current_encounter_index = None;
    let mut current_party = Vec::new();

    for (index, raw_line) in lines.iter().enumerate() {
        if raw_line.starts_with('#') {
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

fn infer_encounter_party_swaps(lines: &[String]) -> HashSet<usize> {
    let mut encounter_party_swaps = HashSet::new();
    let mut current_encounter_index = None;

    for (index, raw_line) in lines.iter().enumerate() {
        if raw_line.trim_start().starts_with('#') {
            continue;
        }
        match parse_raw_action_line(raw_line) {
            ParsedCommand::Encounter { .. } => {
                current_encounter_index = Some(index);
            }
            ParsedCommand::Party { initials } if !initials.is_empty() => {
                if let Some(encounter_index) = current_encounter_index {
                    encounter_party_swaps.insert(encounter_index);
                }
            }
            _ => {}
        }
    }

    encounter_party_swaps
}

fn action_is_counter(action: &ActionData) -> bool {
    matches!(
        action.target,
        ActionTarget::Counter
            | ActionTarget::CounterSelf
            | ActionTarget::CounterSingleCharacter
            | ActionTarget::CounterRandomCharacter
            | ActionTarget::CounterCharactersParty
            | ActionTarget::CounterAll
            | ActionTarget::CounterLastTarget
    ) || action.key.contains("counter")
}

fn calculate_action_damage(
    user: &BattleActor,
    target: &BattleActor,
    action: &ActionData,
    damage_rng: u32,
    crit: bool,
    od_time_remaining: i32,
    pool: DamagePool,
) -> i32 {
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
        DamageFormula::Fixed => base_damage * 50 * (damage_rng as i32 + 0xf0) / 256,
        DamageFormula::FixedNoVariance => base_damage * 50,
        DamageFormula::PercentageTotal => {
            if target.immune_to_percentage_damage {
                0
            } else if pool == DamagePool::Mp {
                target.effective_max_mp() * base_damage / 16
            } else {
                target.effective_max_hp() * base_damage / 16
            }
        }
        DamageFormula::PercentageCurrent => {
            if target.immune_to_percentage_damage {
                0
            } else if pool == DamagePool::Mp {
                target.current_mp * base_damage / 16
            } else {
                target.current_hp * base_damage / 16
            }
        }
        DamageFormula::PercentageTotalMp => target.effective_max_mp() * base_damage / 16,
        DamageFormula::PercentageCurrentMp => target.current_mp * base_damage / 16,
        DamageFormula::Hp => user.effective_max_hp() * base_damage / 10,
        DamageFormula::Ctb | DamageFormula::BaseCtb => target.ctb * base_damage / 16,
        DamageFormula::Deal9999 => 9999 * action.base_damage,
        DamageFormula::Strength
        | DamageFormula::PiercingStrength
        | DamageFormula::Magic
        | DamageFormula::PiercingMagic
        | DamageFormula::SpecialMagic
        | DamageFormula::SpecialMagicNoVariance
        | DamageFormula::PiercingStrengthNoVariance
        | DamageFormula::Healing => {
            stat_based_action_damage(user, target, action, base_damage, damage_rng)
        }
        DamageFormula::Gil => (od_time_remaining / 1000) / 10,
        DamageFormula::CelestialHighHp
        | DamageFormula::CelestialHighMp
        | DamageFormula::CelestialLowHp
        | DamageFormula::Kills => 0,
    };
    if crit {
        damage *= 2;
    }
    damage = apply_damage_status_modifiers(damage, user, target, action, od_time_remaining);
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

fn damage_rng_index(actor: &BattleActor) -> usize {
    match actor.id {
        ActorId::Character(_) => (20 + actor.index).min(27),
        ActorId::Monster(_) => (28 + actor.index).min(35),
    }
}

fn hit_rng_index(actor: &BattleActor) -> usize {
    match actor.id {
        ActorId::Character(_) => (36 + actor.index).min(43),
        ActorId::Monster(_) => (44 + actor.index).min(51),
    }
}

fn base_hit_chance(user: &BattleActor, target: &BattleActor, action: &ActionData) -> Option<i32> {
    if action.uses_hit_chance_table {
        let hit_chance = match action.hit_chance_formula {
            HitChanceFormula::UseAccuracy => user.combat_stats.accuracy,
            HitChanceFormula::UseAccuracyX25 => user.combat_stats.accuracy * 5 / 2,
            HitChanceFormula::UseAccuracyX15 => user.combat_stats.accuracy * 3 / 2,
            HitChanceFormula::UseAccuracyX05 => user.combat_stats.accuracy / 2,
            HitChanceFormula::Always | HitChanceFormula::UseActionAccuracy => {
                user.combat_stats.accuracy
            }
        };
        let scaled = python_div_i64(hit_chance as i64 * 2 * 0x6666_6667, 0xffff_ffff) / 2;
        let table_index = (scaled as i32 - target.combat_stats.evasion + 10).clamp(0, 8);
        return Some(HIT_CHANCE_TABLE[table_index as usize]);
    }
    if action.hit_chance_formula == HitChanceFormula::UseActionAccuracy {
        return Some(action.accuracy - target.combat_stats.evasion);
    }
    None
}

fn temporary_statuses() -> [Status; 5] {
    [
        Status::Defend,
        Status::Shield,
        Status::Boost,
        Status::Guard,
        Status::Sentinel,
    ]
}

fn duration_statuses() -> [Status; 5] {
    [
        Status::Sleep,
        Status::Silence,
        Status::Dark,
        Status::Regen,
        Status::Slow,
    ]
}

fn nul_status_for_element(element: Element) -> Option<Status> {
    match element {
        Element::Fire => Some(Status::NulBlaze),
        Element::Ice => Some(Status::NulFrost),
        Element::Thunder => Some(Status::NulShock),
        Element::Water => Some(Status::NulTide),
        Element::Holy => None,
    }
}

fn stat_based_action_damage(
    user: &BattleActor,
    target: &BattleActor,
    action: &ActionData,
    base_damage: i32,
    damage_rng: u32,
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
    if matches!(
        action.damage_formula,
        DamageFormula::PiercingStrengthNoVariance | DamageFormula::SpecialMagicNoVariance
    ) {
        damage
    } else {
        damage * (damage_rng as i32 + 0xf0) / 256
    }
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
    od_time_remaining: i32,
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
        if user.has_auto_ability(AutoAbility::MagicBooster) {
            damage = damage * 3 / 2;
        }
        if target.statuses.contains(&Status::Shell) {
            damage /= 2;
        }
        if user.statuses.contains(&Status::MagicBreak) {
            damage /= 2;
        }
    }
    if action.affected_by_alchemy && user.has_auto_ability(AutoAbility::Alchemy) {
        damage *= 2;
    }
    let offensive_bonus = match action.damage_type {
        DamageType::Physical => equipment_stat_bonus(user, strength_bonus_abilities()),
        DamageType::Magical => equipment_stat_bonus(user, magic_bonus_abilities()),
        DamageType::Other => 0,
    };
    let defensive_bonus = match action.damage_type {
        DamageType::Physical => equipment_stat_bonus(target, defense_bonus_abilities()),
        DamageType::Magical => equipment_stat_bonus(target, magic_defense_bonus_abilities()),
        DamageType::Other => 0,
    };
    damage += damage * offensive_bonus / 100;
    damage -= damage * defensive_bonus / 100;
    if target.armored
        && action.damage_type == DamageType::Physical
        && !action.ignores_armored
        && !user.has_auto_ability(AutoAbility::Piercing)
        && !target.statuses.contains(&Status::ArmorBreak)
    {
        damage /= 3;
    }
    if let Some(timer) = overdrive_timer_ms(action.overdrive_user) {
        let od_time_remaining = od_time_remaining.min(timer);
        damage += damage * od_time_remaining / (timer * 2);
    }
    let damage_limit = if action.never_break_damage_limit {
        9_999
    } else if action.always_break_damage_limit || user.break_damage_limit {
        99_999
    } else {
        9_999
    };
    damage.min(damage_limit)
}

fn equipment_stat_bonus(actor: &BattleActor, bonuses: &[(AutoAbility, i32)]) -> i32 {
    bonuses
        .iter()
        .filter_map(|(ability, bonus)| actor.has_auto_ability(*ability).then_some(*bonus))
        .sum()
}

fn strength_bonus_abilities() -> &'static [(AutoAbility, i32)] {
    &[
        (AutoAbility::Strength3, 3),
        (AutoAbility::Strength5, 5),
        (AutoAbility::Strength10, 10),
        (AutoAbility::Strength20, 20),
    ]
}

fn magic_bonus_abilities() -> &'static [(AutoAbility, i32)] {
    &[
        (AutoAbility::Magic3, 3),
        (AutoAbility::Magic5, 5),
        (AutoAbility::Magic10, 10),
        (AutoAbility::Magic20, 20),
    ]
}

fn defense_bonus_abilities() -> &'static [(AutoAbility, i32)] {
    &[
        (AutoAbility::Defense3, 3),
        (AutoAbility::Defense5, 5),
        (AutoAbility::Defense10, 10),
        (AutoAbility::Defense20, 20),
    ]
}

fn magic_defense_bonus_abilities() -> &'static [(AutoAbility, i32)] {
    &[
        (AutoAbility::MagicDefense3, 3),
        (AutoAbility::MagicDefense5, 5),
        (AutoAbility::MagicDefense10, 10),
        (AutoAbility::MagicDefense20, 20),
    ]
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

fn parse_summon_party(name: &str) -> Option<Vec<Character>> {
    let name = name.to_ascii_lowercase();
    if "magus_sisters".starts_with(&name) {
        return Some(vec![Character::Cindy, Character::Sandy, Character::Mindy]);
    }
    [
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
    ]
    .into_iter()
    .find(|aeon| aeon.input_name().starts_with(&name))
    .map(|aeon| vec![aeon])
}

fn is_aeon_character(character: Character) -> bool {
    matches!(
        character,
        Character::Valefor
            | Character::Ifrit
            | Character::Ixion
            | Character::Shiva
            | Character::Bahamut
            | Character::Anima
            | Character::Yojimbo
            | Character::Cindy
            | Character::Sandy
            | Character::Mindy
    )
}

fn apply_haste_to_ctb(actor: &BattleActor, ctb: i32) -> i32 {
    if actor.statuses.contains(&Status::Haste) {
        ctb / 2
    } else {
        ctb
    }
}

struct ParsedEquipmentAbilities {
    modeled: HashSet<AutoAbility>,
    display_names: Vec<String>,
}

fn parse_equipment_abilities(args: &[String]) -> Result<ParsedEquipmentAbilities, String> {
    let mut accepted = HashSet::new();
    let mut modeled = HashSet::new();
    let mut display_names = Vec::new();
    for ability_name in args.iter().skip(2) {
        if accepted.len() >= 4 {
            break;
        }
        let Some(canonical) = data::autoability_name_by_key(ability_name) else {
            return Err(format!(
                "Error: ability can only be one of these values: {}",
                autoability_values()
            ));
        };
        if !accepted.insert(equipment_ability_key(canonical)) {
            continue;
        }
        if let Ok(ability) = ability_name.parse::<AutoAbility>() {
            modeled.insert(ability);
        }
        display_names.push(canonical.to_string());
    }
    Ok(ParsedEquipmentAbilities {
        modeled,
        display_names,
    })
}

fn parse_inventory_equipment_abilities(
    ability_args: &[String],
) -> Result<ParsedEquipmentAbilities, String> {
    let mut accepted = HashSet::new();
    let mut modeled = HashSet::new();
    let mut display_names = Vec::new();
    for ability_name in ability_args {
        if accepted.len() >= 4 {
            break;
        }
        let Some(canonical) = data::autoability_names_in_order()
            .into_iter()
            .find(|ability| python_enum_value(ability) == *ability_name)
        else {
            return Err(format!(
                "Error: ability can only be one of these values: {}",
                autoability_values()
            ));
        };
        if !accepted.insert(equipment_ability_key(canonical)) {
            continue;
        }
        if let Ok(ability) = canonical.parse::<AutoAbility>() {
            modeled.insert(ability);
        }
        display_names.push(canonical.to_string());
    }
    Ok(ParsedEquipmentAbilities {
        modeled,
        display_names,
    })
}

fn format_actor_equipment(
    kind: data::EquipmentKind,
    character: Character,
    actor: &BattleActor,
) -> String {
    let (slots, abilities) = match kind {
        data::EquipmentKind::Weapon => (actor.weapon_slots, &actor.weapon_abilities),
        data::EquipmentKind::Armor => (actor.armor_slots, &actor.armor_abilities),
    };
    let display_names = equipment_ability_display_names(abilities);
    format_equipment(kind, character, &display_names, slots)
}

fn format_equipment(
    kind: data::EquipmentKind,
    character: Character,
    ability_names: &[String],
    slots: u8,
) -> String {
    let name = equipment_name(kind, character, ability_names, slots);
    let abilities = format_equipment_ability_slots(ability_names, slots);
    format!("{name} {abilities}")
}

fn format_inventory_equipment(
    kind: data::EquipmentKind,
    character: Character,
    ability_names: &[String],
    slots: u8,
    sell_value: i32,
) -> String {
    let name = equipment_name(kind, character, ability_names, slots);
    let abilities = format_equipment_ability_slots(ability_names, slots);
    format!(
        "{name} ({}) {abilities}[{sell_value} gil]",
        character.display_name()
    )
}

fn format_equipment_ability_slots(ability_names: &[String], slots: u8) -> String {
    let mut rendered = ability_names.to_vec();
    while rendered.len() < slots as usize {
        rendered.push("-".to_string());
    }
    format!("[{}]", rendered.join(", "))
}

fn equipment_kind_display_name(kind: data::EquipmentKind) -> &'static str {
    match kind {
        data::EquipmentKind::Weapon => "Weapon",
        data::EquipmentKind::Armor => "Armor",
    }
}

fn default_equipment_base_weapon_damage(character: Character) -> i32 {
    if matches!(character, Character::Valefor | Character::Shiva) {
        14
    } else {
        16
    }
}

fn equipment_ability_display_names(abilities: &HashSet<AutoAbility>) -> Vec<String> {
    data::autoability_names_in_order()
        .into_iter()
        .filter_map(|name| {
            let ability = name.parse::<AutoAbility>().ok()?;
            abilities.contains(&ability).then(|| name.to_string())
        })
        .collect()
}

fn equipment_gil_value(slots: u8, abilities: &[String]) -> i32 {
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

fn equipment_name(
    kind: data::EquipmentKind,
    owner: Character,
    ability_names: &[String],
    slots: u8,
) -> String {
    match (kind, owner) {
        (data::EquipmentKind::Weapon, Character::Seymour) => return "Seymour Staff".to_string(),
        (data::EquipmentKind::Armor, Character::Seymour) => return "Seymour Armor".to_string(),
        (data::EquipmentKind::Weapon, owner) if !is_standard_equipment_owner(owner) => {
            return format!("{}'s weapon", owner.display_name());
        }
        (data::EquipmentKind::Armor, owner) if !is_standard_equipment_owner(owner) => {
            return format!("{}'s armor", owner.display_name());
        }
        _ => {}
    }
    let ability_keys = ability_names
        .iter()
        .map(|ability| equipment_ability_key(ability))
        .collect::<Vec<_>>();
    let index = match kind {
        data::EquipmentKind::Weapon => equipment_weapon_name_index(&ability_keys, slots),
        data::EquipmentKind::Armor => equipment_armor_name_index(&ability_keys, slots),
    };
    data::equipment_name(kind, owner, index)
        .unwrap_or_else(|| equipment_kind_display_name(kind).to_string())
}

fn is_standard_equipment_owner(owner: Character) -> bool {
    matches!(
        owner,
        Character::Tidus
            | Character::Yuna
            | Character::Auron
            | Character::Kimahri
            | Character::Wakka
            | Character::Lulu
            | Character::Rikku
    )
}

fn equipment_ability_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace("->", "to")
        .replace([' ', '-'], "_")
        .replace(['+', '%', '(', ')', '\''], "")
        .trim_matches('_')
        .to_string()
}

fn equipment_has_ability(abilities: &[String], name: &str) -> bool {
    abilities.iter().any(|ability| ability == name)
}

fn equipment_has_any(abilities: &[String], names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| equipment_has_ability(abilities, name))
}

fn equipment_has_all(abilities: &[String], names: &[&str]) -> bool {
    names
        .iter()
        .all(|name| equipment_has_ability(abilities, name))
}

fn equipment_count_abilities(abilities: &[String], names: &[&str]) -> usize {
    names
        .iter()
        .filter(|name| equipment_has_ability(abilities, name))
        .count()
}

fn equipment_weapon_name_index(abilities: &[String], slots: u8) -> usize {
    let elemental_strikes = equipment_count_abilities(
        abilities,
        &["firestrike", "icestrike", "lightningstrike", "waterstrike"],
    );
    let status_strikes = equipment_count_abilities(
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
    let status_touches = equipment_count_abilities(
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
    let strength_bonuses = equipment_count_abilities(
        abilities,
        &["strength_3", "strength_5", "strength_10", "strength_20"],
    );
    let magic_bonuses =
        equipment_count_abilities(abilities, &["magic_3", "magic_5", "magic_10", "magic_20"]);
    let counter = equipment_has_ability(abilities, "counterattack")
        || equipment_has_ability(abilities, "evade_&_counter");

    if equipment_has_ability(abilities, "capture") {
        2
    } else if elemental_strikes == 4 {
        3
    } else if equipment_has_ability(abilities, "break_damage_limit") {
        4
    } else if equipment_has_all(
        abilities,
        &["triple_overdrive", "triple_ap", "overdrive_to_ap"],
    ) {
        5
    } else if equipment_has_all(abilities, &["triple_overdrive", "overdrive_to_ap"]) {
        6
    } else if equipment_has_all(abilities, &["double_overdrive", "double_ap"]) {
        7
    } else if equipment_has_ability(abilities, "triple_overdrive") {
        8
    } else if equipment_has_ability(abilities, "double_overdrive") {
        9
    } else if equipment_has_ability(abilities, "triple_ap") {
        10
    } else if equipment_has_ability(abilities, "double_ap") {
        11
    } else if equipment_has_ability(abilities, "overdrive_to_ap") {
        12
    } else if equipment_has_ability(abilities, "sos_overdrive") {
        13
    } else if equipment_has_ability(abilities, "one_mp_cost") {
        14
    } else if status_strikes == 4 {
        15
    } else if strength_bonuses == 4 {
        16
    } else if magic_bonuses == 4 {
        17
    } else if equipment_has_ability(abilities, "magic_booster") && magic_bonuses == 3 {
        18
    } else if equipment_has_ability(abilities, "half_mp_cost") {
        19
    } else if equipment_has_ability(abilities, "gillionaire") {
        20
    } else if elemental_strikes == 3 {
        21
    } else if status_strikes == 3 {
        22
    } else if equipment_has_ability(abilities, "magic_counter") && counter {
        23
    } else if counter {
        24
    } else if equipment_has_ability(abilities, "magic_counter") {
        25
    } else if equipment_has_ability(abilities, "magic_booster") {
        26
    } else if equipment_has_ability(abilities, "alchemy") {
        27
    } else if equipment_has_ability(abilities, "first_strike") {
        28
    } else if equipment_has_ability(abilities, "initiative") {
        29
    } else if equipment_has_ability(abilities, "deathstrike") {
        30
    } else if equipment_has_ability(abilities, "slowstrike") {
        31
    } else if equipment_has_ability(abilities, "stonestrike") {
        32
    } else if equipment_has_ability(abilities, "poisonstrike") {
        33
    } else if equipment_has_ability(abilities, "sleepstrike") {
        34
    } else if equipment_has_ability(abilities, "silencestrike") {
        35
    } else if equipment_has_ability(abilities, "darkstrike") {
        36
    } else if strength_bonuses == 3 {
        37
    } else if magic_bonuses == 3 {
        38
    } else if elemental_strikes == 2 {
        39
    } else if status_touches >= 2 {
        40
    } else if equipment_has_ability(abilities, "deathtouch") {
        41
    } else if equipment_has_ability(abilities, "slowtouch") {
        42
    } else if equipment_has_ability(abilities, "stonetouch") {
        43
    } else if equipment_has_ability(abilities, "poisontouch") {
        44
    } else if equipment_has_ability(abilities, "sleeptouch") {
        45
    } else if equipment_has_ability(abilities, "silencetouch") {
        46
    } else if equipment_has_ability(abilities, "darktouch") {
        47
    } else if equipment_has_ability(abilities, "sensor") {
        48
    } else if equipment_has_ability(abilities, "firestrike") {
        49
    } else if equipment_has_ability(abilities, "icestrike") {
        50
    } else if equipment_has_ability(abilities, "lightningstrike") {
        51
    } else if equipment_has_ability(abilities, "waterstrike") {
        52
    } else if equipment_has_ability(abilities, "distill_power") {
        53
    } else if equipment_has_ability(abilities, "distill_mana") {
        54
    } else if equipment_has_ability(abilities, "distill_speed") {
        55
    } else if equipment_has_ability(abilities, "distill_ability") {
        56
    } else if slots == 4 {
        57
    } else if strength_bonuses >= 1 && magic_bonuses >= 1 {
        58
    } else if slots == 2 || slots == 3 {
        59
    } else if equipment_has_any(abilities, &["magic_10", "magic_20"]) {
        60
    } else if equipment_has_any(abilities, &["strength_10", "strength_20"]) {
        61
    } else if equipment_has_ability(abilities, "magic_5") {
        62
    } else if equipment_has_ability(abilities, "magic_3") {
        63
    } else if equipment_has_ability(abilities, "strength_5") {
        64
    } else if equipment_has_ability(abilities, "strength_3") {
        65
    } else if equipment_has_ability(abilities, "piercing") {
        66
    } else {
        67
    }
}

fn equipment_armor_name_index(abilities: &[String], slots: u8) -> usize {
    let elemental_eaters = equipment_count_abilities(
        abilities,
        &["fire_eater", "ice_eater", "lightning_eater", "water_eater"],
    );
    let elemental_proofs = equipment_count_abilities(
        abilities,
        &["fireproof", "iceproof", "lightningproof", "waterproof"],
    );
    let status_proofs = equipment_count_abilities(
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
    let defense_bonuses = equipment_count_abilities(
        abilities,
        &["defense_3", "defense_5", "defense_10", "defense_20"],
    );
    let magic_def_bonuses = equipment_count_abilities(
        abilities,
        &["magic_def_3", "magic_def_5", "magic_def_10", "magic_def_20"],
    );
    let hp_bonuses = equipment_count_abilities(abilities, &["hp_5", "hp_10", "hp_20", "hp_30"]);
    let mp_bonuses = equipment_count_abilities(abilities, &["mp_5", "mp_10", "mp_20", "mp_30"]);
    let auto_statuses = equipment_count_abilities(
        abilities,
        &[
            "auto_shell",
            "auto_protect",
            "auto_haste",
            "auto_regen",
            "auto_reflect",
        ],
    );
    let elemental_sos_auto_statuses = equipment_count_abilities(
        abilities,
        &[
            "sos_nultide",
            "sos_nulfrost",
            "sos_nulshock",
            "sos_nulblaze",
        ],
    );
    let status_soses = equipment_count_abilities(
        abilities,
        &[
            "sos_shell",
            "sos_protect",
            "sos_haste",
            "sos_regen",
            "sos_reflect",
        ],
    );

    if equipment_has_all(abilities, &["break_hp_limit", "break_mp_limit"]) {
        0
    } else if equipment_has_ability(abilities, "ribbon") {
        1
    } else if equipment_has_ability(abilities, "break_hp_limit") {
        2
    } else if equipment_has_ability(abilities, "break_mp_limit") {
        3
    } else if elemental_eaters == 4 {
        4
    } else if elemental_proofs == 4 {
        5
    } else if equipment_has_all(
        abilities,
        &["auto_shell", "auto_protect", "auto_reflect", "auto_regen"],
    ) {
        6
    } else if equipment_has_all(abilities, &["auto_potion", "auto_med", "auto_phoenix"]) {
        7
    } else if equipment_has_all(abilities, &["auto_potion", "auto_med"]) {
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
    } else if equipment_has_ability(abilities, "master_thief") {
        14
    } else if equipment_has_ability(abilities, "pickpocket") {
        15
    } else if equipment_has_all(abilities, &["hp_stroll", "mp_stroll"]) {
        16
    } else if auto_statuses == 3 {
        17
    } else if elemental_eaters == 3 {
        18
    } else if equipment_has_ability(abilities, "hp_stroll") {
        19
    } else if equipment_has_ability(abilities, "mp_stroll") {
        20
    } else if equipment_has_ability(abilities, "auto_phoenix") {
        21
    } else if equipment_has_ability(abilities, "auto_med") {
        22
    } else if elemental_sos_auto_statuses == 4 {
        23
    } else if status_soses == 4 {
        24
    } else if status_proofs == 3 {
        25
    } else if equipment_has_ability(abilities, "no_encounters") {
        26
    } else if equipment_has_ability(abilities, "auto_potion") {
        27
    } else if elemental_proofs == 3 {
        28
    } else if status_soses == 3 {
        29
    } else if auto_statuses == 2 {
        30
    } else if elemental_sos_auto_statuses == 2 {
        31
    } else if equipment_has_any(abilities, &["auto_regen", "sos_regen"]) {
        32
    } else if equipment_has_any(abilities, &["auto_haste", "sos_haste"]) {
        33
    } else if equipment_has_any(abilities, &["auto_reflect", "sos_reflect"]) {
        34
    } else if equipment_has_any(abilities, &["auto_shell", "sos_shell"]) {
        35
    } else if equipment_has_any(abilities, &["auto_protect", "sos_protect"]) {
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
    } else if equipment_has_ability(abilities, "fire_eater") {
        43
    } else if equipment_has_ability(abilities, "ice_eater") {
        44
    } else if equipment_has_ability(abilities, "lightning_eater") {
        45
    } else if equipment_has_ability(abilities, "water_eater") {
        46
    } else if equipment_has_ability(abilities, "curseproof") {
        47
    } else if equipment_has_any(abilities, &["confuse_ward", "confuseproof"]) {
        48
    } else if equipment_has_any(abilities, &["berserk_ward", "berserkproof"]) {
        49
    } else if equipment_has_any(abilities, &["slow_ward", "slowproof"]) {
        50
    } else if equipment_has_any(abilities, &["death_ward", "deathproof"]) {
        51
    } else if equipment_has_any(abilities, &["zombie_ward", "zombieproof"]) {
        52
    } else if equipment_has_any(abilities, &["stone_ward", "stoneproof"]) {
        53
    } else if equipment_has_any(abilities, &["poison_ward", "poisonproof"]) {
        54
    } else if equipment_has_any(abilities, &["sleep_ward", "sleepproof"]) {
        55
    } else if equipment_has_any(abilities, &["silence_ward", "silenceproof"]) {
        56
    } else if equipment_has_any(abilities, &["dark_ward", "darkproof"]) {
        57
    } else if equipment_has_any(abilities, &["fire_ward", "fireproof"]) {
        58
    } else if equipment_has_any(abilities, &["ice_ward", "iceproof"]) {
        59
    } else if equipment_has_any(abilities, &["lightning_ward", "lightningproof"]) {
        60
    } else if equipment_has_any(abilities, &["water_ward", "waterproof"]) {
        61
    } else if equipment_has_ability(abilities, "sos_nultide") {
        62
    } else if equipment_has_ability(abilities, "sos_nulblaze") {
        63
    } else if equipment_has_ability(abilities, "sos_nulshock") {
        64
    } else if equipment_has_ability(abilities, "sos_nulfrost") {
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
    } else if equipment_has_any(abilities, &["defense_10", "defense_20"]) {
        73
    } else if equipment_has_any(abilities, &["magic_def_10", "magic_def_20"]) {
        74
    } else if equipment_has_any(abilities, &["mp_20", "mp_30"]) {
        75
    } else if equipment_has_any(abilities, &["hp_20", "hp_30"]) {
        76
    } else if slots == 3 {
        77
    } else if equipment_has_any(abilities, &["defense_3", "defense_5"]) {
        78
    } else if equipment_has_any(abilities, &["magic_def_3", "magic_def_5"]) {
        79
    } else if equipment_has_any(abilities, &["mp_5", "mp_10"]) {
        80
    } else if equipment_has_any(abilities, &["hp_5", "hp_10"]) {
        81
    } else if slots == 2 {
        82
    } else {
        83
    }
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
            actor.set_status(status, 255);
        }
    }
    if actor.current_hp <= 0 || actor.current_hp * 2 >= actor.effective_max_hp() {
        return;
    }
    for (ability, status) in [
        (AutoAbility::SosShell, Status::Shell),
        (AutoAbility::SosProtect, Status::Protect),
        (AutoAbility::SosHaste, Status::Haste),
        (AutoAbility::SosRegen, Status::Regen),
        (AutoAbility::SosReflect, Status::Reflect),
        (AutoAbility::SosNulTide, Status::NulTide),
        (AutoAbility::SosNulFrost, Status::NulFrost),
        (AutoAbility::SosNulShock, Status::NulShock),
        (AutoAbility::SosNulBlaze, Status::NulBlaze),
    ] {
        if actor.has_auto_ability(ability) {
            actor.set_status(status, 255);
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

    for element in Element::VALUES {
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

fn apply_equipment_status_resistances(actor: &mut BattleActor) {
    for (_, status) in status_proof_abilities()
        .into_iter()
        .chain(status_ward_abilities())
    {
        actor.status_resistances.remove(&status);
    }
    for status in ribbon_immunities()
        .into_iter()
        .chain(aeon_ribbon_immunities())
    {
        actor.status_resistances.remove(&status);
    }
    if matches!(actor.id, ActorId::Character(_)) {
        actor.status_resistances.insert(Status::Threaten, 255);
    }
    if actor.has_auto_ability(AutoAbility::Ribbon) {
        for status in ribbon_immunities() {
            actor.status_resistances.insert(status, 255);
        }
    }
    if actor.has_auto_ability(AutoAbility::AeonRibbon) {
        for status in aeon_ribbon_immunities() {
            actor.status_resistances.insert(status, 255);
        }
    }
    for (ability, status) in status_ward_abilities() {
        if actor.has_auto_ability(ability) {
            let current = actor.status_resistances.get(&status).copied().unwrap_or(0);
            actor.status_resistances.insert(status, current.max(50));
        }
    }
    for (ability, status) in status_proof_abilities() {
        if actor.has_auto_ability(ability) {
            actor.status_resistances.insert(status, 255);
        }
    }
}

fn apply_equipment_resource_bonuses(actor: &mut BattleActor) {
    actor.break_hp_limit = actor.has_auto_ability(AutoAbility::BreakHpLimit);
    actor.break_mp_limit = actor.has_auto_ability(AutoAbility::BreakMpLimit);
    actor.break_damage_limit = actor.has_auto_ability(AutoAbility::BreakDamageLimit);
    actor.hp_multiplier = 100
        + hp_bonus_abilities()
            .into_iter()
            .filter_map(|(ability, bonus)| actor.has_auto_ability(ability).then_some(bonus))
            .sum::<i32>();
    actor.mp_multiplier = 100
        + mp_bonus_abilities()
            .into_iter()
            .filter_map(|(ability, bonus)| actor.has_auto_ability(ability).then_some(bonus))
            .sum::<i32>();
    actor.current_hp = actor.current_hp.min(actor.effective_max_hp());
    actor.current_mp = actor.current_mp.min(actor.effective_max_mp());
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

fn hp_bonus_abilities() -> [(AutoAbility, i32); 4] {
    [
        (AutoAbility::Hp5, 5),
        (AutoAbility::Hp10, 10),
        (AutoAbility::Hp20, 20),
        (AutoAbility::Hp30, 30),
    ]
}

fn mp_bonus_abilities() -> [(AutoAbility, i32); 4] {
    [
        (AutoAbility::Mp5, 5),
        (AutoAbility::Mp10, 10),
        (AutoAbility::Mp20, 20),
        (AutoAbility::Mp30, 30),
    ]
}

fn ribbon_immunities() -> [Status; 11] {
    [
        Status::Zombie,
        Status::Petrify,
        Status::Poison,
        Status::Confuse,
        Status::Berserk,
        Status::Provoke,
        Status::Sleep,
        Status::Silence,
        Status::Dark,
        Status::Slow,
        Status::Doom,
    ]
}

fn aeon_ribbon_immunities() -> [Status; 21] {
    [
        Status::Death,
        Status::Zombie,
        Status::Petrify,
        Status::Poison,
        Status::PowerBreak,
        Status::MagicBreak,
        Status::ArmorBreak,
        Status::MentalBreak,
        Status::Confuse,
        Status::Berserk,
        Status::Provoke,
        Status::Sleep,
        Status::Silence,
        Status::Dark,
        Status::Slow,
        Status::PowerDistiller,
        Status::ManaDistiller,
        Status::SpeedDistiller,
        Status::AbilityDistiller,
        Status::Scan,
        Status::Doom,
    ]
}

fn status_ward_abilities() -> [(AutoAbility, Status); 11] {
    [
        (AutoAbility::DeathWard, Status::Death),
        (AutoAbility::ZombieWard, Status::Zombie),
        (AutoAbility::StoneWard, Status::Petrify),
        (AutoAbility::PoisonWard, Status::Poison),
        (AutoAbility::SleepWard, Status::Sleep),
        (AutoAbility::SilenceWard, Status::Silence),
        (AutoAbility::DarkWard, Status::Dark),
        (AutoAbility::SlowWard, Status::Slow),
        (AutoAbility::ConfuseWard, Status::Confuse),
        (AutoAbility::BerserkWard, Status::Berserk),
        (AutoAbility::CurseWard, Status::Curse),
    ]
}

fn status_proof_abilities() -> [(AutoAbility, Status); 11] {
    [
        (AutoAbility::Deathproof, Status::Death),
        (AutoAbility::Zombieproof, Status::Zombie),
        (AutoAbility::Stoneproof, Status::Petrify),
        (AutoAbility::Poisonproof, Status::Poison),
        (AutoAbility::Sleepproof, Status::Sleep),
        (AutoAbility::Silenceproof, Status::Silence),
        (AutoAbility::Darkproof, Status::Dark),
        (AutoAbility::Slowproof, Status::Slow),
        (AutoAbility::Confuseproof, Status::Confuse),
        (AutoAbility::Berserkproof, Status::Berserk),
        (AutoAbility::Curseproof, Status::Curse),
    ]
}

fn element_values() -> String {
    Element::VALUES
        .into_iter()
        .map(Element::python_name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn element_display_name(element: Element) -> &'static str {
    match element {
        Element::Fire => "Fire",
        Element::Ice => "Ice",
        Element::Thunder => "Thunder",
        Element::Water => "Water",
        Element::Holy => "Holy",
    }
}

fn elemental_affinity_values() -> String {
    ElementalAffinity::VALUES
        .into_iter()
        .map(ElementalAffinity::python_name)
        .collect::<Vec<_>>()
        .join(", ")
}

fn elemental_affinity_display_name(affinity: ElementalAffinity) -> &'static str {
    match affinity {
        ElementalAffinity::Absorbs => "Absorbs",
        ElementalAffinity::Immune => "Immune",
        ElementalAffinity::Resists => "Resists",
        ElementalAffinity::Weak => "Weak",
        ElementalAffinity::Neutral => "Neutral",
    }
}

fn autoability_values() -> String {
    data::autoability_names_in_order()
        .iter()
        .map(|ability| python_enum_value(ability))
        .collect::<Vec<_>>()
        .join(", ")
}

fn python_enum_value(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace(' ', "_")
        .replace(['(', ')', '\''], "")
}

fn normalize_autoability_key(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace([' ', '-', '%'], "_")
        .replace(['+', '(', ')', '\''], "")
        .trim_matches('_')
        .to_string()
}

const PYTHON_STAT_NAMES: &[&str] = &[
    "HP",
    "MP",
    "Strength",
    "Defense",
    "Magic",
    "Magic defense",
    "Agility",
    "Luck",
    "Evasion",
    "Accuracy",
];

fn stat_values() -> String {
    PYTHON_STAT_NAMES
        .iter()
        .map(|stat| python_enum_value(stat))
        .collect::<Vec<_>>()
        .join(", ")
}

const PYTHON_STATUS_NAMES: &[&str] = &[
    "Death",
    "Zombie",
    "Petrify",
    "Poison",
    "Power Break",
    "Magic Break",
    "Armor Break",
    "Mental Break",
    "Confuse",
    "Berserk",
    "Provoke",
    "Threaten",
    "Sleep",
    "Silence",
    "Dark",
    "Shell",
    "Protect",
    "Reflect",
    "NulTide",
    "NulBlaze",
    "NulShock",
    "NulFrost",
    "Regen",
    "Haste",
    "Slow",
    "Scan",
    "Power Distiller",
    "Mana Distiller",
    "Speed Distiller",
    "Ability Distiller",
    "Shield",
    "Boost",
    "Eject",
    "Auto-Life",
    "Curse",
    "Defend",
    "Guard",
    "Sentinel",
    "Doom",
    "MAX HP x 2",
    "MAX MP x 2",
    "MP = 0",
    "Damage 9999",
    "Critical",
    "OverDrive x1.5",
    "OverDrive x 2",
];

fn status_values() -> String {
    PYTHON_STATUS_NAMES
        .iter()
        .map(|status| python_enum_value(status))
        .collect::<Vec<_>>()
        .join(", ")
}

fn status_display_name(status: Status) -> &'static str {
    match status {
        Status::Death => "Death",
        Status::Zombie => "Zombie",
        Status::Eject => "Eject",
        Status::Petrify => "Petrify",
        Status::Poison => "Poison",
        Status::PowerBreak => "Power Break",
        Status::MagicBreak => "Magic Break",
        Status::ArmorBreak => "Armor Break",
        Status::MentalBreak => "Mental Break",
        Status::Confuse => "Confuse",
        Status::Berserk => "Berserk",
        Status::Provoke => "Provoke",
        Status::Threaten => "Threaten",
        Status::Sleep => "Sleep",
        Status::Silence => "Silence",
        Status::Dark => "Dark",
        Status::Shell => "Shell",
        Status::Protect => "Protect",
        Status::Reflect => "Reflect",
        Status::NulTide => "NulTide",
        Status::NulBlaze => "NulBlaze",
        Status::NulShock => "NulShock",
        Status::NulFrost => "NulFrost",
        Status::Haste => "Haste",
        Status::Slow => "Slow",
        Status::Regen => "Regen",
        Status::Scan => "Scan",
        Status::PowerDistiller => "Power Distiller",
        Status::ManaDistiller => "Mana Distiller",
        Status::SpeedDistiller => "Speed Distiller",
        Status::AbilityDistiller => "Ability Distiller",
        Status::Shield => "Shield",
        Status::Boost => "Boost",
        Status::AutoLife => "Auto-Life",
        Status::Curse => "Curse",
        Status::Defend => "Defend",
        Status::Guard => "Guard",
        Status::Sentinel => "Sentinel",
        Status::Doom => "Doom",
        Status::MaxHpX2 => "MAX HP x 2",
        Status::MaxMpX2 => "MAX MP x 2",
        Status::Mp0 => "MP = 0",
        Status::Damage9999 => "Damage 9999",
        Status::Critical => "Critical",
        Status::OverdriveX15 => "OverDrive x1.5",
        Status::OverdriveX2 => "OverDrive x 2",
    }
}

fn is_upstream_status(value: &str) -> bool {
    let normalized = normalize_autoability_key(value);
    PYTHON_STATUS_NAMES
        .iter()
        .any(|status| normalize_autoability_key(status) == normalized)
}

fn weapon_status_application(ability: AutoAbility) -> Option<data::ActionStatus> {
    let (status, chance) = match ability {
        AutoAbility::Deathtouch => (Status::Death, 50),
        AutoAbility::Zombietouch => (Status::Zombie, 50),
        AutoAbility::Stonetouch => (Status::Petrify, 50),
        AutoAbility::Poisontouch => (Status::Poison, 50),
        AutoAbility::Sleeptouch => (Status::Sleep, 50),
        AutoAbility::Silencetouch => (Status::Silence, 50),
        AutoAbility::Darktouch => (Status::Dark, 50),
        AutoAbility::Slowtouch => (Status::Slow, 50),
        AutoAbility::Deathstrike => (Status::Death, 100),
        AutoAbility::Zombiestrike => (Status::Zombie, 100),
        AutoAbility::Stonestrike => (Status::Petrify, 100),
        AutoAbility::Poisonstrike => (Status::Poison, 100),
        AutoAbility::Sleepstrike => (Status::Sleep, 100),
        AutoAbility::Silencestrike => (Status::Silence, 100),
        AutoAbility::Darkstrike => (Status::Dark, 100),
        AutoAbility::Slowstrike => (Status::Slow, 100),
        _ => return None,
    };
    Some(data::ActionStatus {
        status,
        chance,
        stacks: 254,
        ignores_resistance: false,
    })
}

fn status_uses_rng(status: Status) -> bool {
    !matches!(
        status,
        Status::Scan
            | Status::PowerDistiller
            | Status::ManaDistiller
            | Status::SpeedDistiller
            | Status::AbilityDistiller
            | Status::Shield
            | Status::Boost
            | Status::Eject
            | Status::AutoLife
            | Status::Curse
            | Status::Defend
            | Status::Guard
            | Status::Sentinel
            | Status::Doom
            | Status::MaxHpX2
            | Status::MaxMpX2
            | Status::Mp0
            | Status::Damage9999
            | Status::Critical
            | Status::OverdriveX15
            | Status::OverdriveX2
    )
}

fn auto_med_removes(status: Status) -> bool {
    matches!(
        status,
        Status::Zombie
            | Status::Poison
            | Status::PowerBreak
            | Status::MagicBreak
            | Status::ArmorBreak
            | Status::MentalBreak
            | Status::Confuse
            | Status::Berserk
            | Status::Sleep
            | Status::Silence
            | Status::Dark
            | Status::Slow
    )
}

fn parse_status(name: &str) -> Option<Status> {
    name.parse().ok()
}

fn parse_amount(amount: &str, current: i32, error_name: &str) -> Result<i32, String> {
    let parsed = amount
        .parse::<i32>()
        .map_err(|_| format!("{error_name} must be an integer"))?;
    if amount.starts_with(['+', '-']) {
        Ok(current + parsed)
    } else {
        Ok(parsed)
    }
}

fn aeon_power_base(stats: AeonStatBlock) -> i32 {
    (stats.hp.min(9_999) / 100)
        + (stats.mp.min(999) / 10)
        + stats.strength
        + stats.defense
        + stats.magic
        + stats.magic_defense
        + stats.agility
        + stats.evasion
        + stats.accuracy
}

fn aeon_formula_value(stat: i32, power_base: i32, formula: (i32, i32, i32)) -> i32 {
    let (percent, numerator, denominator) = formula;
    (stat * percent / 100) + (power_base * numerator / denominator)
}

fn aeon_calculated_stat(
    yuna_stat: i32,
    encounter_stat: i32,
    yuna_power: i32,
    encounter_power: i32,
    formula: (i32, i32, i32),
    bonus: i32,
) -> i32 {
    aeon_formula_value(yuna_stat, yuna_power, formula)
        .max(aeon_formula_value(encounter_stat, encounter_power, formula))
        .saturating_add(bonus)
        .clamp(0, 255)
}

fn parse_bribe_action_gil(args: &[String]) -> Result<i32, String> {
    let gil = parse_amount(
        args.get(1).map(String::as_str).unwrap_or_default(),
        0,
        "gil",
    )
    .map_err(|message| format!("Error: {message}"))?;
    if gil < 0 {
        return Err("Error: gil must be greater or equal to 0".to_string());
    }
    Ok(gil)
}

fn scripted_respawn_monster(encounter_name: &str, wave_index: usize) -> Option<&'static str> {
    match (encounter_name, wave_index) {
        ("machina_3", 1 | 2) => Some("worker"),
        _ => None,
    }
}

fn is_tanker_placeholder_comment(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(slot) = trimmed.strip_prefix("#m") else {
        return false;
    };
    matches!(slot, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8")
}

fn parse_sahagin_chief_spawn_comment(line: &str) -> Option<(usize, bool)> {
    let comment = line.trim().strip_prefix('#')?.trim();
    let spawn_count = comment.split_whitespace().next()?.parse::<usize>().ok()?;
    let lower = comment.to_ascii_lowercase();
    if !lower.contains("sahagin chief") || !lower.contains("spawn") {
        return None;
    }
    Some((spawn_count, lower.contains("4th appears")))
}

#[derive(Debug, Default)]
struct GeneratedEncounterCtb {
    monsters: HashMap<MonsterSlot, i32>,
    characters: HashMap<Character, i32>,
}

fn parse_generated_encounter_comment_party(
    line: &str,
) -> Option<(Vec<String>, GeneratedEncounterCtb)> {
    let comment = line.trim().strip_prefix('#')?.trim();
    if !comment.starts_with("Encounter:")
        && !comment.starts_with("Random Encounter:")
        && !comment.starts_with("Multizone encounter:")
        && !comment.starts_with("Simulated Encounter:")
    {
        return None;
    }
    let parts = comment.split('|').map(str::trim).collect::<Vec<_>>();
    let formation = parts.get(2)?;
    let ctb = parts.get(3).copied().unwrap_or_default();
    let monster_keys = parse_generated_encounter_monsters(formation)?;
    Some((monster_keys, parse_generated_encounter_ctbs(ctb)))
}

fn parse_generated_encounter_monsters(formation: &str) -> Option<Vec<String>> {
    let mut stripped = formation.trim();
    for suffix in [" Ambush", " Preemptive", " Normal"] {
        if let Some(without_suffix) = stripped.strip_suffix(suffix) {
            stripped = without_suffix;
            break;
        }
    }
    if stripped.is_empty() || stripped == "Empty" || stripped == "-" {
        return None;
    }
    let mut monsters = Vec::new();
    for name in stripped
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let normalized = name
            .to_ascii_lowercase()
            .replace(' ', "_")
            .replace('#', "_")
            .replace(['(', ')', '\''], "");
        let monster = data::monster_stats(name).or_else(|| data::monster_stats(&normalized))?;
        monsters.push(monster.key);
    }
    (!monsters.is_empty()).then_some(monsters)
}

fn parse_generated_encounter_ctbs(ctb_summary: &str) -> GeneratedEncounterCtb {
    let mut ctbs = GeneratedEncounterCtb::default();
    for token in ctb_summary.split_whitespace() {
        let Some((actor_text, rest)) = token.split_once('[') else {
            continue;
        };
        let Some(ctb_text) = rest.strip_suffix(']') else {
            continue;
        };
        let Ok(ctb) = ctb_text.parse::<i32>() else {
            continue;
        };
        if let Some(slot_text) = actor_text.strip_prefix('M') {
            if let Ok(slot) = slot_text.parse::<usize>() {
                ctbs.monsters.insert(MonsterSlot(slot), ctb);
            }
        } else if let Some(character) = character_from_ctb_prefix(actor_text) {
            ctbs.characters.insert(character, ctb);
        }
    }
    ctbs
}

fn character_from_ctb_prefix(prefix: &str) -> Option<Character> {
    Some(match prefix {
        "Ti" => Character::Tidus,
        "Yu" => Character::Yuna,
        "Au" => Character::Auron,
        "Ki" => Character::Kimahri,
        "Wa" => Character::Wakka,
        "Lu" => Character::Lulu,
        "Ri" => Character::Rikku,
        "Se" => Character::Seymour,
        "Va" => Character::Valefor,
        "If" => Character::Ifrit,
        "Ix" => Character::Ixion,
        "Sh" => Character::Shiva,
        "Ba" => Character::Bahamut,
        "An" => Character::Anima,
        "Yo" => Character::Yojimbo,
        "Ci" => Character::Cindy,
        "Sa" => Character::Sandy,
        "Mi" => Character::Mindy,
        _ => return None,
    })
}

fn skipped_action_comment(display_line: &str, actor: &BattleActor) -> String {
    format!(
        "# skipped: {display_line} ({} is {})",
        actor_label(actor),
        actor_unavailable_reason(actor)
    )
}

fn actor_unavailable_reason(actor: &BattleActor) -> &'static str {
    if actor.current_hp <= 0 || actor.statuses.contains(&Status::Death) {
        "KO'd"
    } else if actor.statuses.contains(&Status::Sleep) {
        "asleep"
    } else if actor.statuses.contains(&Status::Petrify) {
        "petrified"
    } else if actor.statuses.contains(&Status::Eject) {
        "ejected"
    } else {
        "unable to act"
    }
}

fn format_character_action_line(actor: Character, action: &str, args: &[String]) -> String {
    format_action_line(actor.input_name(), action, args)
}

fn format_monster_action_line(slot: MonsterSlot, action: &str, args: &[String]) -> String {
    format_action_line(&format!("m{}", slot.0), action, args)
}

fn format_named_monster_action_line(monster: &str, action: &str, args: &[String]) -> String {
    format_action_line(monster, action, args)
}

fn format_action_line(actor: &str, action: &str, args: &[String]) -> String {
    let mut parts = vec![actor.to_string()];
    if !action.is_empty() {
        parts.push(action.to_string());
    }
    parts.extend(args.iter().cloned());
    parts.join(" ")
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
            .map(|key| {
                let slot = actor.display_slot.unwrap_or(slot);
                let display_name = data::monster_stats(key)
                    .map(|stats| stats.display_name)
                    .unwrap_or_else(|| key.to_string());
                format!("{display_name} (M{})", slot.0)
            })
            .unwrap_or_else(|| format!("M{}", slot.0)),
    }
}

fn actor_label_for_id(actor_id: ActorId) -> String {
    match actor_id {
        ActorId::Character(character) => character.display_name().to_string(),
        ActorId::Monster(slot) => format!("M{}", slot.0),
    }
}

fn format_actor_stats(actor: &BattleActor) -> String {
    format!(
        "Stats: {} | HP {} | MP {} | STR {} | DEF {} | MAG {} | MDF {} | AGI {} | LCK {} | EVA {} | ACC {}",
        actor_label(actor),
        actor.max_hp,
        actor.max_mp,
        actor.combat_stats.strength,
        actor.combat_stats.defense,
        actor.combat_stats.magic,
        actor.combat_stats.magic_defense,
        actor.agility,
        actor.combat_stats.luck,
        actor.combat_stats.evasion,
        actor.combat_stats.accuracy
    )
}

fn format_stat_change(actor: &BattleActor, stat: &str, old_value: i32, new_value: i32) -> String {
    format!(
        "Stat: {} | {stat} | {old_value} -> {new_value}",
        actor_label(actor)
    )
}

fn format_item_drop(drop: &data::ItemDrop) -> String {
    let mut output = format!("{} x{}", drop.item, drop.quantity);
    if drop.rare {
        output.push_str(" (rare)");
    }
    output
}

fn inventory_item_values() -> String {
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

fn format_action_output(
    user_label: &str,
    action_name: &str,
    spent_ctb: i32,
    results: &[ActionEffectResult],
    state: &SimulationState,
    action: Option<&ActionData>,
) -> String {
    let prefix = format!("{user_label} -> {action_name} [{spent_ctb}]: ");
    if results.is_empty() {
        return format!("{prefix}Does Nothing");
    }
    let indent = " ".repeat(prefix.len());
    let rendered = results
        .iter()
        .map(|result| format_action_result(result, state, action))
        .collect::<Vec<_>>()
        .join(&format!("\n{indent}"));
    format!("{prefix}{rendered}")
}

fn format_action_result(
    result: &ActionEffectResult,
    state: &SimulationState,
    action_data: Option<&ActionData>,
) -> String {
    let target_label = state
        .actor(result.target)
        .map(actor_label)
        .unwrap_or_else(|| actor_label_for_id(result.target));
    if !result.hit {
        return format!("{target_label} -> Miss");
    }

    let mut action = String::new();
    if action_data.is_none_or(|action| action.damage_formula != DamageFormula::NoDamage) {
        for damage in [
            result.damage.hp.as_ref(),
            result.damage.mp.as_ref(),
            result.damage.ctb.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            action.push(' ');
            action.push_str(&format_action_damage(damage));
        }
    }
    if action.is_empty() {
        action.push_str(" (No damage)");
    }
    let status_tokens = coalesced_status_tokens(&result.statuses);
    if !status_tokens.is_empty() {
        action.push(' ');
        for token in status_tokens {
            let (status, applied) = *token;
            if applied {
                action.push_str(&format!("[{}]", status_display_name(status)));
            } else {
                action.push_str(&format!("[{} Fail]", status_display_name(status)));
            }
        }
    }
    if !result.removed_statuses.is_empty() {
        action.push(' ');
        for status in &result.removed_statuses {
            action.push_str(&format!("[-{}]", status_display_name(*status)));
        }
    }
    for (buff, stacks) in &result.buffs {
        action.push_str(&format!(" [{} {stacks}]", buff_display_name(*buff)));
    }

    let rendered = format!("{target_label} ->{action}");
    if let Some(reflected_from) = result.reflected_from {
        let reflected_label = state
            .actor(reflected_from)
            .map(actor_label)
            .unwrap_or_else(|| actor_label_for_id(reflected_from));
        format!("{reflected_label} (reflected) -> {rendered}")
    } else {
        rendered
    }
}

fn coalesced_status_tokens(statuses: &[(Status, bool)]) -> Vec<&(Status, bool)> {
    let mut tokens: Vec<&(Status, bool)> = Vec::new();
    for status in statuses {
        if let Some(existing) = tokens.iter_mut().find(|existing| existing.0 == status.0) {
            *existing = status;
        } else {
            tokens.push(status);
        }
    }
    tokens
}

fn append_optional_output_line(mut output: String, line: Option<String>) -> String {
    if let Some(line) = line {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&line);
    }
    output
}

fn render_character_action_outcome(outcome: CharacterActionOutcome) -> String {
    let mut output_lines = outcome.pre_lines;
    if !output_lines.is_empty() {
        ensure_blank_line(&mut output_lines);
    }
    output_lines.push(outcome.output);
    if let Some(comment) = outcome.damage_comment {
        output_lines.push(comment);
    }
    output_lines.join("\n")
}

fn first_monster_slot_in_comment(comment: &str) -> Option<MonsterSlot> {
    let start = comment.find("(M")? + 2;
    let digits = comment[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>();
    format!("m{digits}").parse::<MonsterSlot>().ok()
}

fn replace_character_action_target(line: &str, slot: MonsterSlot) -> String {
    let edited_line = edit_action_line(line);
    let mut words: Vec<String> = edited_line
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect();
    if words.len() >= 4 && words[0].eq_ignore_ascii_case("action") {
        words[3] = format!("m{}", slot.0);
    }
    words.join(" ")
}

fn rendered_virtual_monster_turn_count(rendered: &str) -> usize {
    rendered
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed
                .strip_prefix('m')
                .and_then(|tail| tail.chars().next())
                .is_some_and(|character| character.is_ascii_digit())
        })
        .count()
}

fn format_damage_comment(
    prefix: &str,
    results: &[ActionEffectResult],
    state: &SimulationState,
    action: Option<&ActionData>,
) -> Option<String> {
    let action = action?;
    if action.damage_formula == DamageFormula::NoDamage {
        return None;
    }
    if !action.damages_hp && !action.damages_mp && !action.damages_ctb {
        return None;
    }
    let target_parts = results
        .iter()
        .filter_map(|result| {
            let actor = state.actor(result.target)?;
            if !result.hit {
                return Some(format!("{}: Miss", actor_label(actor)));
            }
            let damage_parts = [
                result.damage.hp.as_ref(),
                result.damage.mp.as_ref(),
                result.damage.ctb.as_ref(),
            ]
            .into_iter()
            .flatten()
            .map(format_action_damage_for_comment)
            .collect::<Vec<_>>();
            if damage_parts.is_empty() {
                return None;
            }
            let hp_suffix = format!(" -> {}/{} HP", actor.current_hp, actor.effective_max_hp());
            Some(format!(
                "{}: {}{}",
                actor_label(actor),
                damage_parts.join("; "),
                hp_suffix
            ))
        })
        .collect::<Vec<_>>();
    if target_parts.is_empty() {
        None
    } else {
        Some(format!("{prefix}{}", target_parts.join(" | ")))
    }
}

fn format_action_damage(damage: &ActionDamageResult) -> String {
    let mut output = format!(
        "[{}/31] {} {}",
        damage.damage_rng, damage.damage, damage.pool
    );
    if damage.crit {
        output.push_str(" (Crit)");
    }
    output
}

fn format_action_damage_for_comment(damage: &ActionDamageResult) -> String {
    format_action_damage(damage).replace(" (Crit)", " [CRIT]")
}

fn buff_display_name(buff: Buff) -> &'static str {
    match buff {
        Buff::Cheer => "Cheer",
        Buff::Aim => "Aim",
        Buff::Focus => "Focus",
        Buff::Reflex => "Reflex",
        Buff::Luck => "Luck",
        Buff::Jinx => "Jinx",
    }
}

fn format_status(actor: &BattleActor) -> String {
    let mut status = format!(
        "Status: {} {}/{} HP",
        actor_label(actor),
        actor.current_hp,
        actor.effective_max_hp()
    );
    let mut seen = HashSet::new();
    let mut statuses = actor
        .status_order
        .iter()
        .filter(|status| actor.statuses.contains(status))
        .map(|status| {
            seen.insert(*status);
            (
                status_display_name(*status).to_string(),
                actor.status_stack(*status),
            )
        })
        .collect::<Vec<_>>();
    let mut unordered_statuses = actor
        .statuses
        .iter()
        .filter(|status| !seen.contains(status))
        .map(|status| {
            (
                status_display_name(*status).to_string(),
                actor.status_stack(*status),
            )
        })
        .collect::<Vec<_>>();
    unordered_statuses.sort_by(|left, right| left.0.cmp(&right.0));
    statuses.extend(unordered_statuses);
    if !statuses.is_empty() {
        let details = statuses
            .into_iter()
            .map(|(status, stacks)| format!("{status} ({stacks})"))
            .collect::<Vec<_>>()
            .join(", ");
        status.push_str(&format!(" | Statuses: {details}"));
    }
    let mut buffs = actor
        .buffs
        .iter()
        .filter(|(_, stacks)| **stacks > 0)
        .map(|(buff, stacks)| (format!("{buff:?}"), *stacks))
        .collect::<Vec<_>>();
    buffs.sort_by(|left, right| left.0.cmp(&right.0));
    if !buffs.is_empty() {
        let details = buffs
            .into_iter()
            .map(|(buff, stacks)| format!("{buff} ({stacks})"))
            .collect::<Vec<_>>()
            .join(", ");
        status.push_str(&format!(" | Buffs: {details}"));
    }
    status
}

fn titlecase_encounter_header(name: &str) -> String {
    name.split_whitespace()
        .map(titlecase_ascii_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_action_name(action: &str) -> String {
    action
        .split('_')
        .filter(|part| !part.is_empty())
        .map(display_action_name_part)
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_action_name_part(part: &str) -> String {
    part.split('-')
        .map(titlecase_ascii_word)
        .collect::<Vec<_>>()
        .join("-")
}

fn titlecase_ascii_word(word: &str) -> String {
    if matches!(word, "ctb" | "hd" | "hp" | "mp" | "od" | "ps2" | "rng") {
        return word.to_ascii_uppercase();
    }
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => {
            let mut word = first.to_ascii_uppercase().to_string();
            word.push_str(chars.as_str());
            word
        }
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        actor_sort_index, calculate_action_damage, monster_template, simulate, ActionDamageResult,
        ActionEffectResult, DamagePool, SimulationState,
    };
    use crate::battle::{ActorId, BattleActor};
    use crate::data::{
        action_data, ActionData, ActionStatus, ActionTarget, DamageFormula, DamageType,
        HitChanceFormula,
    };
    use crate::model::{
        AutoAbility, Buff, Character, Element, ElementalAffinity, MonsterSlot, Status,
    };
    use crate::rng::FfxRngTracker;

    #[test]
    fn display_action_names_titlecase_hyphenated_words() {
        assert_eq!(super::display_action_name("hi-potion"), "Hi-Potion");
        assert_eq!(
            super::display_action_name("auto_hi-potion"),
            "Auto Hi-Potion"
        );
        assert_eq!(
            super::display_action_name("auto-life_counter"),
            "Auto-Life Counter"
        );
        assert_eq!(super::display_action_name("quick_hit_ps2"), "Quick Hit PS2");
        assert_eq!(super::display_action_name("quick_hit_hd"), "Quick Hit HD");
    }

    #[test]
    fn prepared_cursor_replay_applies_macros_like_python_web_helpers() {
        let mut state = SimulationState::new(1);

        state.run_until_prepared_line(
            "/macro moonflow grid\n# cursor padding\n# cursor padding\n",
            4,
        );

        assert_eq!(state.character_max_hp(Character::Tidus), Some(720));
    }

    fn test_action(target: ActionTarget) -> ActionData {
        ActionData {
            key: "test".to_string(),
            rank: 3,
            target,
            can_use_in_combat: true,
            overdrive_user: None,
            overdrive_index: 0,
            can_target_dead: false,
            affected_by_silence: false,
            steals_item: false,
            steals_gil: false,
            empties_od_bar: false,
            copied_by_copycat: false,
            hit_chance_formula: HitChanceFormula::Always,
            uses_hit_chance_table: false,
            accuracy: 255,
            affected_by_dark: false,
            affected_by_reflect: false,
            damage_formula: DamageFormula::NoDamage,
            damage_type: DamageType::Other,
            base_damage: 0,
            mp_cost: 0,
            uses_magic_booster: false,
            n_of_hits: 1,
            uses_weapon_properties: false,
            ignores_armored: false,
            never_break_damage_limit: false,
            always_break_damage_limit: false,
            can_crit: false,
            bonus_crit: 0,
            adds_equipment_crit: false,
            affected_by_alchemy: false,
            drains: false,
            misses_if_target_alive: false,
            destroys_user: false,
            heals: false,
            damages_hp: false,
            damages_mp: false,
            damages_ctb: false,
            elements: Vec::new(),
            removes_statuses: false,
            has_weak_delay: false,
            has_strong_delay: false,
            shatter_chance: 0,
            status_applications: Vec::new(),
            statuses: Vec::new(),
            status_flags: Vec::new(),
            buffs: Vec::new(),
        }
    }

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
    fn roll_parse_errors_render_like_python() {
        let output = simulate(
            1,
            &[
                "roll".to_string(),
                "roll rngx 1".to_string(),
                "roll rng10 nope".to_string(),
                "roll rng10 -1".to_string(),
                "roll rng-1 1".to_string(),
                "roll rng68 1".to_string(),
                "roll rng10 201".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Error: rng needs to be an integer\n",
                "Error: rng needs to be an integer\n",
                "Error: amount needs to be an integer\n",
                "Error: amount needs to be an greater or equal to 0\n",
                "Error: Can't advance rng index -1\n",
                "Error: Can't advance rng index 68\n",
                "Error: Can't advance rng more than 200 times",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn spawn_slot_errors_render_like_python() {
        let output = simulate(
            1,
            &[
                "spawn tanker m1".to_string(),
                "spawn tanker 0".to_string(),
                "spawn tanker 2".to_string(),
                "spawn tanker 1".to_string(),
                "spawn tanker 3".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Error: Slot must be an integer\n",
                "Error: Slot must be between 1 and 1\n",
                "Error: Slot must be between 1 and 1\n",
                "spawn tanker 1\n",
                "Error: Slot must be between 1 and 2",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn equipment_ability_errors_render_like_python() {
        let output = simulate(
            1,
            &[
                "equip weapon tidus 1 counterattack".to_string(),
                "equip weapon tidus 1 nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            format!(
                "Equipment: Tidus | Weapon | Longsword [] -> Avenger [Counterattack]\nError: ability can only be one of these values: {}",
                super::autoability_values()
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn equipment_slots_expand_to_fit_abilities_like_python() {
        let mut state = SimulationState::new(1);

        let output = state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "0".to_string(),
                "counterattack".to_string(),
            ],
        );

        assert_eq!(
            output,
            "Equipment: Tidus | Weapon | Longsword [] -> Avenger [Counterattack]"
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .weapon_slots,
            1
        );
    }

    #[test]
    fn unknown_actor_state_commands_render_like_python() {
        let output = simulate(
            1,
            &[
                "equip weapon unknown 1 sensor".to_string(),
                "status unknown haste".to_string(),
                "stat unknown hp 1".to_string(),
                "status unknown".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Equipment: Unknown | Weapon | Unknown's weapon [] -> Unknown's weapon [Sensor]\n",
                "Status: Unknown 99999/99999 HP | Statuses: Haste (254)\n",
                "Stat: Unknown | HP | 99999 -> 1\n",
                "Status: Unknown 1/1 HP | Statuses: Haste (254)",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn equipment_changes_refresh_crit_metadata_like_python() {
        let mut state = SimulationState::new(1);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Kimahri))
                .unwrap()
                .equipment_crit,
            3
        );

        state.change_equipment("armor", &["kimahri".to_string(), "0".to_string()]);

        assert_eq!(
            state
                .actor(ActorId::Character(Character::Kimahri))
                .unwrap()
                .equipment_crit,
            6
        );
    }

    #[test]
    fn status_parse_errors_render_like_python() {
        let output = simulate(
            1,
            &[
                "status tidus nope".to_string(),
                "status tidus poison nope".to_string(),
                "status tidus auto-life 1".to_string(),
                "status tidus power_distiller 1".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            format!(
                "Error: status can only be one of these values: {}\nError: status stacks must be an integer\nStatus: Tidus 520/520 HP | Statuses: Auto-Life (1)\nStatus: Tidus 520/520 HP | Statuses: Auto-Life (1), Power Distiller (1)",
                super::status_values()
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn status_output_preserves_python_insertion_order() {
        let output = simulate(
            1,
            &[
                "status tidus sleep 2".to_string(),
                "status tidus poison 3".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("Status: Tidus 520/520 HP | Statuses: Sleep (2), Poison (3)"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn max_stat_statuses_affect_effective_actor_limits() {
        let output = simulate(
            1,
            &[
                "status tidus max_hp_x_2 1".to_string(),
                "status tidus".to_string(),
                "heal tidus 50".to_string(),
                "status tidus".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("Status: Tidus 520/1040 HP | Statuses: MAX HP x 2 (1)"));
        assert!(output
            .text
            .contains("Status: Tidus 570/1040 HP | Statuses: MAX HP x 2 (1)"));

        let mut state = SimulationState::new(1);
        state.change_status(&[
            "tidus".to_string(),
            "max_mp_x_2".to_string(),
            "1".to_string(),
        ]);
        state.heal_party(&["tidus".to_string(), "1".to_string()]);
        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_mp,
            13
        );
    }

    #[test]
    fn reports_missing_state_command_args_like_python() {
        let output = simulate(
            1,
            &[
                "party".to_string(),
                "summon".to_string(),
                "equip".to_string(),
                "equip weapon".to_string(),
                "equip relic tidus 1".to_string(),
                "equip weapon nope 1".to_string(),
                "equip weapon tidus nope".to_string(),
                "spawn".to_string(),
                "spawn tanker".to_string(),
                "spawn tanker nope".to_string(),
                "spawn nope 1".to_string(),
                "stat".to_string(),
                "status".to_string(),
                "element".to_string(),
                "element m1".to_string(),
                "element nope fire weak".to_string(),
                "element m1 fire weak".to_string(),
                "stat nope".to_string(),
                "status nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Error: Usage: party [characters initials]\n",
                "Error: Usage: summon [aeon name]\n",
                "Error: Usage: equip [equip type] [character] [# of slots] (abilities)\n",
                "Error: Usage: equip [equip type] [character] [# of slots] (abilities)\n",
                "Error: equipment type can only be one of these values: weapon, armor\n",
                "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "Error: Slots must be between 0 and 4\n",
                "Error: Usage: spawn [monster name] [slot] (forced ctb)\n",
                "Error: Usage: spawn [monster name] [slot] (forced ctb)\n",
                "Error: Slot must be an integer\n",
                "Error: No monster named \"nope\"\n",
                "Error: Usage: stat [character/monster slot] (stat) [(+/-)amount]\n",
                "Error: Usage: status [character/monster slot]\n",
                "Error: Usage: element [monster slot] [element] [affinity]\n",
                "Error: Usage: element [monster slot] [element] [affinity]\n",
                "Error: Monster slot must be in the form m#\n",
                "Error: No monster in slot 1\n",
                "Error: \"nope\" is not a valid actor\n",
                "Error: \"nope\" is not a valid actor",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn death_command_advances_rng_and_renders_like_python() {
        let mut baseline = SimulationState::new(1);
        for _ in 0..3 {
            baseline.rng.advance_rng(10);
        }
        let expected_next = baseline.rng.advance_rng(10);

        let mut state = SimulationState::new(1);
        let output = state.run_lines(&["death tidus".to_string()]);
        assert_eq!(output.text, "Character death: Tidus");
        assert_eq!(output.unsupported_count, 0);
        assert_eq!(state.rng.advance_rng(10), expected_next);
    }

    #[test]
    fn yojimbo_death_reduces_compatibility_like_python() {
        let mut state = SimulationState::new(1);
        let starting = state.compatibility;

        let output = state.run_lines(&["death yojimbo".to_string()]);

        assert_eq!(output.text, "Character death: Yojimbo");
        assert_eq!(state.compatibility, starting - 10);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn yojimbo_death_clamps_compatibility_like_python() {
        let mut state = SimulationState::new(1);
        state.compatibility = 5;

        state.character_death(Character::Yojimbo);

        assert_eq!(state.compatibility, 0);
    }

    #[test]
    fn yojimbo_turn_rolls_motivation_and_updates_compatibility_like_python() {
        let output = simulate(
            1,
            &[
                "yojimboturn zanmato piranha overdrive".to_string(),
                "yojimboturn dismiss piranha".to_string(),
                "yojimboturn kozuka piranha".to_string(),
                "yojimboturn zanmato seymour_flux overdrive".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Wakizashi ST -> Piranha: free (OD used) [60/48 motivation][129/255 compatibility]\n",
                "Dismiss -> Piranha: 0 gil [0/None motivation][129/255 compatibility]\n",
                "Kozuka -> Piranha: 1 gil [67/32 motivation][129/255 compatibility]\n",
                "Zanmato -> Seymour Flux: 67108864 gil (OD used) [80/80 motivation][133/255 compatibility]",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn yojimbo_turn_reports_unknown_action_and_monster_like_python() {
        let output = simulate(
            1,
            &[
                "yojimboturn nope piranha".to_string(),
                "yojimboturn zanmato nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Error: No yojimbo action named \"nope\"\n",
                "Error: No monster named \"nope\"",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_parses_commands_and_uses_existing_action_engine() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c fight".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("Party: Tidus, Auron -> Cindy, Sandy, Mindy"));
        assert!(output.text.contains("spawn piranha 1 0"));
        assert!(
            output.text.contains("Cindy -> Fight! [53/100] -> Attack"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_takes_a_break_when_python_command_has_no_monster_target() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "magusturn c fight".to_string(),
                "magusturn c combine".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Cindy -> Fight! [45/100] -> Taking a break..."),
            "{}",
            output.text
        );
        assert!(
            output
                .text
                .contains("Cindy -> Combine powers! [35/100] -> Taking a break..."),
            "{}",
            output.text
        );
        assert!(
            !output.text.contains("Error: No monster in slot"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_sandy_do_as_you_will_can_reflect_without_monsters_like_python() {
        let output = simulate(
            1,
            &["summon magus".to_string(), "magusturn s do".to_string()],
        );

        assert!(
            output
                .text
                .contains("Sandy -> Do as you will. [52/100] -> Reflect"),
            "{}",
            output.text
        );
        assert!(output.text.contains("Cindy -> (No damage) [Reflect]"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_do_as_you_will_rolls_attack_like_python() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn m do".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Mindy -> Do as you will. [53/100] -> Attack"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_do_as_you_will_rolls_spells_like_python() {
        let seed_one = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c do".to_string(),
            ],
        );
        let seed_five = simulate(
            5,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c do".to_string(),
            ],
        );

        assert!(
            seed_one
                .text
                .contains("Cindy -> Do as you will. [45/100] -> Taking a break..."),
            "{}",
            seed_one.text
        );
        assert!(
            seed_five
                .text
                .contains("Cindy -> Do as you will. [52/100] -> Firaga"),
            "{}",
            seed_five.text
        );
        assert_eq!(seed_one.unsupported_count, 0);
        assert_eq!(seed_five.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_help_each_other_rolls_revive_chain_like_python() {
        let no_help = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c help".to_string(),
            ],
        );
        let dead_sandy = simulate(
            2,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status sandy death 254".to_string(),
                "magusturn c help".to_string(),
            ],
        );
        let camisade = simulate(
            34,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c help".to_string(),
            ],
        );

        assert!(
            no_help
                .text
                .contains("Cindy -> Help each other! [45/100] -> Taking a break..."),
            "{}",
            no_help.text
        );
        assert!(
            dead_sandy
                .text
                .contains("Cindy -> Help each other! [51/100] -> Life"),
            "{}",
            dead_sandy.text
        );
        assert!(dead_sandy.text.contains("Sandy ->"));
        assert!(dead_sandy.text.contains("[-Death]"));
        assert!(
            camisade
                .text
                .contains("Cindy -> Help each other! [55/100] -> Camisade"),
            "{}",
            camisade.text
        );
        assert_eq!(no_help.unsupported_count, 0);
        assert_eq!(dead_sandy.unsupported_count, 0);
        assert_eq!(camisade.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_go_go_rolls_attack_spell_and_camisade_like_python() {
        let break_seed = simulate(
            4,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c go".to_string(),
            ],
        );
        let attack_seed = simulate(
            17,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c go".to_string(),
            ],
        );
        let spell_seed = simulate(
            43,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c go".to_string(),
            ],
        );
        let camisade_seed = simulate(
            28,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c go".to_string(),
            ],
        );

        assert!(
            break_seed
                .text
                .contains("Cindy -> Go, go! [45/100] -> Taking a break..."),
            "{}",
            break_seed.text
        );
        assert!(
            attack_seed
                .text
                .contains("Cindy -> Go, go! [52/100] -> Attack"),
            "{}",
            attack_seed.text
        );
        assert!(
            spell_seed
                .text
                .contains("Cindy -> Go, go! [52/100] -> Blizzaga"),
            "{}",
            spell_seed.text
        );
        assert!(
            camisade_seed
                .text
                .contains("Cindy -> Go, go! [55/100] -> Camisade"),
            "{}",
            camisade_seed.text
        );
        assert_eq!(break_seed.unsupported_count, 0);
        assert_eq!(attack_seed.unsupported_count, 0);
        assert_eq!(spell_seed.unsupported_count, 0);
        assert_eq!(camisade_seed.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_are_you_all_right_rolls_recovery_chain_like_python() {
        let mut break_state = SimulationState::new(2);
        break_state.summon("magus");
        break_state.spawn_monster_from_command("piranha", 1, Some(0));
        break_state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_hp = 100;
        let break_output = break_state.magus_turn("m", "are");

        let mut lancet_state = SimulationState::new(4);
        lancet_state.summon("magus");
        lancet_state.spawn_monster_from_command("piranha", 1, Some(0));
        lancet_state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_hp = 100;
        let lancet_output = lancet_state.magus_turn("m", "are");

        let mut drain_state = SimulationState::new(6);
        drain_state.summon("magus");
        drain_state.spawn_monster_from_command("piranha", 1, Some(0));
        drain_state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_hp = 100;
        let drain_output = drain_state.magus_turn("m", "are");

        let mut osmose_state = SimulationState::new(6);
        osmose_state.summon("magus");
        osmose_state.spawn_monster_from_command("piranha", 1, Some(0));
        osmose_state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_mp = 1;
        let osmose_output = osmose_state.magus_turn("m", "are");

        assert!(
            break_output.contains("Mindy -> Are you all right? [49/100] -> Taking a break..."),
            "{break_output}"
        );
        assert!(
            lancet_output.contains("Mindy -> Are you all right? [53/100] -> Lancet"),
            "{lancet_output}"
        );
        assert!(
            drain_output.contains("Mindy -> Are you all right? [53/100] -> Drain"),
            "{drain_output}"
        );
        assert!(
            osmose_output.contains("Mindy -> Are you all right? [53/100] -> Osmose"),
            "{osmose_output}"
        );
    }

    #[test]
    fn magus_turn_mindy_do_as_you_will_uses_reflected_cindy_spell_chain_like_python() {
        let fira_seed = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
            ],
        );
        let flare_seed = simulate(
            3,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
            ],
        );
        let fallback_attack = simulate(
            20,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
            ],
        );

        assert!(
            fira_seed
                .text
                .contains("Mindy -> Do as you will. [52/100] -> Fira"),
            "{}",
            fira_seed.text
        );
        assert!(fira_seed.text.contains("Cindy (reflected)"));
        assert!(
            flare_seed
                .text
                .contains("Mindy -> Do as you will. [53/100] -> Flare"),
            "{}",
            flare_seed.text
        );
        assert!(flare_seed.text.contains("Cindy (reflected)"));
        assert!(
            fallback_attack
                .text
                .contains("Mindy -> Do as you will. [53/100] -> Attack"),
            "{}",
            fallback_attack.text
        );
        assert_eq!(fira_seed.unsupported_count, 0);
        assert_eq!(flare_seed.unsupported_count, 0);
        assert_eq!(fallback_attack.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_do_as_you_will_can_emit_two_reflected_spells_like_python() {
        let output = simulate(
            33,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
            ],
        );

        assert!(
            output.text.contains("Mindy -> Do as you will. [52/100]:"),
            "{}",
            output.text
        );
        assert!(output.text.contains("    Thundara [39]"), "{}", output.text);
        assert!(output.text.contains("    Firaga [39]"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_sandy_defense_rolls_support_chain_like_python() {
        let seed_four = simulate(
            4,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s defense".to_string(),
            ],
        );
        let seed_five = simulate(
            5,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s defense".to_string(),
            ],
        );

        assert!(
            seed_four
                .text
                .contains("Sandy -> Defense! [52/100] -> Shell"),
            "{}",
            seed_four.text
        );
        assert!(
            seed_five
                .text
                .contains("Sandy -> Defense! [45/100] -> Taking a break..."),
            "{}",
            seed_five.text
        );
        assert_eq!(seed_four.unsupported_count, 0);
        assert_eq!(seed_five.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_reports_available_commands_like_python() {
        let output = simulate(
            1,
            &[
                "magusturn c".to_string(),
                "magusturn c nope".to_string(),
                "magusturn nope fight".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("Error: Available commands for Cindy: Do as you will."));
        assert!(output.text.contains("Fight!"));
        assert!(output.text.contains("Combine powers!"));
        assert!(output
            .text
            .contains("Error: Usage: magusturn [name] (command)"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_menu_filters_rng_gated_commands_like_python() {
        let output = simulate(
            1,
            &["magusturn c".to_string(), "magusturn s defense".to_string()],
        );

        let mut lines = output.text.lines();
        let cindy_menu = lines.next().unwrap_or_default();
        let sandy_menu = lines.next().unwrap_or_default();
        assert!(cindy_menu.contains("Fight!"), "{cindy_menu}");
        assert!(cindy_menu.contains("Help each other!"), "{cindy_menu}");
        assert!(!cindy_menu.contains("Go, go!"), "{cindy_menu}");
        assert!(
            sandy_menu.contains("Available commands for Sandy"),
            "{sandy_menu}"
        );
        assert!(!sandy_menu.contains("Defense!"), "{sandy_menu}");
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_exposes_one_more_time_after_previous_command() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c fight".to_string(),
                "magusturn c".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );

        assert!(output.text.contains("One more time."), "{}", output.text);
        let cindy_action_headers = output
            .text
            .lines()
            .filter(|line| line.starts_with("Cindy -> ") && line.contains(" ["))
            .count();
        assert_eq!(cindy_action_headers, 2);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_one_more_time_after_fight_matches_first_pass_python() {
        let target_gone = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c fight".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );
        let fallback_camisade = simulate(
            62,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c fight".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );

        assert!(
            target_gone
                .text
                .contains("Cindy -> One more time. [47/100] -> Taking a break..."),
            "{}",
            target_gone.text
        );
        assert!(
            fallback_camisade
                .text
                .contains("Cindy -> One more time. [47/100] -> Camisade"),
            "{}",
            fallback_camisade.text
        );
        assert_eq!(target_gone.unsupported_count, 0);
        assert_eq!(fallback_camisade.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_one_more_time_after_go_go_matches_first_pass_python() {
        let output = simulate(
            4,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c go".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Cindy -> One more time. [49/100] -> Attack"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_cindy_one_more_time_after_help_matches_first_pass_python() {
        let repeat_break = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c help".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );
        let fallback_camisade = simulate(
            17,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn c help".to_string(),
                "magusturn c one_more_time".to_string(),
            ],
        );

        assert!(
            repeat_break
                .text
                .contains("Cindy -> One more time. [39/100] -> Taking a break..."),
            "{}",
            repeat_break.text
        );
        assert!(
            fallback_camisade
                .text
                .contains("Cindy -> One more time. [47/100] -> Camisade"),
            "{}",
            fallback_camisade.text
        );
        assert_eq!(repeat_break.unsupported_count, 0);
        assert_eq!(fallback_camisade.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_sandy_one_more_time_after_defense_matches_first_pass_python() {
        let fallback_attack = simulate(
            5,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s defense".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );
        let repeat_shell = simulate(
            16,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s defense".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );
        let repeat_protect = simulate(
            17,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s defense".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );

        assert!(
            fallback_attack
                .text
                .contains("Sandy -> One more time. [47/100] -> Attack"),
            "{}",
            fallback_attack.text
        );
        assert!(
            repeat_shell
                .text
                .contains("Sandy -> Defense! [52/100] -> Shell [42]: Mindy"),
            "{}",
            repeat_shell.text
        );
        assert!(
            repeat_shell
                .text
                .contains("Sandy -> One more time. [56/100] -> Shell"),
            "{}",
            repeat_shell.text
        );
        assert!(repeat_shell.text.contains("Shell [42]: Sandy"));
        assert!(
            repeat_protect
                .text
                .contains("Sandy -> Defense! [52/100] -> Protect [42]: Mindy"),
            "{}",
            repeat_protect.text
        );
        assert!(
            repeat_protect
                .text
                .contains("Sandy -> One more time. [56/100] -> Protect"),
            "{}",
            repeat_protect.text
        );
        assert!(repeat_protect.text.contains("Protect [42]: Sandy"));
        assert_eq!(fallback_attack.unsupported_count, 0);
        assert_eq!(repeat_shell.unsupported_count, 0);
        assert_eq!(repeat_protect.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_sandy_one_more_time_after_fight_matches_first_pass_python() {
        let fallback_break = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s fight".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );
        let fallback_razzia = simulate(
            143,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s fight".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );

        assert!(
            fallback_break
                .text
                .contains("Sandy -> One more time. [48/100] -> Taking a break..."),
            "{}",
            fallback_break.text
        );
        assert!(
            fallback_razzia
                .text
                .contains("Sandy -> One more time. [47/100] -> Razzia"),
            "{}",
            fallback_razzia.text
        );
        assert_eq!(fallback_break.unsupported_count, 0);
        assert_eq!(fallback_razzia.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_sandy_one_more_time_after_do_as_you_will_matches_first_pass_python() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn s do".to_string(),
                "magusturn s one_more_time".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Sandy -> One more time. [56/100] -> Attack"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_one_more_time_after_fight_breaks_with_python_motivation() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn m fight".to_string(),
                "magusturn m one_more_time".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Mindy -> One more time. [38/100] -> Taking a break..."),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_one_more_time_after_reflected_do_as_you_will_matches_first_pass_python() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
                "magusturn m one_more_time".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Mindy -> One more time. [56/100] -> Fira"),
            "{}",
            output.text
        );
        assert!(output.text.contains("Cindy (reflected)"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_one_more_time_after_do_as_you_will_fallback_breaks_like_python() {
        let output = simulate(
            3,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "magusturn m do".to_string(),
                "magusturn m one_more_time".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("Mindy -> One more time. [48/100] -> Taking a break..."),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_one_more_time_replays_reflected_spell_list_like_python() {
        let two_repeated = simulate(
            33,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
                "magusturn m one_more_time".to_string(),
            ],
        );
        let second_spell_skipped_by_mp = simulate(
            52,
            &[
                "summon magus".to_string(),
                "spawn piranha 1 0".to_string(),
                "status cindy reflect 254".to_string(),
                "magusturn m do".to_string(),
                "magusturn m one_more_time".to_string(),
            ],
        );

        assert!(
            two_repeated
                .text
                .contains("Mindy -> One more time. [56/100]:"),
            "{}",
            two_repeated.text
        );
        assert!(
            two_repeated.text.contains("    Thundara [39]"),
            "{}",
            two_repeated.text
        );
        assert!(
            two_repeated.text.contains("    Firaga [39]"),
            "{}",
            two_repeated.text
        );
        assert!(
            second_spell_skipped_by_mp
                .text
                .contains("Mindy -> One more time. [56/100] -> Firaga"),
            "{}",
            second_spell_skipped_by_mp.text
        );
        assert_eq!(two_repeated.unsupported_count, 0);
        assert_eq!(second_spell_skipped_by_mp.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_mindy_one_more_time_after_are_you_all_right_matches_first_pass_python() {
        let mut state = SimulationState::new(2);
        state.summon("magus");
        state.spawn_monster_from_command("piranha", 1, Some(0));
        state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_hp = 100;

        let first = state.magus_turn("m", "are");
        let repeat = state.magus_turn("m", "one_more_time");

        assert!(
            first.contains("Mindy -> Are you all right? [49/100] -> Taking a break..."),
            "{first}"
        );
        assert!(
            repeat.contains("Mindy -> One more time. [52/100] -> Attack"),
            "{repeat}"
        );
    }

    #[test]
    fn magus_turn_dismisses_without_generic_action_error() {
        let output = simulate(
            1,
            &[
                "summon magus".to_string(),
                "magusturn c dismiss".to_string(),
            ],
        );

        assert!(
            output.text.contains("Cindy -> Dismiss ["),
            "{}",
            output.text
        );
        assert!(!output.text.contains("can't be used in battle"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn magus_turn_dismiss_applies_on_dismiss_to_all_sisters_like_python() {
        let mut state = SimulationState::new(1);
        state.summon("magus");
        for sister in [Character::Cindy, Character::Sandy, Character::Mindy] {
            state.actor_mut(ActorId::Character(sister)).unwrap().ctb = 7;
        }
        let sandy = state
            .actor_mut(ActorId::Character(Character::Sandy))
            .unwrap();
        sandy.set_status(Status::Death, 254);
        sandy.current_hp = 0;

        let output = state.magus_turn("c", "dismiss");

        assert!(output.contains("Cindy -> Dismiss ["), "{output}");
        for sister in [Character::Cindy, Character::Sandy, Character::Mindy] {
            let actor = state.actor(ActorId::Character(sister)).unwrap();
            assert_eq!(actor.ctb, actor.base_ctb() * 3, "{sister}");
        }
        let sandy = state.actor(ActorId::Character(Character::Sandy)).unwrap();
        assert!(!sandy.statuses.contains(&Status::Death));
        assert_eq!(sandy.current_hp, 1);
    }

    #[test]
    fn magus_turn_menu_rng_side_effects_match_first_pass_python() {
        let mut state = SimulationState::new(1);
        let starting_rng18 = state.rng.current_positions()[18];

        let invalid = state.magus_turn("c", "nope");

        assert!(invalid.contains("Error: Available commands for Cindy"));
        assert_eq!(state.rng.current_positions()[18], starting_rng18);

        let autolife = state.magus_turn("c", "auto-life");

        assert!(autolife.contains("Cindy -> Auto-Life [50/100] ->"));
        assert_eq!(state.rng.current_positions()[18], starting_rng18);

        let dismiss = state.magus_turn("c", "dismiss");

        assert!(dismiss.contains("Cindy -> Dismiss [50/100] ->"));
        assert_eq!(state.rng.current_positions()[18], starting_rng18 + 3);
    }

    #[test]
    fn mindy_are_you_all_right_availability_depends_on_resources_like_python() {
        let mut healthy_state = SimulationState::new(1);
        let starting_rng18 = healthy_state.rng.current_positions()[18];

        let available = healthy_state.magus_turn("m", "");

        assert!(!available.contains("Are you all right?"), "{available}");
        assert_eq!(healthy_state.rng.current_positions()[18], starting_rng18);

        let dismiss = healthy_state.magus_turn("m", "dismiss");

        assert!(dismiss.contains("Mindy -> Dismiss [50/100] ->"));
        assert_eq!(
            healthy_state.rng.current_positions()[18],
            starting_rng18 + 1
        );

        let mut low_state = SimulationState::new(2);
        low_state
            .actor_mut(ActorId::Character(Character::Mindy))
            .unwrap()
            .current_hp = 1;
        let low_starting_rng18 = low_state.rng.current_positions()[18];
        let low_available = low_state.magus_turn("m", "");

        assert!(
            low_available.contains("Are you all right?"),
            "{low_available}"
        );

        let low_dismiss = low_state.magus_turn("m", "dismiss");

        assert!(low_dismiss.contains("Mindy -> Dismiss [50/100] ->"));
        assert_eq!(
            low_state.rng.current_positions()[18],
            low_starting_rng18 + 2
        );
    }

    #[test]
    fn magus_convenience_commands_do_not_expose_one_more_time_like_python() {
        let mut state = SimulationState::new(1);

        let autolife = state.magus_turn("c", "auto-life");
        let menu_after_autolife = state.magus_turn("c", "");

        assert!(autolife.contains("Cindy -> Auto-Life [50/100] ->"));
        assert!(
            !menu_after_autolife.contains("One more time."),
            "{menu_after_autolife}"
        );

        state.spawn_monster_from_command("piranha", 1, Some(0));
        let combine = state.magus_turn("c", "combine");
        let menu_after_combine = state.magus_turn("c", "");

        assert!(combine.contains("Cindy -> Combine powers! [50/100] ->"));
        assert!(combine.contains("Delta Attack ["));
        assert!(
            !menu_after_combine.contains("One more time."),
            "{menu_after_combine}"
        );
    }

    #[test]
    fn magus_action_availability_ignores_mp_cost_abilities_like_python() {
        let mut state = SimulationState::new(1);
        let flare = action_data("flare").expect("flare action should exist");
        let mindy = state
            .actor_mut(ActorId::Character(Character::Mindy))
            .expect("Mindy actor should exist");
        mindy.current_mp = flare.mp_cost - 1;
        mindy.armor_abilities.insert(AutoAbility::HalfMpCost);

        assert!(!state.magus_can_attempt_action(Character::Mindy, "flare"));
    }

    #[test]
    fn end_encounter_revives_dead_characters_to_one_hp_like_python() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);

        state.end_encounter();

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, 1);
        assert!(!tidus.statuses.contains(&Status::Death));
    }

    #[test]
    fn end_encounter_renders_python_style_ctb_and_hp_summary() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("tanker".to_string()),
            10,
            false,
            1_000,
        ));
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 400;
        state.monsters[0].current_hp = 700;
        let output = state.end_encounter();

        assert!(output.contains("End: CTBs:"));
        assert!(output.contains("Characters HPs: Ti[400]"));
        assert!(output.contains("Monsters HPs: Tanker (M1)[700]"));
    }

    #[test]
    fn compatibility_command_clamps_and_renders_like_python() {
        let output = simulate(
            1,
            &[
                "compatibility +10".to_string(),
                "compatibility 300".to_string(),
                "compatibility -999".to_string(),
                "compatibility".to_string(),
                "compatibility nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Compatibility: 128 -> 138\n",
                "Compatibility: 138 -> 255\n",
                "Compatibility: 255 -> 0\n",
                "Error: compatibility must be an integer\n",
                "Error: compatibility must be an integer",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn encounters_count_command_renders_like_python() {
        let output = simulate(
            1,
            &[
                "encounters_count total 5".to_string(),
                "encounters_count random +2".to_string(),
                "encounters_count total nope".to_string(),
                "encounters_count".to_string(),
                "encounters_count bad 1".to_string(),
                "encounter tanker".to_string(),
            ],
        );

        assert!(output.text.contains("Total encounters count set to 5"));
        assert!(output.text.contains("Random encounters count set to 2"));
        assert!(output.text.contains("Error: amount must be an integer"));
        assert!(output
            .text
            .contains("Error: Usage: encounters_count [total/random/zone name] [(+/-)amount]"));
        assert!(output.text.contains("Encounter:   6 | Tanker"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn encounters_count_total_recalculates_aeon_stats_like_python() {
        let output = simulate(
            1,
            &[
                "stat valefor".to_string(),
                "stat valefor strength 30".to_string(),
                "encounters_count total 600".to_string(),
                "stat valefor".to_string(),
                "stat yuna magic +10".to_string(),
                "stat valefor".to_string(),
                "stat valefor hp 3000".to_string(),
                "stat valefor".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Stats: Valefor | HP 725 | MP 24 | STR 18 | DEF 23 | MAG 21 | MDF 23 | AGI 10 | LCK 17 | EVA 19 | ACC 11\n",
                "Stat: Valefor | Strength | 18 -> 30\n",
                "Total encounters count set to 600\n",
                "Stats: Valefor | HP 2225 | MP 71 | STR 57 | DEF 66 | MAG 64 | MDF 69 | AGI 34 | LCK 17 | EVA 42 | ACC 20\n",
                "Stat: Yuna | Magic | 20 -> 30\n",
                "Stats: Valefor | HP 2225 | MP 71 | STR 57 | DEF 66 | MAG 64 | MDF 69 | AGI 34 | LCK 17 | EVA 42 | ACC 20\n",
                "Stat: Valefor | HP | 2225 -> 3000\n",
                "Stats: Valefor | HP 3000 | MP 71 | STR 57 | DEF 66 | MAG 64 | MDF 69 | AGI 34 | LCK 17 | EVA 42 | ACC 20",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn seymour_and_summon_aeons_keep_python_current_resources() {
        let output = simulate(
            1,
            &[
                "status seymour".to_string(),
                "stat seymour".to_string(),
                "status valefor".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Status: Seymour 1200/1200 HP\n",
                "Stats: Seymour | HP 1200 | MP 999 | STR 20 | DEF 25 | MAG 35 | MDF 100 | AGI 20 | LCK 17 | EVA 10 | ACC 10\n",
                "Status: Valefor 725/725 HP",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn seymour_direct_stats_keep_python_bonus_floor_behavior() {
        let output = simulate(
            1,
            &[
                "stat seymour hp 1".to_string(),
                "stat seymour hp 1500".to_string(),
                "stat seymour".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Stat: Seymour | HP | 1200 -> 1200\n",
                "Stat: Seymour | HP | 1200 -> 1500\n",
                "Stats: Seymour | HP 1500 | MP 999 | STR 20 | DEF 25 | MAG 35 | MDF 100 | AGI 20 | LCK 17 | EVA 10 | ACC 10",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn summon_aeon_formula_constants_match_python_after_encounter_tiers() {
        let output = simulate(
            1,
            &[
                "encounters_count total 600".to_string(),
                "stat valefor".to_string(),
                "stat ifrit".to_string(),
                "stat ixion".to_string(),
                "stat shiva".to_string(),
                "stat bahamut".to_string(),
                "stat anima".to_string(),
                "stat yojimbo".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Total encounters count set to 600\n",
                "Stats: Valefor | HP 2225 | MP 71 | STR 45 | DEF 66 | MAG 64 | MDF 69 | AGI 34 | LCK 17 | EVA 42 | ACC 20\n",
                "Stats: Ifrit | HP 3067 | MP 68 | STR 46 | DEF 84 | MAG 62 | MDF 62 | AGI 30 | LCK 17 | EVA 22 | ACC 20\n",
                "Stats: Ixion | HP 3021 | MP 74 | STR 47 | DEF 74 | MAG 61 | MDF 87 | AGI 26 | LCK 17 | EVA 24 | ACC 21\n",
                "Stats: Shiva | HP 2680 | MP 80 | STR 42 | DEF 48 | MAG 70 | MDF 71 | AGI 52 | LCK 17 | EVA 66 | ACC 20\n",
                "Stats: Bahamut | HP 4340 | MP 103 | STR 50 | DEF 79 | MAG 55 | MDF 84 | AGI 34 | LCK 17 | EVA 44 | ACC 20\n",
                "Stats: Anima | HP 5090 | MP 130 | STR 65 | DEF 74 | MAG 66 | MDF 69 | AGI 30 | LCK 17 | EVA 44 | ACC 20\n",
                "Stats: Yojimbo | HP 3064 | MP 0 | STR 61 | DEF 73 | MAG 48 | MDF 69 | AGI 30 | LCK 17 | EVA 122 | ACC 38",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn yuna_hp_caps_before_aeon_stat_formula_like_python() {
        let output = simulate(
            1,
            &[
                "stat yuna hp 99999".to_string(),
                "stat valefor".to_string(),
                "stat yuna hp 20000".to_string(),
                "stat valefor".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Stat: Yuna | HP | 475 -> 99999\n",
                "Stats: Valefor | HP 3199 | MP 43 | STR 31 | DEF 42 | MAG 22 | MDF 26 | AGI 15 | LCK 17 | EVA 23 | ACC 16\n",
                "Stat: Yuna | HP | 99999 -> 20000\n",
                "Stats: Valefor | HP 3199 | MP 43 | STR 31 | DEF 42 | MAG 22 | MDF 26 | AGI 15 | LCK 17 | EVA 23 | ACC 16",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn end_encounter_keeps_aeon_current_hp_clamped_like_python() {
        let output = simulate(
            1,
            &[
                "status valefor".to_string(),
                "endencounter".to_string(),
                "status valefor".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Status: Valefor 725/725 HP\n",
                "End: CTBs: Ti[0] Au[0]\n",
                "     Characters HPs: Characters at full HP\n",
                "     Monsters HPs: Monsters at full HP\n",
                "Status: Valefor 725/725 HP",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_gil_commands_render_like_python_first_pass() {
        let output = simulate(
            1,
            &[
                "inventory show gil".to_string(),
                "inventory get gil 50".to_string(),
                "inventory use gil 25".to_string(),
                "inventory use gil 999".to_string(),
                "inventory get gil nope".to_string(),
                "inventory use gil 0".to_string(),
                "inventory get gil".to_string(),
                "inventory switch".to_string(),
            ],
        );

        assert!(output.text.contains("Gil: 300"));
        assert!(output.text.contains("Added 50 Gil (350 Gil total)"));
        assert!(output.text.contains("Used 25 Gil (325 Gil total)"));
        assert!(output
            .text
            .contains("Error: Not enough gil (need 674 more)"));
        assert!(output
            .text
            .contains("Error: Gil amount needs to be an integer"));
        assert!(output
            .text
            .contains("Error: Gil amount needs to be more than 0"));
        assert!(output
            .text
            .contains("Error: Usage: inventory get gil [amount]"));
        assert!(output
            .text
            .contains("Error: Usage: inventory switch [slot 1] [slot 2]"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_item_commands_render_like_python_first_pass() {
        let output = simulate(
            1,
            &[
                "inventory show".to_string(),
                "inventory show equipment".to_string(),
                "inventory get equipment weapon tidus 1 initiative".to_string(),
                "inventory show equipment".to_string(),
                "inventory buy equipment armor tidus 0".to_string(),
                "inventory sell equipment 1".to_string(),
                "inventory sell equipment weapon tidus 0".to_string(),
                "inventory switch 1 2".to_string(),
                "inventory autosort".to_string(),
                "inventory get potion 2".to_string(),
                "inventory use potion 5".to_string(),
                "inventory sell potion 2".to_string(),
                "inventory buy antidote 2".to_string(),
                "inventory use ether 1".to_string(),
                "inventory get potion nope".to_string(),
                "inventory get potion 0".to_string(),
                "inventory get definitely_not_item 1".to_string(),
                "inventory switch nope 1".to_string(),
                "inventory switch 1 999".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("+-----------+----------------+\n| Potion 10 | Phoenix Down 3 |\n+-----------+----------------+"));
        assert!(output.text.contains("Equipment: Empty"));
        assert!(output.text.contains("Added "));
        assert!(output
            .text
            .contains("Added Vigilante (Tidus) [Initiative][1512 gil]"));
        assert!(output.text.contains("Equipment: #1 "));
        assert!(output.text.contains("Bought "));
        assert!(output.text.contains(" for 50 gil"));
        assert!(output.text.contains("Sold "));
        assert!(output
            .text
            .contains("Switched Potion (slot 1) for Phoenix Down (slot 2)"));
        assert!(output.text.contains("Autosorted inventory"));
        assert!(output.text.contains("Added Potion x2 to inventory"));
        assert!(output.text.contains("Used Potion x5"));
        assert!(output.text.contains("Sold Potion x2 for 24 gil"));
        assert!(output.text.contains("Bought Antidote x2 for 100 gil"));
        assert!(output.text.contains("Error: Ether is not in the inventory"));
        assert!(output.text.contains("Error: Amount needs to be an integer"));
        assert!(output
            .text
            .contains("Error: Amount needs to be more than 0"));
        assert!(output
            .text
            .contains("Error: item can only be one of these values:"));
        assert!(output.text.contains("potion, hi-potion, x-potion"));
        assert!(output.text.contains("turbo_ether, phoenix_down"));
        assert!(!output.text.contains("Turbo Ether, Phoenix Down"));
        assert!(output
            .text
            .contains("Error: Inventory slot needs to be an integer"));
        assert!(output.text.contains(&format!(
            "Error: Inventory slot needs to be between 1 and {}",
            crate::data::item_names_in_order().len()
        )));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_invalid_item_error_lists_python_token_names() {
        let output = simulate(1, &["inventory get definitely_not_item 1".to_string()]);

        assert!(output
            .text
            .starts_with("Error: item can only be one of these values: potion, hi-potion"));
        assert!(output.text.contains("phoenix_down"));
        assert!(!output.text.contains("Phoenix Down"));
    }

    #[test]
    fn inventory_equipment_abilities_use_python_tracker_spellings() {
        let output = simulate(
            1,
            &[
                "inventory get equipment armor tidus 1 auto-haste".to_string(),
                "inventory get equipment armor tidus 1 auto_haste".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            format!(
                "{}{}",
                "Added Haste Shield (Tidus) [Auto-Haste][12512 gil]\n",
                format!(
                    "Error: ability can only be one of these values: {}",
                    super::autoability_values()
                )
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_equipment_duplicate_abilities_do_not_consume_slot_limit_like_python() {
        let output = simulate(
            1,
            &[
                "inventory get equipment weapon tidus 1 sensor sensor first_strike initiative piercing".to_string(),
                "inventory get equipment weapon tidus 1 sensor first_strike initiative piercing nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Added Sonic Steel (Tidus) [Sensor, First Strike, Initiative, Piercing][15312 gil]\n",
                "Added Sonic Steel (Tidus) [Sensor, First Strike, Initiative, Piercing][15312 gil]",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_sell_equipment_kind_usage_matches_python() {
        let output = simulate(
            1,
            &[
                "inventory sell equipment weapon".to_string(),
                "inventory sell equipment armor tidus".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            "Error: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)\nError: Usage: inventory sell equipment [equip type] [character] [slots] (abilities)"
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_item_slots_preserve_python_none_holes() {
        let output = simulate(
            1,
            &[
                "inventory use potion 10".to_string(),
                "inventory show".to_string(),
                "inventory switch 1 3".to_string(),
                "inventory show".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("+---+----------------+\n| - | Phoenix Down 3 |\n+---+----------------+"));
        assert!(output
            .text
            .contains("Switched None (slot 1) for None (slot 3)"));
        assert!(output
            .text
            .contains("+---+----------------+\n| - | Phoenix Down 3 |\n+---+----------------+"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_equipment_slots_preserve_python_none_holes() {
        let output = simulate(
            1,
            &[
                "inventory get equipment weapon tidus 1 initiative".to_string(),
                "inventory get equipment armor tidus 1 auto-haste".to_string(),
                "inventory sell equipment 1".to_string(),
                "inventory show equipment".to_string(),
                "inventory sell equipment 1".to_string(),
                "inventory get equipment weapon auron 0".to_string(),
                "inventory show equipment".to_string(),
            ],
        );

        assert!(output.text.contains("Sold "));
        assert!(output.text.contains("Equipment: #1 None\n           #2 "));
        assert!(output.text.contains("Error: Slot 1 is empty"));
        let final_show = output.text.rsplit("Equipment: ").next().unwrap_or_default();
        assert!(!final_show.starts_with("#1 None"), "{final_show}");
        assert!(final_show.contains("\n           #2 "), "{final_show}");
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn inventory_equipment_renders_owner_and_sell_value_like_python() {
        let output = simulate(
            1,
            &[
                "inventory get equipment weapon tidus 1 initiative".to_string(),
                "inventory show equipment".to_string(),
                "inventory sell equipment 1".to_string(),
            ],
        );

        assert!(output
            .text
            .contains("Added Vigilante (Tidus) [Initiative][1512 gil]"));
        assert!(output
            .text
            .contains("Equipment: #1 Vigilante (Tidus) [Initiative][1512 gil]"));
        assert!(output
            .text
            .contains("Sold Vigilante (Tidus) [Initiative][1512 gil]"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn walk_command_runs_encounter_checks_like_python() {
        let output = simulate(
            1,
            &[
                "walk kilika_woods 30".to_string(),
                "walk kilika_woods 30 cpz".to_string(),
                "walk kilika_woods 200".to_string(),
                "walk bad 10".to_string(),
                "walk kilika_woods nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Encounter checks: Kilika Woods (32) | 0 Encounters |  | 30 steps before end of the zone\n",
                "Encounter checks: Kilika Woods (32) | 1 Encounters | 10 | 20 steps before end of the zone\n",
                "Encounter checks: Kilika Woods (32) | 3 Encounters | 53, 97, 160 | 40 steps before end of the zone\n",
                "Error: No zone named \"bad\"\n",
                "Error: Step must be an integer",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn simulates_encounter_party_icvs_and_atb() {
        let lines = vec![
            "encounter tanker".to_string(),
            "status ctb".to_string(),
            "tidus attack m1".to_string(),
        ];
        let output = simulate(1, &lines);
        assert!(output
            .text
            .contains("Encounter:   1 | Tanker | Tanker, Sinscale#6"));
        assert!(output.text.contains("CTB:"));
        assert!(output.text.contains("tidus attack m1"));
        assert!(output.text.contains("# party rolls:"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn encounter_icvs_leave_reserves_at_zero_like_python() {
        let mut state = SimulationState::new(1);
        state.execute_raw_line("encounter tanker");

        let wakka_ctb = state
            .actor(ActorId::Character(Character::Wakka))
            .unwrap()
            .ctb;

        assert_eq!(wakka_ctb, 0);
        state.execute_raw_line("party taw");
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Wakka))
                .unwrap()
                .ctb,
            wakka_ctb
        );
        state.execute_raw_line("tidus defend");
        let wakka_after_turn = state
            .actor(ActorId::Character(Character::Wakka))
            .unwrap()
            .ctb;
        assert_eq!(wakka_after_turn, wakka_ctb);
    }

    #[test]
    fn reserve_ctbs_do_not_tick_during_core_normalization_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb = 20;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .ctb = 30;
        state
            .actor_mut(ActorId::Character(Character::Wakka))
            .unwrap()
            .ctb = 10;
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            40,
            false,
            1_000,
        ));
        state.monsters[0].ctb = 40;

        let elapsed = state.normalize_after_turn();

        assert_eq!(elapsed, 20);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Wakka))
                .unwrap()
                .ctb,
            10
        );
    }

    #[test]
    fn encounter_aliases_match_python_parser() {
        let output = simulate(
            1,
            &[
                "encounter".to_string(),
                "encounter normal".to_string(),
                "encounter pre".to_string(),
                "encounter amb".to_string(),
                "encounter sim".to_string(),
            ],
        );

        assert_eq!(output.unsupported_count, 0);
        assert!(output.text.contains("Encounter:   1 | Boss |"));
        assert!(output
            .text
            .contains("Encounter:   2 | Boss | Empty Normal |"));
        assert!(output
            .text
            .contains("Encounter:   3 | Boss | Empty Preemptive |"));
        assert!(output
            .text
            .contains("Encounter:   4 | Boss | Empty Ambush |"));
        assert!(output
            .text
            .contains("Simulated Encounter:   4 | Simulation |"));
    }

    #[test]
    fn unknown_encounters_error_without_resetting_state_like_python() {
        let mut state = SimulationState::new(1);
        let tidus_hp = state.character_actor(Character::Tidus).unwrap().current_hp;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Poison, 3);

        let output = state.run_lines(&["encounter nope".to_string()]);

        assert_eq!(output.text, "Error: No encounter named \"nope\"");
        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, tidus_hp);
        assert!(tidus.statuses.contains(&Status::Poison));
        assert_eq!(state.encounters_count, 0);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn simulated_encounters_do_not_increment_encounter_count_like_python() {
        let output = simulate(
            1,
            &["encounter sim".to_string(), "encounter normal".to_string()],
        );

        assert!(output
            .text
            .contains("Simulated Encounter:   0 | Simulation |"));
        assert!(output
            .text
            .contains("Encounter:   1 | Boss | Empty Normal |"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn random_encounters_render_and_increment_random_counters_like_python() {
        let mut state = SimulationState::new(1);

        let output = state.run_lines(&["encounter kilika_woods".to_string()]);

        assert!(output
            .text
            .contains("Random Encounter:   1   1   1 | Kilika Woods |"));
        assert_eq!(state.encounters_count, 1);
        assert_eq!(state.random_encounters_count, 1);
        assert_eq!(state.zone_encounters_counts.get("kilika_woods"), Some(&1));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn multizone_random_encounters_render_combined_zones_like_python() {
        let mut state = SimulationState::new(1);

        let output = state.run_lines(&["encounter multizone kilika_woods besaid_road".to_string()]);

        assert!(output
            .text
            .starts_with("encounter multizone kilika_woods besaid_road\n"));
        assert!(output.text.contains("# ===== Besaid Road ====="));
        assert!(output
            .text
            .contains("# Random Encounter:   1   1   1 | Kilika Woods/Besaid Road |"));
        assert!(output.text.matches(" | ").count() >= 4);
        assert_eq!(state.encounters_count, 1);
        assert_eq!(state.random_encounters_count, 1);
        assert_eq!(state.zone_encounters_counts.get("kilika_woods"), Some(&1));
        assert_eq!(state.zone_encounters_counts.get("besaid_road"), Some(&1));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn multizone_random_encounter_rejects_unknown_zones_like_python() {
        let output = simulate(1, &["encounter multizone kilika_woods nope".to_string()]);

        assert_eq!(output.text, "Error: No zone named \"nope\"");
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
        assert!(output.text.contains("tidus haste auron"));
        assert!(output.text.contains("auron defend"));
        assert!(output.text.contains("m5 spines"));
        assert!(output.text.contains("# enemy rolls:"));
        assert!(output.text.contains("heal"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn non_combat_actions_render_parser_style_error_without_spending_ctb() {
        let mut state = SimulationState::new(1);
        let tidus_ctb = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb;

        let output = state.apply_character_action(Character::Tidus, "designer_wallet", &[]);

        assert_eq!(
            output,
            "Error: Action Designer Wallet can't be used in battle"
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            tidus_ctb
        );
    }

    #[test]
    fn unknown_character_actions_error_without_spending_ctb_like_python() {
        let mut state = SimulationState::new(1);
        let tidus_ctb = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb;

        let output = state.apply_character_action(Character::Tidus, "definitely_not_real", &[]);

        assert_eq!(output, "Error: No action named \"definitely_not_real\"");
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            tidus_ctb
        );
        assert_eq!(state.last_actor, Some(ActorId::Character(Character::Tidus)));
    }

    #[test]
    fn invalid_character_action_actor_errors_without_unsupported_count_like_python() {
        let output = simulate(1, &["action nope attack".to_string()]);

        assert!(output
            .text
            .contains("Error: character can only be one of these values: tidus"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn missing_monster_action_actor_errors_without_unsupported_count_like_python() {
        let output = simulate(1, &["monsteraction".to_string()]);

        assert_eq!(
            output.text,
            "Error: Usage: monsteraction [monster slot/name] (action)"
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn unknown_lines_render_python_parse_errors_without_unsupported_count() {
        let output = simulate(1, &["definitely unknown".to_string()]);

        assert_eq!(
            output.text,
            "Error: Impossible to parse \"definitely unknown\""
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn indented_comments_and_directives_parse_like_python() {
        let output = simulate(1, &[" # comment".to_string(), " /usage".to_string()]);

        assert_eq!(
            output.text,
            "Error: Impossible to parse \" # comment\"\nError: Impossible to parse \" /usage\""
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn target_required_character_actions_error_without_spending_ctb() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let tidus_ctb = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb;

        let attack_output = state.apply_character_action(Character::Tidus, "attack", &[]);
        let haste_output = state.apply_character_action(Character::Tidus, "haste", &[]);
        let invalid_party_output =
            state.apply_character_action(Character::Tidus, "attack", &[String::from("party")]);
        let invalid_target_output =
            state.apply_character_action(Character::Tidus, "attack", &[String::from("nope")]);

        assert_eq!(
            attack_output,
            "Error: Action \"Attack\" requires a target (Character/Monster/Monster Slot)"
        );
        assert_eq!(
            haste_output,
            "Error: Action \"Haste\" requires a target (Character/Monster/Monster Slot)"
        );
        assert_eq!(
            invalid_party_output,
            "Error: \"party\" is not a valid target"
        );
        assert_eq!(
            invalid_target_output,
            "Error: \"nope\" is not a valid target"
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            tidus_ctb
        );
    }

    #[test]
    fn empty_monster_slot_targets_error_before_spending_ctb_like_python() {
        let mut state = SimulationState::new(1);
        let rikku_ctb = state
            .actor(ActorId::Character(Character::Rikku))
            .unwrap()
            .ctb;

        let attack_output =
            state.apply_character_action(Character::Rikku, "attack", &[String::from("m1")]);
        let steal_output =
            state.apply_character_action(Character::Rikku, "steal", &[String::from("m1")]);
        let grenade_output =
            state.apply_character_action(Character::Rikku, "grenade", &[String::from("m1")]);

        assert_eq!(attack_output, "Error: No monster in slot 1");
        assert_eq!(steal_output, "Error: No monster in slot 1");
        assert_eq!(grenade_output, "Error: No monster in slot 1");
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Rikku))
                .unwrap()
                .ctb,
            rikku_ctb
        );
    }

    #[test]
    fn bribe_action_validates_gil_before_spending_ctb_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("piranha".to_string()),
            10,
            false,
            100,
        ));
        let tidus_ctb = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb;

        let missing_gil =
            state.apply_character_action(Character::Tidus, "bribe", &[String::from("m1")]);
        let invalid_gil = state.apply_character_action(
            Character::Tidus,
            "bribe",
            &[String::from("m1"), String::from("nope")],
        );
        let negative_gil = state.apply_character_action(
            Character::Tidus,
            "bribe",
            &[String::from("m1"), String::from("-1")],
        );

        assert_eq!(missing_gil, "Error: gil must be an integer");
        assert_eq!(invalid_gil, "Error: gil must be an integer");
        assert_eq!(negative_gil, "Error: gil must be greater or equal to 0");
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            tidus_ctb
        );
    }

    #[test]
    fn invalid_bribe_gil_cleans_up_temporary_monster_targets() {
        let mut state = SimulationState::new(1);

        let output = state.apply_character_action(
            Character::Rikku,
            "bribe",
            &[String::from("piranha"), String::from("nope")],
        );

        assert_eq!(output, "Error: gil must be an integer");
        assert!(state.temporary_monsters.is_empty());
        assert_eq!(state.last_actor, Some(ActorId::Character(Character::Tidus)));
    }

    #[test]
    fn bribe_action_rolls_item_ejects_target_and_tracks_gil_like_python() {
        let mut state = SimulationState::new(1);
        state.spawn_monster("piranha", MonsterSlot(1), Some(0));

        let output = state.apply_character_action(
            Character::Rikku,
            "bribe",
            &[String::from("m1"), String::from("1000000")],
        );

        assert!(output.contains("Rikku -> Bribe ["));
        assert!(output.contains("Piranha (M1) -> "));
        assert!(!output.contains("-> Fail"));
        assert!(output.contains("Total Gil spent: 1000000"));
        assert!(state.monsters[0].statuses.contains(&Status::Eject));
        assert_eq!(state.monsters[0].bribe_gil_spent, 1_000_000);
    }

    #[test]
    fn bribe_action_updates_turn_and_target_memory_like_python() {
        let mut state = SimulationState::new(1);
        state.spawn_monster("piranha", MonsterSlot(1), Some(0));

        state.apply_character_action(
            Character::Rikku,
            "bribe",
            &[String::from("m1"), String::from("100")],
        );

        assert_eq!(state.last_actor, Some(ActorId::Character(Character::Rikku)));
        assert_eq!(state.last_targets, vec![ActorId::Monster(MonsterSlot(1))]);
        assert_eq!(
            state
                .actor_last_targets
                .get(&ActorId::Character(Character::Rikku)),
            Some(&vec![ActorId::Monster(MonsterSlot(1))])
        );
        assert_eq!(
            state
                .actor_last_attackers
                .get(&ActorId::Monster(MonsterSlot(1))),
            Some(&ActorId::Character(Character::Rikku))
        );
    }

    #[test]
    fn bribe_action_accumulates_and_clamps_gil_like_python() {
        let mut state = SimulationState::new(1);
        state.spawn_monster("piranha", MonsterSlot(1), Some(0));
        state.monsters[0].bribe_gil_spent = 999_999_990;

        let output = state.apply_character_action(
            Character::Rikku,
            "bribe",
            &[String::from("m1"), String::from("100")],
        );

        assert!(output.contains("Total Gil spent: 999999999"));
        assert_eq!(state.monsters[0].bribe_gil_spent, 999_999_999);
    }

    #[test]
    fn known_monsters_reject_actions_outside_their_action_table() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("sinspawn_echuilles".to_string()),
            10,
            false,
            2_000,
        ));
        let monster_ctb = state.monsters[0].ctb;

        let output = state.apply_monster_action(MonsterSlot(1), "cheer", &[]);

        assert!(output.starts_with("Error: Available actions for Sinspawn Echuilles (M1):"));
        assert!(output.contains("blender"));
        assert!(output.contains("does_nothing"));
        assert_eq!(state.monsters[0].ctb, monster_ctb);
    }

    #[test]
    fn monster_actions_error_for_empty_slots_like_python() {
        let mut state = SimulationState::new(1);

        let output = state.apply_monster_action(MonsterSlot(3), "attack", &[]);

        assert_eq!(output, "Error: No monster in slot 3");
        assert!(state.monsters.is_empty());
    }

    #[test]
    fn monster_state_commands_error_for_empty_slots_like_python() {
        let output = simulate(1, &["stat m2".to_string(), "status m2".to_string()]);

        assert_eq!(
            output.text,
            "Error: No monster in slot 2\nError: No monster in slot 2"
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn state_commands_only_accept_exact_python_monster_slots() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "spawn worker 2 0".to_string(),
                "stat m10".to_string(),
                "stat m0".to_string(),
                "stat m9".to_string(),
                "stat x1".to_string(),
                "status m10".to_string(),
                "status m0".to_string(),
                "status m9".to_string(),
                "status x1".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "spawn piranha 1 0\n",
                "spawn worker 2 0\n",
                "Error: \"m10\" is not a valid actor\n",
                "Error: \"m0\" is not a valid actor\n",
                "Error: \"m9\" is not a valid actor\n",
                "Error: \"x1\" is not a valid actor\n",
                "Error: \"m10\" is not a valid actor\n",
                "Error: \"m0\" is not a valid actor\n",
                "Error: \"m9\" is not a valid actor\n",
                "Error: \"x1\" is not a valid actor"
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn monster_with_one_action_defaults_blank_action_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("gandarewa".to_string()),
            10,
            false,
            2_000,
        ));

        let output = state.apply_monster_action(MonsterSlot(1), "", &[]);

        assert!(output.starts_with("Gandarewa (M1) -> Thunder ["));
        assert!(output.contains("# enemy rolls:"), "{output}");
    }

    #[test]
    fn monster_action_trailing_args_do_not_override_targets_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("gandarewa".to_string()),
            10,
            false,
            2_000,
        ));

        state.apply_monster_action(MonsterSlot(1), "thunder", &[String::from("tidus")]);

        assert!(!state
            .last_targets
            .contains(&ActorId::Character(Character::Tidus)));
        assert!(state.last_targets.iter().all(|target| matches!(
            target,
            ActorId::Character(Character::Auron | Character::Yuna)
        )));
    }

    #[test]
    fn monster_action_accepts_monster_names_like_python() {
        let mut state = SimulationState::new(1);

        let output = state.apply_named_monster_action("gandarewa", "thunder", &[]);

        assert!(
            output.starts_with("Gandarewa (M1) -> Thunder ["),
            "{output}"
        );
        assert!(output.contains("# enemy rolls:"), "{output}");
        assert!(state.monsters.is_empty());
        assert!(!state.last_targets.is_empty());
    }

    #[test]
    fn rendered_monster_actions_echo_command_lines_like_python() {
        let output = simulate(
            1,
            &[
                "spawn gandarewa 1 0".to_string(),
                "m1 thunder tidus".to_string(),
                "monsteraction gandarewa thunder".to_string(),
            ],
        );

        assert!(output.text.contains("m1 thunder tidus\n# enemy rolls:"));
        assert!(output.text.contains("gandarewa thunder\n# enemy rolls:"));
        assert!(!output.text.contains("Gandarewa (M1) -> Thunder ["));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn rendered_invalid_monster_actions_echo_without_mutating_like_python() {
        let output = simulate(
            1,
            &[
                "spawn melusine 1 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "m1 thundara".to_string(),
                "status ctb".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(
            output.text.contains("\nm1 thundara\nstatus ctb"),
            "{}",
            output.text
        );
        assert!(
            output.text.contains("# CTB: Au[0] M1[0] Ti[10]"),
            "{}",
            output.text
        );
        assert!(
            output.text.contains("m1 attack\n# enemy rolls:"),
            "{}",
            output.text
        );
        assert!(
            !output.text.contains("Error: Available actions"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn rendered_character_actions_echo_command_lines_like_python() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "tidus attack m1".to_string(),
            ],
        );

        assert!(output.text.contains("tidus attack m1\n# party rolls:"));
        assert!(!output.text.contains("Tidus -> Attack ["));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn bare_monster_name_lines_are_prepared_like_python_editor() {
        let prepared = crate::script::prepare_action_lines("gandarewa thunder\npiranha attack");
        let output = simulate(1, &prepared.lines);

        assert!(output.text.contains("gandarewa thunder"));
        assert!(output.text.contains("piranha attack"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn temporary_monster_actors_are_retired_after_successful_actions() {
        let mut state = SimulationState::new(1);

        let character_output =
            state.apply_character_action(Character::Tidus, "attack", &[String::from("piranha")]);
        assert!(
            character_output.contains("Piranha (M1) -> ["),
            "{character_output}"
        );
        assert!(state.temporary_monsters.is_empty());
        assert!(state
            .actor(ActorId::Monster(MonsterSlot(9)))
            .is_some_and(|actor| actor.temporary));

        let monster_output = state.apply_named_monster_action("gandarewa", "thunder", &[]);
        assert!(
            monster_output.starts_with("Gandarewa (M1) -> Thunder ["),
            "{monster_output}"
        );
        assert!(state.temporary_monsters.is_empty());
        assert!(state
            .actor(ActorId::Monster(MonsterSlot(10)))
            .is_some_and(|actor| actor.temporary));
    }

    #[test]
    fn counters_can_target_retired_named_monster_actions_like_python() {
        let output = simulate(
            1,
            &[
                "party ta".to_string(),
                "monsteraction gandarewa thunder".to_string(),
                "tidus counter".to_string(),
            ],
        );

        assert!(output.text.contains("tidus counter"), "{}", output.text);
        assert!(output.text.contains("# party rolls:"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn monster_action_forced_action_uses_monster_forced_action_like_python() {
        let named = simulate(1, &["monsteraction gandarewa forced_action".to_string()]);
        let slotted = simulate(
            1,
            &[
                "spawn gandarewa 1 0".to_string(),
                "monsteraction m1 forced_action".to_string(),
            ],
        );

        assert!(
            named.text.contains("gandarewa forced_action"),
            "{}",
            named.text
        );
        assert!(
            slotted.text.contains("m1 forced_action"),
            "{}",
            slotted.text
        );
        assert_eq!(named.unsupported_count, 0);
        assert_eq!(slotted.unsupported_count, 0);
    }

    #[test]
    fn monster_action_forced_action_fallbacks_match_python() {
        let no_forced_named = simulate(1, &["monsteraction worker forced_action".to_string()]);
        let forced_attack_named = simulate(1, &["monsteraction piranha forced_action".to_string()]);
        let no_forced_slotted = simulate(
            1,
            &[
                "spawn worker 1 0".to_string(),
                "monsteraction m1 forced_action".to_string(),
            ],
        );
        let forced_attack_slotted = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "monsteraction m1 forced_action".to_string(),
            ],
        );

        assert!(
            no_forced_named.text.contains("worker forced_action"),
            "{}",
            no_forced_named.text
        );
        assert!(
            forced_attack_named.text.contains("piranha forced_action"),
            "{}",
            forced_attack_named.text
        );
        assert!(
            no_forced_slotted.text.contains("m1 forced_action"),
            "{}",
            no_forced_slotted.text
        );
        assert!(
            forced_attack_slotted.text.contains("m1 forced_action"),
            "{}",
            forced_attack_slotted.text
        );
        assert_eq!(no_forced_named.unsupported_count, 0);
        assert_eq!(forced_attack_named.unsupported_count, 0);
        assert_eq!(no_forced_slotted.unsupported_count, 0);
        assert_eq!(forced_attack_slotted.unsupported_count, 0);
    }

    #[test]
    fn named_monster_actions_preserve_existing_monster_party_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("piranha".to_string()),
            10,
            false,
            200,
        ));
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));

        let output = state.apply_named_monster_action("gandarewa", "thunder", &[]);

        assert!(
            output.starts_with("Gandarewa (M1) -> Thunder ["),
            "{output}"
        );
        assert_eq!(
            state
                .monsters
                .iter()
                .filter_map(|actor| actor.monster_key.as_deref())
                .collect::<Vec<_>>(),
            vec!["piranha", "worker"]
        );
        assert!(state
            .current_battle_state()
            .monsters
            .iter()
            .all(|actor| !actor.temporary));
        assert!(state.temporary_monsters.is_empty());
        assert!(state
            .actor(state.last_actor.unwrap())
            .is_some_and(|actor| actor.temporary));
    }

    #[test]
    fn invalid_named_monster_actions_do_not_keep_temporary_actors() {
        let mut state = SimulationState::new(1);

        let output = state.apply_named_monster_action("gandarewa", "cheer", &[]);

        assert!(output.contains("Error: Available actions for Gandarewa (M1): thunder"));
        assert!(state.temporary_monsters.is_empty());
        assert_eq!(state.last_actor, Some(ActorId::Character(Character::Tidus)));
    }

    #[test]
    fn monster_action_name_errors_match_python() {
        let output = simulate(
            1,
            &[
                "monsteraction definitely_not_real attack".to_string(),
                "monsteraction gandarewa cheer".to_string(),
            ],
        );

        assert_eq!(output.unsupported_count, 0);
        assert!(output
            .text
            .contains("Error: No monster name or slot named \"definitely_not_real\""));
        assert!(output
            .text
            .contains("Error: Available actions for Gandarewa (M1): thunder"));
        assert!(output.text.contains("does_nothing"));
        assert!(output.text.contains("forced_action"));
    }

    #[test]
    fn heal_command_renders_like_python() {
        let output = simulate(
            1,
            &[
                "heal".to_string(),
                "heal tidus 50".to_string(),
                "heal nope".to_string(),
                "heal tidus nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "heal\n",
                "heal tidus 50\n",
                "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown\n",
                "heal tidus nope",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn heal_command_adds_amount_without_full_restore_like_python() {
        let mut state = SimulationState::new(1);
        let tidus = state.character_actor(Character::Tidus).unwrap().clone();
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = tidus.current_hp - 200;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_mp = tidus.current_mp - 5;

        state.heal_party(&["tidus".to_string(), "50".to_string()]);

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, 370);
        assert_eq!(tidus.current_mp, 12);
    }

    #[test]
    fn ap_command_renders_sphere_levels_like_python() {
        let output = simulate(
            1,
            &[
                "ap tidus".to_string(),
                "ap tidus 5".to_string(),
                "ap yuna 15".to_string(),
                "ap tidus nope".to_string(),
                "ap z 5".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Tidus: 0 S. Lv (0 AP Total, 5 for next level)\n",
                "Tidus: 1 S. Lv (5 AP Total, 10 for next level) (added 5 AP)\n",
                "Yuna: 1 S. Lv (15 AP Total, 20 for next level) (added 15 AP)\n",
                "Tidus: 1 S. Lv (5 AP Total, 10 for next level)\n",
                "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn ap_command_clamps_negative_total_ap_like_python() {
        let output = simulate(1, &["ap tidus -5".to_string()]);

        assert_eq!(
            output.text,
            "Tidus: 0 S. Lv (0 AP Total, 5 for next level) (added -5 AP)"
        );
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
        assert!(output.text.contains("Tanker (M1)'s CTB changed to"));
        let status_lines = output
            .text
            .lines()
            .filter(|line| line.starts_with("Status: Tanker (M1)"))
            .collect::<Vec<_>>();
        assert!(status_lines
            .iter()
            .any(|line| line.contains("Statuses: Sleep (254)")));
        assert!(status_lines
            .iter()
            .any(|line| !line.contains("Statuses: Sleep")));
        assert!(output.text.contains("spawn sinscale_3 4"));
        assert!(output.text.contains("tidus attack m1"));
        assert!(output.text.contains("Party: Tidus, Auron -> Valefor"));
        assert!(output
            .text
            .contains("Elemental affinity to Thunder of Tanker (M1) changed to Weak"));
        assert!(output.text.contains(
            "Equipment: Tidus | Weapon | Longsword [] -> Variable Steel [Strength +5%, -, -, -]"
        ));
        assert!(output.text.contains("Advanced rng4 1 times"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn character_actions_render_python_style_target_results() {
        let output = simulate(
            1,
            &[
                "encounter tanker".to_string(),
                "tidus attack m1".to_string(),
            ],
        );

        assert!(output.text.contains("tidus attack m1"), "{}", output.text);
        assert!(output.text.contains("# party rolls:"), "{}", output.text);
        assert!(
            !output.text.contains("Tidus -> Attack ["),
            "{}",
            output.text
        );
        assert!(!output.text.contains("Tidus -> Attack [42] |"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn character_actions_accept_temporary_monster_name_targets_like_python() {
        let mut state = SimulationState::new(1);

        let output =
            state.apply_character_action(Character::Tidus, "attack", &[String::from("piranha")]);

        assert!(output.contains("Tidus -> Attack ["), "{output}");
        assert!(output.contains("Piranha (M1) -> ["), "{output}");
        assert!(state
            .current_battle_state()
            .monsters
            .iter()
            .all(|actor| !actor.temporary));
        assert_eq!(state.last_targets.len(), 1);
        assert!(state.temporary_monsters.is_empty());
        assert!(state
            .actor(state.last_targets[0])
            .is_some_and(|actor| actor.temporary));
    }

    #[test]
    fn single_target_actions_accept_temporary_monster_names_like_python() {
        let mut state = SimulationState::new(1);

        let output =
            state.apply_character_action(Character::Tidus, "haste", &[String::from("piranha")]);

        assert!(output.contains("Tidus -> Haste ["), "{output}");
        assert!(output.contains("Piranha (M1) ->"), "{output}");
        assert_eq!(state.last_targets.len(), 1);
        assert!(state.temporary_monsters.is_empty());
        let target = state.actor(state.last_targets[0]).unwrap();
        assert!(target.temporary);
        assert!(target.statuses.contains(&Status::Haste));
    }

    #[test]
    fn summon_accepts_python_prefixes_and_magus_sisters_group() {
        let output = simulate(
            1,
            &[
                "summon val".to_string(),
                "party ta".to_string(),
                "summon magus".to_string(),
            ],
        );

        assert!(output.text.contains("Party: Tidus, Auron -> Valefor"));
        assert!(output
            .text
            .contains("Party: Tidus, Auron -> Cindy, Sandy, Mindy"));
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn unknown_summons_error_without_marking_unsupported_like_python() {
        let output = simulate(1, &["summon nope".to_string()]);

        assert_eq!(output.text, "Error: No aeon named \"nope\"");
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn status_atb_lists_only_actors_who_can_take_turn() {
        let output = simulate(
            1,
            &[
                "status tidus sleep".to_string(),
                "status ctb".to_string(),
                "status tidus sleep 0".to_string(),
                "status ctb".to_string(),
            ],
        );

        let lines = output.text.lines().collect::<Vec<_>>();
        assert_eq!(lines[1], "status ctb");
        assert_eq!(lines[2], "# CTB: Au[0]");
        assert_eq!(lines[4], "status ctb");
        assert_eq!(lines[5], "# CTB: Ti[0] Au[0]");
    }

    #[test]
    fn spawned_monsters_account_for_elapsed_ctb() {
        let mut state = SimulationState::new(1);
        state.ctb_since_last_action = 7;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb = 2;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .ctb = 5;

        state.spawn_monster("worker", MonsterSlot(1), Some(3));

        assert_eq!(state.monsters[0].ctb, 0);
        assert_eq!(state.character_actor(Character::Tidus).unwrap().ctb, 6);
        assert_eq!(state.character_actor(Character::Auron).unwrap().ctb, 9);
        assert_eq!(state.ctb_since_last_action, 3);
    }

    #[test]
    fn escape_uses_one_base_ctb_and_character_rng() {
        let mut state = SimulationState::new(1);
        let before_rng20 = state.rng.current_positions()[20];

        let rendered = state.apply_character_action(Character::Tidus, "escape", &[]);

        assert!(
            matches!(
                rendered.as_str(),
                "Tidus -> Escape [14]: Succeeded" | "Tidus -> Escape [14]: Failed"
            ),
            "{rendered}"
        );
        assert_eq!(state.rng.current_positions()[20], before_rng20 + 1);
    }

    #[test]
    fn successful_escape_ejects_character_like_python() {
        let seed = (1..5000)
            .find(|seed| {
                let mut state = SimulationState::new(*seed);
                state.escape_succeeds(ActorId::Character(Character::Tidus))
            })
            .unwrap();
        let mut state = SimulationState::new(seed);

        let rendered = state.apply_character_action(Character::Tidus, "escape", &[]);

        assert_eq!(rendered, "Tidus -> Escape [14]: Succeeded");
        assert!(state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::Eject));
    }

    #[test]
    fn stat_command_updates_combat_stats_used_by_damage() {
        let mut state = SimulationState::new(1);
        let tidus_strength = state
            .character_actor(Character::Tidus)
            .unwrap()
            .combat_stats
            .strength;
        {
            let tidus = state
                .actor_mut(ActorId::Character(Character::Tidus))
                .unwrap();
            tidus.current_hp = 100;
            tidus.current_mp = 1;
        }

        state.change_stat(&[
            "tidus".to_string(),
            "strength".to_string(),
            "+10".to_string(),
        ]);
        state.change_stat(&["tidus".to_string(), "luck".to_string(), "1".to_string()]);
        state.change_stat(&["tidus".to_string(), "hp".to_string(), "+200".to_string()]);
        state.change_stat(&["tidus".to_string(), "mp".to_string(), "+20".to_string()]);

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.combat_stats.strength, tidus_strength + 10);
        assert_eq!(tidus.combat_stats.luck, 1);
        assert_eq!(tidus.max_hp, 720);
        assert_eq!(tidus.current_hp, 100);
        assert_eq!(tidus.max_mp, 32);
        assert_eq!(tidus.current_mp, 1);
    }

    #[test]
    fn stat_command_without_stat_renders_python_style_summary() {
        let output = simulate(1, &["stat tidus".to_string()]);

        assert_eq!(
            output.text,
            "Stats: Tidus | HP 520 | MP 12 | STR 15 | DEF 10 | MAG 5 | MDF 5 | AGI 10 | LCK 18 | EVA 10 | ACC 10"
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn stat_ctb_command_renders_python_style_message() {
        let output = simulate(
            1,
            &[
                "stat tidus ctb 5".to_string(),
                "stat tidus ctb +5".to_string(),
                "stat tidus ctb -999".to_string(),
                "spawn piranha 1 0".to_string(),
                "stat m1 ctb -999".to_string(),
                "stat m1 ctb +7".to_string(),
                "stat tidus ctb nope".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Tidus's CTB changed to 5\n",
                "Tidus's CTB changed to 10\n",
                "Tidus's CTB changed to 0\n",
                "spawn piranha 1 0\n",
                "Piranha (M1)'s CTB changed to 0\n",
                "Piranha (M1)'s CTB changed to 7\n",
                "Error: amount must be an integer",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn stat_command_handles_mp_and_parse_errors_like_python() {
        let output = simulate(
            1,
            &[
                "stat tidus mp +20".to_string(),
                "stat tidus nope 1".to_string(),
                "stat tidus strength nope".to_string(),
                "stat tidus strength".to_string(),
            ],
        );

        assert!(output.text.contains("Stat: Tidus | MP | 12 -> 32"));
        assert!(output.text.contains(&format!(
            "Error: stat can only be one of these values: {}",
            super::stat_values()
        )));
        assert_eq!(
            output
                .text
                .matches("Error: amount must be an integer")
                .count(),
            2
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn monster_hp_and_mp_stat_edits_use_python_monster_caps() {
        let output = simulate(
            1,
            &[
                "stat tidus hp 999999".to_string(),
                "stat tidus mp 999999".to_string(),
                "spawn piranha 1 0".to_string(),
                "stat m1 hp 999999".to_string(),
                "stat m1 mp 999999".to_string(),
                "stat m1 strength 999999".to_string(),
                "stat m1 hp -5".to_string(),
                "stat m1 mp -5".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "Stat: Tidus | HP | 520 -> 99999\n",
                "Stat: Tidus | MP | 12 -> 9999\n",
                "spawn piranha 1 0\n",
                "Stat: Piranha (M1) | HP | 50 -> 999999\n",
                "Stat: Piranha (M1) | MP | 1 -> 999999\n",
                "Stat: Piranha (M1) | Strength | 6 -> 255\n",
                "Stat: Piranha (M1) | HP | 999999 -> 999994\n",
                "Stat: Piranha (M1) | MP | 999999 -> 999994",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn hp_stat_edits_to_zero_clear_statuses_like_python() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "status m1 haste".to_string(),
                "stat m1 hp 0".to_string(),
                "status m1".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "spawn piranha 1 0\n",
                "Status: Piranha (M1) 50/50 HP | Statuses: Haste (254)\n",
                "Stat: Piranha (M1) | HP | 50 -> 0\n",
                "Status: Piranha (M1) 0/0 HP | Statuses: Death (254)",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn hp_stat_edits_do_not_auto_remove_death_like_python() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "status m1 death".to_string(),
                "stat m1 hp +1".to_string(),
                "status m1".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "spawn piranha 1 0\n",
                "Status: Piranha (M1) 50/50 HP | Statuses: Death (254)\n",
                "Stat: Piranha (M1) | HP | 50 -> 51\n",
                "Status: Piranha (M1) 50/51 HP | Statuses: Death (254)",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn renders_full_default_script_without_parser_gaps() {
        let input = include_str!("../fixtures/ctb_actions_input.txt");
        assert_actions_notes_render_without_parser_gaps(input);
    }

    #[test]
    fn renders_boosters_actions_notes_without_parser_gaps() {
        let input = include_str!("../data/notes/boosters/actions_notes.txt");
        assert_actions_notes_render_without_parser_gaps(input);
    }

    #[test]
    fn renders_nemesis_actions_notes_without_parser_gaps() {
        let input = include_str!("../data/notes/nemesis/actions_notes.txt");
        assert_actions_notes_render_without_parser_gaps(input);
    }

    #[test]
    fn renders_no_sphere_grid_actions_notes_without_parser_gaps() {
        let input = include_str!("../data/notes/no_sphere_grid/actions_notes.txt");
        assert_actions_notes_render_without_parser_gaps(input);
    }

    fn assert_actions_notes_render_without_parser_gaps(input: &str) {
        let prepared = crate::script::prepare_action_lines(input);
        let output = simulate(3096296922, &prepared.lines);
        let errors = output
            .text
            .lines()
            .filter(|line| line.starts_with("Error:"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(output.unsupported_count, 0, "{errors}");
        assert!(errors.is_empty(), "{errors}");
        assert!(output.text.contains("Encounter:"));
    }

    #[test]
    fn render_ctb_preserves_block_comment_lines_like_python_web_editor() {
        let lines = vec![
            "encounter tanker".to_string(),
            "/*".to_string(),
            "party yuna".to_string(),
            "*/".to_string(),
            "status ctb".to_string(),
        ];
        let output = simulate(3096296922, &lines);
        assert_eq!(output.unsupported_count, 0, "{}", output.text);
        assert!(output.text.contains("/*\nparty yuna\n*/"));
        assert!(output.text.contains("Ti["));
        assert!(output.text.contains("Au["));
        assert!(!output.text.contains("Yu["));
    }

    #[test]
    fn render_ctb_preserves_indented_block_comment_lines_like_python_web_editor() {
        let lines = vec![
            "encounter tanker".to_string(),
            "  /*".to_string(),
            "party yuna".to_string(),
            "  */".to_string(),
            "status ctb".to_string(),
        ];
        let output = simulate(3096296922, &lines);
        assert_eq!(output.unsupported_count, 0, "{}", output.text);
        assert!(output.text.contains("  /*\nparty yuna\n  */"));
        assert!(output.text.contains("Ti["));
        assert!(output.text.contains("Au["));
        assert!(!output.text.contains("Yu["));
    }

    #[test]
    fn character_actions_advance_earlier_virtual_monster_turns() {
        let lines = vec![
            "spawn gandarewa 1 0".to_string(),
            "stat tidus ctb 10".to_string(),
            "tidus defend".to_string(),
        ];
        let output = simulate(3096296922, &lines);
        let monster_index = output.text.find("m1\n").unwrap();
        let tidus_index = output.text.find("tidus defend").unwrap();
        assert!(monster_index < tidus_index, "{}", output.text);
    }

    #[test]
    fn virtual_monster_turns_use_python_like_default_actions() {
        let output = simulate(
            1,
            &[
                "encounter lancet_tutorial".to_string(),
                "stat m1 ctb 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("m1 seed_cannon"), "{}", output.text);
        assert!(output.text.contains("# enemy rolls:"), "{}", output.text);
        assert!(output.text.contains(" HP -> "), "{}", output.text);
        assert!(
            output.text.contains("# enemy rolls:") && output.text.contains(" HP\n\ntidus defend"),
            "{}",
            output.text
        );
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn virtual_default_attack_calculates_python_enemy_damage() {
        let output = simulate(
            1,
            &[
                "party ta".to_string(),
                "spawn dingo 1 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("m1 attack"), "{}", output.text);
        assert!(
            output
                .text
                .contains("# enemy rolls: Auron: [25/31] 91 HP -> 939/1030 HP"),
            "{}",
            output.text
        );
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn virtual_monster_turn_preview_does_not_run_auto_potion_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state.spawn_monster("dingo", MonsterSlot(1), Some(0));
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb = 10;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .armor_abilities
            .insert(AutoAbility::AutoPotion);
        let potion_count_before = state
            .item_inventory
            .iter()
            .find(|(item, _)| item.as_deref() == Some("Potion"))
            .map(|(_, quantity)| *quantity);

        let output = state
            .advance_virtual_turns_before(ActorId::Character(Character::Tidus))
            .join("\n");

        assert!(output.contains("m1 attack"), "{output}");
        assert!(output.contains("# enemy rolls:"), "{output}");
        assert!(!output.contains("Auto Potion"), "{output}");
        assert_eq!(
            state
                .item_inventory
                .iter()
                .find(|(item, _)| item.as_deref() == Some("Potion"))
                .map(|(_, quantity)| *quantity),
            potion_count_before
        );
    }

    #[test]
    fn virtual_monster_turn_preview_does_not_persist_target_memory_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state.spawn_monster("dingo", MonsterSlot(1), Some(0));
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb = 10;

        let output = state
            .advance_virtual_turns_before(ActorId::Character(Character::Tidus))
            .join("\n");

        assert!(output.contains("m1 attack"), "{output}");
        assert_eq!(state.last_actor, Some(ActorId::Monster(MonsterSlot(1))));
        assert!(state.last_targets.is_empty());
        assert!(!state
            .actor_last_targets
            .contains_key(&ActorId::Monster(MonsterSlot(1))));
        assert!(state.actor_last_attackers.is_empty());
    }

    #[test]
    fn virtual_monster_turns_use_single_action_shorthand_like_python() {
        let output = simulate(
            1,
            &[
                "spawn gandarewa 1 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("\nm1\n"), "{}", output.text);
        assert!(output.text.contains("# enemy rolls:"), "{}", output.text);
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn virtual_single_action_shorthand_calculates_python_enemy_damage() {
        let output = simulate(
            1,
            &[
                "spawn gandarewa 1 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("\nm1\n"), "{}", output.text);
        assert!(
            output
                .text
                .contains("# enemy rolls: Auron: [25/31] 299 HP -> 731/1030 HP"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn virtual_monster_actions_track_python_preview_display_source() {
        let mut generic_state = SimulationState::new(1);
        generic_state.spawn_monster("gandarewa", MonsterSlot(1), Some(0));
        assert_eq!(
            generic_state
                .virtual_monster_action(MonsterSlot(1))
                .unwrap()
                .preview_display,
            Some("m1".to_string())
        );

        let mut default_state = SimulationState::new(1);
        default_state.start_encounter("lancet_tutorial", false, &[]);
        assert_eq!(
            default_state
                .virtual_monster_action(MonsterSlot(1))
                .unwrap()
                .preview_display,
            None
        );
    }

    #[test]
    fn virtual_default_actions_require_monster_action_table_like_python() {
        let output = simulate(
            3096296922,
            &[
                "encounter seymour_omnis".to_string(),
                "tidus armor_break m1".to_string(),
                "bahamut attack m1".to_string(),
            ],
        );

        assert!(output.text.contains("\nm1\n\n"), "{}", output.text);
        assert!(!output.text.contains("m1 thundara"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn planned_party_restores_after_forced_summary_when_needed_like_python() {
        let output = simulate(
            3096296922,
            &[
                "encounter bfa".to_string(),
                "stat rikku ctb 0".to_string(),
                "stat bahamut ctb 10".to_string(),
                "stat m1 ctb 1".to_string(),
                "rikku armor_break m1".to_string(),
                "# rikku chaos_grenade".to_string(),
                "bahamut attack m1".to_string(),
            ],
        );

        let virtual_turns = output
            .text
            .find("\nm1\nm2\nm3")
            .unwrap_or_else(|| panic!("{}", output.text));
        let bahamut_action = output
            .text
            .find("\nbahamut attack m1")
            .unwrap_or_else(|| panic!("{}", output.text));
        assert!(virtual_turns < bahamut_action, "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn virtual_sinscale_turns_are_manual_only_like_python() {
        let output = simulate(
            1,
            &[
                "spawn sinscale_3 1 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(
            !output.text.contains("Sinscale#3 (M1) ->"),
            "{}",
            output.text
        );
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn echuilles_virtual_monster_turns_are_manual_only_like_python() {
        let output = simulate(
            1,
            &[
                "encounter echuilles".to_string(),
                "stat m1 ctb 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(
            !output.text.contains("Sinspawn Echuilles (M1) ->"),
            "{}",
            output.text
        );
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn geosgaeno_ambush_applies_two_scripted_half_hp_pre_actions_like_python() {
        let seed = (1..5000)
            .find(|seed| {
                let mut state = SimulationState::new(*seed);
                state
                    .start_encounter("geosgaeno", false, &[])
                    .contains("Ambush")
            })
            .expect("test seed with Geosgaeno ambush");
        let mut state = SimulationState::new(seed);
        let encounter = state.start_encounter("geosgaeno", false, &[]);
        assert!(encounter.contains("Ambush"), "{encounter}");

        let first = state.apply_character_action(Character::Tidus, "defend", &[]);
        let second = state.apply_character_action(Character::Tidus, "defend", &[]);
        let third = state.apply_character_action(Character::Tidus, "defend", &[]);

        assert!(first.contains("m1 half_hp"), "{first}");
        assert!(first.contains("# enemy rolls:"), "{first}");
        assert!(first.contains(" HP\n\nTidus -> Defend"), "{first}");
        assert!(second.contains("m1 half_hp"), "{second}");
        assert!(second.contains("# enemy rolls:"), "{second}");
        assert!(second.contains(" HP\n\nTidus -> Defend"), "{second}");
        assert_eq!(state.scripted_turn_index, 2);
        assert!(third.contains("Tidus -> Defend"), "{third}");
    }

    #[test]
    fn geosgaeno_normal_does_not_apply_ambush_scripted_pre_actions() {
        let mut state = SimulationState::new(1);
        let encounter = state.start_encounter("geosgaeno_jp", false, &[]);
        assert!(encounter.contains("Normal"), "{encounter}");

        let _ = state.apply_character_action(Character::Tidus, "defend", &[]);

        assert_eq!(state.scripted_turn_index, 0);
    }

    #[test]
    fn geneaux_uses_scripted_starting_party_like_python() {
        let mut state = SimulationState::new(1);
        state.set_party_from_initials("twa");

        let output = state.start_encounter("geneaux", false, &[]);

        assert_eq!(
            state.party,
            vec![Character::Tidus, Character::Yuna, Character::Lulu]
        );
        assert!(output.contains("Ti["), "{output}");
        assert!(output.contains("Yu["), "{output}");
        assert!(output.contains("Lu["), "{output}");
        assert!(!output.contains("Wa["), "{output}");
        assert!(!output.contains("Au["), "{output}");
    }

    #[test]
    fn ammes_virtual_turn_uses_scripted_demi_like_python() {
        let output = simulate(
            1,
            &[
                "encounter ammes".to_string(),
                "stat m1 ctb 0".to_string(),
                "stat m2 ctb 999".to_string(),
                "stat m3 ctb 999".to_string(),
                "stat m4 ctb 999".to_string(),
                "stat m5 ctb 999".to_string(),
                "stat m6 ctb 999".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("m1 demi"), "{}", output.text);
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn wakka_tutorial_condor_virtual_turn_uses_scripted_attack_wakka_like_python() {
        let output = simulate(
            1,
            &[
                "encounter wakka_tutorial".to_string(),
                "stat m1 ctb 999".to_string(),
                "stat m2 ctb 0".to_string(),
                "stat tidus ctb 10".to_string(),
                "tidus defend".to_string(),
            ],
        );

        assert!(output.text.contains("m2 attack_wakka"), "{}", output.text);
        assert!(output.text.contains("tidus defend"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn sinscales_respawn_once_when_all_monsters_are_dead_like_python() {
        let output = simulate(
            1,
            &[
                "encounter sinscales".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "status m3 death".to_string(),
                "status ctb".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "status m3 death".to_string(),
                "status m4 death".to_string(),
                "status m5 death".to_string(),
                "status ctb".to_string(),
            ],
        );

        assert_eq!(
            output.text.matches("spawn sinscale_6").count(),
            5,
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn scripted_starting_monster_parties_trim_like_python() {
        let sinscales = simulate(1, &["encounter sinscales".to_string()]);
        assert!(sinscales.text.contains("M3["), "{}", sinscales.text);
        assert!(!sinscales.text.contains("M4["), "{}", sinscales.text);

        let sahagin_chiefs = simulate(1, &["encounter sahagin_chiefs".to_string()]);
        assert!(
            sahagin_chiefs.text.contains("M3["),
            "{}",
            sahagin_chiefs.text
        );
        assert!(
            !sahagin_chiefs.text.contains("M4["),
            "{}",
            sahagin_chiefs.text
        );
    }

    #[test]
    fn generated_encounter_comments_sync_monster_slots_for_pasted_routes() {
        let output = simulate(
            3096296922,
            &[
                "encounter sinscales".to_string(),
                "# Encounter:   1 | Sinscales | Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6, Sinscale#6 Normal | Ti[41] M2[45] M5[46] M1[47] M7[47] Au[48] M6[48] M8[48] M4[49] M3[50]".to_string(),
                "status ctb".to_string(),
                "tidus attack m6".to_string(),
            ],
        );

        assert!(
            output.text.contains(
                "# CTB: Ti[41] M2[45] M5[46] M1[47] M7[47] Au[48] M6[48] M8[48] M4[49] M3[50]"
            ),
            "{}",
            output.text
        );
        assert!(output.text.contains("Sinscale#6 (M6):"), "{}", output.text);
        assert!(
            !output.text.contains("Error: No monster in slot 6"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn tanker_placeholder_comments_are_skipped_like_python() {
        let output = simulate(
            1,
            &[
                "encounter tanker".to_string(),
                "#m1".to_string(),
                " #m8 ".to_string(),
                "#m9".to_string(),
                "# not a placeholder".to_string(),
            ],
        );

        assert!(!output.text.contains("#m1"), "{}", output.text);
        assert!(!output.text.contains("#m8"), "{}", output.text);
        assert!(output.text.contains("#m9"), "{}", output.text);
        assert!(
            output.text.contains("# not a placeholder"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn sahagin_chief_spawn_comments_apply_spawns_like_python() {
        let output = simulate(
            1,
            &[
                "encounter sahagin_chiefs".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "# 2 Sahagin Chiefs spawn".to_string(),
                "# 2 Sahagin Chiefs spawn, 4th appears".to_string(),
                "tidus attack m4".to_string(),
            ],
        );

        assert!(
            output.text.contains("# 2 Sahagin Chiefs spawn"),
            "{}",
            output.text
        );
        assert_eq!(output.text.matches("spawn sahagin_chief").count(), 3);
        assert!(
            !output.text.contains("Spawn: Sahagin Chief"),
            "{}",
            output.text
        );
        assert!(output.text.contains("tidus attack m4"), "{}", output.text);
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn unavailable_action_actors_render_skipped_comments_like_python() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1".to_string(),
                "status tidus sleep".to_string(),
                "tidus definitely_not_real m1".to_string(),
                "status m1 death".to_string(),
                "m1 definitely_not_real tidus".to_string(),
            ],
        );

        assert!(
            output
                .text
                .contains("# skipped: tidus definitely_not_real m1 (Tidus is asleep)"),
            "{}",
            output.text
        );
        assert!(
            output
                .text
                .contains("# skipped: m1 definitely_not_real tidus (Piranha (M1) is KO'd)"),
            "{}",
            output.text
        );
        assert!(
            !output.text.contains("Error: No action named"),
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn machina_three_respawns_worker_waves_like_python() {
        let output = simulate(
            1,
            &[
                "encounter machina_3".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "status ctb".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "status ctb".to_string(),
                "status m1 death".to_string(),
                "status m2 death".to_string(),
                "status ctb".to_string(),
            ],
        );

        assert_eq!(
            output.text.matches("spawn worker").count(),
            4,
            "{}",
            output.text
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn directives_render_as_raw_lines_like_python_ctb_editor() {
        let output = simulate(
            3096296922,
            &[
                "/usage".to_string(),
                "/macro nope".to_string(),
                "/nopadding".to_string(),
                "///".to_string(),
                "/foo".to_string(),
                "encounter tanker".to_string(),
            ],
        );
        assert_eq!(output.unsupported_count, 0);
        assert!(output
            .text
            .starts_with("/usage\n/macro nope\n/nopadding\n///\n/foo\n"));
        assert!(output.text.contains("Encounter:"));
    }

    #[test]
    fn blank_lines_are_collapsed_like_python_renderer() {
        let output = simulate(
            3096296922,
            &[
                "encounter tanker".to_string(),
                String::new(),
                String::new(),
                "status ctb".to_string(),
            ],
        );
        assert!(!output.text.contains("\n\n\n"), "{}", output.text);
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
    fn status_flags_immunity_is_silent_like_python() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("tanker".to_string()),
            10,
            false,
            1_000,
        );
        monster.status_resistances.insert(Status::Curse, 255);
        state.monsters.push(monster);

        let mut action_data = test_action(ActionTarget::SingleMonster);
        action_data.status_flags.push(Status::Curse);
        let results = state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action_data),
            &[String::from("m1")],
        );

        assert!(results[0].statuses.is_empty());
        assert!(!state.monsters[0].statuses.contains(&Status::Curse));
        assert!(
            !super::format_action_output("Tidus", "Test", 3, &results, &state, None)
                .contains("Fail")
        );
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
    fn petrified_targets_do_not_receive_action_buffs() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.set_status(Status::Petrify, 254);

        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Tidus), "cheer");
        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert!(tidus.statuses.contains(&Status::Petrify));
        assert!(!tidus.buffs.contains_key(&Buff::Cheer));
    }

    #[test]
    fn petrify_status_application_preserves_existing_buffs_like_python() {
        let mut state = SimulationState::new(1);
        let yuna = state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap();
        yuna.buffs.insert(Buff::Cheer, 2);
        yuna.set_status(Status::Haste, 254);
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.status_applications.push(ActionStatus {
            status: Status::Petrify,
            chance: 254,
            stacks: 254,
            ignores_resistance: false,
        });

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("yuna")],
        );

        let yuna = state.actor(ActorId::Character(Character::Yuna)).unwrap();
        assert_eq!(yuna.buffs.get(&Buff::Cheer), Some(&2));
        assert!(yuna.statuses.contains(&Status::Petrify));
        assert!(!yuna.statuses.contains(&Status::Haste));
    }

    #[test]
    fn petrify_final_cleanup_removes_later_action_statuses_like_python() {
        let mut state = SimulationState::new(1);
        let yuna = state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap();
        yuna.buffs.insert(Buff::Cheer, 2);
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.status_applications.push(ActionStatus {
            status: Status::Petrify,
            chance: 254,
            stacks: 254,
            ignores_resistance: false,
        });
        action.status_applications.push(ActionStatus {
            status: Status::Poison,
            chance: 254,
            stacks: 254,
            ignores_resistance: false,
        });
        action.status_flags.push(Status::Curse);

        let results = state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("yuna")],
        );

        let yuna = state.actor(ActorId::Character(Character::Yuna)).unwrap();
        assert!(yuna.statuses.contains(&Status::Petrify));
        assert!(!yuna.statuses.contains(&Status::Poison));
        assert!(!yuna.statuses.contains(&Status::Curse));
        assert_eq!(yuna.buffs.get(&Buff::Cheer), Some(&2));
        assert!(results[0].statuses.contains(&(Status::Poison, true)));
        assert!(results[0].statuses.contains(&(Status::Curse, true)));
    }

    #[test]
    fn status_output_lists_active_buffs() {
        let output = simulate(
            1,
            &["tidus cheer tidus".to_string(), "status tidus".to_string()],
        );

        assert!(output
            .text
            .contains("Status: Tidus 520/520 HP | Buffs: Cheer (1)"));
    }

    #[test]
    fn delay_actions_ignore_delay_immunity_like_python() {
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

        assert!(state.monsters[0].immune_to_delay);
        assert_eq!(
            state.monsters[0].ctb,
            12 + state.monsters[0].base_ctb() * 3 / 2
        );
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
    fn all_targets_include_party_and_monsters() {
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

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::EitherParty),
                &[]
            ),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Wakka),
                ActorId::Monster(MonsterSlot(1)),
                ActorId::Monster(MonsterSlot(2)),
            ]
        );
    }

    #[test]
    fn party_targets_repeat_whole_target_list_for_multi_hit_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Wakka];
        let mut action = test_action(ActionTarget::CharactersParty);
        action.n_of_hits = 2;

        assert_eq!(
            state.resolve_action_targets(ActorId::Character(Character::Yuna), &action, &[]),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Wakka),
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Wakka),
            ]
        );
    }

    #[test]
    fn empty_default_target_sets_fall_back_to_unknown_actor() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::SingleCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &test_action(ActionTarget::MonstersParty),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn missing_last_attacker_falls_back_to_unknown_actor() {
        let mut state = SimulationState::new(1);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastAttacker),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn filtered_random_targets_respect_status_predicates() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .insert(Status::Poison);
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .statuses
            .insert(Status::Reflect);

        let without_poison = test_action(ActionTarget::RandomCharacterWithout(Status::Poison));
        let with_reflect = test_action(ActionTarget::RandomCharacterWith(Status::Reflect));
        let without_reflect_or_shell = test_action(ActionTarget::RandomCharacterWithoutEither(
            Status::Reflect,
            Status::Shell,
        ));
        let mut expected_rng = FfxRngTracker::new(1);
        let without_poison_candidates = [Character::Yuna, Character::Auron];
        let expected_without_poison = without_poison_candidates
            [expected_rng.advance_rng(4) as usize % without_poison_candidates.len()];
        let without_reflect_or_shell_candidates = [Character::Tidus, Character::Auron];
        let expected_without_reflect_or_shell = without_reflect_or_shell_candidates
            [expected_rng.advance_rng(4) as usize % without_reflect_or_shell_candidates.len()];

        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &without_poison, &[]),
            vec![ActorId::Character(expected_without_poison)]
        );
        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &with_reflect, &[]),
            vec![ActorId::Character(Character::Yuna)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &without_reflect_or_shell,
                &[]
            ),
            vec![ActorId::Character(expected_without_reflect_or_shell)]
        );
    }

    #[test]
    fn filtered_random_monster_targets_respect_status_predicates() {
        let mut state = SimulationState::new(1);
        let mut first = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        first.statuses.insert(Status::Protect);
        state.monsters.push(first);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action = test_action(ActionTarget::RandomMonsterWithout(Status::Protect));

        assert_eq!(
            state.resolve_action_targets(ActorId::Character(Character::Tidus), &action, &[]),
            vec![ActorId::Monster(MonsterSlot(2))]
        );
    }

    #[test]
    fn random_targets_use_python_rng_lanes() {
        let mut character_state = SimulationState::new(1);
        character_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        character_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        character_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(3),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut expected_rng = FfxRngTracker::new(1);
        let expected_monster = MonsterSlot((expected_rng.advance_rng(5) as usize % 3) + 1);

        assert_eq!(
            character_state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &test_action(ActionTarget::RandomMonster),
                &[]
            ),
            vec![ActorId::Monster(expected_monster)]
        );

        let mut monster_state = SimulationState::new(1);
        monster_state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        let mut expected_rng = FfxRngTracker::new(1);
        let expected_character = [Character::Tidus, Character::Yuna, Character::Auron]
            [expected_rng.advance_rng(4) as usize % 3];

        assert_eq!(
            monster_state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::RandomCharacter),
                &[]
            ),
            vec![ActorId::Character(expected_character)]
        );
    }

    #[test]
    fn target_selection_filters_dead_and_ejected_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);
        let yuna = state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap();
        yuna.set_status(Status::Eject, 254);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::SingleCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Auron)]
        );

        let mut can_target_dead = test_action(ActionTarget::SingleCharacter);
        can_target_dead.can_target_dead = true;
        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &can_target_dead, &[]),
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn character_targets_are_sorted_by_actor_index_not_party_order_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron, Character::Tidus];
        let action = test_action(ActionTarget::SingleCharacter);

        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &action, &[]),
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn multi_hit_random_targets_roll_rng5_per_hit_and_sort() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        let mut action = test_action(ActionTarget::RandomCharacter);
        action.n_of_hits = 3;
        let mut expected_rng = FfxRngTracker::new(1);
        let possible = [
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Yuna),
            ActorId::Character(Character::Auron),
        ];
        let mut expected = (0..3)
            .map(|_| possible[expected_rng.advance_rng(5) as usize % possible.len()])
            .collect::<Vec<_>>();
        expected.sort_by_key(|target| actor_sort_index(*target));

        assert_eq!(
            state.resolve_action_targets(ActorId::Character(Character::Tidus), &action, &[]),
            expected
        );
    }

    #[test]
    fn monster_multi_hit_random_character_targets_use_rng5_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        let mut action = test_action(ActionTarget::RandomCharacter);
        action.n_of_hits = 3;
        let possible = [
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Yuna),
            ActorId::Character(Character::Auron),
        ];
        let mut expected_rng = FfxRngTracker::new(1);
        let mut expected = (0..3)
            .map(|_| possible[expected_rng.advance_rng(5) as usize % possible.len()])
            .collect::<Vec<_>>();
        expected.sort_by_key(|target| actor_sort_index(*target));

        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &action, &[]),
            expected
        );
        let positions = state.rng.current_positions();
        assert_eq!(positions[4], 0);
        assert_eq!(positions[5], 3);
    }

    #[test]
    fn last_target_and_last_attacker_targets_use_recent_state() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action = test_action(ActionTarget::SingleCharacter);

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&action),
            &[String::from("yuna")],
        );

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&test_action(ActionTarget::SingleMonster)),
            &[String::from("m1")],
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
        state.last_actor = Some(ActorId::Character(Character::Tidus));
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastAttacker),
                &[]
            ),
            vec![ActorId::Character(Character::Tidus)]
        );
        let yuna = state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap();
        yuna.current_hp = 0;
        yuna.statuses.insert(Status::Death);
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn monster_last_target_fallback_ignores_global_last_targets_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("chocobo_eater".to_string()),
            10,
            false,
            10_000,
        ));
        state.last_targets = vec![ActorId::Character(Character::Tidus)];

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Auron)]
        );
    }

    #[test]
    fn monster_last_target_memory_keeps_petrified_targets_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.actor_last_targets.insert(
            ActorId::Monster(MonsterSlot(1)),
            vec![ActorId::Character(Character::Tidus)],
        );
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Petrify, 254);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn no_target_actions_clear_actor_last_targets_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
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
        state.actor_last_targets.insert(
            ActorId::Monster(MonsterSlot(1)),
            vec![ActorId::Monster(MonsterSlot(2))],
        );

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&test_action(ActionTarget::None)),
            &[],
        );

        assert!(state
            .actor_last_targets
            .get(&ActorId::Monster(MonsterSlot(1)))
            .unwrap()
            .is_empty());
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn encounter_start_preserves_character_memory_without_reusing_old_monster_slots() {
        let mut state = SimulationState::new(1);
        state.last_actor = Some(ActorId::Character(Character::Yuna));
        state.last_targets = vec![
            ActorId::Character(Character::Tidus),
            ActorId::Monster(MonsterSlot(1)),
        ];
        state.actor_last_targets.insert(
            ActorId::Character(Character::Yuna),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Monster(MonsterSlot(1)),
            ],
        );
        state.actor_provokers.insert(
            ActorId::Monster(MonsterSlot(1)),
            ActorId::Character(Character::Auron),
        );
        state.actor_last_attackers.insert(
            ActorId::Character(Character::Auron),
            ActorId::Monster(MonsterSlot(1)),
        );

        state.process_start_of_encounter();

        assert_eq!(state.last_actor, Some(ActorId::Character(Character::Yuna)));
        assert_eq!(
            state.last_targets,
            vec![ActorId::Character(Character::Tidus)]
        );
        assert_eq!(
            state
                .actor_last_targets
                .get(&ActorId::Character(Character::Yuna)),
            Some(&vec![ActorId::Character(Character::Tidus)])
        );
        assert!(!state
            .actor_provokers
            .contains_key(&ActorId::Monster(MonsterSlot(1))));
        assert!(!state
            .actor_last_attackers
            .contains_key(&ActorId::Character(Character::Auron)));
    }

    #[test]
    fn encounter_start_clears_stale_monster_last_actor_before_slots_are_reused() {
        let mut state = SimulationState::new(1);
        state.last_actor = Some(ActorId::Monster(MonsterSlot(1)));

        state.process_start_of_encounter();

        assert_eq!(state.last_actor, None);
    }

    #[test]
    fn missing_last_attacker_ignores_global_last_actor_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.last_actor = Some(ActorId::Character(Character::Tidus));

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LastAttacker),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn counter_monster_target_types_resolve_without_losing_counter_semantics() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.actor_last_targets.insert(
            ActorId::Monster(MonsterSlot(1)),
            vec![ActorId::Character(Character::Yuna)],
        );

        let counter_self = test_action(ActionTarget::CounterSelf);
        assert!(super::action_is_counter(&counter_self));
        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &counter_self, &[]),
            vec![ActorId::Monster(MonsterSlot(1))]
        );

        let counter_party = test_action(ActionTarget::CounterCharactersParty);
        assert!(super::action_is_counter(&counter_party));
        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &counter_party, &[]),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Yuna),
            ]
        );

        let counter_all = test_action(ActionTarget::CounterAll);
        assert!(super::action_is_counter(&counter_all));
        assert_eq!(
            state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &counter_all, &[]),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Yuna),
                ActorId::Monster(MonsterSlot(1)),
            ]
        );

        let counter_last_target = test_action(ActionTarget::CounterLastTarget);
        assert!(super::action_is_counter(&counter_last_target));
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &counter_last_target,
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
    }

    #[test]
    fn counter_random_character_uses_monster_target_rng() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        let action = test_action(ActionTarget::CounterRandomCharacter);

        let targets = state.resolve_action_targets(ActorId::Monster(MonsterSlot(1)), &action, &[]);

        assert_eq!(targets.len(), 1);
        let positions = state.rng.current_positions();
        assert_eq!(positions[4], 1);
        assert_eq!(positions[5], 0);
        assert!(super::action_is_counter(&action));
    }

    #[test]
    fn overdrive_targets_advance_python_pre_target_rng_lanes() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let spiral_cut = state
            .action_data_for_actor(ActorId::Character(Character::Tidus), "spiral_cut")
            .unwrap();

        assert_eq!(
            state.resolve_action_targets(ActorId::Character(Character::Tidus), &spiral_cut, &[]),
            vec![ActorId::Monster(MonsterSlot(1))]
        );
        assert_eq!(state.rng.current_positions()[20], 2);

        let mut wakka_state = SimulationState::new(1);
        wakka_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        wakka_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker_2".to_string()),
            10,
            false,
            1_000,
        ));
        let fire_shot = wakka_state
            .action_data_for_actor(ActorId::Character(Character::Wakka), "fire_shot")
            .unwrap();

        let targets = wakka_state.resolve_action_targets(
            ActorId::Character(Character::Wakka),
            &fire_shot,
            &[],
        );

        assert_eq!(targets.len(), 1);
        let positions = wakka_state.rng.current_positions();
        assert_eq!(positions[24], 2);
        assert_eq!(positions[19], 1);
        assert_eq!(positions[5], 0);

        let mut attack_reels_state = SimulationState::new(1);
        attack_reels_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(1),
                Some("worker".to_string()),
                10,
                false,
                1_000,
            ));
        attack_reels_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(2),
                Some("worker_2".to_string()),
                10,
                false,
                1_000,
            ));
        let mut attack_reels = test_action(ActionTarget::RandomMonster);
        attack_reels.overdrive_user = Some(Character::Wakka);
        attack_reels.overdrive_index = 5;
        attack_reels.base_damage = 10;

        let targets = attack_reels_state.resolve_action_targets(
            ActorId::Character(Character::Wakka),
            &attack_reels,
            &[String::from("3")],
        );

        assert_eq!(targets.len(), 1);
        assert_eq!(attack_reels_state.rng.current_positions()[19], 1);

        let mut attack_reels_hit_state = SimulationState::new(1);
        attack_reels_hit_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(1),
                Some("worker".to_string()),
                10,
                false,
                1_000,
            ));
        attack_reels_hit_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(2),
                Some("worker_2".to_string()),
                10,
                false,
                1_000,
            ));

        let targets = attack_reels_hit_state.resolve_action_targets(
            ActorId::Character(Character::Wakka),
            &attack_reels,
            &[String::from("3"), String::from("4")],
        );

        assert_eq!(targets.len(), 4);
        assert_eq!(attack_reels_hit_state.rng.current_positions()[19], 4);

        let mut lulu_state = SimulationState::new(1);
        lulu_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        lulu_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker_2".to_string()),
            10,
            false,
            1_000,
        ));
        let fire_fury = lulu_state
            .action_data_for_actor(ActorId::Character(Character::Lulu), "fire_fury")
            .unwrap();

        let targets =
            lulu_state.resolve_action_targets(ActorId::Character(Character::Lulu), &fire_fury, &[]);

        assert_eq!(targets.len(), 1);
        let positions = lulu_state.rng.current_positions();
        assert_eq!(positions[25], 16);
        assert_eq!(positions[16], 15);
        assert_eq!(positions[5], 16);

        let mut lulu_six_hit_state = SimulationState::new(1);
        lulu_six_hit_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(1),
                Some("worker".to_string()),
                10,
                false,
                1_000,
            ));
        lulu_six_hit_state
            .monsters
            .push(BattleActor::monster_with_key(
                MonsterSlot(2),
                Some("worker_2".to_string()),
                10,
                false,
                1_000,
            ));
        let fire_fury = lulu_six_hit_state
            .action_data_for_actor(ActorId::Character(Character::Lulu), "fire_fury")
            .unwrap();

        let targets = lulu_six_hit_state.resolve_action_targets(
            ActorId::Character(Character::Lulu),
            &fire_fury,
            &[String::from("6")],
        );

        assert_eq!(targets.len(), 6);
        let positions = lulu_six_hit_state.rng.current_positions();
        assert_eq!(positions[25], 16);
        assert_eq!(positions[16], 10);
        assert_eq!(positions[5], 16);

        let mut explicit_state = SimulationState::new(1);
        explicit_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let spiral_cut = explicit_state
            .action_data_for_actor(ActorId::Character(Character::Tidus), "spiral_cut")
            .unwrap();

        assert_eq!(
            explicit_state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &spiral_cut,
                &[String::from("m1")]
            ),
            vec![ActorId::Monster(MonsterSlot(1))]
        );
        assert_eq!(explicit_state.rng.current_positions()[20], 2);
    }

    #[test]
    fn stat_ranked_character_targets_use_actor_data() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 300;
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_hp = 900;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .current_hp = 100;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_mp = 10;
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_mp = 20;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .current_mp = 80;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .combat_stats
            .strength = 40;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .combat_stats
            .strength = 80;
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .combat_stats
            .magic_defense = 5;
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .combat_stats
            .magic_defense = 15;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .combat_stats
            .magic_defense = 20;

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::HighestHpCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::HighestMpCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Auron)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LowestHpCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Auron)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::HighestStrengthCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Auron)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::LowestMagicDefenseCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
        assert_eq!(state.rng.current_positions()[4], 5);
    }

    #[test]
    fn stat_ranked_character_target_ties_use_rng4() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna, Character::Auron];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 500;
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_hp = 500;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .current_hp = 200;
        let mut expected_rng = FfxRngTracker::new(1);
        expected_rng.advance_rng(4);
        let tied_targets = [
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Yuna),
        ];
        let expected = tied_targets[expected_rng.advance_rng(4) as usize % tied_targets.len()];

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::HighestHpCharacter),
                &[]
            ),
            vec![expected]
        );
        assert_eq!(state.rng.current_positions()[4], 2);
    }

    #[test]
    fn stat_ranked_character_targets_fall_back_to_unknown_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        for character in [Character::Tidus, Character::Yuna] {
            let actor = state.actor_mut(ActorId::Character(character)).unwrap();
            actor.current_hp = 0;
            actor.set_status(Status::Death, 254);
        }

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::HighestHpCharacter),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
        assert_eq!(state.rng.current_positions()[4], 0);
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

        state.monsters.clear();
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Rikku),
                action_data.as_ref().unwrap(),
                &[String::from("monsters")]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );

        state.party.clear();
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Rikku),
                action_data.as_ref().unwrap(),
                &[String::from("party")]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn explicit_party_targets_filter_and_sort_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron, Character::Yuna, Character::Tidus];
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .set_status(Status::Death, 254);
        let action = test_action(ActionTarget::EitherParty);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &action,
                &[String::from("party")]
            ),
            vec![
                ActorId::Character(Character::Tidus),
                ActorId::Character(Character::Auron),
            ]
        );
    }

    #[test]
    fn party_and_random_character_actions_accept_temporary_monster_name_overrides_like_python() {
        for target in [ActionTarget::CharactersParty, ActionTarget::RandomCharacter] {
            let mut state = SimulationState::new(1);
            let action = test_action(target);
            let temporary_actor =
                state.prepare_temporary_explicit_target(&action, &[String::from("piranha")]);

            assert!(temporary_actor.is_some());
            assert_eq!(
                state.explicit_action_targets(&action, &[String::from("piranha")]),
                temporary_actor.into_iter().collect::<Vec<_>>()
            );
            assert!(state
                .temporary_monsters
                .iter()
                .all(|actor| actor.temporary && actor.display_slot == Some(MonsterSlot(1))));
        }
    }

    #[test]
    fn party_and_random_character_actions_ignore_invalid_explicit_names_like_python() {
        for optional_target in ["definitely_not_real", "m1"] {
            let mut party_state = SimulationState::new(1);
            party_state.party = vec![Character::Tidus, Character::Wakka];
            let party_action = test_action(ActionTarget::CharactersParty);

            assert_eq!(
                party_state.resolve_action_targets(
                    ActorId::Character(Character::Yuna),
                    &party_action,
                    &[String::from(optional_target)]
                ),
                vec![
                    ActorId::Character(Character::Tidus),
                    ActorId::Character(Character::Wakka)
                ]
            );
            assert!(party_state.temporary_monsters.is_empty());

            let mut random_state = SimulationState::new(1);
            random_state.party = vec![Character::Tidus, Character::Wakka];
            let random_action = test_action(ActionTarget::RandomCharacter);
            let targets = random_state.resolve_action_targets(
                ActorId::Character(Character::Yuna),
                &random_action,
                &[String::from(optional_target)],
            );

            assert_eq!(targets.len(), 1);
            assert!(matches!(
                targets[0],
                ActorId::Character(Character::Tidus | Character::Wakka)
            ));
            assert!(random_state.temporary_monsters.is_empty());
        }
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
            .contains("Equipment: Tidus | Weapon | Longsword [] -> Sonic Steel [First Strike]"));
        assert!(output.text.contains("Ti[0]"));
    }

    #[test]
    fn default_character_equipment_abilities_are_loaded() {
        let state = SimulationState::new(1);
        let auron = state.character_actor(Character::Auron).unwrap();
        assert!(auron.has_auto_ability(AutoAbility::Piercing));
    }

    #[test]
    fn magus_fallback_stats_match_python_first_pass_defaults() {
        let state = SimulationState::new(1);
        let cindy = state.character_actor(Character::Cindy).unwrap();
        let sandy = state.character_actor(Character::Sandy).unwrap();
        let mindy = state.character_actor(Character::Mindy).unwrap();

        assert_eq!((cindy.max_hp, cindy.max_mp, cindy.agility), (2_190, 46, 10));
        assert_eq!(
            (
                cindy.combat_stats.strength,
                cindy.combat_stats.defense,
                cindy.combat_stats.magic,
                cindy.combat_stats.magic_defense,
                cindy.combat_stats.luck,
                cindy.combat_stats.evasion,
                cindy.combat_stats.accuracy,
            ),
            (28, 32, 21, 28, 17, 20, 11)
        );
        assert_eq!((sandy.max_hp, sandy.max_mp, sandy.agility), (1_790, 35, 10));
        assert_eq!(
            (
                sandy.combat_stats.strength,
                sandy.combat_stats.defense,
                sandy.combat_stats.magic,
                sandy.combat_stats.magic_defense,
                sandy.combat_stats.luck,
                sandy.combat_stats.evasion,
                sandy.combat_stats.accuracy,
            ),
            (42, 26, 24, 28, 17, 17, 13)
        );
        assert_eq!((mindy.max_hp, mindy.max_mp, mindy.agility), (1_237, 58, 12));
        assert_eq!(
            (
                mindy.combat_stats.strength,
                mindy.combat_stats.defense,
                mindy.combat_stats.magic,
                mindy.combat_stats.magic_defense,
                mindy.combat_stats.luck,
                mindy.combat_stats.evasion,
                mindy.combat_stats.accuracy,
            ),
            (23, 24, 28, 28, 17, 23, 12)
        );
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
    fn weapon_statuses_merge_with_action_statuses_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.monsters[0].status_resistances.insert(Status::Dark, 0);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::Darktouch);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.uses_weapon_properties = true;
        action.status_applications.push(ActionStatus {
            status: Status::Dark,
            chance: 100,
            stacks: 3,
            ignores_resistance: false,
        });

        let results = state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        assert_eq!(state.rng.current_positions()[52], 1);
        assert_eq!(
            results[0]
                .statuses
                .iter()
                .filter(|(status, _)| *status == Status::Dark)
                .count(),
            1
        );
        assert_eq!(state.monsters[0].status_stacks.get(&Status::Dark), Some(&3));
    }

    #[test]
    fn weapon_statuses_stop_after_petrify_lands() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.weapon_abilities.insert(AutoAbility::Stonestrike);
        tidus.weapon_abilities.insert(AutoAbility::Silencestrike);
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.uses_weapon_properties = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("yuna")],
        );

        let yuna = state.actor(ActorId::Character(Character::Yuna)).unwrap();
        assert!(yuna.statuses.contains(&Status::Petrify));
        assert_eq!(yuna.statuses.len(), 1);
    }

    #[test]
    fn armor_status_ward_and_proof_update_resistances() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "2".to_string(),
                "deathproof".to_string(),
                "sleep_ward".to_string(),
            ],
        );
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();

        assert_eq!(tidus.status_resistances.get(&Status::Death), Some(&255));
        assert_eq!(tidus.status_resistances.get(&Status::Sleep), Some(&50));

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Death,
                chance: 100,
                stacks: 254,
                ignores_resistance: false,
            },
        );

        assert!(!state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::Death));

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Sleep,
                chance: 50,
                stacks: 3,
                ignores_resistance: false,
            },
        );

        assert!(!state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::Sleep));

        state.change_equipment("armor", &["tidus".to_string(), "0".to_string()]);
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(!tidus.status_resistances.contains_key(&Status::Death));
        assert!(!tidus.status_resistances.contains_key(&Status::Sleep));
        assert_eq!(tidus.status_resistances.get(&Status::Threaten), Some(&255));
    }

    #[test]
    fn overdrive_x2_status_applies_without_status_rng() {
        let mut state = SimulationState::new(1);

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::OverdriveX2,
                chance: 50,
                stacks: 254,
                ignores_resistance: false,
            },
        );

        assert_eq!(state.rng.current_positions()[52], 0);
        assert!(state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::OverdriveX2));
        assert_eq!("overdrive_x_2".parse::<Status>(), Ok(Status::OverdriveX2));
    }

    #[test]
    fn ribbon_abilities_update_status_immunities() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "armor",
            &["tidus".to_string(), "1".to_string(), "ribbon".to_string()],
        );
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.status_resistances.get(&Status::Poison), Some(&255));
        assert_ne!(tidus.status_resistances.get(&Status::Death), Some(&255));

        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "aeon_ribbon".to_string(),
            ],
        );
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.status_resistances.get(&Status::Death), Some(&255));
        assert_eq!(
            tidus.status_resistances.get(&Status::PowerBreak),
            Some(&255)
        );

        state.change_equipment("armor", &["tidus".to_string(), "0".to_string()]);
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(!tidus.status_resistances.contains_key(&Status::Poison));
        assert!(!tidus.status_resistances.contains_key(&Status::Death));
    }

    #[test]
    fn sos_auto_statuses_apply_only_at_low_hp() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "2".to_string(),
                "sos_haste".to_string(),
                "sos_nulblaze".to_string(),
            ],
        );
        assert!(!state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::Haste));

        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 100;
        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "2".to_string(),
                "sos_haste".to_string(),
                "sos_nulblaze".to_string(),
            ],
        );
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(tidus.statuses.contains(&Status::Haste));
        assert!(tidus.statuses.contains(&Status::NulBlaze));
    }

    #[test]
    fn permanent_auto_haste_blocks_slow_application() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "auto_haste".to_string(),
            ],
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .status_stack(Status::Haste),
            255
        );

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Slow,
                chance: 254,
                stacks: 4,
                ignores_resistance: false,
            },
        );

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(tidus.statuses.contains(&Status::Haste));
        assert!(!tidus.statuses.contains(&Status::Slow));
    }

    #[test]
    fn remove_status_effects_skip_permanent_status_stacks() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Haste, 255);

        state.remove_status_from_actor(ActorId::Character(Character::Tidus), Status::Haste);

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(tidus.statuses.contains(&Status::Haste));
        assert_eq!(tidus.status_stack(Status::Haste), 255);
    }

    #[test]
    fn remove_status_actions_do_not_remove_status_flags_like_python() {
        let mut state = SimulationState::new(1);
        {
            let tidus = state
                .actor_mut(ActorId::Character(Character::Tidus))
                .unwrap();
            tidus.set_status(Status::Zombie, 254);
            tidus.set_status(Status::Curse, 254);
        }
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Yuna), "holy_water");

        state.apply_action_effects(
            ActorId::Character(Character::Yuna),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(!tidus.statuses.contains(&Status::Zombie));
        assert!(tidus.statuses.contains(&Status::Curse));
    }

    #[test]
    fn equipment_hp_and_mp_bonuses_update_effective_limits() {
        let mut state = SimulationState::new(1);
        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "2".to_string(),
                "hp_30".to_string(),
                "mp_20".to_string(),
            ],
        );
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.effective_max_hp(), 676);
        assert_eq!(tidus.effective_max_mp(), 14);
        assert_eq!(tidus.current_hp, 520);
        assert_eq!(tidus.current_mp, 12);

        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 676;
        state.change_equipment("armor", &["tidus".to_string(), "0".to_string()]);
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.effective_max_hp(), 520);
        assert_eq!(tidus.effective_max_mp(), 12);
        assert_eq!(tidus.current_hp, 520);
    }

    #[test]
    fn break_hp_and_mp_limit_control_effective_caps() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .max_hp = 10_000;
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .effective_max_hp(),
            9_999
        );

        state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "break_hp_limit".to_string(),
            ],
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .effective_max_hp(),
            10_000
        );

        state.change_equipment(
            "armor",
            &["seymour".to_string(), "1".to_string(), "mp_30".to_string()],
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Seymour))
                .unwrap()
                .effective_max_mp(),
            999
        );
        state.change_equipment(
            "armor",
            &[
                "seymour".to_string(),
                "2".to_string(),
                "mp_30".to_string(),
                "break_mp_limit".to_string(),
            ],
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Seymour))
                .unwrap()
                .effective_max_mp(),
            1298
        );
    }

    #[test]
    fn action_statuses_preserve_duration_stacks() {
        let mut state = SimulationState::new(1);

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Sleep,
                chance: 254,
                stacks: 4,
                ignores_resistance: false,
            },
        );

        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .status_stacks
                .get(&Status::Sleep),
            Some(&4)
        );
    }

    #[test]
    fn death_status_application_preserves_existing_statuses_like_python() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.set_status(Status::Poison, 254);
        tidus.buffs.insert(Buff::Cheer, 2);

        state.apply_action_status_to_actor(
            ActorId::Character(Character::Tidus),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Death,
                chance: 254,
                stacks: 254,
                ignores_resistance: false,
            },
        );

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.current_hp, 0);
        assert!(tidus.statuses.contains(&Status::Death));
        assert!(tidus.statuses.contains(&Status::Poison));
        assert_eq!(tidus.buffs.get(&Buff::Cheer), Some(&2));
    }

    #[test]
    fn monster_status_applications_use_monster_status_rng_lane() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));

        state.apply_action_status_to_actor(
            ActorId::Monster(MonsterSlot(1)),
            ActorId::Character(Character::Tidus),
            &ActionStatus {
                status: Status::Dark,
                chance: 1,
                stacks: 2,
                ignores_resistance: false,
            },
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[52], 0);
        assert_eq!(positions[60], 1);
    }

    #[test]
    fn character_hit_checks_use_character_hit_rng_lane_and_gate_effects() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.hit_chance_formula = HitChanceFormula::UseActionAccuracy;
        action.accuracy = 0;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[36], 1);
        assert_eq!(positions[44], 0);
        assert_eq!(state.monsters[0].current_hp, 1_000);
    }

    #[test]
    fn monster_hit_checks_use_monster_hit_rng_lane() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.hit_chance_formula = HitChanceFormula::UseActionAccuracy;
        action.accuracy = 0;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(ActorId::Monster(MonsterSlot(1)), Some(&action), &[]);

        let positions = state.rng.current_positions();
        assert_eq!(positions[36], 0);
        assert_eq!(positions[44], 1);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .current_hp,
            520
        );
    }

    #[test]
    fn sleep_and_petrify_targets_skip_hit_rng_like_python() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Sleep, 2);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.hit_chance_formula = HitChanceFormula::UseActionAccuracy;
        action.accuracy = 0;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[36], 0);
        assert!(state.monsters[0].current_hp < 1_000);
    }

    #[test]
    fn damage_rolls_use_character_and_monster_damage_rng_lanes() {
        let mut character_state = SimulationState::new(1);
        character_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 1;
        action.damages_hp = true;

        character_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = character_state.rng.current_positions();
        assert_eq!(positions[20], 1);
        assert_eq!(positions[28], 0);

        let mut monster_state = SimulationState::new(1);
        monster_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut monster_action = test_action(ActionTarget::SingleCharacter);
        monster_action.damage_formula = DamageFormula::FixedNoVariance;
        monster_action.damage_type = DamageType::Physical;
        monster_action.base_damage = 1;
        monster_action.damages_hp = true;

        monster_state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&monster_action),
            &[String::from("tidus")],
        );

        let positions = monster_state.rng.current_positions();
        assert_eq!(positions[20], 0);
        assert_eq!(positions[28], 1);
    }

    #[test]
    fn counter_actions_target_last_actor_without_spending_ctb() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state.last_actor = Some(ActorId::Monster(MonsterSlot(1)));
        let tidus_ctb = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .ctb;

        let output = state.apply_character_action(Character::Tidus, "counter", &[]);

        assert!(output.contains("Tidus -> Counter [0]"));
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .ctb,
            tidus_ctb
        );
        assert_eq!(state.last_actor, Some(ActorId::Monster(MonsterSlot(1))));
        assert_eq!(state.last_targets, vec![ActorId::Monster(MonsterSlot(1))]);
        assert!(state.monsters[0].current_hp < 1_000);
    }

    #[test]
    fn counter_actions_default_to_tidus_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));

        let output = state.apply_character_action(Character::Tidus, "counter", &[]);

        assert!(output.contains("Tidus -> Counter [0]"), "{output}");
        assert!(!output.contains("Does Nothing"), "{output}");
        assert_eq!(
            state.last_targets,
            vec![ActorId::Character(Character::Tidus)]
        );
    }

    #[test]
    fn physical_monster_hits_trigger_equipped_counterattack() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::Counterattack);
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.combat_stats.accuracy = 255;
        auron.combat_stats.luck = 255;
        auron.combat_stats.strength = 80;

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(output.contains("Worker (M1) -> Attack ["), "{output}");
        assert!(output.contains("Auron -> Counter [0]:"), "{output}");
        assert!(state.monsters[0].current_hp < 1_000);
    }

    #[test]
    fn monster_hp_damage_triggers_equipped_auto_potion() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        monster.combat_stats.strength = 80;
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.break_hp_limit = true;
        auron.max_hp = 99_999;
        auron.current_hp = auron.effective_max_hp();
        auron.armor_abilities.insert(AutoAbility::AutoPotion);

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(output.contains("Worker (M1) -> Attack ["), "{output}");
        assert!(output.contains("Auron -> Auto Potion [0]:"), "{output}");
        assert_eq!(state.item_inventory[0], (Some("Potion".to_string()), 9));
    }

    #[test]
    fn automatic_auto_potion_requires_inventory_item_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.item_inventory.clear();
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        monster.combat_stats.strength = 80;
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.break_hp_limit = true;
        auron.max_hp = 99_999;
        auron.current_hp = auron.effective_max_hp();
        auron.armor_abilities.insert(AutoAbility::AutoPotion);

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(output.contains("Worker (M1) -> Attack ["), "{output}");
        assert!(!output.contains("Auron -> Auto Potion [0]:"), "{output}");
    }

    #[test]
    fn monster_hits_trigger_equipped_auto_med_for_removable_statuses() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.add_inventory_item("Remedy", 2);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.set_status(Status::Poison, 254);
        auron.armor_abilities.insert(AutoAbility::AutoMed);

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(output.contains("Auron -> Auto Med [0]:"), "{output}");
        assert!(!state
            .actor(ActorId::Character(Character::Auron))
            .unwrap()
            .statuses
            .contains(&Status::Poison));
        assert_eq!(
            state
                .item_inventory
                .iter()
                .find(|(item, _)| item.as_deref() == Some("Remedy"))
                .map(|(_, quantity)| *quantity),
            Some(1)
        );
    }

    #[test]
    fn automatic_auto_med_requires_inventory_item_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.set_status(Status::Poison, 254);
        auron.armor_abilities.insert(AutoAbility::AutoMed);

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(!output.contains("Auron -> Auto Med [0]:"), "{output}");
        assert!(state
            .actor(ActorId::Character(Character::Auron))
            .unwrap()
            .statuses
            .contains(&Status::Poison));
    }

    #[test]
    fn automatic_auto_med_ignores_permanent_statuses_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron];
        state.add_inventory_item("Remedy", 1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let monster = state.actor_mut(ActorId::Monster(MonsterSlot(1))).unwrap();
        monster.combat_stats.accuracy = 255;
        monster.combat_stats.luck = 255;
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.set_status(Status::Poison, 255);
        auron.armor_abilities.insert(AutoAbility::AutoMed);

        let output = state.apply_monster_action(MonsterSlot(1), "attack", &[]);

        assert!(!output.contains("Auron -> Auto Med [0]:"), "{output}");
        assert_eq!(
            state
                .item_inventory
                .iter()
                .find(|(item, _)| item.as_deref() == Some("Remedy"))
                .map(|(_, quantity)| *quantity),
            Some(1)
        );
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Auron))
                .unwrap()
                .status_stack(Status::Poison),
            255
        );
    }

    #[test]
    fn automatic_auto_life_revives_before_auto_phoenix_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Auron, Character::Yuna];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let auron = state
            .actor_mut(ActorId::Character(Character::Auron))
            .unwrap();
        auron.current_hp = 0;
        auron.set_status(Status::Death, 254);
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .armor_abilities
            .insert(AutoAbility::AutoPhoenix);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.auto_life_triggered = true;

        let outputs = state.automatic_reaction_outputs(
            ActorId::Monster(MonsterSlot(1)),
            &test_action(ActionTarget::SingleCharacter),
            &[result],
        );
        let output = outputs.join("\n");

        assert!(output.contains("Auron -> Auto-Life"), "{output}");
        assert!(!output.contains("Yuna -> Auto Phoenix"), "{output}");
        let auron = state.actor(ActorId::Character(Character::Auron)).unwrap();
        assert!(!auron.statuses.contains(&Status::Death));
        assert!(auron.current_hp > 0);
    }

    #[test]
    fn damage_comments_include_misses_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.hit = false;
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damages_hp = true;
        action.damage_formula = DamageFormula::FixedNoVariance;

        assert_eq!(
            super::format_damage_comment("# enemy rolls: ", &[result], &state, Some(&action)),
            Some("# enemy rolls: Auron: Miss".to_string())
        );
    }

    #[test]
    fn damage_comments_skip_non_damaging_misses_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.hit = false;
        let action = test_action(ActionTarget::SingleCharacter);

        assert_eq!(
            super::format_damage_comment("# party rolls: ", &[result], &state, Some(&action)),
            None
        );
    }

    #[test]
    fn damage_comments_skip_no_damage_formula_even_with_damage_flags_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.damage.hp = Some(ActionDamageResult {
            damage_rng: 25,
            damage: 91,
            pool: "HP",
            crit: false,
        });
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damages_hp = true;
        action.damage_formula = DamageFormula::NoDamage;

        assert_eq!(
            super::format_damage_comment("# enemy rolls: ", &[result], &state, Some(&action)),
            None
        );
    }

    #[test]
    fn damage_comments_skip_no_damage_formula_misses_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.hit = false;
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damages_hp = true;
        action.damage_formula = DamageFormula::NoDamage;

        assert_eq!(
            super::format_damage_comment("# enemy rolls: ", &[result], &state, Some(&action)),
            None
        );
    }

    #[test]
    fn action_result_status_tokens_are_grouped_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.statuses.push((Status::Poison, true));
        result.statuses.push((Status::Silence, false));
        result.removed_statuses.push(Status::Dark);
        result.removed_statuses.push(Status::Sleep);

        assert_eq!(
            super::format_action_result(&result, &state, None),
            "Auron -> (No damage) [Poison][Silence Fail] [-Dark][-Sleep]"
        );
    }

    #[test]
    fn action_result_duplicate_statuses_keep_latest_value_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.statuses.push((Status::Poison, false));
        result.statuses.push((Status::Silence, true));
        result.statuses.push((Status::Poison, true));

        assert_eq!(
            super::format_action_result(&result, &state, None),
            "Auron -> (No damage) [Poison][Silence]"
        );
    }

    #[test]
    fn action_result_suppresses_damage_for_no_damage_formula_like_python() {
        let state = SimulationState::new(1);
        let mut result = ActionEffectResult::new(ActorId::Character(Character::Auron));
        result.damage.hp = Some(ActionDamageResult {
            damage_rng: 7,
            damage: 123,
            pool: "HP",
            crit: false,
        });
        result.statuses.push((Status::Poison, true));
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damages_hp = true;
        action.damage_formula = DamageFormula::NoDamage;

        assert_eq!(
            super::format_action_result(&result, &state, Some(&action)),
            "Auron -> (No damage) [Poison]"
        );
    }

    #[test]
    fn self_counter_actions_do_not_spend_ctb() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 100;
        let tidus_ctb = tidus.ctb;

        let output = state.apply_character_action(Character::Tidus, "auto_potion", &[]);

        assert!(output.contains("Tidus -> Auto Potion [0]"));
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.ctb, tidus_ctb);
        assert!(tidus.current_hp > 100);
    }

    #[test]
    fn auto_phoenix_is_counter_single_character_action() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);
        let yuna_ctb = state
            .actor(ActorId::Character(Character::Yuna))
            .unwrap()
            .ctb;

        let output = state.apply_character_action(Character::Yuna, "auto_phoenix", &[]);

        assert!(output.contains("Yuna -> Auto Phoenix [0]"));
        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(!tidus.statuses.contains(&Status::Death));
        assert!(tidus.current_hp > 0);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Yuna))
                .unwrap()
                .ctb,
            yuna_ctb
        );
    }

    #[test]
    fn automatic_auto_phoenix_requires_inventory_item_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        state
            .monsters
            .push(BattleActor::monster(MonsterSlot(1), 10, 1_000));
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .armor_abilities
            .insert(AutoAbility::AutoPhoenix);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);
        let result = ActionEffectResult::new(ActorId::Character(Character::Tidus));

        let outputs = state.automatic_reaction_outputs(
            ActorId::Monster(MonsterSlot(1)),
            &test_action(ActionTarget::SingleCharacter),
            &[result.clone()],
        );

        assert_eq!(outputs.len(), 1);
        assert!(outputs[0].contains("Yuna -> Auto Phoenix [0]"));
        assert_eq!(
            state
                .item_inventory
                .iter()
                .find(|(item, _)| item.as_deref() == Some("Phoenix Down"))
                .map(|(_, quantity)| *quantity),
            Some(2)
        );

        for (item, quantity) in &mut state.item_inventory {
            if item.as_deref() == Some("Phoenix Down") {
                *item = None;
                *quantity = 0;
            }
        }
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);
        let outputs = state.automatic_reaction_outputs(
            ActorId::Monster(MonsterSlot(1)),
            &test_action(ActionTarget::SingleCharacter),
            &[result],
        );
        assert!(outputs.is_empty());
    }

    #[test]
    fn counter_revives_subtract_elapsed_ctb_from_revived_target() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        state.ctb_since_last_action = 7;
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.ctb = 12;
        tidus.set_status(Status::Death, 254);
        let expected_ctb = tidus.base_ctb() * 3 - 7;

        state.apply_character_action(Character::Yuna, "auto_phoenix", &[]);

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert!(!tidus.statuses.contains(&Status::Death));
        assert_eq!(tidus.ctb, expected_ctb);
    }

    #[test]
    fn counter_revive_auto_regen_applies_elapsed_healing_like_python() {
        let mut base_state = SimulationState::new(1);
        base_state.party = vec![Character::Tidus, Character::Yuna];
        base_state.ctb_since_last_action = 8;
        let tidus = base_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.current_hp = 0;
        tidus.set_status(Status::Death, 254);

        let mut regen_state = base_state.clone();
        regen_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .armor_abilities
            .insert(AutoAbility::AutoRegen);

        base_state.apply_character_action(Character::Yuna, "auto_phoenix", &[]);
        regen_state.apply_character_action(Character::Yuna, "auto_phoenix", &[]);

        let base_hp = base_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp;
        let regen_hp = regen_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp;
        assert!(regen_hp > base_hp);
    }

    #[test]
    fn counter_revive_auto_regen_clamps_to_max_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        state.ctb_since_last_action = 400;
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        let max_hp = tidus.effective_max_hp();
        tidus.current_hp = max_hp - 50;
        tidus.set_status(Status::Death, 254);
        tidus.armor_abilities.insert(AutoAbility::AutoRegen);

        state.apply_character_action(Character::Yuna, "auto_phoenix", &[]);

        let tidus = state.actor(ActorId::Character(Character::Tidus)).unwrap();
        assert_eq!(tidus.current_hp, max_hp);
    }

    #[test]
    fn multi_hit_damage_uses_separate_damage_rolls_per_hit() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::Fixed;
        action.damage_type = DamageType::Physical;
        action.base_damage = 2;
        action.damages_hp = true;
        action.n_of_hits = 2;

        let user = state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .clone();
        let target = state
            .actor(ActorId::Monster(MonsterSlot(1)))
            .unwrap()
            .clone();
        let mut expected_rng = FfxRngTracker::new(1);
        let first_damage = calculate_action_damage(
            &user,
            &target,
            &action,
            expected_rng.advance_rng(20) & 31,
            false,
            0,
            DamagePool::Hp,
        );
        let second_damage = calculate_action_damage(
            &user,
            &target,
            &action,
            expected_rng.advance_rng(20) & 31,
            false,
            0,
            DamagePool::Hp,
        );

        state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);

        let positions = state.rng.current_positions();
        assert_eq!(positions[20], 2);
        assert_eq!(
            state.monsters[0].current_hp,
            2_000 - first_damage - second_damage
        );
    }

    #[test]
    fn timed_overdrives_scale_damage_with_capped_remaining_time() {
        let user = BattleActor::character(Character::Tidus, 0, 10, 520, 12);
        let target = BattleActor::monster(MonsterSlot(1), 10, 20_000);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 100;
        action.damages_hp = true;
        action.overdrive_user = Some(Character::Tidus);
        action.overdrive_index = 2;

        assert_eq!(
            calculate_action_damage(&user, &target, &action, 0, false, 0, DamagePool::Hp),
            5_000
        );
        assert_eq!(
            calculate_action_damage(&user, &target, &action, 0, false, 3_000, DamagePool::Hp),
            7_500
        );
        assert_eq!(
            calculate_action_damage(&user, &target, &action, 0, false, 6_000, DamagePool::Hp),
            7_500
        );
    }

    #[test]
    fn crit_actions_consume_second_damage_rng_roll_and_apply_bonus_luck() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 10;
        action.damages_hp = true;
        action.can_crit = true;
        action.bonus_crit = 255;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[20], 2);
        assert_eq!(state.monsters[0].current_hp, 1_000);
    }

    #[test]
    fn petrified_monsters_consume_damage_rng_then_shatter() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Petrify, 254);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Physical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[20], 1);
        assert_eq!(positions[52], 1);
        assert_eq!(state.monsters[0].current_hp, 0);
        assert!(state.monsters[0].statuses.contains(&Status::Death));
        assert!(state.monsters[0].statuses.contains(&Status::Eject));
    }

    #[test]
    fn petrified_monsters_shatter_after_successful_hits() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.status_applications.push(ActionStatus {
            status: Status::Petrify,
            chance: 254,
            stacks: 254,
            ignores_resistance: false,
        });

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[52], 2);
        assert_eq!(state.monsters[0].current_hp, 0);
        assert!(state.monsters[0].statuses.contains(&Status::Death));
        assert!(state.monsters[0].statuses.contains(&Status::Eject));
        assert!(!state.monsters[0].statuses.contains(&Status::Petrify));
    }

    #[test]
    fn character_shatter_chance_uses_status_rng() {
        let mut failed_state = SimulationState::new(1);
        failed_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut failed_action = test_action(ActionTarget::SingleCharacter);
        failed_action.status_applications.push(ActionStatus {
            status: Status::Petrify,
            chance: 254,
            stacks: 254,
            ignores_resistance: false,
        });
        failed_action.shatter_chance = 0;

        failed_state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&failed_action),
            &[String::from("tidus")],
        );

        let tidus = failed_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap();
        assert!(tidus.statuses.contains(&Status::Petrify));
        assert!(!tidus.statuses.contains(&Status::Eject));
        assert_eq!(failed_state.rng.current_positions()[60], 2);

        let mut success_state = SimulationState::new(1);
        success_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut success_action = failed_action;
        success_action.shatter_chance = 255;
        success_state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&success_action),
            &[String::from("tidus")],
        );

        let tidus = success_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap();
        assert_eq!(tidus.current_hp, 0);
        assert!(tidus.statuses.contains(&Status::Death));
        assert!(tidus.statuses.contains(&Status::Eject));
    }

    #[test]
    fn reflected_actions_retarget_to_opposite_party_without_rng_for_single_target() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Yuna];
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Reflect, 254);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.affected_by_reflect = true;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[6], 0);
        assert_eq!(state.monsters[0].current_hp, 1_000);
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Yuna))
                .unwrap()
                .current_hp,
            425
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &test_action(ActionTarget::LastTarget),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
    }

    #[test]
    fn reflected_actions_can_retarget_to_petrified_characters_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Petrify, 254);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Reflect, 254);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.affected_by_reflect = true;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Yuna),
            Some(&action),
            &[String::from("m1")],
        );

        assert_eq!(
            state.last_targets,
            vec![ActorId::Character(Character::Tidus)]
        );
        assert_eq!(state.rng.current_positions()[6], 0);
    }

    #[test]
    fn reflected_actions_can_retarget_to_petrified_monsters_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Reflect, 254);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Petrify, 254);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.affected_by_reflect = true;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            Some(&action),
            &[String::from("tidus")],
        );

        assert_eq!(state.last_targets, vec![ActorId::Monster(MonsterSlot(1))]);
        assert_eq!(state.rng.current_positions()[6], 0);
    }

    #[test]
    fn reflected_actions_use_rng6_for_multiple_reflect_targets() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Yuna];
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.set_status(Status::Reflect, 254);
        state.monsters.push(monster);
        let mut action = test_action(ActionTarget::SingleMonster);
        action.affected_by_reflect = true;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 1;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );

        let positions = state.rng.current_positions();
        assert_eq!(positions[6], 1);
        assert_eq!(state.monsters[0].current_hp, 1_000);
    }

    #[test]
    fn reflected_elemental_actions_check_nul_status_on_original_target_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.set_status(Status::Reflect, 254);
        tidus.set_status(Status::NulShock, 1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.affected_by_reflect = true;
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 100;
        action.damages_hp = true;
        action.elements = vec![Element::Thunder];

        let results = state.apply_action_effects(
            ActorId::Character(Character::Yuna),
            Some(&action),
            &[String::from("tidus")],
        );

        assert_eq!(results.len(), 1);
        assert!(!results[0].hit);
        assert_eq!(
            results[0].reflected_from,
            Some(ActorId::Character(Character::Tidus))
        );
        assert_eq!(results[0].target, ActorId::Monster(MonsterSlot(1)));
        assert_eq!(state.monsters[0].current_hp, 1_000);
        assert!(!state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::NulShock));
    }

    #[test]
    fn provoke_actions_record_per_target_provoker() {
        let mut state = SimulationState::new(1);
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
        let mut provoke = test_action(ActionTarget::SingleMonster);
        provoke.key = "provoke".to_string();

        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&provoke),
            &[String::from("m2")],
        );

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(2)),
                &test_action(ActionTarget::Provoker),
                &[]
            ),
            vec![ActorId::Character(Character::Tidus)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Provoker),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn fixed_data_targets_use_existing_actors_even_when_unavailable_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut dead_monster = BattleActor::monster_with_key(
            MonsterSlot(2),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        dead_monster.current_hp = 0;
        dead_monster.set_status(Status::Death, 254);
        state.monsters.push(dead_monster);

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Character(Character::Tidus),
                &test_action(ActionTarget::Character(Character::Yuna)),
                &[]
            ),
            vec![ActorId::Character(Character::Yuna)]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Monster(MonsterSlot(2))),
                &[]
            ),
            vec![ActorId::Monster(MonsterSlot(2))]
        );
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Monster(MonsterSlot(3))),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn fixed_character_monster_targets_filter_unavailable_characters_like_python() {
        let mut state = SimulationState::new(1);
        state.party = vec![Character::Tidus, Character::Auron];
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("condor_2".to_string()),
            10,
            false,
            1_000,
        ));

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Character(Character::Wakka)),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );

        state.party = vec![Character::Tidus, Character::Wakka];
        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Character(Character::Wakka)),
                &[]
            ),
            vec![ActorId::Character(Character::Wakka)]
        );
    }

    #[test]
    fn counter_target_without_last_actor_uses_unknown_like_python() {
        let mut state = SimulationState::new(1);
        state.last_actor = None;

        assert_eq!(
            state.resolve_action_targets(
                ActorId::Monster(MonsterSlot(1)),
                &test_action(ActionTarget::Counter),
                &[]
            ),
            vec![ActorId::Character(Character::Unknown)]
        );
    }

    #[test]
    fn duration_statuses_tick_down_at_end_of_turn() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Dark, 2);

        state.process_end_of_turn(ActorId::Character(Character::Tidus));
        assert_eq!(
            state
                .actor(ActorId::Character(Character::Tidus))
                .unwrap()
                .status_stack(Status::Dark),
            1
        );

        state.process_end_of_turn(ActorId::Character(Character::Tidus));
        assert!(!state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .contains(&Status::Dark));
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
    fn break_damage_limit_controls_damage_cap() {
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.base_damage = 400;
        action.damages_hp = true;

        let mut capped_state = SimulationState::new(1);
        capped_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            50_000,
        ));
        capped_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );
        assert_eq!(capped_state.monsters[0].current_hp, 40_001);

        let mut break_state = SimulationState::new(1);
        break_state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "break_damage_limit".to_string(),
            ],
        );
        break_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            50_000,
        ));
        break_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );
        assert_eq!(break_state.monsters[0].current_hp, 30_000);

        action.never_break_damage_limit = true;
        break_state.monsters[0].current_hp = 50_000;
        break_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );
        assert_eq!(break_state.monsters[0].current_hp, 40_001);
    }

    #[test]
    fn critical_status_forces_crits_for_crit_actions() {
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.base_damage = 80;
        action.damages_hp = true;
        action.can_crit = true;

        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .set_status(Status::Critical, 254);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            50_000,
        ));
        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );
        assert_eq!(state.monsters[0].current_hp, 42_000);

        action.can_crit = false;
        state.monsters[0].current_hp = 50_000;
        state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&action),
            &[String::from("m1")],
        );
        assert_eq!(state.monsters[0].current_hp, 46_000);
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
    fn alchemy_doubles_item_healing_actions() {
        let mut normal_state = SimulationState::new(1);
        normal_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 100;
        let action_data =
            normal_state.action_data_for_actor(ActorId::Character(Character::Rikku), "potion");
        normal_state.apply_action_effects(
            ActorId::Character(Character::Rikku),
            action_data.as_ref(),
            &[String::from("tidus")],
        );
        let normal_hp = normal_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp;

        let mut alchemy_state = SimulationState::new(1);
        alchemy_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp = 100;
        alchemy_state
            .actor_mut(ActorId::Character(Character::Rikku))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::Alchemy);
        let action_data =
            alchemy_state.action_data_for_actor(ActorId::Character(Character::Rikku), "potion");
        alchemy_state.apply_action_effects(
            ActorId::Character(Character::Rikku),
            action_data.as_ref(),
            &[String::from("tidus")],
        );
        let alchemy_hp = alchemy_state
            .actor(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_hp;

        assert!(alchemy_hp > normal_hp);
        assert_eq!(alchemy_hp - 100, (normal_hp - 100) * 2);
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
    fn drain_actions_clamp_recovery_to_max_like_python() {
        let mut state = SimulationState::new(1);
        let mut monster = BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        );
        monster.current_hp = monster.max_hp;
        let max_hp = monster.effective_max_hp();
        state.monsters.push(monster);
        let action_data =
            state.action_data_for_actor(ActorId::Monster(MonsterSlot(1)), "drain_touch");

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        assert_eq!(state.monsters[0].current_hp, max_hp);
    }

    #[test]
    fn mp_damage_and_drains_update_actor_mp() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_mp = 0;
        let user = state.character_actor(Character::Yuna).unwrap().clone();
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damage_formula = DamageFormula::PercentageTotalMp;
        action.base_damage = 8;
        action.damages_mp = true;
        action.drains = true;

        state.apply_action_damage(&user, ActorId::Character(Character::Tidus), &action, 0);

        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_mp,
            6
        );
        assert_eq!(
            state.character_actor(Character::Yuna).unwrap().current_mp,
            6
        );
    }

    #[test]
    fn mp_drain_is_limited_by_targets_current_mp() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_mp = 2;
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_mp = 0;
        let user = state.character_actor(Character::Yuna).unwrap().clone();
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.base_damage = 10;
        action.damages_mp = true;
        action.drains = true;

        let damage =
            state.apply_action_damage(&user, ActorId::Character(Character::Tidus), &action, 0);

        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_mp,
            0
        );
        assert_eq!(
            state.character_actor(Character::Yuna).unwrap().current_mp,
            2
        );
        assert_eq!(damage.mp.unwrap().damage, 2);
    }

    #[test]
    fn mp_drain_recovery_clamps_to_max_like_python() {
        let mut state = SimulationState::new(1);
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_mp = 10;
        let yuna_max_mp = state
            .character_actor(Character::Yuna)
            .unwrap()
            .effective_max_mp();
        state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap()
            .current_mp = yuna_max_mp - 1;
        let user = state.character_actor(Character::Yuna).unwrap().clone();
        let mut action = test_action(ActionTarget::SingleCharacter);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.base_damage = 10;
        action.damages_mp = true;
        action.drains = true;

        state.apply_action_damage(&user, ActorId::Character(Character::Tidus), &action, 0);

        assert_eq!(
            state.character_actor(Character::Yuna).unwrap().current_mp,
            yuna_max_mp
        );
    }

    #[test]
    fn gil_damage_formula_uses_numeric_gil_argument_like_python() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::Gil;
        action.damages_hp = true;

        state.apply_action_effects(
            ActorId::Character(Character::Rikku),
            Some(&action),
            &[String::from("m1"), String::from("2500")],
        );

        assert_eq!(state.monsters[0].current_hp, 750);
    }

    #[test]
    fn action_mp_costs_update_user_mp() {
        let mut state = SimulationState::new(1);
        let mut action = test_action(ActionTarget::SelfTarget);
        action.mp_cost = 5;

        state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);
        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_mp,
            7
        );

        {
            let tidus = state
                .actor_mut(ActorId::Character(Character::Tidus))
                .unwrap();
            tidus.current_mp = 12;
            tidus.set_status(Status::Mp0, 1);
        }
        state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);
        assert_eq!(
            state.character_actor(Character::Tidus).unwrap().current_mp,
            12
        );

        let mut half_state = SimulationState::new(1);
        half_state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "half_mp_cost".to_string(),
            ],
        );
        half_state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);
        assert_eq!(
            half_state
                .character_actor(Character::Tidus)
                .unwrap()
                .current_mp,
            10
        );

        let mut one_state = SimulationState::new(1);
        one_state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "one_mp_cost".to_string(),
            ],
        );
        one_state.change_equipment(
            "armor",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "half_mp_cost".to_string(),
            ],
        );
        one_state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);
        assert_eq!(
            one_state
                .character_actor(Character::Tidus)
                .unwrap()
                .current_mp,
            11
        );

        let mut boosted_action = action.clone();
        boosted_action.uses_magic_booster = true;
        let mut booster_state = SimulationState::new(1);
        booster_state.change_equipment(
            "weapon",
            &[
                "tidus".to_string(),
                "1".to_string(),
                "magic_booster".to_string(),
            ],
        );
        booster_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&boosted_action),
            &[],
        );
        assert_eq!(
            booster_state
                .character_actor(Character::Tidus)
                .unwrap()
                .current_mp,
            2
        );

        let mut low_mp_state = SimulationState::new(1);
        low_mp_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .current_mp = 2;
        low_mp_state.apply_action_effects(ActorId::Character(Character::Tidus), Some(&action), &[]);
        assert_eq!(
            low_mp_state
                .character_actor(Character::Tidus)
                .unwrap()
                .current_mp,
            0
        );
    }

    #[test]
    fn destroying_actions_eject_the_user() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let action_data =
            state.action_data_for_actor(ActorId::Monster(MonsterSlot(1)), "self-destruct");

        state.apply_action_effects(
            ActorId::Monster(MonsterSlot(1)),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        assert!(state.monsters[0].statuses.contains(&Status::Eject));
    }

    #[test]
    fn turn_start_removes_temporary_statuses() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .statuses
            .insert(Status::Defend);

        state.apply_character_action(Character::Tidus, "attack", &[String::from("m1")]);

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert!(!tidus.statuses.contains(&Status::Defend));
    }

    #[test]
    fn turn_end_applies_poison_and_regen_upkeep() {
        let mut state = SimulationState::new(1);
        state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            1_000,
        ));
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.statuses.insert(Status::Poison);
        let poison_damage = tidus.max_hp / 4;
        let tidus_hp = tidus.current_hp;
        let yuna = state
            .actor_mut(ActorId::Character(Character::Yuna))
            .unwrap();
        yuna.statuses.insert(Status::Regen);
        yuna.current_hp -= 200;
        let yuna_hp = yuna.current_hp;

        state.apply_character_action(Character::Tidus, "attack", &[String::from("m1")]);

        let tidus = state.character_actor(Character::Tidus).unwrap();
        let yuna = state.character_actor(Character::Yuna).unwrap();
        assert_eq!(tidus.current_hp, tidus_hp - poison_damage);
        assert!(yuna.current_hp > yuna_hp);
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
    fn element_command_uses_python_loose_monster_slot_parser() {
        let output = simulate(
            1,
            &[
                "spawn piranha 1 0".to_string(),
                "spawn worker 2 0".to_string(),
                "element m10 fire immune".to_string(),
                "element m20 ice immune".to_string(),
                "element m0 thunder immune".to_string(),
                "element x1 water immune".to_string(),
                "element m9 holy weak".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "spawn piranha 1 0\n",
                "spawn worker 2 0\n",
                "Elemental affinity to Fire of Piranha (M1) changed to Immune\n",
                "Elemental affinity to Ice of Worker (M2) changed to Immune\n",
                "Elemental affinity to Thunder of Worker (M2) changed to Immune\n",
                "Elemental affinity to Water of Piranha (M1) changed to Immune\n",
                "Error: No monster in slot 9",
            )
        );
        assert_eq!(output.unsupported_count, 0);
    }

    #[test]
    fn element_command_reports_enum_errors_like_python() {
        let output = simulate(
            1,
            &[
                "spawn tanker 1".to_string(),
                "element m1 nope weak".to_string(),
                "element m1 fire nope".to_string(),
                "element m1 holy weak".to_string(),
            ],
        );

        assert_eq!(
            output.text,
            concat!(
                "spawn tanker 1\n",
                "Error: element can only be one of these values: fire, ice, thunder, water, holy\n",
                "Error: affinity can only be one of these values: absorbs, immune, resists, weak, neutral\n",
                "Elemental affinity to Holy of Tanker (M1) changed to Weak",
            )
        );
        assert_eq!(output.unsupported_count, 0);
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
    fn nul_statuses_block_matching_elemental_actions() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.set_status(Status::NulShock, 1);
        let tidus_hp = tidus.current_hp;
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Lulu), "thunder");

        state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, tidus_hp);
        assert!(!tidus.statuses.contains(&Status::NulShock));
    }

    #[test]
    fn permanent_nul_statuses_block_without_being_consumed() {
        let mut state = SimulationState::new(1);
        let tidus = state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap();
        tidus.set_status(Status::NulShock, 254);
        let tidus_hp = tidus.current_hp;
        let action_data =
            state.action_data_for_actor(ActorId::Character(Character::Lulu), "thunder");

        state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            action_data.as_ref(),
            &[String::from("tidus")],
        );

        let tidus = state.character_actor(Character::Tidus).unwrap();
        assert_eq!(tidus.current_hp, tidus_hp);
        assert!(tidus.statuses.contains(&Status::NulShock));
        assert_eq!(tidus.status_stack(Status::NulShock), 254);
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
    fn equipment_stat_bonus_abilities_modify_physical_and_magical_damage() {
        let mut physical_state = SimulationState::new(1);
        physical_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        let mut physical_action = test_action(ActionTarget::SingleMonster);
        physical_action.damage_formula = DamageFormula::FixedNoVariance;
        physical_action.damage_type = DamageType::Physical;
        physical_action.base_damage = 10;
        physical_action.damages_hp = true;
        physical_state
            .actor_mut(ActorId::Character(Character::Tidus))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::Strength20);
        physical_state.monsters[0]
            .armor_abilities
            .insert(AutoAbility::Defense20);

        physical_state.apply_action_effects(
            ActorId::Character(Character::Tidus),
            Some(&physical_action),
            &[String::from("m1")],
        );

        assert_eq!(physical_state.monsters[0].current_hp, 1_520);

        let mut magical_state = SimulationState::new(1);
        magical_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        let mut magical_action = physical_action;
        magical_action.damage_type = DamageType::Magical;
        magical_state
            .actor_mut(ActorId::Character(Character::Lulu))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::Magic20);
        magical_state.monsters[0]
            .armor_abilities
            .insert(AutoAbility::MagicDefense20);

        magical_state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            Some(&magical_action),
            &[String::from("m1")],
        );

        assert_eq!(magical_state.monsters[0].current_hp, 1_520);
    }

    #[test]
    fn magic_booster_increases_magical_damage() {
        let mut normal_state = SimulationState::new(1);
        normal_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        let mut action = test_action(ActionTarget::SingleMonster);
        action.damage_formula = DamageFormula::FixedNoVariance;
        action.damage_type = DamageType::Magical;
        action.base_damage = 10;
        action.damages_hp = true;

        normal_state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            Some(&action),
            &[String::from("m1")],
        );
        let normal_hp = normal_state.monsters[0].current_hp;

        let mut boosted_state = SimulationState::new(1);
        boosted_state.monsters.push(BattleActor::monster_with_key(
            MonsterSlot(1),
            Some("worker".to_string()),
            10,
            false,
            2_000,
        ));
        boosted_state
            .actor_mut(ActorId::Character(Character::Lulu))
            .unwrap()
            .weapon_abilities
            .insert(AutoAbility::MagicBooster);

        boosted_state.apply_action_effects(
            ActorId::Character(Character::Lulu),
            Some(&action),
            &[String::from("m1")],
        );

        assert_eq!(normal_hp, 1_500);
        assert_eq!(boosted_state.monsters[0].current_hp, 1_250);
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

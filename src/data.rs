use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::{
    Buff, Character, Element, ElementalAffinity, EncounterCondition, MonsterSlot, Status,
};

const FORMATIONS_JSON: &str = include_str!("../data/formations.json");
const CHARACTERS_JSON: &str = include_str!("../data/characters.json");
const MONSTER_DATA_HD_CSV: &str = include_str!("../data/ffx_mon_data_hd.csv");
const ITEM_CSV: &str = include_str!("../data/ffx_item.csv");
const COMMAND_CSV: &str = include_str!("../data/ffx_command.csv");
const MONMAGIC1_CSV: &str = include_str!("../data/ffx_monmagic1.csv");
const MONMAGIC2_CSV: &str = include_str!("../data/ffx_monmagic2.csv");
const MONSTER_ACTIONS_JSON: &str = include_str!("../data/monster_actions.json");
const TEXT_CHARACTERS_CSV: &str = include_str!("../data/text_characters.csv");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounterFormation {
    pub display_name: String,
    pub monsters: Vec<String>,
    pub forced_condition: Option<EncounterCondition>,
    pub forced_party: Option<String>,
    pub is_random: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterStats {
    pub index: usize,
    pub key: String,
    pub agility: u8,
    pub immune_to_delay: bool,
    pub armored: bool,
    pub immune_to_damage: bool,
    pub immune_to_percentage_damage: bool,
    pub immune_to_physical_damage: bool,
    pub immune_to_magical_damage: bool,
    pub max_hp: i32,
    pub strength: i32,
    pub defense: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub base_weapon_damage: i32,
    pub elemental_affinities: HashMap<Element, ElementalAffinity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterStats {
    pub character: Character,
    pub index: usize,
    pub agility: u8,
    pub max_hp: i32,
    pub strength: i32,
    pub defense: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub base_weapon_damage: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionTarget {
    SelfTarget,
    CharactersParty,
    MonstersParty,
    SingleCharacter,
    SingleMonster,
    RandomCharacter,
    RandomMonster,
    CounterSelf,
    CounterCharactersParty,
    Counter,
    Single,
    EitherParty,
    Character(Character),
    Monster(MonsterSlot),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionData {
    pub key: String,
    pub rank: i32,
    pub target: ActionTarget,
    pub damage_formula: DamageFormula,
    pub damage_type: DamageType,
    pub base_damage: i32,
    pub n_of_hits: i32,
    pub uses_weapon_properties: bool,
    pub ignores_armored: bool,
    pub heals: bool,
    pub damages_hp: bool,
    pub damages_mp: bool,
    pub damages_ctb: bool,
    pub elements: Vec<Element>,
    pub removes_statuses: bool,
    pub has_weak_delay: bool,
    pub has_strong_delay: bool,
    pub statuses: Vec<Status>,
    pub buffs: Vec<ActionBuff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageFormula {
    NoDamage,
    Strength,
    PiercingStrength,
    Magic,
    PiercingMagic,
    PercentageCurrent,
    FixedNoVariance,
    Healing,
    PercentageTotal,
    Fixed,
    PercentageTotalMp,
    BaseCtb,
    PercentageCurrentMp,
    Ctb,
    PiercingStrengthNoVariance,
    SpecialMagic,
    Hp,
    CelestialHighHp,
    CelestialHighMp,
    CelestialLowHp,
    SpecialMagicNoVariance,
    Gil,
    Kills,
    Deal9999,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageType {
    Physical,
    Magical,
    Other,
}

impl DamageFormula {
    fn from_index(index: u8) -> Self {
        match index {
            1 => Self::Strength,
            2 => Self::PiercingStrength,
            3 => Self::Magic,
            4 => Self::PiercingMagic,
            5 => Self::PercentageCurrent,
            6 => Self::FixedNoVariance,
            7 => Self::Healing,
            8 => Self::PercentageTotal,
            9 => Self::Fixed,
            10 => Self::PercentageTotalMp,
            11 => Self::BaseCtb,
            12 => Self::PercentageCurrentMp,
            13 => Self::Ctb,
            14 => Self::PiercingStrengthNoVariance,
            15 => Self::SpecialMagic,
            16 => Self::Hp,
            17 => Self::CelestialHighHp,
            18 => Self::CelestialHighMp,
            19 => Self::CelestialLowHp,
            20 => Self::SpecialMagicNoVariance,
            21 => Self::Gil,
            22 => Self::Kills,
            23 => Self::Deal9999,
            _ => Self::NoDamage,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionBuff {
    pub buff: Buff,
    pub amount: i32,
}

#[derive(Debug, Deserialize)]
struct FormationsFile {
    #[serde(default)]
    bosses: HashMap<String, JsonBossFormation>,
    #[serde(default, alias = "simulation")]
    simulations: HashMap<String, JsonSimulationFormation>,
    #[serde(default)]
    zones: HashMap<String, JsonZone>,
}

#[derive(Debug, Deserialize)]
struct JsonBossFormation {
    name: String,
    #[serde(default)]
    formation: Vec<String>,
    #[serde(default)]
    forced_condition: String,
    #[serde(default)]
    forced_party: String,
}

#[derive(Debug, Deserialize)]
struct JsonSimulationFormation {
    name: String,
    #[serde(default)]
    monsters: Vec<String>,
    #[serde(default)]
    forced_condition: String,
}

#[derive(Debug, Deserialize)]
struct JsonZone {
    name: String,
    #[serde(default, alias = "encounters")]
    formations: Vec<JsonZoneFormation>,
}

#[derive(Debug, Deserialize)]
struct JsonZoneFormation {
    #[serde(default)]
    monsters: Vec<String>,
    #[serde(default)]
    forced_condition: String,
}

#[derive(Debug, Deserialize)]
struct JsonCharacterDefaults {
    character: String,
    index: usize,
    stats: HashMap<String, i32>,
    #[serde(default)]
    weapon: JsonEquipmentDefaults,
}

#[derive(Debug, Default, Deserialize)]
struct JsonEquipmentDefaults {
    #[serde(default)]
    base_weapon_damage: i32,
}

#[derive(Debug, Deserialize)]
struct JsonMonsterAction {
    name: String,
    target: String,
}

static FORMATIONS: OnceLock<FormationsFile> = OnceLock::new();
static CHARACTERS: OnceLock<HashMap<Character, CharacterStats>> = OnceLock::new();
static MONSTERS: OnceLock<HashMap<String, MonsterStats>> = OnceLock::new();
static ACTIONS: OnceLock<HashMap<String, ActionData>> = OnceLock::new();
static MONSTER_ACTION_TARGETS: OnceLock<HashMap<usize, HashMap<String, ActionTarget>>> =
    OnceLock::new();

pub fn boss_or_simulated_formation(name: &str) -> Option<EncounterFormation> {
    let data = formations();
    if let Some(formation) = data.bosses.get(name) {
        return Some(EncounterFormation {
            display_name: formation.name.clone(),
            monsters: formation.formation.clone(),
            forced_condition: parse_forced_condition(&formation.forced_condition),
            forced_party: non_empty_string(&formation.forced_party),
            is_random: false,
        });
    }
    data.simulations
        .get(name)
        .map(|formation| EncounterFormation {
            display_name: formation.name.clone(),
            monsters: formation.monsters.clone(),
            forced_condition: parse_forced_condition(&formation.forced_condition),
            forced_party: None,
            is_random: false,
        })
}

pub fn random_formation(name: &str, formation_roll: u32) -> Option<EncounterFormation> {
    let zone = formations().zones.get(name)?;
    if zone.formations.is_empty() {
        return Some(EncounterFormation {
            display_name: zone.name.clone(),
            monsters: Vec::new(),
            forced_condition: None,
            forced_party: None,
            is_random: true,
        });
    }
    let index = formation_roll as usize % zone.formations.len();
    let formation = &zone.formations[index];
    Some(EncounterFormation {
        display_name: zone.name.clone(),
        monsters: formation.monsters.clone(),
        forced_condition: parse_forced_condition(&formation.forced_condition),
        forced_party: None,
        is_random: true,
    })
}

pub fn has_random_zone(name: &str) -> bool {
    formations().zones.contains_key(name)
}

pub fn monster_stats(name: &str) -> Option<MonsterStats> {
    monsters().get(name).cloned()
}

pub fn character_stats(character: Character) -> Option<CharacterStats> {
    characters().get(&character).cloned()
}

pub fn action_rank(name: &str) -> Option<i32> {
    action_data(name).map(|action| action.rank)
}

pub fn action_data(name: &str) -> Option<ActionData> {
    actions().get(name).cloned()
}

pub fn monster_action_data(monster_key: &str, name: &str) -> Option<ActionData> {
    let mut action = action_data(name)?;
    let monster = monsters().get(monster_key)?;
    if let Some(target) = monster_action_targets()
        .get(&monster.index)
        .and_then(|actions| actions.get(name))
        .copied()
    {
        action.target = target;
    }
    Some(action)
}

fn formations() -> &'static FormationsFile {
    FORMATIONS.get_or_init(|| {
        serde_json::from_str(FORMATIONS_JSON)
            .expect("upstream formations.json should parse for Rust CTB port")
    })
}

fn monsters() -> &'static HashMap<String, MonsterStats> {
    MONSTERS.get_or_init(parse_monsters)
}

fn characters() -> &'static HashMap<Character, CharacterStats> {
    CHARACTERS.get_or_init(parse_characters)
}

fn actions() -> &'static HashMap<String, ActionData> {
    ACTIONS.get_or_init(parse_actions)
}

fn monster_action_targets() -> &'static HashMap<usize, HashMap<String, ActionTarget>> {
    MONSTER_ACTION_TARGETS.get_or_init(parse_monster_action_targets)
}

fn parse_characters() -> HashMap<Character, CharacterStats> {
    let parsed = serde_json::from_str::<HashMap<String, JsonCharacterDefaults>>(CHARACTERS_JSON)
        .expect("upstream characters.json should parse for Rust CTB port");
    parsed
        .into_values()
        .filter_map(|entry| {
            let character = entry.character.parse::<Character>().ok()?;
            let agility = entry
                .stats
                .get("Agility")
                .copied()
                .unwrap_or_default()
                .clamp(0, 255) as u8;
            let max_hp = entry.stats.get("HP").copied().unwrap_or_default();
            let base_weapon_damage = if entry.weapon.base_weapon_damage == 0 {
                16
            } else {
                entry.weapon.base_weapon_damage
            };
            Some((
                character,
                CharacterStats {
                    character,
                    index: entry.index,
                    agility,
                    max_hp,
                    strength: entry.stats.get("Strength").copied().unwrap_or_default(),
                    defense: entry.stats.get("Defense").copied().unwrap_or_default(),
                    magic: entry.stats.get("Magic").copied().unwrap_or_default(),
                    magic_defense: entry
                        .stats
                        .get("Magic defense")
                        .copied()
                        .unwrap_or_default(),
                    base_weapon_damage,
                },
            ))
        })
        .collect()
}

fn parse_actions() -> HashMap<String, ActionData> {
    let text_characters = parse_text_characters();
    let mut actions = HashMap::new();
    for csv in [ITEM_CSV, COMMAND_CSV, MONMAGIC1_CSV, MONMAGIC2_CSV] {
        let mut rows = csv
            .lines()
            .map(parse_hex_csv_line)
            .filter(|row| !row.is_empty())
            .collect::<Vec<_>>();
        let Some(string_data) = rows.pop() else {
            continue;
        };

        for row in rows {
            if row.len() <= 36 {
                continue;
            }
            let name_offset = add_bytes(&row[0..2]).max(0) as usize;
            let name = decode_string_data(&string_data, name_offset, &text_characters);
            if name.is_empty() {
                continue;
            }
            let key = stringify(&name);
            actions.entry(key.clone()).or_insert(ActionData {
                key,
                rank: row[36] as i32,
                target: parse_action_target(&row),
                damage_formula: DamageFormula::from_index(row.get(40).copied().unwrap_or_default()),
                damage_type: parse_damage_type(&row),
                base_damage: row.get(42).copied().unwrap_or_default() as i32,
                n_of_hits: row.get(43).copied().unwrap_or(1).max(1) as i32,
                uses_weapon_properties: row[30] & 0x04 != 0,
                ignores_armored: row[30] & 0x01 != 0,
                heals: row[32] & 0x10 != 0,
                damages_hp: row.get(35).copied().unwrap_or_default() & 0x01 != 0,
                damages_mp: row.get(35).copied().unwrap_or_default() & 0x02 != 0,
                damages_ctb: row.get(35).copied().unwrap_or_default() & 0x04 != 0,
                elements: parse_action_elements(&row),
                removes_statuses: row[32] & 0x20 != 0,
                has_weak_delay: row[29] & 0x20 != 0,
                has_strong_delay: row[29] & 0x40 != 0,
                statuses: parse_action_statuses(&row),
                buffs: parse_action_buffs(&row),
            });
        }
    }

    if let Some(action) = actions.get("attack").cloned() {
        let mut attacknocrit = action;
        attacknocrit.key = "attacknocrit".to_string();
        actions.insert(attacknocrit.key.clone(), attacknocrit);
    }
    if let Some(action) = actions.get("quick_hit").cloned() {
        let mut quick_hit_hd = action.clone();
        quick_hit_hd.key = "quick_hit_hd".to_string();
        actions.insert(quick_hit_hd.key.clone(), quick_hit_hd);

        let mut quick_hit_ps2 = action;
        quick_hit_ps2.key = "quick_hit_ps2".to_string();
        quick_hit_ps2.rank = 1;
        actions.insert(quick_hit_ps2.key.clone(), quick_hit_ps2);
    }

    actions
}

fn parse_damage_type(row: &[u8]) -> DamageType {
    let flags = row.get(32).copied().unwrap_or_default();
    if flags & 0b01 != 0 {
        DamageType::Physical
    } else if flags & 0b10 != 0 {
        DamageType::Magical
    } else {
        DamageType::Other
    }
}

fn parse_action_elements(row: &[u8]) -> Vec<Element> {
    let flags = row.get(45).copied().unwrap_or_default();
    [
        Element::Fire,
        Element::Ice,
        Element::Thunder,
        Element::Water,
    ]
    .into_iter()
    .enumerate()
    .filter_map(|(index, element)| {
        if flags & (1 << index) == 0 {
            return None;
        }
        Some(element)
    })
    .collect()
}

fn parse_action_buffs(row: &[u8]) -> Vec<ActionBuff> {
    const BUFF_ORDER: [&str; 6] = ["cheer", "aim", "focus", "reflex", "luck", "jinx"];
    let flags = row.get(86).copied().unwrap_or_default();
    let amount = row.get(89).copied().unwrap_or_default() as i32;
    BUFF_ORDER
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            if flags & (1 << index) == 0 {
                return None;
            }
            Some(ActionBuff {
                buff: name.parse::<Buff>().ok()?,
                amount,
            })
        })
        .collect()
}

fn parse_action_statuses(row: &[u8]) -> Vec<Status> {
    const STATUS_ORDER: [&str; 25] = [
        "death",
        "zombie",
        "petrify",
        "poison",
        "power_break",
        "magic_break",
        "armor_break",
        "mental_break",
        "confuse",
        "berserk",
        "provoke",
        "threaten",
        "sleep",
        "silence",
        "dark",
        "shell",
        "protect",
        "reflect",
        "nultide",
        "nulblaze",
        "nulshock",
        "nulfrost",
        "regen",
        "haste",
        "slow",
    ];
    const FLAG_STATUS_ORDER: [&str; 21] = [
        "scan",
        "power_distiller",
        "mana_distiller",
        "speed_distiller",
        "ability_distiller",
        "shield",
        "boost",
        "eject",
        "autolife",
        "curse",
        "defend",
        "guard",
        "sentinel",
        "doom",
        "max_hp_x_2",
        "max_mp_x_2",
        "mp_0",
        "damage_9999",
        "critical",
        "overdrive_x_1_5",
        "overdrive_x_2",
    ];

    let mut statuses = Vec::new();
    for (offset, name) in STATUS_ORDER.iter().enumerate() {
        if row.get(46 + offset).copied().unwrap_or_default() == 0 {
            continue;
        }
        if let Ok(status) = name.parse::<Status>() {
            statuses.push(status);
        }
    }

    let status_flags_bytes = add_bytes(&[
        row.get(84).copied().unwrap_or_default(),
        row.get(85).copied().unwrap_or_default(),
        row.get(90).copied().unwrap_or_default(),
    ]);
    for (index, name) in FLAG_STATUS_ORDER.iter().enumerate() {
        let bit_index = if index <= 3 {
            index
        } else if index <= 14 {
            index + 1
        } else {
            index + 2
        };
        if status_flags_bytes & (1 << bit_index) == 0 {
            continue;
        }
        if let Ok(status) = name.parse::<Status>() {
            statuses.push(status);
        }
    }

    statuses
}

fn parse_action_target(row: &[u8]) -> ActionTarget {
    if row.len() <= 31 {
        return ActionTarget::None;
    }
    let targeting_byte = row[26];
    let random_targeting_byte = row[29];
    let has_target = targeting_byte & 0b00000001 != 0;
    let targets_enemies_by_default = targeting_byte & 0b00000010 != 0;
    let aoe = targeting_byte & 0b00000100 != 0;
    let targets_self = targeting_byte & 0b00001000 != 0;
    let can_switch_party = targeting_byte & 0b00100000 != 0;
    let random_targeting = random_targeting_byte & 0x80 != 0;
    let is_character_action = row.len() == 96;

    if !has_target {
        return ActionTarget::None;
    }

    if is_character_action {
        if can_switch_party {
            return if aoe {
                ActionTarget::EitherParty
            } else {
                ActionTarget::Single
            };
        }
        if targets_enemies_by_default {
            return if random_targeting {
                ActionTarget::RandomMonster
            } else if aoe {
                ActionTarget::MonstersParty
            } else {
                ActionTarget::SingleMonster
            };
        }
        if random_targeting {
            return ActionTarget::RandomCharacter;
        }
        if targets_self {
            return ActionTarget::SelfTarget;
        }
        if aoe {
            return ActionTarget::CharactersParty;
        }
        return ActionTarget::SingleCharacter;
    }

    if random_targeting {
        return ActionTarget::RandomCharacter;
    }
    if targets_enemies_by_default {
        return if aoe || targets_self {
            ActionTarget::CharactersParty
        } else {
            ActionTarget::SingleCharacter
        };
    }
    if aoe {
        return ActionTarget::MonstersParty;
    }
    if targets_self {
        return ActionTarget::SelfTarget;
    }
    ActionTarget::SingleMonster
}

fn parse_monster_action_targets() -> HashMap<usize, HashMap<String, ActionTarget>> {
    let parsed =
        serde_json::from_str::<HashMap<String, Vec<JsonMonsterAction>>>(MONSTER_ACTIONS_JSON)
            .expect("upstream monster_actions.json should parse for Rust CTB port");
    parsed
        .into_iter()
        .filter_map(|(monster_id, actions)| {
            let index = monster_id.strip_prefix('m')?.parse::<usize>().ok()?;
            let action_targets = actions
                .into_iter()
                .filter_map(|action| {
                    Some((
                        stringify(&action.name),
                        parse_named_action_target(&action.target)?,
                    ))
                })
                .collect::<HashMap<_, _>>();
            Some((index, action_targets))
        })
        .collect()
}

fn parse_named_action_target(value: &str) -> Option<ActionTarget> {
    match value {
        "Self" | "Counter Self" => Some(ActionTarget::SelfTarget),
        "Characters' Party" | "Counter Characters' Party" => Some(ActionTarget::CharactersParty),
        "Monsters' Party" => Some(ActionTarget::MonstersParty),
        "Single Character" => Some(ActionTarget::SingleCharacter),
        "Single Monster" => Some(ActionTarget::SingleMonster),
        "Random Character"
        | "Lowest HP Character"
        | "Highest HP Character"
        | "Highest Str Character"
        | "Highest MP Character"
        | "Lowest Mag Def Character"
        | "Random Character Affected By Reflect"
        | "Random Character Affected By Zombie"
        | "Random Character Affected By Petrify"
        | "Random Character Not Affected By Petrify"
        | "Random Character Not Affected By Doom"
        | "Random Character Not Affected By Berserk"
        | "Random Character Not Affected By Confuse"
        | "Random Character Not Affected By Curse"
        | "Random Character Not Affected By Poison"
        | "Random Character Not Affected By Auto-Life"
        | "Random Character Affected By Death"
        | "Random Character Not Affected By Shell or Reflect"
        | "Random Character Not Affected By Protect or Reflect"
        | "Random Character Not Affected By Haste or Reflect"
        | "Random Character (Not User) Damaged" => Some(ActionTarget::RandomCharacter),
        "Random Monster"
        | "Random Monster Not Affected By Shell"
        | "Random Monster Not Affected By Protect"
        | "Random Monster Not Affected By Reflect" => Some(ActionTarget::RandomMonster),
        "Counter" | "Counter Random Character" | "Counter Last Target" => {
            Some(ActionTarget::Counter)
        }
        "All" | "Counter All" => Some(ActionTarget::EitherParty),
        _ => {
            if let Ok(character) = value.parse::<Character>() {
                return Some(ActionTarget::Character(character));
            }
            let slot_name = value.to_ascii_lowercase();
            if let Ok(slot) = slot_name.parse::<MonsterSlot>() {
                return Some(ActionTarget::Monster(slot));
            }
            None
        }
    }
}

fn parse_monsters() -> HashMap<String, MonsterStats> {
    let text_characters = parse_text_characters();
    let mut monsters = HashMap::new();
    let mut seen_names: HashMap<String, usize> = HashMap::new();

    for (index, line) in MONSTER_DATA_HD_CSV.lines().enumerate() {
        let data = parse_hex_csv_line(line);
        if data.len() < 409 {
            continue;
        }
        let name = monster_name_override(index)
            .unwrap_or_else(|| decode_monster_name(&data, &text_characters));
        if name.is_empty() {
            continue;
        }
        let mut key = stringify(&name);
        let count = seen_names.entry(key.clone()).or_insert(0);
        if *count > 0 {
            key = format!("{key}_{}", *count + 1);
        }
        *count += 1;

        monsters.insert(
            key.clone(),
            MonsterStats {
                index,
                key,
                max_hp: add_bytes(&data[20..24]),
                agility: data[36],
                immune_to_delay: data[41] & 0b00000001 != 0,
                armored: data[40] & 0b00000001 != 0,
                immune_to_percentage_damage: data[40] & 0b00000010 != 0,
                immune_to_physical_damage: data[40] & 0b00100000 != 0,
                immune_to_magical_damage: data[40] & 0b01000000 != 0,
                immune_to_damage: data[40] & 0b10000000 != 0,
                strength: data[32] as i32,
                defense: data[33] as i32,
                magic: data[34] as i32,
                magic_defense: data[35] as i32,
                base_weapon_damage: data[176] as i32,
                elemental_affinities: parse_monster_elemental_affinities(&data),
            },
        );
    }

    monsters
}

fn parse_monster_elemental_affinities(data: &[u8]) -> HashMap<Element, ElementalAffinity> {
    let elements = [
        Element::Fire,
        Element::Ice,
        Element::Thunder,
        Element::Water,
    ];
    elements
        .into_iter()
        .enumerate()
        .map(|(index, element)| {
            let affinity = if data.get(43).copied().unwrap_or_default() & (1 << index) != 0 {
                ElementalAffinity::Absorbs
            } else if data.get(44).copied().unwrap_or_default() & (1 << index) != 0 {
                ElementalAffinity::Immune
            } else if data.get(45).copied().unwrap_or_default() & (1 << index) != 0 {
                ElementalAffinity::Resists
            } else if data.get(46).copied().unwrap_or_default() & (1 << index) != 0 {
                ElementalAffinity::Weak
            } else {
                ElementalAffinity::Neutral
            };
            (element, affinity)
        })
        .collect()
}

fn parse_text_characters() -> HashMap<u8, String> {
    let mut mapping = HashMap::new();
    for line in TEXT_CHARACTERS_CSV.lines().skip(1) {
        let mut fields = line.splitn(3, ',');
        let Some(id) = fields.next().and_then(|value| value.parse::<u8>().ok()) else {
            continue;
        };
        let _hex = fields.next();
        let Some(character) = fields.next() else {
            continue;
        };
        mapping.insert(id, character.trim_matches('"').to_string());
    }
    mapping
}

fn decode_monster_name(data: &[u8], text_characters: &HashMap<u8, String>) -> String {
    decode_string_data(data, 408, text_characters)
}

fn decode_string_data(data: &[u8], offset: usize, text_characters: &HashMap<u8, String>) -> String {
    let mut name = String::new();
    if offset >= data.len() {
        return name;
    }
    for byte in &data[offset..] {
        if *byte == 0 {
            break;
        }
        if let Some(character) = text_characters.get(byte) {
            name.push_str(character);
        }
    }
    name
}

fn parse_hex_csv_line(line: &str) -> Vec<u8> {
    line.split(',')
        .filter_map(|byte| u8::from_str_radix(byte, 16).ok())
        .collect()
}

fn parse_forced_condition(value: &str) -> Option<EncounterCondition> {
    match value {
        "preemptive" => Some(EncounterCondition::Preemptive),
        "normal" => Some(EncounterCondition::Normal),
        "ambush" => Some(EncounterCondition::Ambush),
        _ => None,
    }
}

fn non_empty_string(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn stringify(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace(' ', "_")
        .replace(['(', ')', '\''], "")
}

fn add_bytes(values: &[u8]) -> i32 {
    values
        .iter()
        .enumerate()
        .map(|(position, value)| (*value as i32) * 256_i32.pow(position as u32))
        .sum()
}

fn monster_name_override(index: usize) -> Option<String> {
    let name = match index {
        163 => "Possessed Valefor",
        164 => "Possessed Ifrit",
        165 => "Possessed Ixion",
        166 => "Possessed Shiva",
        167 => "Possessed Bahamut",
        168 => "Possessed Anima",
        169 => "Possessed Yojimbo",
        178 => "Possessed Cindy",
        179 => "Possessed Sandy",
        180 => "Possessed Mindy",
        334 => "Dark Valefor",
        335 => "Dark Ifrit",
        336 => "Dark Ixion",
        337 => "Dark Shiva",
        338 => "Dark Bahamut",
        339 => "Dark Anima",
        340 => "Dark Yojimbo",
        341 => "Dark Cindy",
        342 => "Dark Sandy",
        343 => "Dark Mindy",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        action_data, boss_or_simulated_formation, character_stats, monster_stats, random_formation,
        ActionTarget, DamageFormula, DamageType,
    };
    use crate::model::{Buff, Character, Element, ElementalAffinity, EncounterCondition, Status};

    #[test]
    fn loads_boss_formations_from_upstream_data() {
        let tanker = boss_or_simulated_formation("tanker").unwrap();
        assert_eq!(tanker.display_name, "Tanker");
        assert_eq!(tanker.forced_condition, Some(EncounterCondition::Normal));
        assert_eq!(tanker.forced_party.as_deref(), Some("ta"));
        assert_eq!(tanker.monsters[0], "tanker");
        assert_eq!(tanker.monsters.len(), 8);
    }

    #[test]
    fn loads_random_formations_from_upstream_data() {
        let formation = random_formation("kilika_woods", 0).unwrap();
        assert_eq!(formation.display_name, "Kilika Woods");
        assert!(formation.is_random);
        assert!(!formation.monsters.is_empty());
    }

    #[test]
    fn loads_monster_stats_from_hd_monster_data() {
        let geosgaeno = monster_stats("geosgaeno").unwrap();
        assert_eq!(geosgaeno.index, 109);
        assert_eq!(geosgaeno.max_hp, 32_767);
        assert_eq!(geosgaeno.agility, 48);
        assert!(geosgaeno.immune_to_delay);
        let sahagin = monster_stats("sahagin_4").unwrap();
        assert_eq!(sahagin.max_hp, 100);
        assert_eq!(sahagin.agility, 5);
        assert_eq!(sahagin.strength, 3);
        assert_eq!(sahagin.base_weapon_damage, 0);
        assert!(!sahagin.armored);
        assert_eq!(
            sahagin.elemental_affinities.get(&Element::Thunder),
            Some(&ElementalAffinity::Weak)
        );
    }

    #[test]
    fn loads_character_stats_from_upstream_data() {
        let tidus = character_stats(Character::Tidus).unwrap();
        assert_eq!(tidus.index, 0);
        assert_eq!(tidus.max_hp, 520);
        assert_eq!(tidus.agility, 10);
        assert_eq!(tidus.strength, 15);
        assert_eq!(tidus.base_weapon_damage, 16);
    }

    #[test]
    fn loads_action_ranks_from_upstream_command_data() {
        assert_eq!(super::action_rank("attack"), Some(3));
        assert_eq!(super::action_rank("defend"), Some(2));
        assert_eq!(super::action_rank("cheer"), Some(2));
        assert_eq!(super::action_rank("energy_ray"), Some(8));
        assert!(super::action_rank("potion").is_some());
        assert_eq!(super::action_rank("quick_hit_ps2"), Some(1));
    }

    #[test]
    fn loads_action_effects_from_upstream_data() {
        let dark_attack = action_data("dark_attack").unwrap();
        assert!(dark_attack.statuses.contains(&Status::Dark));

        let attack = action_data("attack").unwrap();
        assert_eq!(attack.damage_formula, DamageFormula::Strength);
        assert_eq!(attack.damage_type, DamageType::Physical);
        assert!(attack.damages_hp);
        assert!(attack.uses_weapon_properties);

        let grenade = action_data("grenade").unwrap();
        assert!(matches!(
            grenade.damage_formula,
            DamageFormula::Fixed | DamageFormula::FixedNoVariance
        ));
        assert!(grenade.damages_hp);
        assert!(!grenade.uses_weapon_properties);

        let fire = action_data("fire").unwrap();
        assert!(fire.elements.contains(&Element::Fire));

        let haste = action_data("haste").unwrap();
        assert_eq!(haste.target, ActionTarget::Single);
        assert!(haste.statuses.contains(&Status::Haste));

        let delay_attack = action_data("delay_attack").unwrap();
        assert!(delay_attack.has_weak_delay || delay_attack.has_strong_delay);

        let cheer = action_data("cheer").unwrap();
        assert!(cheer
            .buffs
            .iter()
            .any(|buff| { buff.buff == Buff::Cheer && buff.amount == 1 }));
    }

    #[test]
    fn loads_monster_action_target_overrides() {
        let blender = super::monster_action_data("sinspawn_echuilles", "blender").unwrap();
        assert_eq!(blender.target, ActionTarget::CharactersParty);
    }
}

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::{
    AutoAbility, Buff, Character, Element, ElementalAffinity, EncounterCondition, MonsterSlot,
    Status,
};

const FORMATIONS_JSON: &str = include_str!("../data/formations.json");
const CHARACTERS_JSON: &str = include_str!("../data/characters.json");
const MONSTER_DATA_HD_CSV: &str = include_str!("../data/ffx_mon_data_hd.csv");
const ITEM_CSV: &str = include_str!("../data/ffx_item.csv");
const ITEMS_CSV: &str = include_str!("../data/items.csv");
const COMMAND_CSV: &str = include_str!("../data/ffx_command.csv");
const MONMAGIC1_CSV: &str = include_str!("../data/ffx_monmagic1.csv");
const MONMAGIC2_CSV: &str = include_str!("../data/ffx_monmagic2.csv");
const MONSTER_ACTIONS_JSON: &str = include_str!("../data/monster_actions.json");
const DEFAULT_MONSTER_ACTIONS_CSV: &str =
    include_str!("../data/anypercent_enemy_multi_action_candidates.csv");
const TEXT_CHARACTERS_CSV: &str = include_str!("../data/text_characters.csv");
const AUTOABILITIES_CSV: &str = include_str!("../data/autoabilities.csv");
const EQUIPMENT_NAMES_JSON: &str = include_str!("../data/equipment_names.json");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncounterFormation {
    pub display_name: String,
    pub zone_display_name: Option<String>,
    pub monsters: Vec<String>,
    pub forced_condition: Option<EncounterCondition>,
    pub forced_party: Option<String>,
    pub is_random: bool,
    pub is_simulated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomZoneStats {
    pub display_name: String,
    pub grace_period: i32,
    pub threat_modifier: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterStats {
    pub index: usize,
    pub key: String,
    pub display_name: String,
    pub agility: u8,
    pub immune_to_delay: bool,
    pub armored: bool,
    pub immune_to_damage: bool,
    pub immune_to_percentage_damage: bool,
    pub immune_to_physical_damage: bool,
    pub immune_to_magical_damage: bool,
    pub immune_to_life: bool,
    pub immune_to_bribe: bool,
    pub zanmato_level: u8,
    pub max_hp: i32,
    pub max_mp: i32,
    pub strength: i32,
    pub defense: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub luck: i32,
    pub evasion: i32,
    pub accuracy: i32,
    pub base_weapon_damage: i32,
    pub elemental_affinities: HashMap<Element, ElementalAffinity>,
    pub status_resistances: HashMap<Status, u8>,
    pub auto_statuses: Vec<Status>,
    pub gil: i32,
    pub normal_ap: i32,
    pub overkill_ap: i32,
    pub item_1: MonsterItemDropInfo,
    pub item_2: MonsterItemDropInfo,
    pub steal: MonsterStealInfo,
    pub bribe: Option<ItemDrop>,
    pub forced_action: Option<ActionData>,
    pub equipment_drop_chance: u8,
    pub equipment: MonsterEquipmentDropInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterItemDropInfo {
    pub drop_chance: u8,
    pub normal_common: Option<ItemDrop>,
    pub normal_rare: Option<ItemDrop>,
    pub overkill_common: Option<ItemDrop>,
    pub overkill_rare: Option<ItemDrop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterStealInfo {
    pub base_chance: u8,
    pub common: Option<ItemDrop>,
    pub rare: Option<ItemDrop>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDrop {
    pub item: String,
    pub quantity: u8,
    pub rare: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EquipmentKind {
    Weapon,
    Armor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterEquipmentDropInfo {
    pub drop_chance: u8,
    pub bonus_critical_chance: u8,
    pub base_weapon_damage: i32,
    pub slots_range: Vec<u8>,
    pub max_ability_rolls_range: Vec<u8>,
    pub added_to_inventory: bool,
    pub ability_lists: HashMap<EquipmentKind, HashMap<Character, Vec<Option<String>>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterStats {
    pub character: Character,
    pub index: usize,
    pub starting_s_lv: i32,
    pub agility: u8,
    pub max_hp: i32,
    pub max_mp: i32,
    pub strength: i32,
    pub defense: i32,
    pub magic: i32,
    pub magic_defense: i32,
    pub luck: i32,
    pub evasion: i32,
    pub accuracy: i32,
    pub base_weapon_damage: i32,
    pub equipment_crit: i32,
    pub weapon_bonus_crit: i32,
    pub armor_bonus_crit: i32,
    pub weapon_slots: u8,
    pub armor_slots: u8,
    pub weapon_abilities: Vec<AutoAbility>,
    pub armor_abilities: Vec<AutoAbility>,
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
    HighestHpCharacter,
    HighestMpCharacter,
    LowestHpCharacter,
    HighestStrengthCharacter,
    LowestMagicDefenseCharacter,
    RandomCharacterWith(Status),
    RandomCharacterWithout(Status),
    RandomCharacterWithoutEither(Status, Status),
    RandomMonsterWithout(Status),
    Provoker,
    LastTarget,
    LastAttacker,
    CounterSelf,
    CounterSingleCharacter,
    CounterRandomCharacter,
    CounterCharactersParty,
    CounterAll,
    CounterLastTarget,
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
    pub can_use_in_combat: bool,
    pub overdrive_user: Option<Character>,
    pub overdrive_index: i32,
    pub can_target_dead: bool,
    pub affected_by_silence: bool,
    pub steals_item: bool,
    pub steals_gil: bool,
    pub empties_od_bar: bool,
    pub copied_by_copycat: bool,
    pub hit_chance_formula: HitChanceFormula,
    pub uses_hit_chance_table: bool,
    pub accuracy: i32,
    pub affected_by_dark: bool,
    pub affected_by_reflect: bool,
    pub damage_formula: DamageFormula,
    pub damage_type: DamageType,
    pub base_damage: i32,
    pub mp_cost: i32,
    pub uses_magic_booster: bool,
    pub n_of_hits: i32,
    pub uses_weapon_properties: bool,
    pub ignores_armored: bool,
    pub never_break_damage_limit: bool,
    pub always_break_damage_limit: bool,
    pub can_crit: bool,
    pub bonus_crit: i32,
    pub adds_equipment_crit: bool,
    pub affected_by_alchemy: bool,
    pub drains: bool,
    pub misses_if_target_alive: bool,
    pub destroys_user: bool,
    pub heals: bool,
    pub damages_hp: bool,
    pub damages_mp: bool,
    pub damages_ctb: bool,
    pub elements: Vec<Element>,
    pub removes_statuses: bool,
    pub has_weak_delay: bool,
    pub has_strong_delay: bool,
    pub shatter_chance: i32,
    pub status_applications: Vec<ActionStatus>,
    pub statuses: Vec<Status>,
    pub status_flags: Vec<Status>,
    pub buffs: Vec<ActionBuff>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitChanceFormula {
    Always,
    UseActionAccuracy,
    UseAccuracy,
    UseAccuracyX25,
    UseAccuracyX15,
    UseAccuracyX05,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionStatus {
    pub status: Status,
    pub chance: u8,
    pub stacks: i32,
    pub ignores_resistance: bool,
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
    #[serde(default)]
    danger_value: i32,
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
    #[serde(default)]
    starting_s_lv: i32,
    stats: HashMap<String, i32>,
    #[serde(default)]
    weapon: JsonEquipmentDefaults,
    #[serde(default)]
    armor: JsonEquipmentDefaults,
}

#[derive(Debug, Default, Deserialize)]
struct JsonEquipmentDefaults {
    #[serde(default)]
    slots: u8,
    #[serde(default)]
    abilities: Vec<String>,
    #[serde(default)]
    base_weapon_damage: i32,
    #[serde(default)]
    bonus_crit: i32,
}

#[derive(Debug, Deserialize)]
struct JsonMonsterAction {
    actions_file: i32,
    action_id: usize,
    name: String,
    target: String,
}

static FORMATIONS: OnceLock<FormationsFile> = OnceLock::new();
static CHARACTERS: OnceLock<HashMap<Character, CharacterStats>> = OnceLock::new();
static MONSTERS: OnceLock<HashMap<String, MonsterStats>> = OnceLock::new();
static ACTIONS: OnceLock<HashMap<String, ActionData>> = OnceLock::new();
static MONSTER_ACTION_TARGETS: OnceLock<HashMap<usize, HashMap<String, MonsterActionOverride>>> =
    OnceLock::new();
static DEFAULT_MONSTER_ACTIONS: OnceLock<HashMap<String, String>> = OnceLock::new();

pub fn boss_or_simulated_formation(name: &str) -> Option<EncounterFormation> {
    let key = stringify(name);
    let data = formations();
    if let Some(formation) = data.bosses.get(&key) {
        return Some(EncounterFormation {
            display_name: formation.name.clone(),
            zone_display_name: None,
            monsters: formation.formation.clone(),
            forced_condition: parse_forced_condition(&formation.forced_condition),
            forced_party: non_empty_string(&formation.forced_party),
            is_random: false,
            is_simulated: false,
        });
    }
    data.simulations
        .get(&key)
        .map(|formation| EncounterFormation {
            display_name: formation.name.clone(),
            zone_display_name: None,
            monsters: formation.monsters.clone(),
            forced_condition: parse_forced_condition(&formation.forced_condition),
            forced_party: None,
            is_random: false,
            is_simulated: true,
        })
}

pub fn random_formation(name: &str, formation_roll: u32) -> Option<EncounterFormation> {
    let key = stringify(name);
    let zone = formations().zones.get(&key)?;
    if zone.formations.is_empty() {
        return Some(EncounterFormation {
            display_name: zone.name.clone(),
            zone_display_name: Some(zone.name.clone()),
            monsters: Vec::new(),
            forced_condition: None,
            forced_party: None,
            is_random: true,
            is_simulated: false,
        });
    }
    let index = formation_roll as usize % zone.formations.len();
    let formation = &zone.formations[index];
    let display_name = format_monster_list(&formation.monsters);
    Some(EncounterFormation {
        display_name,
        zone_display_name: Some(zone.name.clone()),
        monsters: formation.monsters.clone(),
        forced_condition: parse_forced_condition(&formation.forced_condition),
        forced_party: None,
        is_random: true,
        is_simulated: false,
    })
}

pub fn format_monster_list(monsters: &[String]) -> String {
    if monsters.is_empty() {
        return "Empty".to_string();
    }
    monsters
        .iter()
        .map(|monster| {
            monster_stats(monster)
                .map(|stats| stats.display_name)
                .unwrap_or_else(|| monster.clone())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn has_random_zone(name: &str) -> bool {
    formations().zones.contains_key(&stringify(name))
}

pub fn random_zone_display_name(name: &str) -> Option<String> {
    formations()
        .zones
        .get(&stringify(name))
        .map(|zone| zone.name.clone())
}

pub fn random_zone_stats(name: &str) -> Option<RandomZoneStats> {
    let zone = formations().zones.get(&stringify(name))?;
    Some(RandomZoneStats {
        display_name: zone.name.clone(),
        grace_period: zone.danger_value / 2,
        threat_modifier: zone.danger_value * 4,
    })
}

pub fn monster_stats(name: &str) -> Option<MonsterStats> {
    monsters()
        .get(name)
        .or_else(|| monsters().get(&stringify(name)))
        .cloned()
}

pub fn character_stats(character: Character) -> Option<CharacterStats> {
    characters().get(&character).cloned()
}

pub fn item_names_in_order() -> &'static [&'static str] {
    ITEM_NAMES
}

pub fn item_name_by_key(value: &str) -> Option<&'static str> {
    let key = stringify(value);
    ITEM_NAMES
        .iter()
        .copied()
        .find(|name| stringify(name) == key)
}

pub fn item_price(name: &str) -> Option<i32> {
    let canonical = item_name_by_key(name)?;
    ITEMS_CSV.lines().skip(1).find_map(|line| {
        let (item, price) = line.split_once(',')?;
        (item == canonical)
            .then(|| price.parse::<i32>().ok())
            .flatten()
    })
}

pub fn autoability_gil_value(name: &str) -> Option<i32> {
    AUTOABILITIES_CSV.lines().skip(1).find_map(|line| {
        let (ability, value) = line.split_once(',')?;
        (stringify_autoability(ability) == stringify_autoability(name))
            .then(|| value.parse::<i32>().ok())
            .flatten()
    })
}

pub fn autoability_name_by_key(name: &str) -> Option<&'static str> {
    let key = stringify_autoability(name);
    AUTOABILITIES_CSV.lines().skip(1).find_map(|line| {
        let (ability, _) = line.split_once(',')?;
        (stringify_autoability(ability) == key).then_some(ability)
    })
}

pub fn autoability_names_in_order() -> Vec<&'static str> {
    AUTOABILITIES_CSV
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(',').map(|(ability, _)| ability))
        .collect()
}

pub fn equipment_name(kind: EquipmentKind, owner: Character, index: usize) -> Option<String> {
    let parsed = serde_json::from_str::<serde_json::Value>(EQUIPMENT_NAMES_JSON).ok()?;
    let kind_key = match kind {
        EquipmentKind::Weapon => "Weapon",
        EquipmentKind::Armor => "Armor",
    };
    parsed
        .get(kind_key)?
        .get(index)?
        .get(owner.display_name())?
        .as_str()
        .map(ToOwned::to_owned)
}

pub fn action_rank(name: &str) -> Option<i32> {
    action_data(name).map(|action| action.rank)
}

pub fn action_data(name: &str) -> Option<ActionData> {
    actions().get(name).cloned()
}

pub fn monster_action_data(monster_key: &str, name: &str) -> Option<ActionData> {
    if name == "forced_action" {
        let monster = monsters().get(monster_key)?;
        let action = monster
            .forced_action
            .clone()
            .or_else(|| action_data("does_nothing"))?;
        return Some(action);
    }
    let monster = monsters().get(monster_key)?;
    if let Some(override_data) = monster_action_targets()
        .get(&monster.index)
        .and_then(|actions| actions.get(name))
    {
        let mut action =
            parse_action_from_file_index(override_data.actions_file, override_data.action_id)
                .or_else(|| action_data(&override_data.action_key))?;
        action.key = override_data.action_key.clone();
        action.target = override_data.target;
        return Some(action);
    }
    action_data(name)
}

pub fn monster_action_names(monster_key: &str) -> Option<Vec<String>> {
    let monster = monsters().get(monster_key)?;
    let mut names = monster_action_targets()
        .get(&monster.index)
        .map(|actions| actions.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    names.sort();
    Some(names)
}

pub fn default_monster_action(monster_key: &str) -> Option<&'static str> {
    default_monster_actions()
        .get(monster_key)
        .map(String::as_str)
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

fn default_monster_actions() -> &'static HashMap<String, String> {
    DEFAULT_MONSTER_ACTIONS.get_or_init(parse_default_monster_actions)
}

fn characters() -> &'static HashMap<Character, CharacterStats> {
    CHARACTERS.get_or_init(parse_characters)
}

fn actions() -> &'static HashMap<String, ActionData> {
    ACTIONS.get_or_init(parse_actions)
}

fn monster_action_targets() -> &'static HashMap<usize, HashMap<String, MonsterActionOverride>> {
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
            let max_mp = entry.stats.get("MP").copied().unwrap_or_default();
            let base_weapon_damage = if entry.weapon.base_weapon_damage == 0 {
                16
            } else {
                entry.weapon.base_weapon_damage
            };
            let equipment_crit = entry.weapon.bonus_crit + entry.armor.bonus_crit;
            let weapon_abilities = entry
                .weapon
                .abilities
                .iter()
                .filter_map(|ability| ability.parse::<AutoAbility>().ok())
                .collect();
            let armor_abilities = entry
                .armor
                .abilities
                .iter()
                .filter_map(|ability| ability.parse::<AutoAbility>().ok())
                .collect();
            Some((
                character,
                CharacterStats {
                    character,
                    index: entry.index,
                    starting_s_lv: entry.starting_s_lv,
                    agility,
                    max_hp,
                    max_mp,
                    strength: entry.stats.get("Strength").copied().unwrap_or_default(),
                    defense: entry.stats.get("Defense").copied().unwrap_or_default(),
                    magic: entry.stats.get("Magic").copied().unwrap_or_default(),
                    magic_defense: entry
                        .stats
                        .get("Magic defense")
                        .copied()
                        .unwrap_or_default(),
                    luck: entry.stats.get("Luck").copied().unwrap_or_default(),
                    evasion: entry.stats.get("Evasion").copied().unwrap_or_default(),
                    accuracy: entry.stats.get("Accuracy").copied().unwrap_or_default(),
                    base_weapon_damage,
                    equipment_crit,
                    weapon_bonus_crit: entry.weapon.bonus_crit,
                    armor_bonus_crit: entry.armor.bonus_crit,
                    weapon_slots: entry.weapon.slots,
                    armor_slots: entry.armor.slots,
                    weapon_abilities,
                    armor_abilities,
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
            let Some(action) =
                parse_action_row(&row, &string_data, &text_characters, csv == ITEM_CSV)
            else {
                continue;
            };
            actions.entry(action.key.clone()).or_insert(action);
        }
    }

    if let Some(action) = actions.get("attack").cloned() {
        let mut attacknocrit = action;
        attacknocrit.key = "attacknocrit".to_string();
        attacknocrit.can_crit = false;
        actions.insert(attacknocrit.key.clone(), attacknocrit);

        let mut counter = actions.get("attack").cloned().unwrap();
        counter.key = "counter".to_string();
        counter.target = ActionTarget::Counter;
        actions.insert(counter.key.clone(), counter);
    }
    if let Some(action) = actions.get("switch").cloned() {
        let mut does_nothing = action;
        does_nothing.key = "does_nothing".to_string();
        actions.insert(does_nothing.key.clone(), does_nothing);
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
    for (source, key) in [
        ("auto-life", "auto-life_counter"),
        ("potion", "auto_potion"),
        ("hi-potion", "auto_hi-potion"),
        ("x-potion", "auto_x-potion"),
        ("remedy", "auto_med"),
    ] {
        if let Some(action) = actions.get(source).cloned() {
            let mut counter_action = action;
            counter_action.key = key.to_string();
            counter_action.target = ActionTarget::CounterSelf;
            counter_action.can_use_in_combat = true;
            actions.insert(counter_action.key.clone(), counter_action);
        }
    }
    if let Some(action) = actions.get("phoenix_down").cloned() {
        let mut auto_phoenix = action;
        auto_phoenix.key = "auto_phoenix".to_string();
        auto_phoenix.target = ActionTarget::CounterSingleCharacter;
        auto_phoenix.can_use_in_combat = true;
        actions.insert(auto_phoenix.key.clone(), auto_phoenix);
    }
    if let Some(mut delta_attack) = parse_action_from_file_index(4, 262) {
        delta_attack.key = "delta_attack".to_string();
        delta_attack.target = ActionTarget::MonstersParty;
        delta_attack.can_use_in_combat = true;
        actions.insert(delta_attack.key.clone(), delta_attack);
    }
    if let Some(attack) = actions.get("attack").cloned() {
        let mut attack_wakka = attack;
        attack_wakka.key = "attack_wakka".to_string();
        attack_wakka.target = ActionTarget::Character(Character::Wakka);
        attack_wakka.can_use_in_combat = true;
        actions.insert(attack_wakka.key.clone(), attack_wakka);
    }
    if let Some(bribe) = actions.get_mut("bribe") {
        bribe.target = ActionTarget::SingleMonster;
    }

    actions
}

fn parse_action_from_file_index(file_id: i32, action_index: usize) -> Option<ActionData> {
    let csv = action_csv_by_file_id(file_id)?;
    let text_characters = parse_text_characters();
    let mut rows = csv
        .lines()
        .map(parse_hex_csv_line)
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>();
    let string_data = rows.pop()?;
    let row = rows.get(action_index)?;
    parse_action_row(row, &string_data, &text_characters, csv == ITEM_CSV)
}

fn action_csv_by_file_id(file_id: i32) -> Option<&'static str> {
    match file_id {
        2 => Some(ITEM_CSV),
        3 => Some(COMMAND_CSV),
        4 => Some(MONMAGIC1_CSV),
        6 => Some(MONMAGIC2_CSV),
        _ => None,
    }
}

fn parse_action_row(
    row: &[u8],
    string_data: &[u8],
    text_characters: &HashMap<u8, String>,
    is_item: bool,
) -> Option<ActionData> {
    if row.len() <= 36 {
        return None;
    }
    let name_offset = add_bytes(&row[0..2]).max(0) as usize;
    let name = decode_string_data(string_data, name_offset, text_characters);
    if name.is_empty() {
        return None;
    }
    let key = stringify(&name);
    let damage_formula = DamageFormula::from_index(row.get(40).copied().unwrap_or_default());
    let heals = row[32] & 0x10 != 0;
    let overdrive_info = row.get(88).copied().unwrap_or_default();
    let overdrive_user = parse_overdrive_user(overdrive_info);
    let overdrive_index = (overdrive_info >> 4) as i32;
    let status_applications = parse_action_status_applications(row);
    let status_flags = parse_action_status_flags(row);
    Some(ActionData {
        key,
        rank: row[36] as i32,
        target: parse_action_target(row, overdrive_user, overdrive_index),
        can_use_in_combat: row.get(28).copied().unwrap_or_default() & 0x02 != 0,
        overdrive_user,
        overdrive_index,
        can_target_dead: row.get(26).copied().unwrap_or_default() & 0b0100_0000 != 0,
        affected_by_silence: row.get(30).copied().unwrap_or_default() & 0x02 != 0,
        steals_item: row.get(29).copied().unwrap_or_default() & 0x02 != 0,
        steals_gil: row.get(33).copied().unwrap_or_default() & 0x01 != 0,
        empties_od_bar: row.get(31).copied().unwrap_or_default() & 0x02 != 0,
        copied_by_copycat: row.get(31).copied().unwrap_or_default() & 0x10 != 0,
        hit_chance_formula: parse_hit_chance_formula(row),
        uses_hit_chance_table: uses_hit_chance_table(row),
        accuracy: row.get(41).copied().unwrap_or_default() as i32,
        affected_by_dark: row.get(28).copied().unwrap_or_default() & 0x40 != 0,
        affected_by_reflect: row.get(28).copied().unwrap_or_default() & 0x80 != 0,
        damage_formula,
        damage_type: parse_damage_type(row),
        base_damage: row.get(42).copied().unwrap_or_default() as i32,
        mp_cost: row.get(37).copied().unwrap_or_default() as i32,
        uses_magic_booster: matches!(row.get(24).copied(), Some(1 | 2)),
        n_of_hits: row.get(43).copied().unwrap_or(1).max(1) as i32,
        uses_weapon_properties: row[30] & 0x04 != 0,
        ignores_armored: row[30] & 0x01 != 0,
        never_break_damage_limit: row[32] & 0x40 != 0,
        always_break_damage_limit: row[32] & 0x80 != 0,
        can_crit: row[32] & 0x04 != 0,
        bonus_crit: row.get(39).copied().unwrap_or_default() as i32,
        adds_equipment_crit: row[32] & 0x08 != 0,
        affected_by_alchemy: is_item
            && heals
            && matches!(
                damage_formula,
                DamageFormula::FixedNoVariance | DamageFormula::PercentageTotal
            ),
        drains: row[29] & 0x01 != 0,
        misses_if_target_alive: row[30] & 0x80 != 0,
        destroys_user: row[30] & 0x40 != 0,
        heals,
        damages_hp: row.get(35).copied().unwrap_or_default() & 0x01 != 0,
        damages_mp: row.get(35).copied().unwrap_or_default() & 0x02 != 0,
        damages_ctb: row.get(35).copied().unwrap_or_default() & 0x04 != 0,
        elements: parse_action_elements(row),
        removes_statuses: row[32] & 0x20 != 0,
        has_weak_delay: row[29] & 0x20 != 0,
        has_strong_delay: row[29] & 0x40 != 0,
        shatter_chance: row.get(44).copied().unwrap_or_default() as i32,
        status_applications: status_applications.clone(),
        statuses: status_applications
            .into_iter()
            .map(|application| application.status)
            .collect(),
        status_flags,
        buffs: parse_action_buffs(row),
    })
}

fn parse_hit_chance_formula(row: &[u8]) -> HitChanceFormula {
    match row.get(28).copied().unwrap_or_default() / 0x08 % 8 {
        0 => HitChanceFormula::Always,
        1 | 2 => HitChanceFormula::UseActionAccuracy,
        3 | 4 => HitChanceFormula::UseAccuracy,
        5 => HitChanceFormula::UseAccuracyX25,
        6 => HitChanceFormula::UseAccuracyX15,
        7 => HitChanceFormula::UseAccuracyX05,
        _ => HitChanceFormula::Always,
    }
}

fn uses_hit_chance_table(row: &[u8]) -> bool {
    let flags = row.get(28).copied().unwrap_or_default();
    flags & 0x08 != 0 || (flags / 0x08 % 8) == 6
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
    Element::VALUES
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

fn parse_action_status_applications(row: &[u8]) -> Vec<ActionStatus> {
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

    let mut statuses = Vec::new();
    for (offset, name) in STATUS_ORDER.iter().enumerate() {
        let chance = row.get(46 + offset).copied().unwrap_or_default();
        if chance == 0 {
            continue;
        }
        if let Ok(status) = name.parse::<Status>() {
            statuses.push(ActionStatus {
                status,
                chance,
                stacks: if offset < 12 {
                    254
                } else {
                    row.get(59 + offset).copied().unwrap_or(254) as i32
                },
                ignores_resistance: chance == 255,
            });
        }
    }
    statuses
}

fn parse_action_status_flags(row: &[u8]) -> Vec<Status> {
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

    let status_flags_bytes = add_bytes(&[
        row.get(84).copied().unwrap_or_default(),
        row.get(85).copied().unwrap_or_default(),
        row.get(90).copied().unwrap_or_default(),
    ]);
    let mut statuses = Vec::new();
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

fn parse_overdrive_user(overdrive_info: u8) -> Option<Character> {
    if overdrive_info == 0 {
        return None;
    }
    match overdrive_info & 0b1111 {
        0 => Some(Character::Tidus),
        1 => Some(Character::Yuna),
        2 => Some(Character::Auron),
        3 => Some(Character::Kimahri),
        4 => Some(Character::Wakka),
        5 => Some(Character::Lulu),
        6 => Some(Character::Rikku),
        7 => Some(Character::Seymour),
        8 => Some(Character::Valefor),
        9 => Some(Character::Ifrit),
        10 => Some(Character::Ixion),
        11 => Some(Character::Shiva),
        12 => Some(Character::Bahamut),
        13 => Some(Character::Anima),
        14 => Some(Character::Yojimbo),
        15 => Some(Character::Cindy),
        _ => None,
    }
}

fn parse_action_target(
    row: &[u8],
    overdrive_user: Option<Character>,
    overdrive_index: i32,
) -> ActionTarget {
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

    if overdrive_index == 1 {
        return ActionTarget::None;
    }
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
            } else if overdrive_user == Some(Character::Wakka) {
                ActionTarget::RandomMonster
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
        if overdrive_user == Some(Character::Rikku) {
            return ActionTarget::RandomCharacter;
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

#[derive(Debug, Clone)]
struct MonsterActionOverride {
    action_key: String,
    actions_file: i32,
    action_id: usize,
    target: ActionTarget,
}

fn parse_monster_action_targets() -> HashMap<usize, HashMap<String, MonsterActionOverride>> {
    let parsed =
        serde_json::from_str::<HashMap<String, Vec<JsonMonsterAction>>>(MONSTER_ACTIONS_JSON)
            .expect("upstream monster_actions.json should parse for Rust CTB port");
    parsed
        .into_iter()
        .filter_map(|(monster_id, actions)| {
            let index = monster_id.strip_prefix('m')?.parse::<usize>().ok()?;
            let mut action_targets = HashMap::new();
            for action in actions {
                if action.target.is_empty() {
                    continue;
                }
                let Some(target) = parse_named_action_target(&action.target) else {
                    continue;
                };
                let action_key = stringify(&action.name);
                let base_alias = action_key.replace('-', "_");
                let alias = if action_targets.contains_key(&base_alias) {
                    format!("{}_{}", base_alias, stringify(&action.target))
                } else {
                    base_alias
                };
                action_targets
                    .entry(alias)
                    .or_insert(MonsterActionOverride {
                        action_key,
                        actions_file: action.actions_file,
                        action_id: action.action_id,
                        target,
                    });
            }
            Some((index, action_targets))
        })
        .collect()
}

fn parse_default_monster_actions() -> HashMap<String, String> {
    let mut actions = HashMap::new();
    for line in DEFAULT_MONSTER_ACTIONS_CSV.lines().skip(1) {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(4, ',');
        let Some(monster_id) = fields
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let _monster_name = fields.next();
        let _action_count = fields.next();
        let Some(action_field) = fields.next() else {
            continue;
        };
        let action = action_field
            .split('|')
            .next()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(stringify);
        if let Some(action) = action {
            actions.insert(monster_id.to_string(), action);
        }
    }
    actions
        .entry("ragora_2".to_string())
        .or_insert_with(|| "seed_cannon".to_string());
    actions
        .entry("larva".to_string())
        .or_insert_with(|| "thundara".to_string());
    actions
}

fn parse_named_action_target(value: &str) -> Option<ActionTarget> {
    match value {
        "" => Some(ActionTarget::None),
        "Self" => Some(ActionTarget::SelfTarget),
        "Counter Self" => Some(ActionTarget::CounterSelf),
        "Characters' Party" => Some(ActionTarget::CharactersParty),
        "Counter Characters' Party" => Some(ActionTarget::CounterCharactersParty),
        "Monsters' Party" => Some(ActionTarget::MonstersParty),
        "Single Character" => Some(ActionTarget::SingleCharacter),
        "Single Monster" => Some(ActionTarget::SingleMonster),
        "Random Character" | "Random Character (Not User) Damaged" => {
            Some(ActionTarget::RandomCharacter)
        }
        "Highest MP Character" => Some(ActionTarget::HighestMpCharacter),
        "Lowest HP Character" => Some(ActionTarget::LowestHpCharacter),
        "Highest HP Character" => Some(ActionTarget::HighestHpCharacter),
        "Highest Str Character" => Some(ActionTarget::HighestStrengthCharacter),
        "Lowest Mag Def Character" => Some(ActionTarget::LowestMagicDefenseCharacter),
        "Random Character Affected By Reflect" => {
            Some(ActionTarget::RandomCharacterWith(Status::Reflect))
        }
        "Random Character Affected By Zombie" => {
            Some(ActionTarget::RandomCharacterWith(Status::Zombie))
        }
        "Random Character Affected By Petrify" => {
            Some(ActionTarget::RandomCharacterWith(Status::Petrify))
        }
        "Random Character Affected By Death" => {
            Some(ActionTarget::RandomCharacterWith(Status::Death))
        }
        "Random Character Not Affected By Petrify" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Petrify))
        }
        "Random Character Not Affected By Doom" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Doom))
        }
        "Random Character Not Affected By Berserk" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Berserk))
        }
        "Random Character Not Affected By Confuse" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Confuse))
        }
        "Random Character Not Affected By Curse" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Curse))
        }
        "Random Character Not Affected By Poison" => {
            Some(ActionTarget::RandomCharacterWithout(Status::Poison))
        }
        "Random Character Not Affected By Auto-Life" => {
            Some(ActionTarget::RandomCharacterWithout(Status::AutoLife))
        }
        "Random Character Not Affected By Shell or Reflect" => Some(
            ActionTarget::RandomCharacterWithoutEither(Status::Shell, Status::Reflect),
        ),
        "Random Character Not Affected By Protect or Reflect" => Some(
            ActionTarget::RandomCharacterWithoutEither(Status::Protect, Status::Reflect),
        ),
        "Random Character Not Affected By Haste or Reflect" => Some(
            ActionTarget::RandomCharacterWithoutEither(Status::Haste, Status::Reflect),
        ),
        "Random Monster" => Some(ActionTarget::RandomMonster),
        "Random Monster Not Affected By Shell" => {
            Some(ActionTarget::RandomMonsterWithout(Status::Shell))
        }
        "Random Monster Not Affected By Protect" => {
            Some(ActionTarget::RandomMonsterWithout(Status::Protect))
        }
        "Random Monster Not Affected By Reflect" => {
            Some(ActionTarget::RandomMonsterWithout(Status::Reflect))
        }
        "Provoker" => Some(ActionTarget::Provoker),
        "Counter Single Character" => Some(ActionTarget::CounterSingleCharacter),
        "Counter Random Character" => Some(ActionTarget::CounterRandomCharacter),
        "Last Target" => Some(ActionTarget::LastTarget),
        "Counter Last Target" => Some(ActionTarget::CounterLastTarget),
        "Last Attacker" => Some(ActionTarget::LastAttacker),
        "Counter" => Some(ActionTarget::Counter),
        "All" => Some(ActionTarget::EitherParty),
        "Counter All" => Some(ActionTarget::CounterAll),
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
        let display_name = if *count > 0 {
            format!("{name}#{}", *count + 1)
        } else {
            name.clone()
        };
        if *count > 0 {
            key = format!("{key}_{}", *count + 1);
        }
        *count += 1;

        monsters.insert(
            key.clone(),
            MonsterStats {
                index,
                key,
                display_name,
                max_hp: add_bytes(&data[20..24]),
                max_mp: add_bytes(&data[24..28]),
                agility: data[36],
                immune_to_delay: data[41] & 0b00000001 != 0,
                armored: data[40] & 0b00000001 != 0,
                immune_to_percentage_damage: data[40] & 0b00000010 != 0,
                immune_to_life: data[40] & 0b00000100 != 0,
                immune_to_bribe: data[41] & 0b00000100 != 0,
                zanmato_level: data[402],
                immune_to_physical_damage: data[40] & 0b00100000 != 0,
                immune_to_magical_damage: data[40] & 0b01000000 != 0,
                immune_to_damage: data[40] & 0b10000000 != 0,
                strength: data[32] as i32,
                defense: data[33] as i32,
                magic: data[34] as i32,
                magic_defense: data[35] as i32,
                luck: data[37] as i32,
                evasion: data[38] as i32,
                accuracy: data[39] as i32,
                base_weapon_damage: data[176] as i32,
                elemental_affinities: parse_monster_elemental_affinities(&data),
                status_resistances: parse_monster_status_resistances(&data),
                auto_statuses: parse_monster_auto_statuses(&data),
                gil: add_bytes(&data[128..130]),
                normal_ap: add_bytes(&data[130..132]),
                overkill_ap: add_bytes(&data[132..134]),
                item_1: parse_monster_item_1(&data),
                item_2: parse_monster_item_2(&data),
                steal: parse_monster_steal(&data),
                bribe: parse_item_drop(&data, 170, 171, 172, false),
                forced_action: parse_monster_forced_action(&data),
                equipment_drop_chance: data[139],
                equipment: parse_monster_equipment(&data),
            },
        );
    }

    monsters
}

fn parse_monster_forced_action(data: &[u8]) -> Option<ActionData> {
    let forced_action_byte = add_bytes(&data[112..114]);
    if forced_action_byte == 0 {
        return None;
    }
    let file_id = forced_action_byte >> 12;
    let action_index = (forced_action_byte & 0b1111_1111_1111) as usize;
    let mut action = parse_action_from_file_index(file_id, action_index)?;
    action.target = ActionTarget::Provoker;
    Some(action)
}

fn parse_monster_equipment(data: &[u8]) -> MonsterEquipmentDropInfo {
    let slots_modifier = data[173];
    let max_ability_rolls_modifier = data[177];
    let autoabilities = parse_autoability_names();
    let mut ability_lists = empty_equipment_ability_lists();
    for (character, base_address) in drops_equipment_characters()
        .into_iter()
        .zip((178..371).step_by(32))
    {
        for (kind, type_offset) in [(EquipmentKind::Weapon, 0), (EquipmentKind::Armor, 16)] {
            let Some(character_lists) = ability_lists.get_mut(&kind) else {
                continue;
            };
            let Some(abilities) = character_lists.get_mut(&character) else {
                continue;
            };
            for ability_offset in 0..8 {
                let address = base_address + (ability_offset * 2) + type_offset;
                if data.get(address + 1).copied() != Some(128) {
                    continue;
                }
                if let Some(name) = data
                    .get(address)
                    .and_then(|index| autoabilities.get(*index as usize))
                    .cloned()
                {
                    abilities[ability_offset] = Some(name);
                }
            }
        }
    }
    MonsterEquipmentDropInfo {
        drop_chance: data[139],
        bonus_critical_chance: data[175],
        base_weapon_damage: data[176] as i32,
        slots_range: (0..8)
            .map(|roll| ((i32::from(slots_modifier) + roll - 4) / 4).clamp(1, 4) as u8)
            .collect(),
        max_ability_rolls_range: (0..8)
            .map(|roll| ((i32::from(max_ability_rolls_modifier) + roll - 4) / 8).max(0) as u8)
            .collect(),
        added_to_inventory: data[174] != 0,
        ability_lists,
    }
}

fn empty_equipment_ability_lists() -> HashMap<EquipmentKind, HashMap<Character, Vec<Option<String>>>>
{
    [EquipmentKind::Weapon, EquipmentKind::Armor]
        .into_iter()
        .map(|kind| {
            (
                kind,
                drops_equipment_characters()
                    .into_iter()
                    .map(|character| (character, vec![None; 8]))
                    .collect(),
            )
        })
        .collect()
}

fn drops_equipment_characters() -> Vec<Character> {
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

fn parse_autoability_names() -> Vec<String> {
    AUTOABILITIES_CSV
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.split_once(',')
                .map(|(name, _)| stringify_autoability(name))
        })
        .collect()
}

fn parse_monster_item_1(data: &[u8]) -> MonsterItemDropInfo {
    MonsterItemDropInfo {
        drop_chance: data[136],
        normal_common: parse_item_drop(data, 140, 141, 148, false),
        normal_rare: parse_item_drop(data, 142, 143, 149, true),
        overkill_common: parse_item_drop(data, 152, 153, 160, false),
        overkill_rare: parse_item_drop(data, 154, 155, 161, true),
    }
}

fn parse_monster_item_2(data: &[u8]) -> MonsterItemDropInfo {
    MonsterItemDropInfo {
        drop_chance: data[137],
        normal_common: parse_item_drop(data, 144, 145, 150, false),
        normal_rare: parse_item_drop(data, 146, 147, 151, true),
        overkill_common: parse_item_drop(data, 156, 157, 162, false),
        overkill_rare: parse_item_drop(data, 158, 159, 163, true),
    }
}

fn parse_monster_steal(data: &[u8]) -> MonsterStealInfo {
    MonsterStealInfo {
        base_chance: data[138],
        common: parse_item_drop(data, 164, 165, 168, false),
        rare: parse_item_drop(data, 166, 167, 169, true),
    }
}

fn parse_item_drop(
    data: &[u8],
    item_index: usize,
    sentinel_index: usize,
    quantity_index: usize,
    rare: bool,
) -> Option<ItemDrop> {
    if data.get(sentinel_index).copied() != Some(32) {
        return None;
    }
    Some(ItemDrop {
        item: item_name(data.get(item_index).copied()?),
        quantity: data.get(quantity_index).copied().unwrap_or_default(),
        rare,
    })
}

fn item_name(index: u8) -> String {
    ITEM_NAMES
        .get(index as usize)
        .copied()
        .unwrap_or("Unknown Item")
        .to_string()
}

const ITEM_NAMES: &[&str] = &[
    "Potion",
    "Hi-Potion",
    "X-Potion",
    "Mega-Potion",
    "Ether",
    "Turbo Ether",
    "Phoenix Down",
    "Mega Phoenix",
    "Elixir",
    "Megalixir",
    "Antidote",
    "Soft",
    "Eye Drops",
    "Echo Screen",
    "Holy Water",
    "Remedy",
    "Power Distiller",
    "Mana Distiller",
    "Speed Distiller",
    "Ability Distiller",
    "Al Bhed Potion",
    "Healing Water",
    "Tetra Element",
    "Antarctic Wind",
    "Arctic Wind",
    "Ice Gem",
    "Bomb Fragment",
    "Bomb Core",
    "Fire Gem",
    "Electro Marble",
    "Lightning Marble",
    "Lightning Gem",
    "Fish Scale",
    "Dragon Scale",
    "Water Gem",
    "Grenade",
    "Frag Grenade",
    "Sleeping Powder",
    "Dream Powder",
    "Silence Grenade",
    "Smoke Bomb",
    "Shadow Gem",
    "Shining Gem",
    "Blessed Gem",
    "Supreme Gem",
    "Poison Fang",
    "Silver Hourglass",
    "Gold Hourglass",
    "Candle of Life",
    "Petrify Grenade",
    "Farplane Shadow",
    "Farplane Wind",
    "Designer Wallet",
    "Dark Matter",
    "Chocobo Feather",
    "Chocobo Wing",
    "Lunar Curtain",
    "Light Curtain",
    "Star Curtain",
    "Healing Spring",
    "Mana Spring",
    "Stamina Spring",
    "Soul Spring",
    "Purifying Salt",
    "Stamina Tablet",
    "Mana Tablet",
    "Twin Stars",
    "Stamina Tonic",
    "Mana Tonic",
    "Three Stars",
    "Power Sphere",
    "Mana Sphere",
    "Speed Sphere",
    "Ability Sphere",
    "Fortune Sphere",
    "Attribute Sphere",
    "Special Sphere",
    "Skill Sphere",
    "Wht Magic Sphere",
    "Blk Magic Sphere",
    "Master Sphere",
    "Lv. 1 Key Sphere",
    "Lv. 2 Key Sphere",
    "Lv. 3 Key Sphere",
    "Lv. 4 Key Sphere",
    "HP Sphere",
    "MP Sphere",
    "Strength Sphere",
    "Defense Sphere",
    "Magic Sphere",
    "Magic Def Sphere",
    "Agility Sphere",
    "Evasion Sphere",
    "Accuracy Sphere",
    "Luck Sphere",
    "Clear Sphere",
    "Return Sphere",
    "Friend Sphere",
    "Teleport Sphere",
    "Warp Sphere",
    "Map",
    "Rename Card",
    "Musk",
    "Hypello Potion",
    "Shining Thorn",
    "Pendulum",
    "Amulet",
    "Door to Tomorrow",
    "Wings to Discovery",
    "Gambler's Spirit",
    "Underdog's Secret",
    "Winning Formula",
];

fn parse_monster_status_resistances(data: &[u8]) -> HashMap<Status, u8> {
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
    let mut resistances = STATUS_ORDER
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            Some((
                name.parse::<Status>().ok()?,
                data.get(47 + index).copied().unwrap_or_default(),
            ))
        })
        .collect::<HashMap<_, _>>();

    let immunities = u16::from(data.get(78).copied().unwrap_or_default())
        | (u16::from(data.get(79).copied().unwrap_or_default()) << 8);
    for (mut bit_index, name) in [
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
    ]
    .into_iter()
    .enumerate()
    {
        if bit_index > 3 {
            bit_index += 1;
        }
        let resistance = if immunities & (1 << bit_index) == 0 {
            0
        } else {
            255
        };
        if let Ok(status) = name.parse::<Status>() {
            resistances.insert(status, resistance);
        }
    }
    resistances
}

fn parse_monster_auto_statuses(data: &[u8]) -> Vec<Status> {
    const PYTHON_STATUS_ORDER: [&str; 47] = [
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
        "unused",
    ];
    let flags = (72..78)
        .enumerate()
        .map(|(position, index)| {
            u64::from(data.get(index).copied().unwrap_or_default()) << (position * 8)
        })
        .sum::<u64>();
    PYTHON_STATUS_ORDER
        .iter()
        .enumerate()
        .filter_map(|(mut bit_index, name)| {
            if bit_index > 14 {
                bit_index += 4;
            }
            if bit_index > 28 {
                bit_index += 3;
            }
            if bit_index > 35 {
                bit_index += 1;
            }
            if bit_index > 46 || flags & (1_u64 << bit_index) == 0 {
                return None;
            }
            name.parse::<Status>().ok()
        })
        .collect()
}

fn parse_monster_elemental_affinities(data: &[u8]) -> HashMap<Element, ElementalAffinity> {
    Element::VALUES
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

fn stringify_autoability(value: &str) -> String {
    value
        .to_ascii_lowercase()
        .replace("->", "to")
        .replace([' ', '-'], "_")
        .replace(['+', '%', '(', ')', '\''], "")
        .trim_matches('_')
        .to_string()
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
        ActionTarget, DamageFormula, DamageType, HitChanceFormula,
    };
    use crate::model::{
        AutoAbility, Buff, Character, Element, ElementalAffinity, EncounterCondition, Status,
    };

    #[test]
    fn loads_boss_formations_from_upstream_data() {
        let tanker = boss_or_simulated_formation("tanker").unwrap();
        assert_eq!(tanker.display_name, "Tanker");
        assert_eq!(tanker.forced_condition, Some(EncounterCondition::Normal));
        assert_eq!(tanker.forced_party.as_deref(), Some("ta"));
        assert_eq!(tanker.monsters[0], "tanker");
        assert_eq!(tanker.monsters.len(), 8);
        let tanker_upper = boss_or_simulated_formation("TANKER").unwrap();
        assert_eq!(tanker_upper.display_name, "Tanker");
    }

    #[test]
    fn loads_random_formations_from_upstream_data() {
        let formation = random_formation("kilika_woods", 0).unwrap();
        assert_eq!(formation.zone_display_name.as_deref(), Some("Kilika Woods"));
        assert!(formation.is_random);
        assert!(!formation.monsters.is_empty());
        let first_monster = monster_stats(&formation.monsters[0]).unwrap();
        assert!(formation.display_name.contains(&first_monster.display_name));
        assert!(super::has_random_zone("Kilika Woods"));
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
        assert_eq!(
            monster_stats("SAHAGIN_4").unwrap().display_name,
            "Sahagin#4"
        );
        assert_eq!(sahagin.agility, 5);
        assert_eq!(sahagin.strength, 3);
        assert_eq!(sahagin.base_weapon_damage, 0);
        assert!(!sahagin.armored);
        assert_eq!(
            sahagin.elemental_affinities.get(&Element::Thunder),
            Some(&ElementalAffinity::Weak)
        );
        let tutorial_chest = monster_stats("???_3").unwrap();
        assert_eq!(tutorial_chest.display_name, "???#3");
        let biran = monster_stats("biran_ronso").unwrap();
        assert_eq!(biran.equipment.drop_chance, 255);
        assert!(!biran.equipment.slots_range.is_empty());
        let forced_monster = super::monsters()
            .values()
            .find(|monster| monster.forced_action.is_some())
            .expect("upstream monster data should include forced actions");
        let forced_action = super::monster_action_data(&forced_monster.key, "forced_action")
            .expect("forced_action should resolve through monster action data");
        assert_ne!(forced_action.key, "forced_action");
        assert_eq!(forced_action.target, ActionTarget::Provoker);
    }

    #[test]
    fn loads_character_stats_from_upstream_data() {
        let tidus = character_stats(Character::Tidus).unwrap();
        assert_eq!(tidus.index, 0);
        assert_eq!(tidus.max_hp, 520);
        assert_eq!(tidus.agility, 10);
        assert_eq!(tidus.strength, 15);
        assert_eq!(tidus.accuracy, 10);
        assert_eq!(tidus.luck, 18);
        assert_eq!(tidus.starting_s_lv, 0);
        assert_eq!(tidus.base_weapon_damage, 16);
        assert_eq!(tidus.equipment_crit, 6);
        let auron = character_stats(Character::Auron).unwrap();
        assert!(auron.weapon_abilities.contains(&AutoAbility::Piercing));
        let yuna = character_stats(Character::Yuna).unwrap();
        assert_eq!(yuna.starting_s_lv, 2);
    }

    #[test]
    fn resolves_item_names_and_prices_from_upstream_data() {
        assert_eq!(
            super::item_name_by_key("phoenix_down"),
            Some("Phoenix Down")
        );
        assert_eq!(super::item_name_by_key("Water Gem"), Some("Water Gem"));
        assert_eq!(super::item_price("Phoenix Down"), Some(100));
        assert_eq!(super::item_price("antidote"), Some(50));
        assert_eq!(super::autoability_gil_value("first_strike"), Some(6000));
        assert_eq!(super::autoability_gil_value("overdrive_->_ap"), Some(25000));
        assert_eq!(
            super::autoability_name_by_key("auto_haste"),
            Some("Auto-Haste")
        );
        assert_eq!(
            super::autoability_name_by_key("overdrive_->_ap"),
            Some("Overdrive -> AP")
        );
        assert_eq!(
            super::equipment_name(super::EquipmentKind::Weapon, Character::Tidus, 48),
            Some("Hunter's Sword".to_string())
        );
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

        let drain_touch = action_data("drain_touch").unwrap();
        assert!(drain_touch.drains);

        let phoenix_down = action_data("phoenix_down").unwrap();
        assert!(phoenix_down.misses_if_target_alive);

        let potion = action_data("potion").unwrap();
        assert!(potion.heals);
        assert!(potion.affected_by_alchemy);

        let self_destruct = action_data("self-destruct").unwrap();
        assert!(self_destruct.destroys_user);

        let attack = action_data("attack").unwrap();
        assert!(attack.can_use_in_combat);
        assert_eq!(attack.damage_formula, DamageFormula::Strength);
        assert_eq!(attack.damage_type, DamageType::Physical);
        assert_eq!(attack.hit_chance_formula, HitChanceFormula::UseAccuracy);
        assert!(attack.uses_hit_chance_table);
        assert!(attack.can_crit);
        assert!(attack.adds_equipment_crit);
        assert!(attack.damages_hp);
        assert!(attack.uses_weapon_properties);

        let counter = action_data("counter").unwrap();
        assert_eq!(counter.target, ActionTarget::Counter);

        let auto_potion = action_data("auto_potion").unwrap();
        assert_eq!(auto_potion.target, ActionTarget::CounterSelf);
        assert!(auto_potion.affected_by_alchemy);

        let auto_med = action_data("auto_med").unwrap();
        assert_eq!(auto_med.target, ActionTarget::CounterSelf);
        assert!(auto_med.removes_statuses);
        assert!(auto_med.can_use_in_combat);

        let auto_life_counter = action_data("auto-life_counter").unwrap();
        assert_eq!(auto_life_counter.target, ActionTarget::CounterSelf);
        assert!(auto_life_counter.can_use_in_combat);

        let auto_phoenix = action_data("auto_phoenix").unwrap();
        assert_eq!(auto_phoenix.target, ActionTarget::CounterSingleCharacter);
        assert!(auto_phoenix.misses_if_target_alive);
        assert!(auto_phoenix.can_use_in_combat);

        let delta_attack = action_data("delta_attack").unwrap();
        assert_eq!(delta_attack.target, ActionTarget::MonstersParty);
        assert!(delta_attack.can_use_in_combat);

        let does_nothing = action_data("does_nothing").unwrap();
        assert_eq!(does_nothing.target, action_data("switch").unwrap().target);

        let bribe = action_data("bribe").unwrap();
        assert_eq!(bribe.target, ActionTarget::SingleMonster);
        assert!(bribe.can_use_in_combat);

        let designer_wallet = action_data("designer_wallet").unwrap();
        assert!(!designer_wallet.can_use_in_combat);

        let swordplay = action_data("swordplay").unwrap();
        assert_eq!(swordplay.overdrive_user, Some(Character::Tidus));
        assert_eq!(swordplay.overdrive_index, 1);
        assert_eq!(swordplay.target, ActionTarget::None);

        let spiral_cut = action_data("spiral_cut").unwrap();
        assert_eq!(spiral_cut.overdrive_user, Some(Character::Tidus));
        assert_eq!(spiral_cut.overdrive_index, 2);

        let fire_shot = action_data("fire_shot").unwrap();
        assert_eq!(fire_shot.overdrive_user, Some(Character::Wakka));
        assert_eq!(fire_shot.target, ActionTarget::RandomMonster);

        let nulall = action_data("nulall").unwrap();
        assert_eq!(nulall.overdrive_user, Some(Character::Rikku));
        assert_eq!(nulall.target, ActionTarget::RandomCharacter);

        let grenade = action_data("grenade").unwrap();
        assert!(matches!(
            grenade.damage_formula,
            DamageFormula::Fixed | DamageFormula::FixedNoVariance
        ));
        assert_eq!(grenade.hit_chance_formula, HitChanceFormula::Always);
        assert!(grenade.damages_hp);
        assert!(!grenade.uses_weapon_properties);

        let fire = action_data("fire").unwrap();
        assert!(fire.elements.contains(&Element::Fire));
        assert!(fire.affected_by_reflect);
        assert!(fire.affected_by_silence);
        assert!(fire.copied_by_copycat);

        let steal = action_data("steal").unwrap();
        assert!(steal.steals_item);
        assert!(!steal.steals_gil);

        let nab_gil = action_data("nab_gil").unwrap();
        assert!(!nab_gil.steals_item);
        assert!(nab_gil.steals_gil);

        let entrust = action_data("entrust").unwrap();
        assert!(entrust.empties_od_bar);

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

        let actions = super::monster_action_names("sinspawn_echuilles").unwrap();
        assert!(actions.contains(&"blender".to_string()));
    }

    #[test]
    fn duplicate_monster_action_names_keep_python_first_target() {
        let attack = super::monster_action_data("dingo", "attack").unwrap();
        assert_eq!(attack.target, ActionTarget::RandomCharacter);
    }

    #[test]
    fn duplicate_monster_action_aliases_include_counter_target_suffix_like_python() {
        let monster_key = super::monsters()
            .iter()
            .find_map(|(key, monster)| (monster.index == 314).then_some(key.as_str()))
            .unwrap();
        let names = super::monster_action_names(monster_key).unwrap();

        assert!(names.contains(&"10000_needles".to_string()));
        assert!(names.contains(&"10000_needles_counter_random_character".to_string()));
        assert_eq!(
            super::monster_action_data(monster_key, "10000_needles")
                .unwrap()
                .target,
            ActionTarget::RandomCharacter
        );
        assert_eq!(
            super::monster_action_data(monster_key, "10000_needles_counter_random_character")
                .unwrap()
                .target,
            ActionTarget::CounterRandomCharacter
        );
    }

    #[test]
    fn loads_default_monster_actions_from_python_enemy_candidates() {
        assert_eq!(
            super::default_monster_action("chocobo_eater"),
            Some("attack")
        );
        assert_eq!(
            super::default_monster_action("sinspawn_geneaux"),
            Some("water")
        );
        assert_eq!(
            super::default_monster_action("ragora_2"),
            Some("seed_cannon")
        );
        assert_eq!(super::default_monster_action("larva"), Some("thundara"));
    }

    #[test]
    fn attack_wakka_alias_targets_wakka_for_scripted_condor_action() {
        let action = super::action_data("attack_wakka").unwrap();
        assert_eq!(action.target, ActionTarget::Character(Character::Wakka));
        assert!(action.can_use_in_combat);
    }

    #[test]
    fn preserves_counter_monster_action_target_types() {
        assert_eq!(
            super::parse_named_action_target("Counter Self"),
            Some(ActionTarget::CounterSelf)
        );
        assert_eq!(
            super::parse_named_action_target("Counter Random Character"),
            Some(ActionTarget::CounterRandomCharacter)
        );
        assert_eq!(
            super::parse_named_action_target("Counter Characters' Party"),
            Some(ActionTarget::CounterCharactersParty)
        );
        assert_eq!(
            super::parse_named_action_target("Counter All"),
            Some(ActionTarget::CounterAll)
        );
        assert_eq!(
            super::parse_named_action_target("Counter Last Target"),
            Some(ActionTarget::CounterLastTarget)
        );
    }

    #[test]
    fn parses_monster_auto_status_bits_in_python_order() {
        let mut data = vec![0_u8; 78];
        data[72] = 0b0000_0001;

        assert_eq!(
            super::parse_monster_auto_statuses(&data),
            vec![Status::Death]
        );
    }
}

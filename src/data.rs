use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::model::EncounterCondition;

const FORMATIONS_JSON: &str =
    include_str!("../data/formations.json");
const MONSTER_DATA_HD_CSV: &str =
    include_str!("../data/ffx_mon_data_hd.csv");
const COMMAND_CSV: &str =
    include_str!("../data/ffx_command.csv");
const TEXT_CHARACTERS_CSV: &str =
    include_str!("../data/text_characters.csv");

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
    pub key: String,
    pub agility: u8,
    pub max_hp: i32,
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

static FORMATIONS: OnceLock<FormationsFile> = OnceLock::new();
static MONSTERS: OnceLock<HashMap<String, MonsterStats>> = OnceLock::new();
static ACTION_RANKS: OnceLock<HashMap<String, i32>> = OnceLock::new();

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

pub fn action_rank(name: &str) -> Option<i32> {
    action_ranks().get(name).copied()
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

fn action_ranks() -> &'static HashMap<String, i32> {
    ACTION_RANKS.get_or_init(parse_action_ranks)
}

fn parse_action_ranks() -> HashMap<String, i32> {
    let text_characters = parse_text_characters();
    let mut rows = COMMAND_CSV
        .lines()
        .map(parse_hex_csv_line)
        .filter(|row| !row.is_empty())
        .collect::<Vec<_>>();
    let Some(string_data) = rows.pop() else {
        return HashMap::new();
    };

    let mut ranks = HashMap::new();
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
        ranks.insert(key, row[36] as i32);
    }

    if let Some(rank) = ranks.get("attack").copied() {
        ranks.insert("attacknocrit".to_string(), rank);
    }
    if let Some(rank) = ranks.get("quick_hit").copied() {
        ranks.insert("quick_hit_hd".to_string(), rank);
        ranks.insert("quick_hit_ps2".to_string(), 1);
    }

    ranks
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
                key,
                max_hp: add_bytes(&data[20..24]),
                agility: data[36],
            },
        );
    }

    monsters
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
    use super::{boss_or_simulated_formation, monster_stats, random_formation};
    use crate::model::EncounterCondition;

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
        assert_eq!(geosgaeno.max_hp, 32_767);
        assert_eq!(geosgaeno.agility, 48);
        let sahagin = monster_stats("sahagin_4").unwrap();
        assert_eq!(sahagin.max_hp, 100);
        assert_eq!(sahagin.agility, 5);
    }

    #[test]
    fn loads_action_ranks_from_upstream_command_data() {
        assert_eq!(super::action_rank("attack"), Some(3));
        assert_eq!(super::action_rank("defend"), Some(2));
        assert_eq!(super::action_rank("cheer"), Some(2));
        assert_eq!(super::action_rank("energy_ray"), Some(8));
        assert_eq!(super::action_rank("quick_hit_ps2"), Some(1));
    }
}

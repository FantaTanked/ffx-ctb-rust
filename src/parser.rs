use crate::model::{Character, MonsterSlot};
use crate::script::edit_action_line;

const CHARACTER_VALUES: &str = "tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonsterActionActor {
    Slot(MonsterSlot),
    Name(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    Blank,
    Comment(String),
    Directive(String),
    Encounter {
        name: String,
        multizone: bool,
        zones: Vec<String>,
    },
    CharacterAction {
        actor: Character,
        action: String,
        args: Vec<String>,
    },
    MonsterAction {
        actor: MonsterActionActor,
        action: String,
        args: Vec<String>,
    },
    Equip {
        kind: String,
        args: Vec<String>,
    },
    Summon {
        aeon: String,
    },
    Spawn {
        monster: String,
        slot: usize,
        forced_ctb: Option<i32>,
    },
    Element {
        args: Vec<String>,
    },
    Status {
        args: Vec<String>,
    },
    Stat {
        args: Vec<String>,
    },
    Party {
        initials: String,
    },
    Heal {
        args: Vec<String>,
    },
    Ap {
        args: Vec<String>,
    },
    Death {
        character: Character,
    },
    Compatibility {
        amount: String,
    },
    YojimboTurn {
        action: String,
        monster: String,
        overdrive: bool,
    },
    MagusTurn {
        sister: String,
        command: String,
    },
    EncountersCount {
        name: String,
        amount: String,
    },
    Inventory {
        args: Vec<String>,
    },
    Walk {
        zone: String,
        steps: String,
        continue_previous_zone: bool,
    },
    EndEncounter,
    AdvanceRng {
        index: u32,
        amount: u32,
    },
    Unknown {
        edited_line: String,
    },
    ParserError {
        message: String,
    },
}

pub fn parse_raw_action_line(raw_line: &str) -> ParsedCommand {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return ParsedCommand::Blank;
    }
    if raw_line.starts_with('#') {
        return ParsedCommand::Comment(raw_line.to_string());
    }
    if raw_line.starts_with('/') {
        return ParsedCommand::Directive(raw_line.to_string());
    }
    parse_edited_action_line(&edit_action_line(raw_line))
}

pub fn parse_edited_action_line(edited_line: &str) -> ParsedCommand {
    let normalized_line = edited_line.to_ascii_lowercase();
    let words: Vec<&str> = normalized_line.split_whitespace().collect();
    let Some(command) = words.first() else {
        return ParsedCommand::Blank;
    };

    match *command {
        "encounter" => parse_encounter(&words),
        "action" => parse_character_action(&words),
        "monsteraction" => parse_monster_action(&words),
        "equip" => ParsedCommand::Equip {
            kind: words.get(1).copied().unwrap_or_default().to_string(),
            args: words
                .iter()
                .skip(2)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "summon" => ParsedCommand::Summon {
            aeon: words.get(1).copied().unwrap_or_default().to_string(),
        },
        "spawn" => parse_spawn(&words),
        "element" => ParsedCommand::Element {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "status" => ParsedCommand::Status {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "stat" => ParsedCommand::Stat {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "party" => ParsedCommand::Party {
            initials: words.get(1).copied().unwrap_or_default().to_string(),
        },
        "heal" => ParsedCommand::Heal {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "ap" => ParsedCommand::Ap {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "death" => ParsedCommand::Death {
            character: words
                .get(1)
                .and_then(|word| word.parse::<Character>().ok())
                .unwrap_or(Character::Unknown),
        },
        "compatibility" => ParsedCommand::Compatibility {
            amount: words.get(1).copied().unwrap_or_default().to_string(),
        },
        "yojimboturn" => parse_yojimbo_turn(&words),
        "magusturn" => parse_magus_turn(&words),
        "encounters_count" => ParsedCommand::EncountersCount {
            name: words.get(1).copied().unwrap_or_default().to_string(),
            amount: words.get(2).copied().unwrap_or_default().to_string(),
        },
        "inventory" => ParsedCommand::Inventory {
            args: words
                .iter()
                .skip(1)
                .map(|word| (*word).to_string())
                .collect(),
        },
        "walk" => parse_walk(&words),
        "endencounter" => ParsedCommand::EndEncounter,
        "end"
            if words
                .get(1)
                .is_some_and(|word| word.eq_ignore_ascii_case("encounter")) =>
        {
            ParsedCommand::EndEncounter
        }
        "roll" | "advance" | "waste" => parse_advance_rng(&words),
        _ => ParsedCommand::Unknown {
            edited_line: edited_line.to_string(),
        },
    }
}

fn parse_walk(words: &[&str]) -> ParsedCommand {
    let (Some(zone), Some(steps)) = (words.get(1), words.get(2)) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: walk [zone] [steps] (continue previous zone)".to_string(),
        };
    };
    ParsedCommand::Walk {
        zone: (*zone).to_string(),
        steps: (*steps).to_string(),
        continue_previous_zone: words
            .get(3)
            .is_some_and(|word| *word == "true" || *word == "cpz"),
    }
}

fn parse_yojimbo_turn(words: &[&str]) -> ParsedCommand {
    let (Some(action), Some(monster)) = (words.get(1), words.get(2)) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: yojimboturn [action] [monster] (overdrive)".to_string(),
        };
    };
    ParsedCommand::YojimboTurn {
        action: (*action).to_string(),
        monster: (*monster).to_string(),
        overdrive: words.get(3).is_some_and(|word| *word == "overdrive"),
    }
}

fn parse_magus_turn(words: &[&str]) -> ParsedCommand {
    let Some(sister) = words.get(1) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: magusturn [name] (command)".to_string(),
        };
    };
    ParsedCommand::MagusTurn {
        sister: (*sister).to_string(),
        command: words.get(2).copied().unwrap_or_default().to_string(),
    }
}

fn parse_encounter(words: &[&str]) -> ParsedCommand {
    let multizone = words
        .get(1)
        .is_some_and(|word| word.eq_ignore_ascii_case("multizone"));
    let name_index = if multizone { 2 } else { 1 };
    let zones = if multizone {
        words
            .iter()
            .skip(2)
            .map(|word| (*word).to_string())
            .collect()
    } else {
        Vec::new()
    };
    ParsedCommand::Encounter {
        name: words
            .get(name_index)
            .copied()
            .unwrap_or_default()
            .to_string(),
        multizone,
        zones,
    }
}

fn parse_character_action(words: &[&str]) -> ParsedCommand {
    let (Some(actor_name), Some(action)) = (words.get(1), words.get(2)) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: action [character] [action] [target]".to_string(),
        };
    };
    let Ok(actor) = actor_name.parse::<Character>() else {
        return ParsedCommand::ParserError {
            message: format!(
                "Error: character can only be one of these values: {CHARACTER_VALUES}"
            ),
        };
    };
    ParsedCommand::CharacterAction {
        actor,
        action: (*action).to_string(),
        args: words
            .iter()
            .skip(3)
            .map(|word| (*word).to_string())
            .collect(),
    }
}

fn parse_monster_action(words: &[&str]) -> ParsedCommand {
    let Some(actor_token) = words.get(1) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: monsteraction [monster slot/name] (action)".to_string(),
        };
    };
    let actor = parse_monster_slot_token(actor_token)
        .map(MonsterActionActor::Slot)
        .unwrap_or_else(|| MonsterActionActor::Name((*actor_token).to_string()));
    ParsedCommand::MonsterAction {
        actor,
        action: words.get(2).copied().unwrap_or_default().to_string(),
        args: words
            .iter()
            .skip(3)
            .map(|word| (*word).to_string())
            .collect(),
    }
}

fn parse_spawn(words: &[&str]) -> ParsedCommand {
    let (Some(monster), Some(slot_token)) = (words.get(1), words.get(2)) else {
        return ParsedCommand::ParserError {
            message: "Error: Usage: spawn [monster name] [slot] (forced ctb)".to_string(),
        };
    };
    let Ok(slot) = slot_token.parse::<usize>() else {
        return ParsedCommand::ParserError {
            message: "Error: Slot must be an integer".to_string(),
        };
    };
    ParsedCommand::Spawn {
        monster: (*monster).to_string(),
        slot,
        forced_ctb: words.get(3).and_then(|value| value.parse::<i32>().ok()),
    }
}

fn parse_monster_slot_token(slot_token: &str) -> Option<MonsterSlot> {
    let slot = slot_token.parse::<MonsterSlot>().ok()?;
    (1..=8).contains(&slot.0).then_some(slot)
}

fn parse_advance_rng(words: &[&str]) -> ParsedCommand {
    let Some(index_token) = words.get(1) else {
        return ParsedCommand::ParserError {
            message: "Error: rng needs to be an integer".to_string(),
        };
    };
    let Ok(index) = index_token
        .strip_prefix("rng")
        .unwrap_or(index_token)
        .parse::<i32>()
    else {
        return ParsedCommand::ParserError {
            message: "Error: rng needs to be an integer".to_string(),
        };
    };
    let amount = match words.get(2) {
        Some(value) => match value.parse::<i32>() {
            Ok(amount) => amount,
            Err(_) => {
                return ParsedCommand::ParserError {
                    message: "Error: amount needs to be an integer".to_string(),
                }
            }
        },
        None => 1,
    };
    if !(0..68).contains(&index) {
        return ParsedCommand::ParserError {
            message: format!("Error: Can't advance rng index {index}"),
        };
    }
    if amount < 0 {
        return ParsedCommand::ParserError {
            message: "Error: amount needs to be an greater or equal to 0".to_string(),
        };
    }
    if amount > 200 {
        return ParsedCommand::ParserError {
            message: "Error: Can't advance rng more than 200 times".to_string(),
        };
    }
    ParsedCommand::AdvanceRng {
        index: index as u32,
        amount: amount as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_raw_action_line, MonsterActionActor, ParsedCommand};
    use crate::model::{Character, MonsterSlot};

    #[test]
    fn parses_common_ctb_lines() {
        assert_eq!(
            parse_raw_action_line("encounter multizone ruins"),
            ParsedCommand::Encounter {
                name: "ruins".to_string(),
                multizone: true,
                zones: vec!["ruins".to_string()],
            }
        );
        assert_eq!(
            parse_raw_action_line("tidus attack m1"),
            ParsedCommand::CharacterAction {
                actor: Character::Tidus,
                action: "attack".to_string(),
                args: vec!["m1".to_string()],
            }
        );
        assert_eq!(
            parse_raw_action_line("Tidus Attack M1"),
            ParsedCommand::CharacterAction {
                actor: Character::Tidus,
                action: "attack".to_string(),
                args: vec!["m1".to_string()],
            }
        );
        assert_eq!(
            parse_raw_action_line("m7 wings_flicker"),
            ParsedCommand::MonsterAction {
                actor: MonsterActionActor::Slot(MonsterSlot(7)),
                action: "wings_flicker".to_string(),
                args: Vec::new(),
            }
        );
        assert_eq!(
            parse_raw_action_line("monsteraction 7 wings_flicker"),
            ParsedCommand::MonsterAction {
                actor: MonsterActionActor::Name("7".to_string()),
                action: "wings_flicker".to_string(),
                args: Vec::new(),
            }
        );
        assert_eq!(
            parse_raw_action_line("monsteraction gandarewa thunder"),
            ParsedCommand::MonsterAction {
                actor: MonsterActionActor::Name("gandarewa".to_string()),
                action: "thunder".to_string(),
                args: Vec::new(),
            }
        );
        assert_eq!(
            parse_raw_action_line("roll rng20 x3"),
            ParsedCommand::AdvanceRng {
                index: 20,
                amount: 3
            }
        );
        assert_eq!(
            parse_raw_action_line("magusturn cindy fight"),
            ParsedCommand::MagusTurn {
                sister: "cindy".to_string(),
                command: "fight".to_string(),
            }
        );
        assert_eq!(
            parse_raw_action_line("roll rng4"),
            ParsedCommand::AdvanceRng {
                index: 4,
                amount: 1
            }
        );
        assert_eq!(
            parse_raw_action_line("spawn sinscale_3 4 -2"),
            ParsedCommand::Spawn {
                monster: "sinscale_3".to_string(),
                slot: 4,
                forced_ctb: Some(-2),
            }
        );
        assert_eq!(
            parse_raw_action_line("end encounter"),
            ParsedCommand::EndEncounter
        );
        assert_eq!(
            parse_raw_action_line("death tidus"),
            ParsedCommand::Death {
                character: Character::Tidus
            }
        );
        assert_eq!(
            parse_raw_action_line("compatibility +10"),
            ParsedCommand::Compatibility {
                amount: "+10".to_string()
            }
        );
        assert_eq!(
            parse_raw_action_line("ap tidus 5"),
            ParsedCommand::Ap {
                args: vec!["tidus".to_string(), "5".to_string()]
            }
        );
        assert_eq!(
            parse_raw_action_line("yojimboturn zanmato piranha overdrive"),
            ParsedCommand::YojimboTurn {
                action: "zanmato".to_string(),
                monster: "piranha".to_string(),
                overdrive: true,
            }
        );
        assert_eq!(
            parse_raw_action_line("encounters_count total 5"),
            ParsedCommand::EncountersCount {
                name: "total".to_string(),
                amount: "5".to_string()
            }
        );
        assert_eq!(
            parse_raw_action_line("inventory get gil 50"),
            ParsedCommand::Inventory {
                args: vec!["get".to_string(), "gil".to_string(), "50".to_string()]
            }
        );
        assert_eq!(
            parse_raw_action_line("walk kilika_woods 30 cpz"),
            ParsedCommand::Walk {
                zone: "kilika_woods".to_string(),
                steps: "30".to_string(),
                continue_previous_zone: true,
            }
        );
    }

    #[test]
    fn keeps_comments_and_directives_out_of_event_parsing() {
        assert_eq!(
            parse_raw_action_line("# tidus attack"),
            ParsedCommand::Comment("# tidus attack".to_string())
        );
        assert_eq!(
            parse_raw_action_line("/usage"),
            ParsedCommand::Directive("/usage".to_string())
        );
        assert_eq!(
            parse_raw_action_line("/foo  "),
            ParsedCommand::Directive("/foo  ".to_string())
        );
        assert_eq!(
            parse_raw_action_line(" # tidus attack"),
            ParsedCommand::Unknown {
                edited_line: " # tidus attack".to_string()
            }
        );
        assert_eq!(
            parse_raw_action_line(" /usage"),
            ParsedCommand::Unknown {
                edited_line: " /usage".to_string()
            }
        );
        assert_eq!(parse_raw_action_line(""), ParsedCommand::Blank);
    }

    #[test]
    fn character_action_parse_errors_match_python() {
        assert_eq!(
            parse_raw_action_line("action nope attack"),
            ParsedCommand::ParserError {
                message: "Error: character can only be one of these values: tidus, yuna, auron, kimahri, wakka, lulu, rikku, seymour, valefor, ifrit, ixion, shiva, bahamut, anima, yojimbo, cindy, sandy, mindy, unknown".to_string(),
            }
        );
        assert_eq!(
            parse_raw_action_line("action tidus"),
            ParsedCommand::ParserError {
                message: "Error: Usage: action [character] [action] [target]".to_string(),
            }
        );
    }

    #[test]
    fn monster_action_missing_actor_usage_matches_python() {
        assert_eq!(
            parse_raw_action_line("monsteraction"),
            ParsedCommand::ParserError {
                message: "Error: Usage: monsteraction [monster slot/name] (action)".to_string(),
            }
        );
    }
}

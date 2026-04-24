use crate::model::{Character, MonsterSlot};
use crate::script::edit_action_line;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCommand {
    Blank,
    Comment(String),
    Directive(String),
    Encounter {
        name: String,
        multizone: bool,
    },
    CharacterAction {
        actor: Character,
        action: String,
        args: Vec<String>,
    },
    MonsterAction {
        slot: MonsterSlot,
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
        slot: MonsterSlot,
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
    EndEncounter,
    AdvanceRng {
        index: u32,
        amount: u32,
    },
    Unknown {
        edited_line: String,
    },
}

pub fn parse_raw_action_line(raw_line: &str) -> ParsedCommand {
    let trimmed = raw_line.trim();
    if trimmed.is_empty() {
        return ParsedCommand::Blank;
    }
    if trimmed.starts_with('#') {
        return ParsedCommand::Comment(raw_line.to_string());
    }
    if trimmed.starts_with('/') {
        return ParsedCommand::Directive(trimmed.to_string());
    }
    parse_edited_action_line(&edit_action_line(raw_line))
}

pub fn parse_edited_action_line(edited_line: &str) -> ParsedCommand {
    let words: Vec<&str> = edited_line.split_whitespace().collect();
    let Some(command) = words.first().map(|word| word.to_ascii_lowercase()) else {
        return ParsedCommand::Blank;
    };

    match command.as_str() {
        "encounter" => parse_encounter(&words),
        "action" => parse_character_action(&words).unwrap_or_else(|| ParsedCommand::Unknown {
            edited_line: edited_line.to_string(),
        }),
        "monsteraction" => parse_monster_action(&words).unwrap_or_else(|| ParsedCommand::Unknown {
            edited_line: edited_line.to_string(),
        }),
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
        "spawn" => parse_spawn(&words).unwrap_or_else(|| ParsedCommand::Unknown {
            edited_line: edited_line.to_string(),
        }),
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
        "endencounter" => ParsedCommand::EndEncounter,
        "roll" | "advance" | "waste" => {
            parse_advance_rng(&words).unwrap_or_else(|| ParsedCommand::Unknown {
                edited_line: edited_line.to_string(),
            })
        }
        _ => ParsedCommand::Unknown {
            edited_line: edited_line.to_string(),
        },
    }
}

fn parse_encounter(words: &[&str]) -> ParsedCommand {
    let multizone = words
        .get(1)
        .is_some_and(|word| word.eq_ignore_ascii_case("multizone"));
    let name_index = if multizone { 2 } else { 1 };
    ParsedCommand::Encounter {
        name: words
            .get(name_index)
            .copied()
            .unwrap_or("unknown")
            .to_string(),
        multizone,
    }
}

fn parse_character_action(words: &[&str]) -> Option<ParsedCommand> {
    let actor = words.get(1)?.parse::<Character>().ok()?;
    Some(ParsedCommand::CharacterAction {
        actor,
        action: words.get(2).copied().unwrap_or_default().to_string(),
        args: words
            .iter()
            .skip(3)
            .map(|word| (*word).to_string())
            .collect(),
    })
}

fn parse_monster_action(words: &[&str]) -> Option<ParsedCommand> {
    let slot = words.get(1)?.parse::<MonsterSlot>().ok()?;
    Some(ParsedCommand::MonsterAction {
        slot,
        action: words.get(2).copied().unwrap_or_default().to_string(),
        args: words
            .iter()
            .skip(3)
            .map(|word| (*word).to_string())
            .collect(),
    })
}

fn parse_spawn(words: &[&str]) -> Option<ParsedCommand> {
    let slot_token = words.get(2)?;
    let slot = slot_token
        .parse::<MonsterSlot>()
        .or_else(|_| slot_token.parse::<usize>().map(MonsterSlot).map_err(|_| ()))
        .ok()?;
    if !(1..=8).contains(&slot.0) {
        return None;
    }
    Some(ParsedCommand::Spawn {
        monster: words.get(1)?.to_string(),
        slot,
        forced_ctb: words.get(3).and_then(|value| value.parse::<i32>().ok()),
    })
}

fn parse_advance_rng(words: &[&str]) -> Option<ParsedCommand> {
    let index_token = words.get(1)?;
    let index = index_token
        .strip_prefix("rng")
        .unwrap_or(index_token)
        .parse()
        .ok()?;
    Some(ParsedCommand::AdvanceRng {
        index,
        amount: words
            .get(2)
            .and_then(|value| value.parse().ok())
            .unwrap_or(1),
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_raw_action_line, ParsedCommand};
    use crate::model::{Character, MonsterSlot};

    #[test]
    fn parses_common_ctb_lines() {
        assert_eq!(
            parse_raw_action_line("encounter multizone ruins"),
            ParsedCommand::Encounter {
                name: "ruins".to_string(),
                multizone: true,
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
            parse_raw_action_line("m7 wings_flicker"),
            ParsedCommand::MonsterAction {
                slot: MonsterSlot(7),
                action: "wings_flicker".to_string(),
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
                slot: MonsterSlot(4),
                forced_ctb: Some(-2),
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
        assert_eq!(parse_raw_action_line(""), ParsedCommand::Blank);
    }
}

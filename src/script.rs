use std::collections::HashSet;

const DEFAULT_MACROS_TOML: &str = include_str!("../data/default_macros.toml");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedScript {
    pub lines: Vec<String>,
}

pub fn prepare_action_lines(input: &str) -> PreparedScript {
    let expanded_input = apply_action_macros(input);
    let mut lines: Vec<String> = expanded_input.lines().map(ToOwned::to_owned).collect();
    let mut history = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let raw_line = lines[index].clone();
        if raw_line == "/repeat" || raw_line.starts_with("/repeat ") {
            expand_repeat(&mut lines, index, &history);
            history.push(raw_line);
            index += 1;
            continue;
        }
        history.push(raw_line);
        index += 1;
    }
    PreparedScript { lines }
}

pub fn apply_action_macros(input: &str) -> String {
    let mut text = format!("\n{input}\n");
    for (name, macro_text) in action_macros() {
        text = text.replace(&format!("\n/macro {name}\n"), &format!("\n{macro_text}\n"));
    }
    text[1..text.len() - 1].to_string()
}

pub fn edit_action_line(line: &str) -> String {
    let normalized = normalize_roll_alias_line(line);
    let mut words = normalized.split_whitespace();
    let Some(head) = words.next() else {
        return normalized;
    };
    let lowered = head.to_ascii_lowercase();
    if is_character_name(&lowered) {
        return format!("action {normalized}");
    }
    if is_monster_slot(&lowered) {
        return format!("monsteraction {normalized}");
    }
    if is_monster_name(&lowered) {
        return format!("monsteraction {normalized}");
    }
    if matches!(lowered.as_str(), "weapon" | "armor") {
        return format!("equip {normalized}");
    }
    normalized
}

pub fn normalize_roll_alias_line(line: &str) -> String {
    let words: Vec<&str> = line.split_whitespace().collect();
    if words.len() < 3 {
        return line.to_string();
    }
    let command = words[0].to_ascii_lowercase();
    if !matches!(command.as_str(), "roll" | "advance" | "waste") {
        return line.to_string();
    }

    let tail: Vec<String> = words[1..]
        .iter()
        .map(|word| word.to_ascii_lowercase())
        .collect();
    let (index_token, amount_token, trailing_tokens) =
        if tail.first().is_some_and(|word| word == "rng") {
            if tail.len() < 3 {
                return line.to_string();
            }
            (&tail[1], &tail[2], &tail[3..])
        } else if tail.first().is_some_and(|word| word.starts_with("rng")) {
            if tail.len() < 2 {
                return line.to_string();
            }
            let index_token = tail[0].strip_prefix("rng").unwrap_or("");
            return normalize_split_roll_alias(&command, index_token, &tail[1], &tail[2..])
                .unwrap_or_else(|| line.to_string());
        } else {
            return line.to_string();
        };

    normalize_split_roll_alias(&command, index_token, amount_token, trailing_tokens)
        .unwrap_or_else(|| line.to_string())
}

fn normalize_split_roll_alias(
    command: &str,
    index_token: &str,
    amount_token: &str,
    trailing_tokens: &[String],
) -> Option<String> {
    let amount_token = amount_token.strip_prefix('x').unwrap_or(amount_token);
    let meaningful_trailing: Vec<&String> = trailing_tokens
        .iter()
        .filter(|token| !matches!(token.as_str(), "time" | "times"))
        .collect();
    if !meaningful_trailing.is_empty() {
        return None;
    }
    let index = index_token.parse::<u32>().ok()?;
    let amount = amount_token.parse::<u32>().ok()?;
    Some(format!("{command} {index} {amount}"))
}

fn expand_repeat(lines: &mut Vec<String>, index: usize, history: &[String]) {
    let rest: Vec<&str> = lines[index].split_whitespace().skip(1).collect();
    let times = rest
        .first()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 5000);
    let n_of_lines = rest
        .get(1)
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 5000 / times);
    let count = history.len().min(n_of_lines);
    let mut insertion = Vec::with_capacity(times * count);
    for _ in 0..times {
        insertion.extend(history[history.len() - count..].iter().cloned());
    }
    for (offset, raw) in insertion.into_iter().enumerate() {
        lines.insert(index + 1 + offset, raw);
    }
}

fn is_character_name(value: &str) -> bool {
    matches!(
        value,
        "tidus"
            | "yuna"
            | "auron"
            | "kimahri"
            | "wakka"
            | "lulu"
            | "rikku"
            | "seymour"
            | "valefor"
            | "ifrit"
            | "ixion"
            | "shiva"
            | "bahamut"
            | "anima"
            | "yojimbo"
            | "cindy"
            | "sandy"
            | "mindy"
            | "unknown"
    )
}

fn is_monster_slot(value: &str) -> bool {
    let slots: HashSet<&str> = ["m1", "m2", "m3", "m4", "m5", "m6", "m7", "m8"]
        .into_iter()
        .collect();
    slots.contains(value)
}

fn is_monster_name(value: &str) -> bool {
    crate::data::monster_stats(value).is_some()
}

fn action_macros() -> Vec<(String, String)> {
    let mut macros = Vec::new();
    let mut in_actions = false;
    let mut current_key: Option<String> = None;
    let mut current_value = Vec::new();

    for raw_line in DEFAULT_MACROS_TOML.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            finish_multiline_macro(&mut macros, &mut current_key, &mut current_value);
            in_actions = trimmed == "[Actions]";
            continue;
        }
        if !in_actions || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(key) = current_key.as_ref() {
            if trimmed == "\"\"\"" {
                let macro_text = current_value.join("\n").trim_matches('\n').to_string();
                macros.push((key.clone(), macro_text));
                current_key = None;
                current_value.clear();
            } else {
                current_value.push(line.to_string());
            }
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let key = key.trim().trim_matches('"').to_string();
        let value = value.trim();
        if value == "\"\"\"" {
            current_key = Some(key);
            current_value.clear();
        } else if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
            macros.push((key, value[1..value.len() - 1].to_string()));
        }
    }
    finish_multiline_macro(&mut macros, &mut current_key, &mut current_value);
    macros
}

fn finish_multiline_macro(
    macros: &mut Vec<(String, String)>,
    current_key: &mut Option<String>,
    current_value: &mut Vec<String>,
) {
    if let Some(key) = current_key.take() {
        macros.push((key, current_value.join("\n").trim_matches('\n').to_string()));
        current_value.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_action_macros, edit_action_line, normalize_roll_alias_line, prepare_action_lines,
    };

    #[test]
    fn normalizes_roll_aliases_like_python() {
        assert_eq!(normalize_roll_alias_line("roll rng1 x5 times"), "roll 1 5");
        assert_eq!(normalize_roll_alias_line("advance rng20 3"), "advance 20 3");
        assert_eq!(normalize_roll_alias_line("waste rng 4 2"), "waste 4 2");
        assert_eq!(
            normalize_roll_alias_line("roll rng1 x5 extra"),
            "roll rng1 x5 extra"
        );
    }

    #[test]
    fn edits_action_lines_like_python_for_supported_heads() {
        assert_eq!(
            edit_action_line("tidus attack m1"),
            "action tidus attack m1"
        );
        assert_eq!(edit_action_line("Yuna defend"), "action Yuna defend");
        assert_eq!(
            edit_action_line("m1 attack tidus"),
            "monsteraction m1 attack tidus"
        );
        assert_eq!(edit_action_line("m8 spines"), "monsteraction m8 spines");
        assert_eq!(
            edit_action_line("gandarewa thunder"),
            "monsteraction gandarewa thunder"
        );
        assert_eq!(
            edit_action_line("Piranha attack"),
            "monsteraction Piranha attack"
        );
        assert_eq!(
            edit_action_line("weapon tidus 1 initiative"),
            "equip weapon tidus 1 initiative"
        );
        assert_eq!(edit_action_line("encounter tanker"), "encounter tanker");
        assert_eq!(edit_action_line("#tidus attack"), "#tidus attack");
        assert_eq!(edit_action_line(""), "");
    }

    #[test]
    fn expands_repeat_like_python() {
        let prepared = prepare_action_lines("a\nb\n/repeat 2 2\nc");
        assert_eq!(
            prepared.lines,
            vec![
                "a".to_string(),
                "b".to_string(),
                "/repeat 2 2".to_string(),
                "a".to_string(),
                "b".to_string(),
                "a".to_string(),
                "b".to_string(),
                "c".to_string(),
            ]
        );
    }

    #[test]
    fn expands_action_macros_before_repeat_like_python() {
        assert_eq!(
            apply_action_macros("/macro besaid grid"),
            "stat tidus strength +1"
        );
        let prepared = prepare_action_lines("/macro moonflow grid\n/repeat 1 2");
        assert_eq!(
            prepared.lines,
            vec![
                "stat tidus hp +200".to_string(),
                "stat tidus strength +1".to_string(),
                "stat tidus agility +2".to_string(),
                "/repeat 1 2".to_string(),
                "stat tidus strength +1".to_string(),
                "stat tidus agility +2".to_string(),
            ]
        );
    }
}

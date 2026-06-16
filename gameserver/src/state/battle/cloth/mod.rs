pub mod first_melody;

use sonettobuf::{Fight};

pub fn active_cloth_level(fight: &Fight) -> Option<config::cloth_level::ClothLevel> {
    let cloth_id = fight.attacker.as_ref().and_then(|a| a.cloth_id)?;
    config::configs::get()
        .cloth_level
        .iter()
        .find(|c| c.id == cloth_id && c.level == 1)
        .cloned()
}

pub fn parse_cloth_recover_delta(recover: &str, round_index: i32) -> i32 {
    recover
        .split('|')
        .filter_map(|entry| {
            let mut parts = entry.trim().split('#');
            let start_round = parts.next()?.trim().parse::<i32>().ok()?;
            let amount = parts.next()?.trim().parse::<i32>().ok()?;
            if start_round <= round_index { Some((start_round, amount)) } else { None }
        })
        .max_by_key(|(start_round, _)| *start_round)
        .map(|(_, amount)| amount.max(0))
        .unwrap_or(0)
}

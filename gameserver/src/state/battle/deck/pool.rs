use sonettobuf::{CardInfo, FightEntityInfo};

use crate::state::battle::card::utils::make_card;

fn build_cards(entities: &[FightEntityInfo], copies: usize) -> Vec<CardInfo> {
    let mut cards = Vec::new();
    for e in entities {
        let uid = e.uid.unwrap_or(0);
        let hero_id = e.model_id.unwrap_or(0);
        let is_trial = e.trial_id.map_or(false, |t| t > 0);
        for &skill_id in e.skill_group1.iter().take(1).chain(e.skill_group2.iter().take(1)) {
            if skill_id == 0 { continue; }
            for _ in 0..copies {
                cards.push(make_card(hero_id, skill_id, uid, is_trial));
            }
        }
    }
    cards
}

pub fn build_deck(entities: &[FightEntityInfo]) -> Vec<CardInfo> {
    build_cards(entities, 8)
}

pub fn build_pool(entities: &[FightEntityInfo]) -> Vec<CardInfo> {
    build_cards(entities, 1)
}

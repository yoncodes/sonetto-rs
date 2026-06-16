use rand::{Rng, thread_rng};
use sonettobuf::{CardInfo, Fight, FightEntityInfo};
use std::collections::HashSet;

use super::draw::draw_deck_guaranteed_by_uid_with_rng;
use super::pool::build_pool;
use super::utils::card_limit;
use crate::state::battle::card::apply_card_upgrades;

pub fn generate_initial_hand(entities: &[FightEntityInfo], has_support: bool) -> Vec<CardInfo> {
    let candidate_pool = build_pool(entities);
    let active_uids: Vec<i64> = entities.iter().filter_map(|e| e.uid).collect();
    let opening_hand_size = card_limit(entities.len(), has_support);
    let mut rng = thread_rng();
    draw_deck_guaranteed_by_uid_with_rng(&candidate_pool, &active_uids, opening_hand_size, &mut rng)
}

pub(crate) fn refill_hand(
    rng: &mut impl Rng,
    hand: &mut Vec<CardInfo>,
    deck: &mut Vec<CardInfo>,
    ex_deck: &mut Vec<CardInfo>,
    alive_uids: &HashSet<i64>,
    extra: usize,
    fight: &Fight,
    mut rebuild_deck: impl FnMut() -> Vec<CardInfo>,
) -> (Vec<CardInfo>, usize) {
    let has_support = fight.attacker.as_ref().map_or(false, |a| {
        a.sub_entitys.iter().any(|e| e.uid.unwrap_or(0) > 0)
    });
    let target_size = card_limit(alive_uids.len(), has_support) + extra;
    let mut pulled_raw: Vec<CardInfo> = Vec::new();
    let mut upgrades: usize = 0;
    let non_temp = |h: &Vec<CardInfo>| h.iter().filter(|c| !c.temp_card.unwrap_or(false)).count();
    while non_temp(hand) < target_size && !ex_deck.is_empty() {
        let card = ex_deck.remove(0);
        pulled_raw.push(card.clone());
        hand.push(card);
        upgrades += apply_card_upgrades(hand, fight);
    }
    while non_temp(hand) < target_size && !deck.is_empty() {
        let idx = rng.gen_range(0..deck.len());
        let raw = deck.remove(idx);
        pulled_raw.push(raw.clone());
        hand.push(raw);
        upgrades += apply_card_upgrades(hand, fight);
        if deck.is_empty() {
            *deck = rebuild_deck();
        }
    }
    (pulled_raw, upgrades)
}

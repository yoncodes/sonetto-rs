use rand::Rng;
use rand::rngs::StdRng;
use sonettobuf::{CardInfo, Fight};

use crate::state::battle::{
    card::{apply_card_upgrades, skill_level},
    deck::DeckManager,
    manager::entity_mgr::EntityMgr,
};

pub(crate) fn select_enemy_cards(
    deck_mgr: &mut DeckManager,
    fight: &Fight,
    entity_mgr: &EntityMgr,
    rng: &mut StdRng,
) -> Vec<CardInfo> {
    let mut ai_use_cards: Vec<CardInfo> = Vec::new();
    if let Some(defender) = &fight.defender {
        let entities: Vec<_> = defender.entitys.iter()
            .filter(|e| e.current_hp.unwrap_or(0) > 0)
            .cloned()
            .collect();
        let ex_skill_ids: std::collections::HashSet<i32> = entities.iter()
            .filter_map(|e| e.ex_skill)
            .filter(|&id| id != 0)
            .collect();
        let enemy_ap = entity_mgr.get_ac_point(fight, false).max(0) as usize;
        for _ in 0..enemy_ap {
            if deck_mgr.enemy_hand.is_empty() { break; }
            let best_pos = deck_mgr.enemy_hand.iter().enumerate()
                .max_by_key(|(_, c)| {
                    let sid = c.skill_id.unwrap_or(0);
                    let is_ex = ex_skill_ids.contains(&sid);
                    (is_ex, skill_level(sid, &entities))
                })
                .map(|(i, _)| i)
                .unwrap_or(0);
            let mut card = deck_mgr.enemy_hand.remove(best_pos);
            if let Some(mid) = card.uid {
                if let Some(entity) = entities.iter().find(|e| e.model_id == Some(mid as i32)) {
                    card.uid = entity.uid;
                }
            }
            let is_ex = ex_skill_ids.contains(&card.skill_id.unwrap_or(0));
            tracing::info!(
                "enemy selects card: uid={} skill_id={} is_ex={}",
                card.uid.unwrap_or(0), card.skill_id.unwrap_or(0), is_ex
            );
            ai_use_cards.push(card);
            apply_card_upgrades(&mut deck_mgr.enemy_hand, fight);
        }
    }
    let attacker_uids: Vec<i64> = fight.attacker.as_ref()
        .map(|a| a.entitys.iter().filter(|e| e.current_hp.unwrap_or(0) > 0).filter_map(|e| e.uid).collect())
        .unwrap_or_default();
    if !attacker_uids.is_empty() {
        for card in &mut ai_use_cards {
            card.target_uid = Some(attacker_uids[rng.gen_range(0..attacker_uids.len())]);
        }
    }
    ai_use_cards
}

use rand::Rng;
use sonettobuf::{CardInfo, CardInfoPush, Fight};
use crate::state::battle::card::{is_ex_card, skills_at_same_rank};
use crate::state::battle::card::utils::make_card;
use crate::state::battle::manager::entity_mgr::EntityMgr;
use crate::state::battle::deck::cleanup::{purge_dead_entity_cards, purge_and_maybe_rebuild_deck};
use crate::state::battle::deck::hand::{generate_initial_hand, refill_hand};
use crate::state::battle::deck::pool::build_deck;

#[derive(Default, Debug, Clone)]
pub struct DeckManager {
    pub player_hand: Vec<CardInfo>,
    pub player_deck: Vec<CardInfo>,
    pub player_ex_deck: Vec<CardInfo>,
    pub enemy_hand: Vec<CardInfo>,
    pub enemy_deck: Vec<CardInfo>,
    pub enemy_ex_deck: Vec<CardInfo>,
    pub next_ai_use_cards: Vec<CardInfo>,
}

impl DeckManager {
    pub fn init_player(&mut self, fight: &Fight, max_ap: i32) -> CardInfoPush {
        let attacker = fight.attacker.as_ref();
        let active_entities: Vec<_> = attacker.map(|a| a.entitys.clone()).unwrap_or_default();
        let has_support = attacker.map_or(false, |a| !a.sub_entitys.is_empty());
        let opening_hand = generate_initial_hand(&active_entities, has_support);
        self.player_hand = opening_hand.clone();
        self.player_deck = build_deck(&active_entities);
        self.player_ex_deck = vec![];
        CardInfoPush {
            card_group: opening_hand.clone(),
            deal_card_group: opening_hand,
            act_point: Some(max_ap),
            move_num: Some(0),
            before_cards: vec![],
            extra_move_act: Some(0),
            is_gm: Some(false),
        }
    }

    pub fn init_enemy(&mut self, fight: &Fight) {
        let defender = match &fight.defender {
            Some(d) => d,
            None => return,
        };
        let active_entities: Vec<_> = defender.entitys.iter().cloned().collect();
        self.enemy_hand = generate_initial_hand(&active_entities, false);
        self.enemy_deck = build_deck(&active_entities);
        self.enemy_ex_deck = vec![];
    }

    pub fn purge_player_dead_cards(&mut self, fight: &Fight, entity_mgr: &EntityMgr) {
        let alive_uids = entity_mgr.alive_hero_uids();
        let entities: Vec<_> = fight.attacker.as_ref().map(|a| a.entitys.iter().filter(|e| alive_uids.contains(&e.uid.unwrap_or(0))).cloned().collect()).unwrap_or_default();
        purge_dead_entity_cards(&mut self.player_hand, &alive_uids);
        purge_dead_entity_cards(&mut self.player_ex_deck, &alive_uids);
        purge_and_maybe_rebuild_deck(&mut self.player_deck, &alive_uids, || build_deck(&entities));
    }

    pub fn purge_enemy_dead_cards(&mut self, fight: &Fight, entity_mgr: &EntityMgr) {
        let alive_uids = entity_mgr.alive_enemy_uids();
        let entities: Vec<_> = fight.defender.as_ref().map(|d| d.entitys.iter().filter(|e| alive_uids.contains(&e.uid.unwrap_or(0))).cloned().collect()).unwrap_or_default();
        purge_dead_entity_cards(&mut self.enemy_hand, &alive_uids);
        purge_dead_entity_cards(&mut self.enemy_ex_deck, &alive_uids);
        purge_and_maybe_rebuild_deck(&mut self.enemy_deck, &alive_uids, || build_deck(&entities));
    }

    pub fn refill_player_hand(&mut self, rng: &mut impl Rng, extra: usize, fight: &Fight, entity_mgr: &EntityMgr) -> (Vec<CardInfo>, usize) {
        let alive_uids = entity_mgr.alive_hero_uids();
        let entities: Vec<_> = fight.attacker.as_ref().map(|a| a.entitys.clone()).unwrap_or_default();
        tracing::info!("player refill: hand={} deck={}", self.player_hand.len(), self.player_deck.len());
        let result = refill_hand(rng, &mut self.player_hand, &mut self.player_deck, &mut self.player_ex_deck, &alive_uids, extra, fight, || build_deck(&entities));
        tracing::info!("player refill done: hand={} deck={}", self.player_hand.len(), self.player_deck.len());
        result
    }

    pub fn accumulate_enemy_ex_cards(&mut self, fight: &Fight, entity_mgr: &EntityMgr) {
        crate::state::battle::card::ex_card::accumulate_enemy_ex_cards(self, fight, entity_mgr);
    }

    pub fn refill_enemy_hand(&mut self, rng: &mut impl Rng, fight: &Fight, entity_mgr: &EntityMgr) -> Vec<CardInfo> {
        let alive_uids = entity_mgr.alive_enemy_uids();
        let entities: Vec<_> = fight.defender.as_ref().map(|d| d.entitys.clone()).unwrap_or_default();
        tracing::info!("enemy refill: hand={} deck={}", self.enemy_hand.len(), self.enemy_deck.len());
        let (result, _) = refill_hand(rng, &mut self.enemy_hand, &mut self.enemy_deck, &mut self.enemy_ex_deck, &alive_uids, 0, fight, || build_deck(&entities));
        tracing::info!("enemy refill done: hand={} deck={}", self.enemy_hand.len(), self.enemy_deck.len());
        result
    }

    pub fn clear_universal_card(&mut self) {
        self.player_hand.retain(|c| c.skill_id != Some(30000001));
    }

    pub fn redeal_player_hand_keep_rank(&mut self, fight: &Fight, rng: &mut impl Rng) -> Vec<CardInfo> {
        let entities = fight.attacker.as_ref().map(|a| a.entitys.as_slice()).unwrap_or(&[]);
        for card in &mut self.player_hand {
            if is_ex_card(card, entities) { continue; }
            let skill_id = match card.skill_id { Some(id) => id, None => continue };
            let uid = card.uid.unwrap_or(0);
            let Some(entity) = entities.iter().find(|e| e.uid.unwrap_or(0) == uid) else { continue };
            let options = skills_at_same_rank(entity, skill_id);
            if !options.is_empty() {
                let new_skill = options[rng.gen_range(0..options.len())];
                let hero_id = entity.model_id.unwrap_or(0);
                let is_trial = uid < 0;
                *card = make_card(hero_id, new_skill, uid, is_trial);
            }
        }
        self.player_hand.clone()
    }
}

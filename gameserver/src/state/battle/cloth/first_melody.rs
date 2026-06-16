use crate::state::battle::{
    card::utils::make_card,
    deck::DeckManager,
    fight_step::{ActEffectBuilder, FightStepBuilder},
    types::effects::EffectType as BattleEffectType,
};
use rand::rngs::StdRng;
use sonettobuf::{Fight, FightStep};

pub fn universal_card(skill_id: i32, deck_mgr: &mut DeckManager) -> Vec<FightStep> {
    deck_mgr.player_hand.push(make_card(0, 30000001, 0, true));
    let push_cards = deck_mgr.player_hand.clone();
    let deck_len = deck_mgr.player_deck.len() as i32;

    vec![
        FightStepBuilder::skill(0, -1, skill_id)
            .with(ActEffectBuilder::new(BattleEffectType::UniversalCard as i32, 0)
                .effect_num(30000001).config_effect(60002).team_type(1).build())
            .build(),
        FightStepBuilder::effect()
            .with(ActEffectBuilder::cards_push(push_cards, Some(1)))
            .with(ActEffectBuilder::card_deck_num(deck_len))
            .build(),
    ]
}

pub fn redeal_card(skill_id: i32, fight: &mut Fight, deck_mgr: &mut DeckManager, rng: &mut StdRng) -> Vec<FightStep> {
    let new_cards = deck_mgr.redeal_player_hand_keep_rank(fight, rng);
    let deck_len = deck_mgr.player_deck.len() as i32;

    vec![
        FightStepBuilder::skill(0, -1, skill_id)
            .with(ActEffectBuilder::after_redeal_card(new_cards.clone()))
            .build(),
        FightStepBuilder::effect()
            .with(ActEffectBuilder::cards_push(new_cards, Some(1)))
            .with(ActEffectBuilder::card_deck_num(deck_len))
            .build(),
    ]
}

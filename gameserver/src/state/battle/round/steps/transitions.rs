use crate::state::battle::fight_step::ActEffectBuilder;
use sonettobuf::{ActEffect, CardInfo, FightStep, fight_step};

fn effect_step(effects: Vec<ActEffect>) -> FightStep {
    FightStep {
        act_type: Some(fight_step::ActType::Effect.into()),
        from_id: Some(0),
        to_id: Some(0),
        act_id: Some(0),
        act_effect: effects,
        card_index: Some(0),
        support_hero_id: Some(0),
        fake_timeline: Some(false),
        real_skill_type: Some(0),
        real_skin_id: Some(0),
    }
}

/// Live-style transition block right before enemy cards start:
///   ROUNDEND(61), SMALLROUNDEND(211), DEALCARD2(60), CARDDECKNUM(310)
pub fn build_pre_enemy_transition_steps(deck_num: i32) -> Vec<FightStep> {
    vec![
        effect_step(vec![
            ActEffectBuilder::round_end(Some(0), Some(0)),
            ActEffectBuilder::small_round_end(Some(0), 0),
            ActEffectBuilder::deal_card2(),
        ]),
        effect_step(vec![ActEffectBuilder::card_deck_num_with_target(
            0, deck_num,
        )]),
    ]
}

/// Opening step for the next round begin:
///   DEALCARD1(59), CARDSPUSH(154), CARDDECKNUM(310)
pub fn build_next_round_begin_step(cards: Vec<CardInfo>, deck_num: i32) -> Vec<FightStep> {
    vec![
        effect_step(vec![
            ActEffectBuilder::deal_card1(),
        ]),
        effect_step(vec![
            ActEffectBuilder::cards_push(cards, None),
            ActEffectBuilder::card_deck_num(deck_num),
        ]),
    ]
}

/// Live-style turn close-out block after enemy actions when battle is still ongoing:
///   SMALLROUNDEND(211), CLEARUNIVERSALCARD(96), CHANGEROUND(212), CARDDECKNUM(310)
#[allow(dead_code)]
pub fn build_post_enemy_transition_steps(deck_num: i32) -> Vec<FightStep> {
    vec![
        effect_step(vec![ActEffectBuilder::small_round_end(Some(0), 0)]),
        effect_step(vec![ActEffectBuilder::clear_universal_card(
            Some(0),
            Some(0),
            None,
        )]),
        effect_step(vec![ActEffectBuilder::change_round(Some(0), Some(0))]),
        effect_step(vec![ActEffectBuilder::card_deck_num_with_target(
            0, deck_num,
        )]),
    ]
}

use anyhow::Result;
use rand::{SeedableRng, rngs::StdRng};
use sonettobuf::FightRound;

use crate::state::battle::{
    ai,
    deck::DeckManager,
    context::RoundContext,
    manager::{entity_mgr::{EntityMgr, build_ex_point_info}, round_mgr::check_battle_end},
    fight_step::split_step_by_effect_limit,
};
use crate::state::battle::event::apply::events_to_steps;
use crate::state::battle::round::steps::transitions::build_next_round_begin_step;
use super::round_open::RoundOpenPhaseData;

pub(crate) fn build_round_output(
    round_ctx: &mut RoundContext<'_, '_>,
    mut open: RoundOpenPhaseData,
    deck_mgr: &mut DeckManager,
    rng: &mut StdRng,
) -> Result<FightRound> {
    let ctx = &mut *round_ctx.fight_ctx;
    open.state.is_finish = check_battle_end(ctx.fight);

    crate::state::battle::manager::entity_mgr::sync_to_fight(ctx.fight, &ctx.managers.entity_mgr);
    let uids = EntityMgr::all_positioned_uids(ctx.fight);
    for uid in &uids {
        let evs = ctx.on_round_end(*uid);
        open.steps.extend(events_to_steps(evs));
    }
    for uid in &uids {
        let evs = ctx.on_round_start(*uid);
        open.steps.extend(events_to_steps(evs));
    }
    round_ctx.fight_ctx.on_round_end(0);
    let ctx = &mut *round_ctx.fight_ctx;
    let ex_point_info = build_ex_point_info(ctx.fight, &ctx.managers.entity_mgr);
    tracing::warn!("=== ROUND END ===");

    let skill_infos = ctx.managers.calculate_mgr.build_player_skills();
    let hero_sp_attributes = ctx
        .managers
        .calculate_mgr
        .build_hero_sp_attributes(ctx.fight);
    let power = ctx
        .fight
        .attacker
        .as_ref()
        .and_then(|a| a.power)
        .unwrap_or(0);

    // Purge cards belonging to dead heroes
    deck_mgr.purge_player_dead_cards(ctx.fight, &ctx.managers.entity_mgr);

    let before_cards1 = deck_mgr.player_hand.clone();
    let (team_a_cards1, upgrades1) = deck_mgr.refill_player_hand(rng, 0, ctx.fight, &ctx.managers.entity_mgr);
    for _ in 0..upgrades1 {
        let evt = crate::state::battle::event::Event::CardUpgrade { card: sonettobuf::CardInfo::default() };
        ctx.on_card_upgrade(&evt);
    }

    // Accumulate EX cards for enemies that have reached max EX points
    deck_mgr.accumulate_enemy_ex_cards(ctx.fight, &ctx.managers.entity_mgr);
    // Purge and refill enemy hand after EX accumulation
    deck_mgr.purge_enemy_dead_cards(ctx.fight, &ctx.managers.entity_mgr);
    deck_mgr.refill_enemy_hand(rng, ctx.fight, &ctx.managers.entity_mgr);

    let next_round_begin_step: Vec<sonettobuf::FightStep> = build_next_round_begin_step(deck_mgr.player_hand.clone(), deck_mgr.player_deck.len() as i32);
    open.steps = open
        .steps
        .into_iter()
        .flat_map(split_step_by_effect_limit)
        .collect();

    let attacker_ac = ctx.managers.entity_mgr.get_ac_point(ctx.fight, true);

    let mut rng_for_ai = StdRng::from_entropy();
    deck_mgr.next_ai_use_cards = ai::select_enemy_cards(
        deck_mgr,
        ctx.fight,
        &ctx.managers.entity_mgr,
        &mut rng_for_ai,
    );
    let next_ai_use_cards = deck_mgr.next_ai_use_cards.clone();

    let result = FightRound {
            fight_step: open.steps,
            act_point: Some(if open.state.is_finish { 0 } else { attacker_ac }),
            is_finish: Some(open.state.is_finish),
            move_num: Some(open.state.move_num),
            ex_point_info,
            ai_use_cards: next_ai_use_cards,
            power: Some(power),
            skill_infos,
            before_cards1,
            team_a_cards1,
            before_cards2: open.state.before_cards2,
            team_a_cards2: open.state.team_a_cards2,
            next_round_begin_step,
            use_card_list: vec![],
            cur_round: Some(ctx.fight.cur_round.unwrap_or(1) + 1),
            hero_sp_attributes,
            last_change_hero_uid: Some(0),
        };
    Ok(result)
}